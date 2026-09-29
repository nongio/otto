//! Agent seats: creating, finding and removing them — see
//! `crate::agent_cursor` for what a seat is, and `specs/agent-seats.md`.
//!
//! There are two kinds. The static seat (`[agent_cursor] enabled = true`) is
//! named `agent`, has no owner, and lasts as long as Otto: it is there for
//! stock tools. Every other seat was asked for over D-Bus by an agent, is
//! named `agent-<n>`, carries the agent's name as a label, and goes when the
//! agent releases it or its bus connection closes.

use std::time::{Duration, Instant};

use smithay::{
    input::{pointer::MotionEvent, Seat},
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::SERIAL_COUNTER,
};
use tracing::info;

use super::{add_configured_keyboard, Backend, Otto};
use crate::{
    agent_cursor::{to_hex, AgentCursor, AgentSeat, AGENT_SEAT_NAME, PALETTE},
    config::Config,
};

/// What Otto remembers of an agent that has had a seat this session.
#[derive(Debug, Clone)]
pub struct PastAgent {
    /// The `n` in its seat's name, `agent-<n>`.
    pub index: u32,
    pub color: [u8; 3],
    /// Its grant, held while it has no seat, for when it comes back.
    pub grant: Option<Grant>,
}

/// Where an agent may act (`specs/agent-seats.md`, Workspace grants).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grant {
    /// A workspace Otto made for the agent, by output and view id.
    OwnWorkspace { output: String, workspace: usize },
}

/// Where an agent's input can land now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// Any client window on any shown workspace: the static seat.
    Everywhere,
    /// The windows of one workspace, whether or not it is on screen.
    Workspace { output: String, workspace: usize },
    /// Nothing: a seat with no grant, or one whose workspace is gone.
    Nowhere,
}

/// An agent's own workspace, as it is told about it: the output it is on and
/// that output's logical geometry and scale, which its coordinates address.
#[derive(Debug, Clone, PartialEq)]
pub struct OwnWorkspace {
    pub output: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f64,
}

/// Why a seat was not given.
#[derive(Debug, PartialEq, Eq)]
pub enum AgentSeatError {
    /// No name, or one that cannot be told apart on screen.
    InvalidName,
    /// Another connection holds a seat under this name.
    NameInUse,
    /// The caller has no seat to grant anything to.
    NoSeat,
    /// There is no output to put a workspace on.
    NoOutput,
    /// The caller has no workspace of its own to launch onto.
    NoWorkspace,
    /// The program could not be started.
    Launch(String),
}

impl std::fmt::Display for AgentSeatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidName => write!(f, "an agent name must be 1 to 32 visible characters"),
            Self::NameInUse => write!(f, "another agent holds a seat under this name"),
            Self::NoSeat => write!(f, "ask for a seat first"),
            Self::NoOutput => write!(f, "no output to put a workspace on"),
            Self::NoWorkspace => write!(f, "ask for a workspace of your own first"),
            Self::Launch(err) => write!(f, "could not start the program: {err}"),
        }
    }
}

/// A seat handed to an agent: its `wl_seat` name and colour, as `#RRGGBB`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantedSeat {
    pub seat: String,
    pub color: String,
}

