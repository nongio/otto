//! The PipeWire connection, on a thread of its own.
//!
//! The thread connects, reports every global as an [`Event`] and binds each
//! node to follow its state and full properties. When the server goes away it
//! reports [`Event::Disconnected`] and tries again every [`RETRY`] until one
//! answers. Commands reach the thread over a channel its loop wakes on.

// Rust guideline compliant 2026-02-21

use std::any::Any;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::time::Duration;

use pipewire as pw;
use pw::types::ObjectType;

use crate::model::{Client, Direction, Event, Link, MediaType, Node, NodeState, Port};
use crate::virtual_devices::{Made, VirtualDevice};

/// How long to wait before trying a server that did not answer, or that went
/// away, again. A restarted PipeWire is listening again within a second or
/// so; waiting longer would only make the window look stuck.
const RETRY: Duration = Duration::from_secs(2);

/// `EPIPE`: the core error PipeWire reports when the connection is gone.
const EPIPE: i32 = 32;

/// The factory every PipeWire server has for links.
const LINK_FACTORY: &str = "link-factory";

/// How long a link made through [`Connection::link`] lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkLifetime {
    /// Until something removes it, after this program is gone too.
    Lingering,
    /// Only while this program stays connected.
    Owned,
}

#[derive(Debug)]
pub(crate) enum Command {
    Link {
        output: u32,
        input: u32,
        lifetime: LinkLifetime,
    },
    Unlink(u32),
    Setting {
        key: String,
        value: Option<String>,
    },
    VirtualDevices(Vec<VirtualDevice>),
    Device(crate::devices::DeviceCommand),
}

/// A connection to PipeWire, followed on a thread of its own.
///
/// Cheap to clone; every clone sends to the same thread. The thread lives as
/// long as the process.
#[derive(Clone)]
pub struct Connection {
    commands: pw::channel::Sender<Command>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection").finish_non_exhaustive()
    }
}

impl Connection {
    /// Connect as `app` (its `application.name`) and report every change to
    /// `on_event`, which runs on the connection's thread.
    ///
    /// # Panics
    ///
    /// When the thread cannot be started.
    pub fn start(app: &str, on_event: impl Fn(Event) + Send + 'static) -> Self {
        let (commands, receiver) = pw::channel::channel();
        let app = app.to_string();
        std::thread::Builder::new()
            .name("pipewire".into())
            .spawn(move || run(&app, Box::new(on_event), receiver))
            .expect("start the PipeWire thread");
        Self { commands }
    }

    /// Link `output` to `input`, both port ids.
    pub fn link(&self, output: u32, input: u32, lifetime: LinkLifetime) {
        self.send(Command::Link {
            output,
            input,
            lifetime,
        });
    }

    /// Remove the link with global id `link`.
    pub fn unlink(&self, link: u32) {
        self.send(Command::Unlink(link));
    }

    /// Set an engine setting in the `settings` metadata; `None` clears it.
    pub fn set_setting(&self, key: &str, value: Option<&str>) {
        self.send(Command::Setting {
            key: key.to_string(),
            value: value.map(str::to_string),
        });
    }

    /// Keep exactly `devices` made while this connection lasts, making them
    /// again whenever the server comes back. They go when this program does.
    pub fn keep_virtual_devices(&self, devices: Vec<VirtualDevice>) {
        self.send(Command::VirtualDevices(devices));
    }

    pub(crate) fn send(&self, command: Command) {
        if let Err(command) = self.commands.send(command) {
            tracing::warn!(?command, "PipeWire thread is gone");
        }
    }
}

fn run(
    app: &str,
    on_event: Box<dyn Fn(Event) + Send>,
    mut commands: pw::channel::Receiver<Command>,
) {
    pw::init();
    let emit: Rc<dyn Fn(Event)> = Rc::new(on_event);
    let mut was_connected = true;
    let wanted = Rc::new(RefCell::new(Vec::new()));
    loop {
        let (receiver, connected) = session(app, &emit, commands, &wanted);
        commands = receiver;
        if connected || was_connected {
            emit(Event::Disconnected);
        }
        was_connected = connected;
        std::thread::sleep(RETRY);
    }
}

