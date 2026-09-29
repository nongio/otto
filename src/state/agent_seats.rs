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
}

impl std::fmt::Display for AgentSeatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidName => write!(f, "an agent name must be 1 to 32 visible characters"),
            Self::NameInUse => write!(f, "another agent holds a seat under this name"),
            Self::NoSeat => write!(f, "ask for a seat first"),
            Self::NoOutput => write!(f, "no output to put a workspace on"),
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