impl<BackendData: Backend + 'static> Otto<BackendData> {
    /// Advertise the static agent seat, if it is not already.
    pub fn enable_agent_seat(&mut self) {
        if self.agent_seat(AGENT_SEAT_NAME).is_some() {
            return;
        }
        let color = Config::with(|c| c.agent_cursor.color.clone());
        let cursor = AgentCursor::new(&color, agent_hide_after());
        self.add_agent_seat(AGENT_SEAT_NAME, cursor, None, None);
    }

    /// Give the agent called `agent_name`, on the bus connection `owner`, a
    /// seat of its own.
    ///
    /// An agent that already had one this session gets the same seat name and
    /// colour back. Asking again from the connection that holds it returns
    /// the seat it has.
    pub fn request_agent_seat(
        &mut self,
        agent_name: &str,
        owner: &str,
    ) -> Result<GrantedSeat, AgentSeatError> {
        let agent_name = agent_name.trim();
        if agent_name.is_empty()
            || agent_name.chars().count() > 32
            || agent_name.chars().any(char::is_control)
        {
            return Err(AgentSeatError::InvalidName);
        }
        if let Some(held) = self
            .agent_seats
            .iter()
            .find(|agent| agent.agent_name.as_deref() == Some(agent_name))
        {
            if held.owner.as_deref() != Some(owner) {
                return Err(AgentSeatError::NameInUse);
            }
            return Ok(GrantedSeat {
                seat: held.name(),
                color: to_hex(held.cursor.color()),
            });
        }

        let past = match self.agent_history.get_mut(agent_name) {
            // The held grant moves back onto the seat; history no longer
            // holds it, so one the agent then releases stays released.
            Some(past) => {
                let restored = past.clone();
                past.grant = None;
                restored
            }
            None => {
                let index = self.agent_history.len() as u32 + 1;
                let past = PastAgent {
                    index,
                    color: PALETTE[(index as usize - 1) % PALETTE.len()],
                    grant: None,
                };
                self.agent_history
                    .insert(agent_name.to_string(), past.clone());
                past
            }
        };
        let seat_name = format!("agent-{}", past.index);
        let cursor =
            AgentCursor::with_rgb(past.color, Some(agent_name.to_string()), agent_hide_after());
        self.add_agent_seat(
            &seat_name,
            cursor,
            Some(agent_name.to_string()),
            Some(owner.to_string()),
        );
        // Back under the same name: its grant, held since it left, is its
        // again, without asking.
        if let Some(agent) = self.agent_seat_mut(&seat_name) {
            agent.grant = past.grant;
        }
        info!(
            agent = agent_name,
            seat = seat_name,
            owner,
            "Agent seat created"
        );
        Ok(GrantedSeat {
            seat: seat_name,
            color: to_hex(past.color),
        })
    }

    /// Remove every seat `owner` holds: it released them, or left the bus.
    /// Returns whether it held any.
    pub fn release_agent_seats(&mut self, owner: &str) -> bool {
        let names: Vec<String> = self
            .agent_seats
            .iter()
            .filter(|agent| agent.owner.as_deref() == Some(owner))
            .map(AgentSeat::name)
            .collect();
        for name in &names {
            self.remove_agent_seat(name);
        }
        !names.is_empty()
    }

    fn add_agent_seat(
        &mut self,
        seat_name: &str,
        cursor: AgentCursor,
        agent_name: Option<String>,
        owner: Option<String>,
    ) {
        let mut seat = self.seat_state.new_wl_seat(&self.display_handle, seat_name);
        let pointer = seat.add_pointer();
        add_configured_keyboard(&mut seat);
        self.agent_seats.push(AgentSeat {
            seat,
            pointer,
            cursor,
            agent_name,
            owner,
            idle_timer: None,
            grant: None,
        });
    }

    /// Take a seat away: it lets go of what it was pointing at and typing
    /// into, its cursor goes, and clients see its `wl_seat` removed.
    ///
    /// Smithay keeps a handle to every seat it made for the rest of the
    /// session, so each one costs a little memory even once removed.
    fn remove_agent_seat(&mut self, seat_name: &str) {
        let Some(index) = self
            .agent_seats
            .iter()
            .position(|agent| agent.name() == seat_name)
        else {
            return;
        };
        self.release_focus_of(seat_name);
        let agent = self.agent_seats.remove(index);
        // The grant waits for the agent to come back.
        if let Some(past) = agent
            .agent_name
            .as_ref()
            .and_then(|name| self.agent_history.get_mut(name))
        {
            past.grant = agent.grant.clone();
        }
        if let Some(token) = agent.idle_timer {
            self.handle.remove(token);
        }
        if let Some(global) = agent.seat.global() {
            self.display_handle.remove_global::<Self>(global);
        }
        info!(seat = seat_name, "Agent seat removed");
        self.backend_data.request_redraw();
    }

    /// Give `owner`'s agent a workspace of its own: a new one on the primary
    /// output, named after the agent and not switched to. Asking again
    /// returns the one it has.
    pub fn request_own_workspace(&mut self, owner: &str) -> Result<OwnWorkspace, AgentSeatError> {
        let seat_name = self
            .agent_seats
            .iter()
            .find(|agent| agent.owner.as_deref() == Some(owner))
            .map(AgentSeat::name)
            .ok_or(AgentSeatError::NoSeat)?;
        let agent = self.agent_seat(&seat_name).expect("found above");
        if let Reach::Workspace { output, .. } = agent.reach(&self.workspaces) {
            return self.describe_workspace(&output);
        }
        let agent_name = agent.agent_name.clone().unwrap_or_default();

        let output = self
            .workspaces
            .primary_output()
            .or_else(|| self.workspaces.outputs().next())
            .map(|output| output.name())
            .ok_or(AgentSeatError::NoOutput)?;
        let (_, view) = self
            .workspaces
            .add_workspace_to_output(&output)
            .ok_or(AgentSeatError::NoOutput)?;
        self.workspaces
            .name_workspace_for_session(&output, view.index, &agent_name);
        info!(
            agent = agent_name,
            output,
            workspace = view.index,
            "Agent workspace created"
        );
        if let Some(agent) = self.agent_seat_mut(&seat_name) {
            agent.grant = Some(Grant::OwnWorkspace {
                output: output.clone(),
                workspace: view.index,
            });
        }
        // Its pointer starts on its workspace, not wherever it was.
        self.release_focus_of(&seat_name);
        self.backend_data.request_redraw();
        self.describe_workspace(&output)
    }

    /// End `owner`'s own-workspace grant. The workspace stays, with its
    /// windows and its name: it is the user's now. Returns whether there
    /// was a grant to end.
    pub fn release_own_workspace(&mut self, owner: &str) -> bool {
        let Some(seat_name) = self
            .agent_seats
            .iter()
            .find(|agent| agent.owner.as_deref() == Some(owner) && agent.grant.is_some())
            .map(AgentSeat::name)
        else {
            return false;
        };
        self.release_focus_of(&seat_name);
        if let Some(agent) = self.agent_seat_mut(&seat_name) {
            agent.grant = None;
        }
        self.backend_data.request_redraw();
        true
    }

    fn describe_workspace(&self, output_name: &str) -> Result<OwnWorkspace, AgentSeatError> {
        let output = self
            .workspaces
            .outputs()
            .find(|output| output.name() == output_name)
            .ok_or(AgentSeatError::NoOutput)?;
        let geometry = self
            .workspaces
            .output_geometry(output)
            .ok_or(AgentSeatError::NoOutput)?;
        Ok(OwnWorkspace {
            output: output_name.to_string(),
            x: geometry.loc.x,
            y: geometry.loc.y,
            width: geometry.size.w,
            height: geometry.size.h,
            scale: output.current_scale().fractional_scale(),
        })
    }

    /// The output an agent's absolute coordinates map onto, when its virtual
    /// pointer was not bound to one: its workspace's.
    pub fn agent_output(&self, seat_name: &str) -> Option<smithay::output::Output> {
        let Reach::Workspace { output, .. } = self.agent_seat(seat_name)?.reach(&self.workspaces)
        else {
            return None;
        };
        self.workspaces
            .outputs()
            .find(|candidate| candidate.name() == output)
            .cloned()
    }

    /// Whether the agent on `seat_name` may type into `target`: it is on a
    /// workspace the agent was granted.
    pub fn agent_may_type_into(
        &self,
        seat_name: &str,
        target: &crate::focus::KeyboardFocusTarget<BackendData>,
    ) -> bool {
        use crate::focus::KeyboardFocusTarget;
        use smithay::reexports::wayland_server::Resource;

        let Some(agent) = self.agent_seat(seat_name) else {
            return false;
        };
        let (output, workspace) = match agent.reach(&self.workspaces) {
            Reach::Everywhere => return true,
            Reach::Nowhere => return false,
            Reach::Workspace { output, workspace } => (output, workspace),
        };
        let Some(space) = self.workspaces.space_of_view(&output, workspace) else {
            return false;
        };
        let window = match target {
            KeyboardFocusTarget::Window(window) => Some(window.clone()),
            KeyboardFocusTarget::Popup(popup) => smithay::desktop::find_popup_root_surface(popup)
                .ok()
                .and_then(|root| self.workspaces.get_window_for_surface(&root.id()).cloned()),
            _ => None,
        };
        window.is_some_and(|window| space.elements().any(|candidate| *candidate == window))
    }

    /// Bring the agent borders in line with the grants: one frame per
    /// granted workspace, in the colour of its agent, with a chip unless a
    /// window is fullscreen there. Cheap when nothing changed.
    pub fn sync_agent_frames(&mut self) {
        use crate::workspaces::agent_frame::AgentFrameLook;

        let mut wanted: Vec<(String, usize, AgentFrameLook)> = Vec::new();
        let live = self.agent_seats.iter().filter_map(|agent| {
            Some((
                agent.grant.as_ref()?,
                agent.cursor.color(),
                agent.agent_name.clone(),
            ))
        });
        // An agent that has gone keeps its frame while its grant waits for it.
        let held = self.agent_history.iter().filter_map(|(name, past)| {
            Some((past.grant.as_ref()?, past.color, Some(name.clone())))
        });
        for (Grant::OwnWorkspace { output, workspace }, color, name) in live.chain(held) {
            let Some(space) = self.workspaces.space_of_view(output, *workspace) else {
                continue;
            };
            let fullscreen = space.elements().any(|window| window.is_fullscreen());
            wanted.push((
                output.clone(),
                *workspace,
                AgentFrameLook {
                    color,
                    names: name.into_iter().collect(),
                    chip: !fullscreen,
                    // An agent's own workspace does not dim until it lets go.
                    strength: 1.0,
                },
            ));
        }
        self.workspaces.set_agent_frames(wanted);
    }

    /// The user pressed at `location`: if that is the Stop on an agent
    /// chip, every grant on that workspace ends. Returns whether it was.
    pub fn press_agent_stop(
        &mut self,
        location: smithay::utils::Point<f64, smithay::utils::Logical>,
    ) -> bool {
        let Some(output) = self
            .workspaces
            .outputs()
            .find(|output| {
                self.workspaces
                    .output_geometry(output)
                    .is_some_and(|geometry| geometry.to_f64().contains(location))
            })
            .cloned()
        else {
            return false;
        };
        let output_name = output.name();
        let Some(view) = self.workspaces.current_view_index(&output_name) else {
            return false;
        };
        let Some(look) = self.workspaces.agent_frame_look(&output_name, view) else {
            return false;
        };
        if !look.chip {
            return false;
        }
        let Some(geometry) = self.workspaces.output_geometry(&output) else {
            return false;
        };
        let chip =
            crate::workspaces::agent_frame::chip_geometry(&look.names, geometry.size.w as f32);
        let local = location - geometry.loc.to_f64();
        let (x, y, w, h) = chip.stop;
        let (px, py) = (local.x as f32, local.y as f32);
        if px < x || px >= x + w || py < y || py >= y + h {
            return false;
        }
        self.revoke_workspace_grants(&output_name, view);
        true
    }

    /// End every grant on the workspace `view` of `output`: the user said
    /// stop. The workspace stays, with its windows.
    pub fn revoke_workspace_grants(&mut self, output: &str, view: usize) {
        let grant = Grant::OwnWorkspace {
            output: output.to_string(),
            workspace: view,
        };
        let seats: Vec<String> = self
            .agent_seats
            .iter()
            .filter(|agent| agent.grant.as_ref() == Some(&grant))
            .map(AgentSeat::name)
            .collect();
        for seat_name in seats {
            info!(
                seat = seat_name,
                output,
                workspace = view,
                "Agent grant revoked by the user"
            );
            self.release_focus_of(&seat_name);
            if let Some(agent) = self.agent_seat_mut(&seat_name) {
                agent.grant = None;
            }
        }
        for past in self.agent_history.values_mut() {
            if past.grant.as_ref() == Some(&grant) {
                past.grant = None;
            }
        }
        self.backend_data.request_redraw();
    }

    /// Start `argv` for `owner`'s agent, so that its windows open on the
    /// agent's own workspace. Returns the process id.
    ///
    /// The program gets an activation token Otto made for the agent. A new
    /// process is recognised by the token in the environment it started
    /// with; an application already running, that opens its window from an
    /// existing process, hands the token back through xdg-activation.
    pub fn launch_on_own_workspace(
        &mut self,
        owner: &str,
        argv: &[String],
    ) -> Result<u32, AgentSeatError> {
        let agent = self
            .agent_seats
            .iter()
            .find(|agent| agent.owner.as_deref() == Some(owner))
            .ok_or(AgentSeatError::NoSeat)?;
        if !matches!(agent.reach(&self.workspaces), Reach::Workspace { .. }) {
            return Err(AgentSeatError::NoWorkspace);
        }
        let agent_name = agent.agent_name.clone().unwrap_or_default();
        let (program, args) = argv
            .split_first()
            .ok_or_else(|| AgentSeatError::Launch("no program".into()))?;

        let (token, _) = self.xdg_activation_state.create_external_token(None);
        let token = token.as_str().to_string();
        let mut command = self.program_command(program, args);
        command
            .env("XDG_ACTIVATION_TOKEN", &token)
            .env("DESKTOP_STARTUP_ID", &token);
        let child = command
            .spawn()
            .map_err(|err| AgentSeatError::Launch(err.to_string()))?;
        let pid = child.id();
        crate::input::actions::reap_in_background(program, child);
        info!(agent = agent_name, program, pid, "Launched for an agent");
        self.agent_launch_tokens.insert(token, agent_name);
        Ok(pid)
    }

    /// The agent seat whose launch made the client of `surface`, found by the
    /// activation token in the environment its process started with.
    pub fn agent_seat_for_client(
        &self,
        surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) -> Option<String> {
        use smithay::reexports::wayland_server::Resource;

        if self.agent_launch_tokens.is_empty() {
            return None;
        }
        let client = surface.client()?;
        let pid = client.get_credentials(&self.display_handle).ok()?.pid;
        // What the process was started with: a toolkit unsetting the token
        // after reading it does not change this.
        let environ = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
        let token = environ.split(|byte| *byte == 0).find_map(|entry| {
            let entry = std::str::from_utf8(entry).ok()?;
            entry
                .strip_prefix("XDG_ACTIVATION_TOKEN=")
                .or_else(|| entry.strip_prefix("DESKTOP_STARTUP_ID="))
                .filter(|token| self.agent_launch_tokens.contains_key(*token))
                .map(str::to_string)
        })?;
        self.agent_seat_for_token(&token)
    }

    /// The agent seat an activation token was made for, while the agent
    /// still holds a workspace of its own.
    pub fn agent_seat_for_token(&self, token: &str) -> Option<String> {
        let agent_name = self.agent_launch_tokens.get(token)?;
        let agent = self
            .agent_seats
            .iter()
            .find(|agent| agent.agent_name.as_deref() == Some(agent_name.as_str()))?;
        matches!(agent.reach(&self.workspaces), Reach::Workspace { .. }).then(|| agent.name())
    }

    /// Put `window` on the workspace of the agent on `seat_name`, without
    /// raising it or taking the user's focus, and give it the agent's
    /// keyboard: an agent types into what it launched.
    pub fn place_on_agent_workspace(
        &mut self,
        seat_name: &str,
        window: &crate::shell::WindowElement,
    ) -> bool {
        let Some(agent) = self.agent_seat(seat_name) else {
            return false;
        };
        let Reach::Workspace { output, workspace } = agent.reach(&self.workspaces) else {
            return false;
        };
        let seat = agent.seat.clone();
        let Some(output) = self
            .workspaces
            .outputs()
            .find(|candidate| candidate.name() == output)
            .cloned()
        else {
            return false;
        };
        let Some(position) = self
            .workspaces
            .output_workspaces
            .get(&output.name())
            .and_then(|ows| {
                ows.workspace_views
                    .iter()
                    .position(|view| view.index == workspace)
            })
        else {
            return false;
        };
        let location = self.workspaces.element_location(window).unwrap_or_default();
        self.workspaces
            .move_window_to_workspace_on_output_with_activate(
                &output, window, position, location, false,
            );
        if let Some(keyboard) = seat.get_keyboard() {
            keyboard.set_focus(
                self,
                Some(window.clone().into()),
                SERIAL_COUNTER.next_serial(),
            );
        }
        true
    }

    /// The agent seat named `seat_name`.
    pub fn agent_seat(&self, seat_name: &str) -> Option<&AgentSeat<BackendData>> {
        self.agent_seats
            .iter()
            .find(|agent| agent.seat.name() == seat_name)
    }

    pub fn agent_seat_mut(&mut self, seat_name: &str) -> Option<&mut AgentSeat<BackendData>> {
        self.agent_seats
            .iter_mut()
            .find(|agent| agent.seat.name() == seat_name)
    }

    /// Whether `seat` belongs to an agent.
    pub fn is_agent_seat(&self, seat: &Seat<Otto<BackendData>>) -> bool {
        self.agent_seats.iter().any(|agent| &agent.seat == seat)
    }

    /// The agent on `seat_name` did something: its cursor is shown again, and
    /// the timer that will start hiding it starts over.
    pub fn note_agent_activity(&mut self, seat_name: &str) {
        let handle = self.handle.clone();
        let Some(agent) = self.agent_seat_mut(seat_name) else {
            return;
        };
        agent.cursor.note_activity(Instant::now());
        if let Some(token) = agent.idle_timer.take() {
            handle.remove(token);
        }
        let Some(hide_after) = agent.cursor.hide_after() else {
            return;
        };
        // Only the first frame of the fade needs waking for: the renderer
        // keeps drawing while the cursor is part-way faded.
        let seat_name = seat_name.to_string();
        agent.idle_timer = handle
            .insert_source(Timer::from_duration(hide_after), move |_, _, state| {
                if let Some(agent) = state.agent_seat_mut(&seat_name) {
                    agent.idle_timer = None;
                }
                state.backend_data.request_redraw();
                TimeoutAction::Drop
            })
            .ok();
    }

    /// The session locked: every agent lets go of what it was pointing at or
    /// typing into, and gets none of it back on unlock.
    pub fn release_agent_focus(&mut self) {
        let names: Vec<String> = self.agent_seats.iter().map(AgentSeat::name).collect();
        for name in names {
            self.release_focus_of(&name);
        }
    }

    fn release_focus_of(&mut self, seat_name: &str) {
        let Some(agent) = self.agent_seat(seat_name) else {
            return;
        };
        let (seat, pointer) = (agent.seat.clone(), agent.pointer.clone());
        let serial = SERIAL_COUNTER.next_serial();
        if let Some(keyboard) = seat.get_keyboard() {
            keyboard.set_focus(self, None, serial);
        }
        if pointer.is_grabbed() {
            pointer.unset_grab(
                self,
                serial,
                smithay::backend::input::InputTime::from_millis(0),
            );
        }
        let location = pointer.current_location();
        pointer.motion(
            self,
            None,
            &MotionEvent {
                location,
                serial,
                time: smithay::backend::input::InputTime::from_millis(0),
            },
        );
        pointer.frame(self);
    }
}

fn agent_hide_after() -> Duration {
    Duration::from_millis(Config::with(|c| c.agent_cursor.hide_after_ms))
}

/// Every window on a workspace some agent holds a grant on, whether its seat
/// is connected or waiting for it to come back. Their applications are
/// watched, so they draw at full rate even while the workspace is hidden.
#[allow(clippy::mutable_key_type)] // ObjectId as key — see window_throttle.rs
pub fn agent_workspace_window_ids<B: Backend + 'static>(
    agent_seats: &[AgentSeat<B>],
    agent_history: &std::collections::HashMap<String, PastAgent>,
    workspaces: &crate::workspaces::Workspaces,
) -> std::collections::HashSet<smithay::reexports::wayland_server::backend::ObjectId> {
    agent_seats
        .iter()
        .filter_map(|agent| agent.grant.as_ref())
        .chain(
            agent_history
                .values()
                .filter_map(|past| past.grant.as_ref()),
        )
        .filter_map(|Grant::OwnWorkspace { output, workspace }| {
            workspaces.space_of_view(output, *workspace)
        })
        .flat_map(|space| space.elements().map(|window| window.id()))
        .collect()
}
