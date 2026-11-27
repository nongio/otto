//! The effects made on one connection, on the PipeWire thread.

// Rust guideline compliant 2026-02-21

use std::collections::HashMap;
use std::io::Cursor;

use pipewire as pw;
use pw::spa::param::ParamType;
use pw::spa::pod::serialize::PodSerializer;
use pw::spa::pod::{Object, Pod, Property, Value};
use pw::spa::utils::SpaTypes;

use super::{chain, Effect};
use crate::connection::State;
use crate::module::LoadedModule;

/// The module every effect is made with.
const FILTER_CHAIN_MODULE: &str = "libpipewire-module-filter-chain";

/// The effects made on one connection, each with the module that holds it.
///
/// Must be dropped, or [`Made::clear`]ed, before the context it loaded
/// modules into.
#[derive(Default)]
pub(crate) struct Made {
    made: HashMap<String, (Effect, LoadedModule)>,
}

impl Made {
    /// Make every effect in `wanted` that is not made (or that went away by
    /// itself), remove every made one that is not wanted, and bring the rest
    /// up to date: through their controls when only values changed, by
    /// making them again otherwise.
    pub(crate) fn sync(
        &mut self,
        context: &pw::context::ContextRc,
        state: &State,
        wanted: &[Effect],
    ) {
        self.made.retain(|id, (made, module)| {
            if !module.is_alive() {
                tracing::info!(id, "an effect went away by itself, making it again");
                return false;
            }
            let keep = wanted.iter().any(|w| w.same_chain(made));
            if !keep {
                tracing::info!(id, "removing an effect");
            }
            keep
        });
        for effect in wanted {
            if let Some((made, _)) = self.made.get_mut(&effect.id) {
                if made == effect {
                    continue;
                }
                if set_controls(state, effect) {
                    tracing::debug!(id = effect.id, "changed an effect's controls");
                    *made = effect.clone();
                    continue;
                }
                // Not bound yet: its controls cannot be set, so it is made
                // again with the new values instead.
                self.made.remove(&effect.id);
            }
            let args = match chain::args(effect) {
                Ok(args) => args,
                Err(chain::Unmade::NoPlugin) => {
                    tracing::warn!(
                        id = effect.id,
                        kind = ?effect.kind,
                        "the effect's plugin is not installed, not making it"
                    );
                    continue;
                }
                Err(chain::Unmade::NoImpulse(path)) => {
                    tracing::warn!(
                        id = effect.id,
                        path = %path.display(),
                        "the effect's impulse response is missing, not making it"
                    );
                    continue;
                }
            };
            tracing::info!(id = effect.id, name = effect.name, "making an effect");
            tracing::trace!(id = effect.id, args, "the effect's filter chain");
            if let Some(module) = crate::module::load(context, FILTER_CHAIN_MODULE, &args) {
                self.made
                    .insert(effect.id.clone(), (effect.clone(), module));
            }
        }
    }

    /// Remove every effect made.
    pub(crate) fn clear(&mut self) {
        self.made.clear();
    }
}

/// Set every control of `effect` on its node, whoever made it. `false` when
/// its node is not there or not bound yet.
pub(crate) fn set_controls(state: &State, effect: &Effect) -> bool {
    let name = effect.node_name();
    let Some(id) = state.node_named(&name) else {
        return false;
    };
    let bound = state.bound.borrow();
    let Some((_, _, proxy)) = bound
        .get(&id)
        .and_then(|b| b.downcast_ref::<crate::mixer::BoundNode>())
    else {
        return false;
    };
    let Some(bytes) = props_pod(&chain::controls(effect)) else {
        return false;
    };
    let Some(pod) = Pod::from_bytes(&bytes) else {
        return false;
    };
    proxy.set_param(ParamType::Props, 0, pod);
    true
}

/// A `Props` param setting a filter chain's controls: `params` is a struct
/// of `"node:port"` names, each followed by its value.
fn props_pod(controls: &[(String, f32)]) -> Option<Vec<u8>> {
    let mut params = Vec::with_capacity(controls.len() * 2);
    for (name, value) in controls {
        params.push(Value::String(name.clone()));
        params.push(Value::Float(*value));
    }
    let value = Value::Object(Object {
        type_: SpaTypes::ObjectParamProps.as_raw(),
        id: ParamType::Props.as_raw(),
        properties: vec![Property::new(
            pw::spa::sys::SPA_PROP_params,
            Value::Struct(params),
        )],
    });
    match PodSerializer::serialize(Cursor::new(Vec::new()), &value) {
        Ok((cursor, _)) => Some(cursor.into_inner()),
        Err(error) => {
            tracing::warn!(?error, "building an effect's controls failed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pw::spa::pod::deserialize::PodDeserializer;

    #[test]
    fn controls_go_as_a_params_struct() {
        let bytes = props_pod(&[("mix_0:Gain 1".into(), 0.5), ("eq0_0:b0".into(), 1.0)]).unwrap();
        let (_, value) = PodDeserializer::deserialize_any_from(&bytes).unwrap();
        let Value::Object(object) = value else {
            panic!("an object");
        };
        assert_eq!(object.properties[0].key, pw::spa::sys::SPA_PROP_params);
        let Value::Struct(fields) = &object.properties[0].value else {
            panic!("a struct");
        };
        assert_eq!(
            fields,
            &vec![
                Value::String("mix_0:Gain 1".into()),
                Value::Float(0.5),
                Value::String("eq0_0:b0".into()),
                Value::Float(1.0),
            ]
        );
    }
}
