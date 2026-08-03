//! Resolve a PipeWire node id for an Otto virtual output by name, so users
//! don't have to read the numeric node id out of Otto's log.
//!
//! Otto tags each virtual output's PipeWire node with a custom
//! `otto.output.name` property (`src/screenshare/pipewire_stream.rs`); this
//! walks the registry for a `Node` whose property matches.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use pipewire as pw;
use pw::proxy::Listener;
use pw::types::ObjectType;

/// Bound node proxies + their info listeners, kept alive until we're done
/// discovering — PipeWire tears both down otherwise before `info` fires.
type BoundNodes = Rc<RefCell<Vec<(pw::node::Node, Box<dyn Listener>)>>>;

/// Block (spins its own short-lived PipeWire main loop) until a node tagged
/// with `otto.output.name == output` appears, or `timeout` elapses.
pub fn node_for_output(output: &str, timeout: Duration) -> Result<u32> {
    pw::init();

    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;

    let found: Rc<RefCell<Option<u32>>> = Rc::new(RefCell::new(None));
    let wanted = output.to_string();

    // The registry's `global` event only carries a reduced property set
    // (whatever the server advertises at binding time) — custom keys like
    // `otto.output.name` aren't in it, even though they were passed at
    // stream creation. The full property set only shows up once the node
    // is bound and its `info` event fires, so bind every Node and check
    // there instead. Bound proxies/listeners must be kept alive until we're
    // done, or PipeWire tears them down before `info` arrives.
    let bound: BoundNodes = Rc::new(RefCell::new(Vec::new()));

    let found_cb = found.clone();
    let bound_cb = bound.clone();
    let main_loop_weak = Rc::new(main_loop.downgrade());
    let registry_rc = registry.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |obj| {
            if obj.type_ != ObjectType::Node {
                return;
            }
            let Ok(node) = registry_rc.bind::<pw::node::Node, _>(obj) else {
                return;
            };

            let found_info = found_cb.clone();
            let wanted_info = wanted.clone();
            let main_loop_weak_info = main_loop_weak.clone();
            let listener = node
                .add_listener_local()
                .info(move |info| {
                    let matches = info
                        .props()
                        .and_then(|p| p.get("otto.output.name"))
                        .map(|name| name == wanted_info)
                        .unwrap_or(false);
                    if matches {
                        *found_info.borrow_mut() = Some(info.id());
                        if let Some(ml) = main_loop_weak_info.upgrade() {
                            ml.quit();
                        }
                    }
                })
                .register();
            bound_cb.borrow_mut().push((node, Box::new(listener)));
        })
        .register();

    // Bound the wait so a nonexistent output doesn't hang forever.
    let main_loop_weak = main_loop.downgrade();
    let timer = main_loop.loop_().add_timer(move |_| {
        if let Some(ml) = main_loop_weak.upgrade() {
            ml.quit();
        }
    });
    timer.update_timer(Some(timeout), None);

    main_loop.run();

    let found = *found.borrow();
    found.ok_or_else(|| {
        anyhow!(
            "no virtual output named '{output}' found on PipeWire — is it enabled \
             (`[[virtual_outputs]]` with that `name`) in otto_config.toml, and Otto running?"
        )
    })
}

/// Collect every currently-advertised Otto virtual output (name, node id),
/// by the same bind-and-check-`info` approach as [`node_for_output`]. Spins
/// the loop for the full `timeout` — unlike `node_for_output` there's no
/// single match to quit early on.
pub fn list_outputs(timeout: Duration) -> Result<Vec<(String, u32)>> {
    pw::init();

    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;

    let outputs: Rc<RefCell<Vec<(String, u32)>>> = Rc::new(RefCell::new(Vec::new()));
    let bound: BoundNodes = Rc::new(RefCell::new(Vec::new()));

    let outputs_cb = outputs.clone();
    let bound_cb = bound.clone();
    let registry_rc = registry.clone();
    let _registry_listener = registry
        .add_listener_local()
        .global(move |obj| {
            if obj.type_ != ObjectType::Node {
                return;
            }
            let Ok(node) = registry_rc.bind::<pw::node::Node, _>(obj) else {
                return;
            };

            let outputs_info = outputs_cb.clone();
            let listener = node
                .add_listener_local()
                .info(move |info| {
                    if let Some(name) = info.props().and_then(|p| p.get("otto.output.name")) {
                        let entry = (name.to_string(), info.id());
                        let mut outputs = outputs_info.borrow_mut();
                        if !outputs.contains(&entry) {
                            outputs.push(entry);
                        }
                    }
                })
                .register();
            bound_cb.borrow_mut().push((node, Box::new(listener)));
        })
        .register();

    let main_loop_weak = main_loop.downgrade();
    let timer = main_loop.loop_().add_timer(move |_| {
        if let Some(ml) = main_loop_weak.upgrade() {
            ml.quit();
        }
    });
    timer.update_timer(Some(timeout), None);

    main_loop.run();

    let outputs = outputs.borrow().clone();
    Ok(outputs)
}
