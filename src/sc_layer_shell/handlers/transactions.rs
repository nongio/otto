use wayland_backend::server::ClientId;
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, Resource};

use super::super::protocol::gen::otto_transaction_v1::{self, OttoTransactionV1};
use crate::{sc_layer_shell::handlers::commit_transaction, state::Backend, Otto};

impl<BackendData: Backend> Dispatch<OttoTransactionV1, ()> for Otto<BackendData> {
    fn request(
        state: &mut Self,
        _client: &Client,
        transaction: &OttoTransactionV1,
        request: otto_transaction_v1::Request,
        _data: &(),
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        let txn_id = transaction.id();

        match request {
            otto_transaction_v1::Request::SetDuration { duration } => {
                if let Some(txn) = state.sc_transactions.get_mut(&txn_id) {
                    txn.duration = Some(duration as f32);
                    tracing::debug!("Transaction duration set: {}s", duration);
                }
            }

            otto_transaction_v1::Request::SetDelay { delay } => {
                if let Some(txn) = state.sc_transactions.get_mut(&txn_id) {
                    txn.delay = Some(delay as f32);
                    tracing::debug!("Transaction delay set: {}s", delay);
                }
            }

            otto_transaction_v1::Request::SetTimingFunction { timing } => {
                if let Some(txn) = state.sc_transactions.get_mut(&txn_id) {
                    // Get the timing function data from the object
                    if let Some(timing_data) = timing.data::<super::timing_function::ScTimingFunctionData>() {
                        // Store the timing function for later use when creating the transition
                        txn.timing_function = Some(layers::prelude::Transition {
                            timing: timing_data.timing.clone(),
                            delay: 0.0, // Will be set from txn.delay
                        });
                        txn.spring_uses_duration = timing_data.spring_uses_duration;
                        txn.spring_bounce = timing_data.spring_bounce;
                        txn.spring_initial_velocity = timing_data.spring_initial_velocity;
                    }
                }
            }

            otto_transaction_v1::Request::EnableCompletionEvent => {
                if let Some(txn) = state.sc_transactions.get_mut(&txn_id) {
                    txn.send_completion = true;
                }
            }

            otto_transaction_v1::Request::Commit => {
                commit_transaction(state, txn_id);
            }

            otto_transaction_v1::Request::Cancel => {
                // Cancel the transaction - discard all pending changes without applying them
                state.sc_transactions.remove(&txn_id);
            }
        }
    }

    fn destroyed(state: &mut Self, _client: ClientId, transaction: &OttoTransactionV1, _data: &()) {
        // Clean up transaction if still present
        state.sc_transactions.remove(&transaction.id());
    }
}