/// One connection, from connecting to the server going away. Hands the
/// command receiver back, and whether the server was reached at all.
fn session(
    app: &str,
    emit: &Rc<dyn Fn(Event)>,
    commands: pw::channel::Receiver<Command>,
    wanted: &Rc<RefCell<Vec<VirtualDevice>>>,
) -> (pw::channel::Receiver<Command>, bool) {
    let Ok(mainloop) = pw::main_loop::MainLoopRc::new(None) else {
        return (commands, false);
    };
    let Ok(context) = pw::context::ContextRc::new(&mainloop, None) else {
        return (commands, false);
    };
    let mut props = pw::properties::PropertiesBox::new();
    props.insert("application.name", app);
    let core = match context.connect_rc(Some(props)) {
        Ok(core) => core,
        Err(error) => {
            tracing::debug!(%error, "PipeWire is not answering");
            return (commands, false);
        }
    };
    let Ok(registry) = core.get_registry_rc() else {
        return (commands, false);
    };
    emit(Event::Connected);

    let state = Rc::new(State::default());
    state
        .virtual_devices
        .borrow_mut()
        .sync(&core, &context, &wanted.borrow());

    let pending = core.sync(0).ok();
    let _core_listener = core
        .add_listener_local()
        .done({
            let emit = emit.clone();
            move |id, seq| {
                if id == pw::core::PW_ID_CORE && Some(seq) == pending {
                    emit(Event::Synced);
                }
            }
        })
        .error({
            let mainloop = mainloop.downgrade();
            move |id, _seq, res, message| {
                if id == pw::core::PW_ID_CORE && res == -EPIPE {
                    if let Some(mainloop) = mainloop.upgrade() {
                        mainloop.quit();
                    }
                } else {
                    tracing::warn!(id, res, message, "PipeWire error");
                }
            }
        })
        .register();

    let _registry_listener = registry
        .add_listener_local()
        .global({
            let registry = registry.downgrade();
            let core = core.downgrade();
            let state = state.clone();
            let emit = emit.clone();
            move |global| {
                let Some(registry) = registry.upgrade() else {
                    return;
                };
                if global.type_ == ObjectType::Device {
                    if let Some(core) = core.upgrade() {
                        crate::devices::bind(&core, &registry, &state, &emit, global);
                    }
                    return;
                }
                on_global(&registry, &state, &emit, global);
            }
        })
        .global_remove({
            let state = state.clone();
            let emit = emit.clone();
            move |id| {
                state.bound.borrow_mut().remove(&id);
                state.nodes.borrow_mut().remove(&id);
                state.port_nodes.borrow_mut().remove(&id);
                if let Some(ports) = state.links.borrow_mut().remove(&id) {
                    state.owned.borrow_mut().remove(&ports);
                }
                if *state.settings_id.borrow() == Some(id) {
                    *state.settings.borrow_mut() = None;
                    *state.settings_id.borrow_mut() = None;
                }
                emit(Event::Removed(id));
            }
        })
        .register();

    let attached = commands.attach(mainloop.loop_(), {
        let core = core.clone();
        let registry = registry.clone();
        let state = state.clone();
        let context = context.clone();
        let wanted = wanted.clone();
        move |command| match command {
            Command::VirtualDevices(devices) => {
                *wanted.borrow_mut() = devices;
                state
                    .virtual_devices
                    .borrow_mut()
                    .sync(&core, &context, &wanted.borrow());
            }
            command => run_command(&core, &registry, &state, command),
        }
    });

    mainloop.run();

    let commands = attached.deattach();
    state.clear();
    (commands, true)
}

