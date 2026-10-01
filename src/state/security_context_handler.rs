//! `wp_security_context_v1`: a sandbox engine's listener, whose clients are
//! sandboxed; or an agent's, whose clients are the agent's.
//!
//! A listener made on an agent's own connection (`crate::state::agent_seats`)
//! connects more of the agent's clients: each one sees the agent's seat
//! alone and drives it alone, and goes with the seat. That is how an agent
//! started in a sandbox gets every client inside it onto its seat, with the
//! protocol sandboxes already speak.

use std::sync::Arc;

use smithay::wayland::security_context::{
    SecurityContext, SecurityContextHandler, SecurityContextListenerSource,
};

use super::{Backend, ClientState, Otto};

impl<BackendData: Backend + 'static> SecurityContextHandler for Otto<BackendData> {
    fn context_created(
        &mut self,
        source: SecurityContextListenerSource,
        security_context: SecurityContext,
    ) {
        let agent_seat = self
            .display_handle
            .backend_handle()
            .get_client_data(security_context.creator_client_id.clone())
            .ok()
            .and_then(|data| {
                data.downcast_ref::<ClientState>()
                    .and_then(|creator| creator.agent_seat.clone())
            });
        let seat_for_source = agent_seat.clone();
        let token = self
            .handle
            .insert_source(source, move |client_stream, _, data| {
                if let Some(seat) = &seat_for_source {
                    if data.agent_seat(seat).is_none() {
                        // The seat went; nothing more connects as the agent.
                        return;
                    }
                    if let Err(err) = data.insert_agent_client(seat, client_stream) {
                        tracing::warn!(seat, %err, "cannot connect an agent's client");
                    }
                    return;
                }
                let client_state = ClientState {
                    security_context: Some(security_context.clone()),
                    ..ClientState::default()
                };
                if let Err(err) = data
                    .display_handle
                    .insert_client(client_stream, Arc::new(client_state))
                {
                    tracing::warn!("Error adding wayland client: {}", err);
                };
            })
            .expect("Failed to init wayland socket source");
        if let Some(seat) = agent_seat {
            match self.agent_seat_mut(&seat) {
                Some(agent) => agent.listeners.push(token),
                None => self.handle.remove(token),
            }
        }
    }
}
