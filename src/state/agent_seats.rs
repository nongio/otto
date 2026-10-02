//! Agent seats: creating, finding and removing them — see
//! `crate::agent_cursor` for what a seat is, and `specs/agent-seats.md`.
//!
//! Every seat was asked for over D-Bus by an agent: it is named `agent-<n>`,
//! carries the agent's name as a label, and goes when the agent releases it
//! or its bus connection closes.

use std::time::{Duration, Instant};

use smithay::{
    input::{pointer::MotionEvent, Seat},
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::SERIAL_COUNTER,
};
use tracing::info;


use super::{add_configured_keyboard, Backend, ClientState, Otto};
use crate::{
    agent_cursor::{to_hex, AgentCursor, AgentSeat, PALETTE},
    config::Config,
};

/// What Otto remembers of an agent that has had a seat this session.
#[derive(Debug, Clone)]
pub struct PastAgent {
    /// The `n` in its seat's name, `agent-<n>`.
    pub index: u32,
    pub color: [u8; 3],
}

/// Where an agent may act (`specs/security-model.md`, Agent sessions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grant {
    /// One workspace, by output and view id: one Otto made for the agent,
    /// or, `shared`, one of the user's they let it work on.
    Workspace {
        output: String,
        workspace: usize,
        shared: bool,
    },
}

impl Grant {
    /// The workspace granted, as output and view id.
    pub fn workspace(&self) -> (&str, usize) {
        match self {
            Self::Workspace {
                output, workspace, ..
            } => (output, *workspace),
        }
    }
}

/// A workspace as an agent is told of it: where it is, its name, and
/// whether its output shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceEntry {
    pub output: String,
    pub name: String,
    pub shown: bool,
}

/// A workspace an agent asked for, found and waiting for the user's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRequest {
    pub output: String,
    pub workspace: usize,
    pub workspace_name: String,
    pub agent_name: String,
}

/// Where an agent's input can land now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
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
    /// No workspace goes by the name asked for.
    NoSuchWorkspace,
    /// The program could not be started.
    Launch(String),
    /// No Wayland connection could be made for the agent.
    Connect(String),
}