/// What the thread keeps for one connection.
#[derive(Default)]
pub(crate) struct State {
    /// Proxies and their listeners, by global id; dropping one unbinds it.
    pub(crate) bound: RefCell<HashMap<u32, Box<dyn Any>>>,
    /// The last report of each node, so an update without properties keeps
    /// the ones already known.
    nodes: RefCell<HashMap<u32, Node>>,
    /// Each port's node, which a new link names.
    port_nodes: RefCell<HashMap<u32, u32>>,
    /// Each link's `(output, input)` ports.
    links: RefCell<HashMap<u32, (u32, u32)>>,
    /// Links this program made, by `(output, input)` ports. An owned link
    /// lives as long as its proxy.
    owned: RefCell<HashMap<(u32, u32), pw::link::Link>>,
    settings: RefCell<Option<pw::metadata::Metadata>>,
    settings_id: RefCell<Option<u32>>,
    /// The virtual devices made on this connection.
    virtual_devices: RefCell<Made>,
}

impl State {
    fn clear(&self) {
        self.virtual_devices.borrow_mut().clear();
        self.owned.borrow_mut().clear();
        *self.settings.borrow_mut() = None;
        self.bound.borrow_mut().clear();
    }
}

pub(crate) fn props_map(dict: Option<&pw::spa::utils::dict::DictRef>) -> BTreeMap<String, String> {
    dict.map(|d| {
        d.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    })
    .unwrap_or_default()
}

fn parse_id(props: &pw::spa::utils::dict::DictRef, key: &str) -> Option<u32> {
    props.get(key).and_then(|v| v.parse().ok())
}

fn on_global(
    registry: &pw::registry::RegistryRc,
    state: &Rc<State>,
    emit: &Rc<dyn Fn(Event)>,
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
) {
    let id = global.id;
    match global.type_ {
        ObjectType::Node => {
            let node = Node {
                id,
                props: props_map(global.props),
                state: NodeState::Creating,
            };
            state.nodes.borrow_mut().insert(id, node.clone());
            emit(Event::Node(node));
            bind_node(registry, state, emit, global);
        }
        ObjectType::Port => {
            let Some(props) = global.props else { return };
            let Some(node) = parse_id(props, "node.id") else {
                return;
            };
            let direction = match props.get("port.direction") {
                Some("in") => Some(Direction::Input),
                Some("out") => Some(Direction::Output),
                _ => None,
            };
            let port = Port {
                id,
                node,
                direction,
                name: props.get("port.name").unwrap_or_default().to_string(),
                channel: props
                    .get("audio.channel")
                    .filter(|c| !c.is_empty() && *c != "UNK")
                    .map(str::to_string),
                format: props.get("format.dsp").map(MediaType::from_format_dsp),
                monitor: props.get("port.monitor") == Some("true"),
                props: props_map(Some(props)),
            };
            state.port_nodes.borrow_mut().insert(id, node);
            emit(Event::Port(port));
        }
        ObjectType::Link => {
            let Some(props) = global.props else { return };
            let ids = (
                parse_id(props, "link.output.node"),
                parse_id(props, "link.output.port"),
                parse_id(props, "link.input.node"),
                parse_id(props, "link.input.port"),
            );
            let (Some(output_node), Some(output_port), Some(input_node), Some(input_port)) = ids
            else {
                return;
            };
            state
                .links
                .borrow_mut()
                .insert(id, (output_port, input_port));
            emit(Event::Link(Link {
                id,
                output_node,
                output_port,
                input_node,
                input_port,
                client: parse_id(props, "client.id"),
            }));
        }
        ObjectType::Client => {
            let app = global
                .props
                .and_then(|p| p.get("application.name"))
                .map(str::to_string);
            emit(Event::Client(Client { id, app }));
        }
        ObjectType::Metadata => {
            if global.props.and_then(|p| p.get("metadata.name")) != Some("settings") {
                return;
            }
            bind_settings(registry, state, emit, global);
        }
        _ => {}
    }
}