impl std::fmt::Display for AgentSeatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidName => write!(f, "an agent name must be 1 to 32 visible characters"),
            Self::NameInUse => write!(f, "another agent holds a seat under this name"),
            Self::NoSeat => write!(f, "ask for a seat first"),
            Self::NoOutput => write!(f, "no output to put a workspace on"),
            Self::NoWorkspace => write!(f, "ask for a workspace of your own first"),
            Self::NoSuchWorkspace => write!(f, "no workspace goes by that name"),
            Self::Launch(err) => write!(f, "could not start the program: {err}"),
            Self::Connect(err) => write!(f, "could not connect: {err}"),
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

        let past = match self.agent_history.get(agent_name) {
            Some(past) => past.clone(),
            None => {
                let index = self.agent_history.len() as u32 + 1;
                let past = PastAgent {
                    index,
                    color: PALETTE[(index as usize - 1) % PALETTE.len()],
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
        // Advertised only on the agent's own connections: every other client
        // keeps seeing the user's seat alone, and toolkits that take every
        // seat they see never bind this one.
        let own = seat_name.to_string();
        let mut seat = self.seat_state.new_wl_seat_with_filter(
            &self.display_handle,
            seat_name,
            move |client| ClientState::agent_seat_of(client) == Some(own.as_str()),
        );
        let pointer = seat.add_pointer();
        add_configured_keyboard(&mut seat);
        // The user's seat too, so the user can work in the agent's windows
        // and they stay usable when the agent leaves. Made after the agent's
        // seat, it is announced after it.
        static NEXT_SESSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let session = NEXT_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let user_global = self
            .seat
            .create_global_with_filter(&self.display_handle, move |client| {
                client
                    .get_data::<ClientState>()
                    .is_some_and(|state| state.agent_session == session)
            });
        self.agent_seats.push(AgentSeat {
            seat,
            pointer,
            cursor,
            agent_name,
            owner,
            idle_timer: None,
            grant: None,
            connections: Vec::new(),
            listeners: Vec::new(),
            lent_global: None,
            session,
            user_global,
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
        // Its grant ends with it: the workspace and its windows are the
        // user's, unframed, and an agent back later asks again.
        let agent = self.agent_seats.remove(index);
        if let Some(grant) = &agent.grant {
            self.end_grant(grant);
        }
        if let Some(token) = agent.idle_timer {
            self.handle.remove(token);
        }
        if let Some(global) = agent.seat.global() {
            self.display_handle.remove_global::<Self>(global);
        }
        if let Some(global) = agent.lent_global {
            self.display_handle.remove_global::<Self>(global);
        }
        for token in agent.listeners {
            self.handle.remove(token);
        }
        // Its clients stay, for the user: they already see the user's seat,
        // and are no longer the agent's, so an agent back under the same
        // seat name does not get them back.
        for client in &agent.connections {
            if let Some(state) = client.get_data::<ClientState>() {
                state
                    .handed_over
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        self.handed_over.push((agent.user_global, agent.connections));
        info!(seat = seat_name, "Agent seat removed");
        self.backend_data.request_redraw();
    }

    /// Connect a Wayland client for `owner`'s agent, returning its end of the
    /// socket.
    ///
    /// The client may create virtual input on the agent's seat and no other,
    /// and is kept off the globals a sandboxed client is kept off (see
    /// [`crate::sandbox`]). It sees the user's seat after the agent's, and
    /// stays for the user when the seat goes. Every
    /// other connection is refused the agent's seat, so this is the only way
    /// to drive it: a `wp_security_context_v1` listener made on such a
    /// connection connects more of the agent's clients
    /// (`crate::state::security_context_handler`).
    pub fn connect_agent_client(
        &mut self,
        owner: &str,
    ) -> Result<std::os::unix::net::UnixStream, AgentSeatError> {
        let seat_name = self.seat_of_owner(owner)?;
        let (ours, theirs) = std::os::unix::net::UnixStream::pair()
            .map_err(|err| AgentSeatError::Connect(err.to_string()))?;
        self.insert_agent_client(&seat_name, ours)?;
        Ok(theirs)
    }

    /// The seat `owner` holds.
    fn seat_of_owner(&self, owner: &str) -> Result<String, AgentSeatError> {
        self.agent_seats
            .iter()
            .find(|agent| agent.owner.as_deref() == Some(owner))
            .map(AgentSeat::name)
            .ok_or(AgentSeatError::NoSeat)
    }

    /// Insert `stream` as a client of the agent seat `seat_name`.
    pub(crate) fn insert_agent_client(
        &mut self,
        seat_name: &str,
        stream: std::os::unix::net::UnixStream,
    ) -> Result<(), AgentSeatError> {
        let session = self
            .agent_seat(seat_name)
            .map(|agent| agent.session)
            .ok_or(AgentSeatError::NoSeat)?;
        let client = self
            .display_handle
            .insert_client(
                stream,
                std::sync::Arc::new(ClientState {
                    agent_seat: Some(seat_name.to_string()),
                    agent_session: session,
                    ..ClientState::default()
                }),
            )
            .map_err(|err| AgentSeatError::Connect(err.to_string()))?;
        let backend = self.display_handle.backend_handle();
        if let Some(agent) = self
            .agent_seats
            .iter_mut()
            .find(|agent| agent.name() == seat_name)
        {
            agent
                .connections
                .retain(|client| backend.get_client_data(client.id()).is_ok());
            agent.connections.push(client);
        }
        info!(seat = seat_name, "Agent connection made");
        Ok(())
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
            .name_workspace_for_session(&output, view.index, Some(&agent_name));
        info!(
            agent = agent_name,
            output,
            workspace = view.index,
            "Agent workspace created"
        );
        if let Some(agent) = self.agent_seat_mut(&seat_name) {
            agent.grant = Some(Grant::Workspace {
                output: output.clone(),
                workspace: view.index,
                shared: false,
            });
        }
        // Its pointer starts on its workspace, not wherever it was.
        self.release_focus_of(&seat_name);
        self.backend_data.request_redraw();
        self.describe_workspace(&output)
    }

    /// The workspaces there are, for `owner`'s agent to pick one from.
    pub fn agent_workspace_list(&self, owner: &str) -> Result<Vec<WorkspaceEntry>, AgentSeatError> {
        if !self
            .agent_seats
            .iter()
            .any(|agent| agent.owner.as_deref() == Some(owner))
        {
            return Err(AgentSeatError::NoSeat);
        }
        let mut entries = Vec::new();
        for output in self.workspaces.outputs() {
            let name = output.name();
            let Some(ows) = self.workspaces.output_workspaces.get(&name) else {
                continue;
            };
            for (position, view) in ows.workspace_views.iter().enumerate() {
                entries.push(WorkspaceEntry {
                    output: name.clone(),
                    name: view.display_name(),
                    shown: position == ows.current_workspace,
                });
            }
        }
        Ok(entries)
    }

    /// Find the workspace `owner`'s agent asks for: by name, or, `""`, the
    /// one the user is looking at. Nothing is granted until the user says.
    pub fn find_agent_workspace(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<WorkspaceRequest, AgentSeatError> {
        let agent = self
            .agent_seats
            .iter()
            .find(|agent| agent.owner.as_deref() == Some(owner))
            .ok_or(AgentSeatError::NoSeat)?;
        let found = if name.is_empty() {
            self.workspaces
                .focused_output()
                .or_else(|| self.workspaces.primary_output())
                .and_then(|output| {
                    let output = output.name();
                    let ows = self.workspaces.output_workspaces.get(&output)?;
                    let view = ows.workspace_views.get(ows.current_workspace)?;
                    Some((output, view.index, view.display_name()))
                })
        } else {
            self.workspaces.outputs().find_map(|output| {
                let output = output.name();
                let ows = self.workspaces.output_workspaces.get(&output)?;
                let view = ows
                    .workspace_views
                    .iter()
                    .find(|view| view.display_name() == name)?;
                Some((output, view.index, view.display_name()))
            })
        };
        let (output, workspace, workspace_name) = found.ok_or(AgentSeatError::NoSuchWorkspace)?;
        Ok(WorkspaceRequest {
            output,
            workspace,
            workspace_name,
            agent_name: agent.agent_name.clone().unwrap_or_default(),
        })
    }

    /// Let `owner`'s agent work on the workspace `workspace` of `output`,
    /// once the user said it may: its seat acts there in place of anywhere
    /// it acted before. The user is not moved, and keeps their focus.
    pub fn grant_agent_workspace(
        &mut self,
        owner: &str,
        output: &str,
        workspace: usize,
    ) -> Result<OwnWorkspace, AgentSeatError> {
        let seat_name = self
            .agent_seats
            .iter()
            .find(|agent| agent.owner.as_deref() == Some(owner))
            .map(AgentSeat::name)
            .ok_or(AgentSeatError::NoSeat)?;
        if self.workspaces.space_of_view(output, workspace).is_none() {
            return Err(AgentSeatError::NoSuchWorkspace);
        }
        self.release_focus_of(&seat_name);
        if let Some(agent) = self.agent_seat_mut(&seat_name) {
            agent.grant = Some(Grant::Workspace {
                output: output.to_string(),
                workspace,
                shared: true,
            });
        }
        info!(
            seat = seat_name,
            output, workspace, "Agent given a workspace of the user's"
        );
        self.backend_data.request_redraw();
        self.describe_workspace(output)
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
        let grant = self
            .agent_seat_mut(&seat_name)
            .and_then(|agent| agent.grant.take());
        if let Some(grant) = grant {
            self.end_grant(&grant);
        }
        self.backend_data.request_redraw();
        true
    }

    /// A grant has ended: a workspace Otto made for the agent loses the
    /// agent's name and is the user's like any other; one the user lent
    /// never had it.
    fn end_grant(&self, grant: &Grant) {
        if let Grant::Workspace {
            output,
            workspace,
            shared: false,
        } = grant
        {
            self.workspaces
                .name_workspace_for_session(output, *workspace, None);
        }
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

    /// Drop the user's-seat global of an agent that left once the last of
    /// the clients it handed over has closed.
    fn prune_handed_over(&mut self) {
        let backend = self.display_handle.backend_handle();
        let (live, done): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.handed_over)
                .into_iter()
                .partition(|(_, clients)| {
                    clients
                        .iter()
                        .any(|client| backend.get_client_data(client.id()).is_ok())
                });
        self.handed_over = live;
        for (global, _) in done {
            self.display_handle.remove_global::<Self>(global);
        }
    }

    /// Show an agent's seat to the user's programs while it holds a workspace
    /// the user lent it, and only then: a client cannot be told of the seat's
    /// own global later, so it gets one more for the loan
    /// (`specs/security-model.md`, Agent sessions).
    fn sync_lent_seat_globals(&mut self) {
        for index in 0..self.agent_seats.len() {
            let agent = &self.agent_seats[index];
            let lent = matches!(agent.grant, Some(Grant::Workspace { shared: true, .. }));
            match (lent, agent.lent_global.is_some()) {
                (true, false) => {
                    let global =
                        agent
                            .seat
                            .create_global_with_filter(&self.display_handle, |client| {
                                ClientState::agent_seat_of(client).is_none()
                                    && !crate::sandbox::is_sandboxed_client(client)
                            });
                    info!(
                        seat = agent.name(),
                        "Agent seat shown to the user's programs"
                    );
                    self.agent_seats[index].lent_global = Some(global);
                }
                (false, true) => {
                    if let Some(global) = self.agent_seats[index].lent_global.take() {
                        self.display_handle.remove_global::<Self>(global);
                    }
                }
                _ => {}
            }
        }
    }

    /// Bring the agent borders in line with the grants: one frame per
    /// granted workspace, in the colour of its agent, with a chip unless a
    /// window is fullscreen there. Cheap when nothing changed.
    pub fn sync_agent_frames(&mut self) {
        use crate::workspaces::agent_frame::AgentFrameLook;

        self.sync_lent_seat_globals();
        self.prune_handed_over();

        let mut wanted: Vec<(String, usize, AgentFrameLook)> = Vec::new();
        let live = self.agent_seats.iter().filter_map(|agent| {
            Some((
                agent.grant.as_ref()?,
                agent.cursor.color(),
                agent.agent_name.clone(),
            ))
        });
        for (grant, color, name) in live {
            let (output, workspace) = grant.workspace();
            let Some(space) = self.workspaces.space_of_view(output, workspace) else {
                continue;
            };
            let fullscreen = space.elements().any(|window| window.is_fullscreen());
            wanted.push((
                output.to_string(),
                workspace,
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
        // Exposé draws no chip, so there is no Stop to press.
        if !look.chip || self.workspaces.get_show_all() {
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

    /// The user said stop to every agent there is (the secure attention key):
    /// as [`Self::revoke_workspace_grants`], for every seat.
    pub fn stop_all_agents(&mut self) {
        let seats: Vec<(String, Option<String>)> = self
            .agent_seats
            .iter()
            .map(|agent| (agent.name(), agent.owner.clone()))
            .collect();
        for (seat_name, owner) in seats {
            let program = owner
                .as_deref()
                .and_then(crate::agent_consent::suspend_owner);
            info!(seat = seat_name, ?program, "Agent stopped by the user");
            self.remove_agent_seat(&seat_name);
        }
        self.backend_data.request_redraw();
    }

    /// The user said stop to every agent on the workspace `view` of `output`:
    /// each one's seat goes, with its connections and cursor, and its program
    /// gets no seat again until the user logs in anew. The workspace stays,
    /// with its windows.
    pub fn revoke_workspace_grants(&mut self, output: &str, view: usize) {
        let on_it = |grant: Option<&Grant>| grant.is_some_and(|g| g.workspace() == (output, view));
        let seats: Vec<(String, Option<String>)> = self
            .agent_seats
            .iter()
            .filter(|agent| on_it(agent.grant.as_ref()))
            .map(|agent| (agent.name(), agent.owner.clone()))
            .collect();
        for (seat_name, owner) in seats {
            let program = owner
                .as_deref()
                .and_then(crate::agent_consent::suspend_owner);
            info!(
                seat = seat_name,
                output,
                workspace = view,
                ?program,
                "Agent stopped by the user"
            );
            self.remove_agent_seat(&seat_name);
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

        let client = surface.client()?;
        // A client on the agent's own connection.
        if let Some(seat) = ClientState::agent_seat_of(&client) {
            return Some(seat.to_string());
        }
        if self.agent_launch_tokens.is_empty() {
            return None;
        }
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

    /// The windows within the agent seat `seat_name`'s scope: those on its
    /// workspace.
    #[allow(clippy::mutable_key_type)]
    pub fn agent_scope_window_ids(
        &self,
        seat_name: &str,
    ) -> std::collections::HashSet<smithay::reexports::wayland_server::backend::ObjectId> {
        let Some(agent) = self.agent_seat(seat_name) else {
            return Default::default();
        };
        let Reach::Workspace { output, workspace } = agent.reach(&self.workspaces) else {
            return Default::default();
        };
        self.workspaces
            .space_of_view(&output, workspace)
            .map(|space| space.elements().map(|window| window.id()).collect())
            .unwrap_or_default()
    }

    /// Give the agent seat `seat_name`'s keyboard to `window`, if the window
    /// is within the agent's scope. The user's focus and the stacking order
    /// do not change. Returns whether it was.
    pub fn focus_on_agent_seat(
        &mut self,
        seat_name: &str,
        window_id: &smithay::reexports::wayland_server::backend::ObjectId,
    ) -> bool {
        if !self.agent_scope_window_ids(seat_name).contains(window_id) {
            return false;
        }
        let Some(window) = self.workspaces.get_window_for_surface(window_id).cloned() else {
            return false;
        };
        let Some(seat) = self.agent_seat(seat_name).map(|agent| agent.seat.clone()) else {
            return false;
        };
        if let Some(keyboard) = seat.get_keyboard() {
            keyboard.set_focus(self, Some(window.into()), SERIAL_COUNTER.next_serial());
        }
        self.note_agent_activity(seat_name);
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

/// Every window on a workspace some agent holds a grant on. Their
/// applications are watched, so they draw at full rate even while the
/// workspace is hidden.
#[allow(clippy::mutable_key_type)] // ObjectId as key — see window_throttle.rs
pub fn agent_workspace_window_ids<B: Backend + 'static>(
    agent_seats: &[AgentSeat<B>],
    workspaces: &crate::workspaces::Workspaces,
) -> std::collections::HashSet<smithay::reexports::wayland_server::backend::ObjectId> {
    agent_seats
        .iter()
        .filter_map(|agent| agent.grant.as_ref())
        .filter_map(|grant| {
            let (output, workspace) = grant.workspace();
            workspaces.space_of_view(output, workspace)
        })
        .flat_map(|space| space.elements().map(|window| window.id()))
        .collect()
}