fn bind_node(
    registry: &pw::registry::RegistryRc,
    state: &Rc<State>,
    emit: &Rc<dyn Fn(Event)>,
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
) {
    let id = global.id;
    let proxy: pw::node::Node = match registry.bind(global) {
        Ok(proxy) => proxy,
        Err(error) => {
            tracing::debug!(%error, id, "binding a node failed");
            return;
        }
    };
    let listener = proxy
        .add_listener_local()
        .info({
            let state = Rc::downgrade(state);
            let emit = Rc::downgrade(emit);
            move |info| {
                let (Some(state), Some(emit)) = (state.upgrade(), emit.upgrade()) else {
                    return;
                };
                let mut nodes = state.nodes.borrow_mut();
                let node = nodes.entry(id).or_insert_with(|| Node {
                    id,
                    ..Node::default()
                });
                if info.change_mask().contains(pw::node::NodeChangeMask::PROPS) {
                    if let Some(props) = info.props() {
                        node.props = props_map(Some(props));
                    }
                }
                node.state = match info.state() {
                    pw::node::NodeState::Error(_) => NodeState::Error,
                    pw::node::NodeState::Creating => NodeState::Creating,
                    pw::node::NodeState::Suspended => NodeState::Suspended,
                    pw::node::NodeState::Idle => NodeState::Idle,
                    pw::node::NodeState::Running => NodeState::Running,
                };
                let node = node.clone();
                drop(nodes);
                emit(Event::Node(node));
            }
        })
        .register();
    state
        .bound
        .borrow_mut()
        .insert(id, Box::new((listener, proxy)));
}

fn bind_settings(
    registry: &pw::registry::RegistryRc,
    state: &Rc<State>,
    emit: &Rc<dyn Fn(Event)>,
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
) {
    let id = global.id;
    let proxy: pw::metadata::Metadata = match registry.bind(global) {
        Ok(proxy) => proxy,
        Err(error) => {
            tracing::debug!(%error, id, "binding the settings metadata failed");
            return;
        }
    };
    let listener = proxy
        .add_listener_local()
        .property({
            let emit = Rc::downgrade(emit);
            move |subject, key, _type, value| {
                let Some(emit) = emit.upgrade() else {
                    return 0;
                };
                if subject == 0 {
                    if let Some(key) = key {
                        emit(Event::Setting {
                            key: key.to_string(),
                            value: value.map(str::to_string),
                        });
                    }
                }
                0
            }
        })
        .register();
    *state.settings.borrow_mut() = Some(proxy);
    *state.settings_id.borrow_mut() = Some(id);
    state.bound.borrow_mut().insert(id, Box::new(listener));
}

fn run_command(
    core: &pw::core::CoreRc,
    registry: &pw::registry::RegistryRc,
    state: &State,
    command: Command,
) {
    match command {
        Command::Link {
            output,
            input,
            lifetime,
        } => {
            let nodes = state.port_nodes.borrow();
            let (Some(output_node), Some(input_node)) = (nodes.get(&output), nodes.get(&input))
            else {
                tracing::debug!(output, input, "linking ports that are gone");
                return;
            };
            let mut props = pw::properties::PropertiesBox::new();
            props.insert("link.output.node", output_node.to_string());
            props.insert("link.output.port", output.to_string());
            props.insert("link.input.node", input_node.to_string());
            props.insert("link.input.port", input.to_string());
            let linger = match lifetime {
                LinkLifetime::Lingering => "true",
                LinkLifetime::Owned => "false",
            };
            props.insert("object.linger", linger);
            match core.create_object::<pw::link::Link>(LINK_FACTORY, &props) {
                Ok(link) => {
                    state.owned.borrow_mut().insert((output, input), link);
                }
                Err(error) => tracing::warn!(%error, output, input, "linking failed"),
            }
        }
        Command::Unlink(id) => {
            let ports = state.links.borrow().get(&id).copied();
            // A link this program owns goes with its proxy; anyone's goes
            // when the registry destroys it.
            if let Some(ports) = ports {
                state.owned.borrow_mut().remove(&ports);
            }
            registry.destroy_global(id);
        }
        Command::Setting { key, value } => match state.settings.borrow().as_ref() {
            Some(metadata) => metadata.set_property(0, &key, None, value.as_deref()),
            None => tracing::warn!(key, "no settings metadata to set"),
        },
        Command::VirtualDevices(_) => {}
        Command::Device(command) => crate::devices::run(state, command),
    }
}
