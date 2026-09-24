//! otto-gather: gather selections from any app into a balloon, then ask about
//! them in Ask (proof of concept, plan 0013).
//!
//! Run it once; it binds `zwp_input_method_v2`, owns `org.otto.Gather1` on
//! the session bus, and waits. Bind the commands to shortcuts:
//!
//! - `otto-gather add`: start gathering, and add what is selected: the text
//!   in the focused field, or the files in a focused Files window. A balloon
//!   lists what was gathered until it is sent or cancelled; it starts in the
//!   top-right corner and can be dragged.
//! - `otto-gather add-file PATH`: add a file.
//! - `otto-gather add-region`: drag out a screen region and add a capture of
//!   it (needs `slurp` and `grim`).
//! - `otto-gather send`: open Ask. However Ask is opened, it shows what is
//!   gathered in the balloon's place, following it over the bus, and the
//!   gathering ends when Ask sends it. Closed without sending, Ask leaves it
//!   and the balloon comes back.
//! - `otto-gather cancel`: throw the gathering away.
//!
//! Gathering never takes the keyboard, so the app keeps its selection and
//! caret. Selections come from the focused field's surrounding text, or,
//! when the field reports none (GTK3, Qt, terminals), from the primary
//! selection.
//!
//! Environment:
//! - `OTTO_GATHER_LAUNCHER`: the launcher to open (default `otto-launcher`).

mod balloon;
mod dbus;
mod drop;
mod panel;
mod primary;
mod region;
mod request;

// Rust guideline compliant 2026-02-21

use std::path::PathBuf;
use std::process::Command as Process;
use std::time::{Duration, Instant};

use anyhow::Context;
use otto_kit::clipboard::URI_LIST;
use smithay_client_toolkit::delegate_shm;
use smithay_client_toolkit::reexports::calloop::channel;
use smithay_client_toolkit::reexports::calloop::generic::Generic;
use smithay_client_toolkit::reexports::calloop::{
    EventLoop, Interest, LoopHandle, Mode, PostAction,
};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{
    wl_callback, wl_compositor, wl_data_device_manager, wl_data_offer, wl_pointer, wl_region,
    wl_registry, wl_seat, wl_shm, wl_subcompositor, wl_subsurface, wl_surface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1::{self, WpCursorShapeDeviceV1},
    wp_cursor_shape_manager_v1::WpCursorShapeManagerV1,
};
use wayland_protocols_misc::zwp_input_method_v2::client::{
    zwp_input_method_manager_v2::ZwpInputMethodManagerV2,
    zwp_input_method_v2::{self, ZwpInputMethodV2},
};
use wayland_protocols_wlr::data_control::v1::client::zwlr_data_control_manager_v1::ZwlrDataControlManagerV1;
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::ZwlrLayerShellV1,
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};

use otto_kit::components::scroll::ScrollView;
use otto_kit::protocols::{
    otto_style_transaction_v1, otto_surface_style_manager_v1, otto_surface_style_v1,
    otto_timing_function_v1,
};
use otto_kit::skia::Rect;

use crate::balloon::{Balloon, Hit, Layout};
use crate::dbus::{Command, Items};
use crate::drop::Drops;
use crate::panel::{Panel, Shell};
use crate::primary::Primary;
use crate::request::{Gathering, Item, Surrounding};
use otto_kit::components::attachments::ICON_SIZE;
use otto_peek::thumbcache::Size;
use otto_peek::thumbnailer::Thumbnailer;

/// Initial size of the balloon's buffer pool; it grows with the balloon.
const POOL_BYTES: usize = 512 * 256 * 4;
/// Balloon buffers are drawn at this scale, so they stay sharp on HiDPI.
const BALLOON_SCALE: i32 = 2;
/// Linux's code for the left mouse button (`BTN_LEFT`).
const BTN_LEFT: u32 = 0x110;
/// The most primary-selection text gathered at once; the rest is dropped.
const PRIMARY_MAX_BYTES: usize = 1 << 20;
/// How long a removed item takes to shrink away.
const LEAVE: Duration = Duration::from_millis(220);

fn main() -> anyhow::Result<()> {
    // Thumbnails are decoded by this same executable, started as a sandboxed
    // worker; that start ends here.
    otto_peek::run_worker_if_requested();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "otto_gather=info".into()),
        )
        .init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()?;

    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None => serve(&runtime),
        Some("add") => runtime.block_on(dbus::call("Add", &())).map_err(Into::into),
        Some("add-region") => runtime
            .block_on(dbus::call("AddRegion", &()))
            .map_err(Into::into),
        Some("send") => runtime
            .block_on(dbus::call("Send", &()))
            .map_err(Into::into),
        Some("cancel") => runtime
            .block_on(dbus::call("Cancel", &()))
            .map_err(Into::into),
        Some("add-file") => {
            let path = args.next().context("add-file needs a path")?;
            let path =
                std::fs::canonicalize(&path).with_context(|| format!("no such file: {path}"))?;
            let path = path.to_str().context("the path is not UTF-8")?.to_owned();
            runtime
                .block_on(dbus::call("AddFile", &(path,)))
                .map_err(Into::into)
        }
        Some(other) => {
            anyhow::bail!(
                "unknown command `{other}`; use add, add-file PATH, add-region, send or cancel"
            )
        }
    }
}

/// Run the gathering service until the compositor goes away.
fn serve(runtime: &tokio::runtime::Runtime) -> anyhow::Result<()> {
    let conn = Connection::connect_to_env()?;
    let (globals, event_queue) = registry_queue_init::<State>(&conn)?;
    let qh = event_queue.handle();

    let shm = Shm::bind(&globals, &qh)?;
    let pool = SlotPool::new(POOL_BYTES, &shm)?;
    let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 1..=4, ())?;
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=7, ())?;
    let manager: ZwpInputMethodManagerV2 = globals.bind(&qh, 1..=1, ())?;
    let input_method = manager.get_input_method(&seat, &qh, ());
    // Version 2 is the first with the primary selection.
    let data_control: ZwlrDataControlManagerV1 = globals
        .bind(&qh, 2..=2, ())
        .context("the compositor has no wlr-data-control v2")?;
    let primary = Primary::new(&data_control, &seat, &qh);
    let data_devices: wl_data_device_manager::WlDataDeviceManager = globals.bind(&qh, 1..=3, ())?;
    let drops = Drops::new(&data_devices, &seat, &qh);
    let shell = Shell {
        compositor: compositor.clone(),
        subcompositor: globals.bind(&qh, 1..=1, ())?,
        layer_shell: globals
            .bind(&qh, 1..=4, ())
            .context("the compositor has no wlr-layer-shell")?,
        // Optional: without it the card draws its own plain background.
        style: globals.bind(&qh, 1..=5, ()).ok(),
    };
    // The card follows the desktop's colour scheme and accent.
    otto_kit::color_scheme::spawn_color_scheme_watcher();
    otto_kit::accent::spawn_accent_watcher();
    // File icons come from the desktop's icon theme, as in Files.
    otto_kit::icon_theme::spawn_icon_theme_watcher();
    let pointer = seat.get_pointer(&qh, ());
    // Optional: without it the cursor stays whatever it was.
    let cursor_shape = globals
        .bind::<WpCursorShapeManagerV1, _, _>(&qh, 1..=1, ())
        .ok()
        .map(|manager| manager.get_pointer(&pointer, &qh, ()));

    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue)
        .insert(handle.clone())
        .map_err(|error| anyhow::anyhow!("cannot watch the Wayland socket: {error}"))?;

    let (commands_tx, commands_rx) = channel::channel::<Command>();
    handle
        .insert_source(commands_rx, |event, _, state| {
            if let channel::Event::Msg(command) = event {
                state.on_command(command);
            }
        })
        .map_err(|error| anyhow::anyhow!("cannot watch the command channel: {error}"))?;
    // Files on the card show their thumbnails, as in Files.
    let thumbnailer = Thumbnailer::new(Size::for_box(ICON_SIZE, BALLOON_SCALE as f32))
        .context("cannot start making thumbnails")?;
    let made = Generic::new(thumbnailer.wake_handle()?, Interest::READ, Mode::Level);
    handle
        .insert_source(made, |_, _, state| {
            state.take_thumbnails();
            Ok(PostAction::Continue)
        })
        .map_err(|error| anyhow::anyhow!("cannot watch the thumbnails: {error}"))?;
    // Kept alive for as long as the service runs.
    let bus = runtime
        .block_on(dbus::serve(commands_tx.clone()))
        .with_context(|| {
            format!(
                "cannot own {} (is otto-gather running already?)",
                dbus::NAME
            )
        })?;
    tracing::info!("otto-gather ready");
    // Every change goes out in order, from one task.
    let (announcements, mut announced) = tokio::sync::mpsc::unbounded_channel::<Items>();
    runtime.spawn(async move {
        while let Some(items) = announced.recv().await {
            if let Err(error) = dbus::announce(&bus, items).await {
                tracing::warn!(%error, "cannot announce the gathering");
            }
        }
    });

    let mut state = State {
        qh,
        shm,
        pool,
        buffer: None,
        _input_method: input_method,
        loop_handle: handle,
        commands: commands_tx,
        picking: false,
        primary,
        drops,
        thumbnailer,
        shell,
        cursor_shape,
        panel: None,
        pending: Field::default(),
        field: Field::default(),
        gathering: None,
        gathering_dir: PathBuf::new(),
        held_by: None,
        announcements,
        last_announced: Items::new(),
        balloon: Balloon::default(),
        layout: None,
        pressed: None,
        pointer_serial: 0,
        runtime: runtime.handle().clone(),
        hovering: None,
        hovered_item: None,
        leaving: None,
        pressed_item: None,
        scroll: ScrollView::new(Rect::default()),
        frame_pending: false,
        wheel_discrete: false,
    };
    loop {
        event_loop.dispatch(None, &mut state)?;
    }
}

/// The focused text field, as the text-input client describes it.
#[derive(Debug, Default, Clone)]
struct Field {
    active: bool,
    surrounding: Option<Surrounding>,
}

struct State {
    qh: QueueHandle<State>,
    shm: Shm,
    pool: SlotPool,
    buffer: Option<Buffer>,
    /// Held so the focused field's surrounding text keeps arriving.
    _input_method: ZwpInputMethodV2,
    loop_handle: LoopHandle<'static, State>,
    /// For the region-picking thread to report back on.
    commands: channel::Sender<Command>,
    /// A region is being picked; the balloon is hidden so it isn't captured.
    picking: bool,
    primary: Primary,
    /// Files dragged onto the card.
    drops: Drops,
    /// Makes the thumbnails files on the card show.
    thumbnailer: Thumbnailer,
    shell: Shell,
    cursor_shape: Option<WpCursorShapeDeviceV1>,
    /// The balloon's surface, for as long as something is being gathered.
    panel: Option<Panel>,
    /// The field as announced, applied on the next `done`.
    pending: Field,
    field: Field,
    /// `None` when not gathering.
    gathering: Option<Gathering>,
    /// Where the gathering's text is written for Ask.
    gathering_dir: PathBuf,
    /// The bus client showing the gathering in the card's place.
    held_by: Option<String>,
    /// Where changes to the gathering go out on the bus, and the last that
    /// did.
    announcements: tokio::sync::mpsc::UnboundedSender<Items>,
    last_announced: Items,
    balloon: Balloon,
    /// The card as last drawn; `None` when the items changed since.
    layout: Option<Layout>,
    /// The button a left press went down on, acted on if released there.
    pressed: Option<Hit>,
    /// The pointer's last enter, for setting its shape.
    pointer_serial: u32,
    /// For asking Otto's settings over the bus.
    runtime: tokio::runtime::Handle,
    /// The button under the pointer, which gets the hand cursor.
    hovering: Option<Hit>,
    /// The item under the pointer, which is highlighted.
    hovered_item: Option<usize>,
    /// The item shrinking away after it was removed, and since when.
    leaving: Option<(usize, Instant)>,
    /// The item a left press went down on, and where the card was then: if
    /// it is released there without the card having moved, it is a click.
    pressed_item: Option<(usize, Option<(i32, i32)>)>,
    /// How far the items are scrolled, and their glide and bounce.
    scroll: ScrollView,
    /// A frame callback is on its way, to advance the scroll.
    frame_pending: bool,
    /// The axis events of this pointer frame are a notched wheel's.
    wheel_discrete: bool,
}

impl State {
    fn on_command(&mut self, command: Command) {
        match command {
            Command::Add(done) => {
                let selection = self
                    .field
                    .surrounding
                    .as_ref()
                    .and_then(Surrounding::selection)
                    .map(str::to_owned);
                let gathering = self.start_gathering();
                match selection {
                    Some(text) => {
                        tracing::info!(chars = text.chars().count(), "add selection");
                        gathering.add(Item::Text(text));
                    }
                    // Files has no text field to report; it says what is
                    // selected when asked.
                    None => {
                        let commands = self.commands.clone();
                        self.runtime.spawn(async move {
                            let files = dbus::focused_files().await;
                            let _ = commands.send(Command::AddFocusedFiles(files, done));
                        });
                        return self.refresh();
                    }
                }
            }
            Command::AddFocusedFiles(files, done) => {
                if files.is_empty() {
                    self.add_primary(done);
                    return self.refresh();
                }
                let gathering = self.start_gathering();
                for file in files {
                    tracing::info!(path = %file.display(), "add selected file");
                    gathering.add(Item::File(file));
                }
            }
            Command::AddFile(path) => {
                tracing::info!(path = %path.display(), "add file");
                self.start_gathering().add(Item::File(path));
            }
            Command::AddRegion => self.pick_region(),
            Command::RegionCaptured(path) => {
                self.picking = false;
                if let Some(path) = path {
                    tracing::info!(path = %path.display(), "add region");
                    self.start_gathering().add(Item::Region(path));
                }
            }
            // Ask shows what is gathered as it opens, as it does when opened
            // any other way.
            Command::Send => {
                if self.gathering.is_some() {
                    if let Err(error) = open_ask() {
                        tracing::error!(error = format!("{error:#}"), "cannot open Ask");
                    }
                }
            }
            Command::Items(reply) => {
                let _ = reply.send(self.items());
            }
            Command::Hold(holder) => {
                tracing::info!(%holder, "held");
                self.held_by = Some(holder);
            }
            Command::Release(holder) => {
                if self.held_by.as_ref() == Some(&holder) {
                    tracing::info!(%holder, "released");
                    self.held_by = None;
                }
            }
            Command::Toggle(index) => {
                if let Some(gathering) = self.gathering.as_mut() {
                    gathering.toggle(index);
                }
            }
            Command::Remove(index) => self.remove_item(index),
            Command::Sent => {
                if self.gathering.take().is_some() {
                    tracing::info!("sent to Ask");
                }
            }
            Command::Cancel => {
                if self.gathering.take().is_some() {
                    tracing::info!("gathering thrown away");
                }
            }
        }
        self.refresh();
    }

    /// The gathering, started if there is none.
    fn start_gathering(&mut self) -> &mut Gathering {
        if self.gathering.is_none() {
            tracing::info!("start gathering");
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis());
            self.gathering_dir = runtime_dir().join("otto-gather").join(stamp.to_string());
        }
        self.gathering.get_or_insert_with(Gathering::default)
    }

    /// Everything gathered, as files; nothing when the text can't be
    /// written.
    fn items(&self) -> Items {
        let Some(gathering) = self.gathering.as_ref() else {
            return Items::new();
        };
        gathering
            .hand_over(&self.gathering_dir)
            .unwrap_or_else(|error| {
                let dir = self.gathering_dir.display();
                tracing::error!(%error, %dir, "cannot write the selections");
                Items::new()
            })
    }

    /// Take the item at `index` out; taking the last one out ends the
    /// gathering.
    fn remove_item(&mut self, index: usize) {
        // On the card, it shrinks away first; see `on_frame`.
        if self.panel.is_some() && self.held_by.is_none() {
            let index = match self.leaving.take() {
                Some((gone, _)) => {
                    self.remove_now(gone);
                    if index > gone {
                        index - 1
                    } else if index == gone {
                        return;
                    } else {
                        index
                    }
                }
                None => index,
            };
            if self
                .gathering
                .as_ref()
                .is_some_and(|g| index < g.items.len())
            {
                self.leaving = Some((index, Instant::now()));
                self.hovering = None;
                self.hovered_item = None;
                self.layout = None;
                self.draw_panel();
            }
            return;
        }
        self.remove_now(index);
    }

    /// Take the item at `index` out at once.
    fn remove_now(&mut self, index: usize) {
        if let Some(gathering) = self.gathering.as_mut() {
            gathering.remove(index);
            if gathering.items.is_empty() {
                self.gathering = None;
            }
        }
        self.hovering = None;
        self.hovered_item = None;
    }

    /// Hide the balloon and let the user pick a region; the capture comes
    /// back as [`Command::RegionCaptured`].
    fn pick_region(&mut self) {
        if self.picking {
            return;
        }
        self.picking = true;
        let dir = runtime_dir().join("otto-gather");
        let commands = self.commands.clone();
        // slurp and grim block until the user is done, so off this thread.
        let spawned = std::thread::Builder::new()
            .name("otto-gather-region".into())
            .spawn(move || {
                let path = region::capture(&dir)
                    .inspect_err(|error| {
                        tracing::warn!(error = format!("{error:#}"), "cannot capture a region");
                    })
                    .ok()
                    .flatten();
                let _ = commands.send(Command::RegionCaptured(path));
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "cannot start picking a region");
            self.picking = false;
        }
    }

    /// Add the primary selection, once the app that owns it has written it,
    /// and signal `done` then.
    fn add_primary(&mut self, done: dbus::Done) {
        let Some(reader) = self.primary.receive() else {
            tracing::info!("nothing is selected");
            return;
        };
        let mut done = done;
        let mut text = Vec::new();
        let source = Generic::new(reader, Interest::READ, Mode::Level);
        let inserted = self
            .loop_handle
            .insert_source(source, move |_, reader, state| {
                let mut chunk = [0; 16 * 1024];
                let read = std::io::Read::read(&mut reader.as_ref(), &mut chunk)?;
                if read > 0 && text.len() < PRIMARY_MAX_BYTES {
                    text.extend_from_slice(&chunk[..read]);
                    return Ok(PostAction::Continue);
                }
                text.truncate(PRIMARY_MAX_BYTES);
                state.on_primary_read(&String::from_utf8_lossy(&text));
                if let Some(done) = done.take() {
                    let _ = done.send(());
                }
                Ok(PostAction::Remove)
            });
        if let Err(error) = inserted {
            tracing::warn!(%error, "cannot read the primary selection");
        }
    }

    /// Add the files dropped with `offer`, once the app they came from has
    /// written them.
    fn receive_drop(&mut self, offer: wl_data_offer::WlDataOffer) {
        let pipe = std::io::pipe();
        let (reader, writer) = match pipe {
            Ok(pipe) => pipe,
            Err(error) => {
                tracing::warn!(%error, "no pipe for the dropped files");
                offer.destroy();
                return;
            }
        };
        offer.receive(URI_LIST.into(), std::os::fd::AsFd::as_fd(&writer));
        drop(writer);
        let mut list = Vec::new();
        let mut offer = Some(offer);
        let source = Generic::new(reader, Interest::READ, Mode::Level);
        let inserted = self
            .loop_handle
            .insert_source(source, move |_, reader, state| {
                let mut chunk = [0; 16 * 1024];
                let read = std::io::Read::read(&mut reader.as_ref(), &mut chunk)?;
                if read > 0 {
                    list.extend_from_slice(&chunk[..read]);
                    return Ok(PostAction::Continue);
                }
                if let Some(offer) = offer.take() {
                    if offer.version() >= 3 {
                        offer.finish();
                    }
                    offer.destroy();
                }
                let (files, _) = otto_kit::clipboard::parse_file_payload(URI_LIST, &list);
                for file in files {
                    state.on_command(Command::AddFile(file));
                }
                Ok(PostAction::Remove)
            });
        if let Err(error) = inserted {
            tracing::warn!(%error, "cannot read the dropped files");
        }
    }

    fn on_primary_read(&mut self, text: &str) {
        let Some(gathering) = self.gathering.as_mut() else {
            // Sent or cancelled while the app was still writing.
            return;
        };
        if text.trim().is_empty() {
            tracing::info!("nothing is selected");
            return;
        }
        tracing::info!(chars = text.chars().count(), "add primary selection");
        gathering.add(Item::Text(text.to_owned()));
        self.refresh();
    }

    /// Show the thumbnails made since the last time.
    fn take_thumbnails(&mut self) {
        let made = self.thumbnailer.take();
        if made.is_empty() {
            return;
        }
        for (path, image) in made {
            self.balloon.set_thumbnail(&path, image);
        }
        self.layout = None;
        self.draw_panel();
    }

    /// Tell whoever follows the gathering, if it changed.
    fn announce(&mut self) {
        let items = self.items();
        if items != self.last_announced {
            self.last_announced = items.clone();
            let _ = self.announcements.send(items);
        }
    }

    /// Show, redraw or remove the balloon to match the gathering. It stays
    /// up from the first add until the gathering is sent or cancelled.
    fn refresh(&mut self) {
        self.announce();
        self.layout = None;
        // Held, Ask shows the gathering in its place.
        if self.gathering.is_none() {
            self.leaving = None;
        }
        if self.gathering.is_none() || self.picking || self.held_by.is_some() {
            self.close_panel();
            return;
        }
        if self.panel.is_none() {
            let panel = Panel::new(&self.shell, &self.qh, BALLOON_SCALE);
            // Asked each time the card opens, so a rebound shortcut shows.
            self.balloon.send_shortcut = self.runtime.block_on(dbus::send_shortcut());
            self.balloon.frosted = panel.frosted();
            self.panel = Some(panel);
            self.scroll = ScrollView::new(Rect::default());
        }
        self.draw_panel();
    }

    /// Draw the balloon, once the compositor has sized the overlay.
    fn draw_panel(&mut self) {
        let (Some(gathering), Some(panel)) = (self.gathering.as_ref(), self.panel.as_mut()) else {
            return;
        };
        if !panel.configured() {
            return;
        }
        let layout = match &mut self.layout {
            Some(layout) => layout,
            empty => {
                let leaving = self.leaving.map(|(index, since)| {
                    let t = (since.elapsed().as_secs_f32() / LEAVE.as_secs_f32()).min(1.0);
                    // Ease in: it holds a moment, then goes.
                    (index, 1.0 - t * t)
                });
                let layout = self
                    .balloon
                    .layout(gathering, leaving, panel.max_card_height());
                for path in self.balloon.thumbnails_wanted() {
                    self.thumbnailer.request(path);
                }
                self.scroll.set_viewport(layout.viewport);
                self.scroll.set_content_length(layout.body_length);
                // Still in range after an item was taken out.
                self.scroll.scroll_to(self.scroll.offset());
                empty.insert(layout)
            }
        };
        panel.set_card_size(
            (layout.width as i32, layout.height as i32),
            self.leaving.is_some(),
        );
        let scale = BALLOON_SCALE as f32;
        let w = (layout.width * scale) as i32;
        let h = (panel.buffer_height(layout.height) * scale) as i32;
        // Reuse the buffer when it is the right size and the compositor has
        // released it; otherwise take a fresh one from the pool.
        let reusable = self.buffer.as_mut().is_some_and(|b| {
            b.height() == h && b.stride() == w * 4 && b.canvas(&mut self.pool).is_some()
        });
        if !reusable {
            match self
                .pool
                .create_buffer(w, h, w * 4, wl_shm::Format::Argb8888)
            {
                Ok((buffer, _)) => self.buffer = Some(buffer),
                Err(error) => {
                    tracing::warn!(%error, "no buffer for the balloon");
                    return;
                }
            }
        }
        let Some(buffer) = self.buffer.as_mut() else {
            return;
        };
        let Some(canvas) = buffer.canvas(&mut self.pool) else {
            return;
        };
        self.balloon.draw(
            layout,
            &self.scroll,
            self.hovered_item,
            canvas,
            (w, h),
            scale,
        );
        if buffer.attach_to(panel.card()).is_ok() {
            panel.card().damage_buffer(0, 0, w, h);
            if (self.scroll.is_animating() || self.leaving.is_some()) && !self.frame_pending {
                panel.card().frame(&self.qh, ());
                self.frame_pending = true;
            }
            panel.card().commit();
        }
    }

    /// The items were scrolled: redraw, and keep going while they glide.
    fn on_scrolled(&mut self, moved: bool) {
        // What is under the pointer moved with the items.
        self.update_cursor();
        if moved || (self.scroll.is_animating() && !self.frame_pending) {
            self.draw_panel();
        }
    }

    /// A frame went up: advance the glide and draw the next one.
    fn on_frame(&mut self) {
        self.frame_pending = false;
        if self.panel.is_none() {
            return;
        }
        self.scroll.tick();
        if let Some((index, since)) = self.leaving {
            self.layout = None;
            if since.elapsed() >= LEAVE {
                self.leaving = None;
                self.remove_now(index);
                return self.refresh();
            }
        }
        // Draws where the glide is now, and asks for another frame while it
        // is still going.
        self.draw_panel();
    }

    /// The button at the pointer, if it is over one on the card.
    fn hit_at_pointer(&self) -> Option<Hit> {
        let (x, y) = self.panel.as_ref()?.pointer_on_card()?;
        self.layout
            .as_ref()?
            .hit(x as f32, y as f32, self.scroll.offset())
    }

    /// The item at the pointer, if it is over one on the card.
    fn item_at_pointer(&self) -> Option<usize> {
        let (x, y) = self.panel.as_ref()?.pointer_on_card()?;
        self.layout
            .as_ref()?
            .item_at(x as f32, y as f32, self.scroll.offset())
    }

    /// Highlight the item under the pointer, if it changed.
    fn update_hovered_item(&mut self) {
        let hovered = self.item_at_pointer();
        if hovered != self.hovered_item {
            self.hovered_item = hovered;
            self.draw_panel();
        }
    }

    /// Strike the item out, or bring it back.
    fn toggle_item(&mut self, index: usize) {
        if let Some(gathering) = self.gathering.as_mut() {
            gathering.toggle(index);
            self.refresh();
        }
    }

    /// Show the hand over buttons and the arrow elsewhere.
    fn update_cursor(&mut self) {
        self.update_hovered_item();
        let hovering = self.hit_at_pointer();
        if hovering == self.hovering {
            return;
        }
        self.hovering = hovering;
        if let Some(device) = self.cursor_shape.as_ref() {
            let shape = match hovering {
                Some(_) => wp_cursor_shape_device_v1::Shape::Pointer,
                None => wp_cursor_shape_device_v1::Shape::Default,
            };
            device.set_shape(self.pointer_serial, shape);
        }
    }

    /// Do what a click on `hit` asks for.
    fn on_click(&mut self, hit: Hit) {
        match hit {
            Hit::Remove(index) => {
                self.remove_item(index);
                self.refresh();
            }
        }
    }

    fn close_panel(&mut self) {
        if let Some(panel) = self.panel.take() {
            panel.destroy();
        }
    }
}

/// Where gathered things are written: the user's runtime directory.
fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from)
}

/// Open Ask. It shows what is gathered, following it over the bus.
fn open_ask() -> anyhow::Result<()> {
    let launcher = std::env::var("OTTO_GATHER_LAUNCHER").unwrap_or_else(|_| "otto-launcher".into());
    tracing::info!(%launcher, "open Ask");
    // Not waited for: the launcher runs until it is closed. A thread reaps it
    // so it doesn't linger as a zombie.
    let mut child = Process::new(&launcher)
        .arg("--ask")
        .spawn()
        .with_context(|| format!("cannot start {launcher}"))?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

impl Dispatch<ZwpInputMethodV2, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => {
                // Activation resets the field's state.
                state.pending = Field {
                    active: true,
                    surrounding: None,
                };
            }
            zwp_input_method_v2::Event::Deactivate => state.pending.active = false,
            zwp_input_method_v2::Event::SurroundingText {
                text,
                cursor,
                anchor,
            } => {
                tracing::debug!(bytes = text.len(), cursor, anchor, "surrounding text");
                state.pending.surrounding = Some(Surrounding {
                    text,
                    cursor,
                    anchor,
                });
            }
            zwp_input_method_v2::Event::Done => {
                if state.field.active != state.pending.active {
                    tracing::debug!(active = state.pending.active, "field focus changed");
                }
                state.field = state.pending.clone();
            }
            zwp_input_method_v2::Event::Unavailable => {
                tracing::error!("another input method is already bound; exiting");
                std::process::exit(1);
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                match state.panel.as_mut() {
                    Some(panel) => panel.configure(serial, (width, height), &mut state.pool),
                    None => layer.ack_configure(serial),
                }
                state.draw_panel();
            }
            zwlr_layer_surface_v1::Event::Closed => {
                // The output went away; the next change opens a new one.
                state.close_panel();
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(panel) = state.panel.as_mut() else {
            return;
        };
        match event {
            wl_pointer::Event::Enter {
                serial,
                surface,
                surface_x,
                surface_y,
            } if panel.takes_pointer(&surface) => {
                // Not held yet on entering, so the card doesn't move.
                panel.pointer_moved(surface_x, surface_y);
                state.pointer_serial = serial;
                state.hovering = None;
                if let Some(device) = state.cursor_shape.as_ref() {
                    device.set_shape(serial, wp_cursor_shape_device_v1::Shape::Default);
                }
                state.update_cursor();
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                if panel.pointer_moved(surface_x, surface_y) {
                    state.draw_panel();
                    return;
                }
                let on_card = panel.pointer_on_card();
                state.update_cursor();
                if let Some((x, y)) = on_card {
                    let (x, y) = (x as f32, y as f32);
                    let dragged = state.scroll.on_pointer_drag(x, y);
                    let hovered = state.scroll.on_pointer_move(x, y);
                    state.on_scrolled(dragged || hovered);
                }
            }
            wl_pointer::Event::AxisDiscrete { .. } => state.wheel_discrete = true,
            wl_pointer::Event::Axis {
                axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
                value,
                ..
            } if panel.pointer_on_card().is_some() => {
                let delta = value as f32;
                let moved = if state.wheel_discrete {
                    state.scroll.on_wheel_discrete(delta)
                } else {
                    state.scroll.on_wheel(delta)
                };
                state.on_scrolled(moved);
            }
            wl_pointer::Event::AxisStop {
                axis: WEnum::Value(wl_pointer::Axis::VerticalScroll),
                ..
            } => {
                state.scroll.on_wheel_end();
                state.on_scrolled(false);
            }
            wl_pointer::Event::Frame => state.wheel_discrete = false,
            wl_pointer::Event::Button {
                button: BTN_LEFT,
                state: WEnum::Value(pressed),
                ..
            } => match pressed {
                wl_pointer::ButtonState::Pressed => {
                    // A press on a button clicks it, one on the scrollbar
                    // scrolls, and anywhere else drags the card.
                    state.pressed = state.hit_at_pointer();
                    let on_card = state.panel.as_ref().and_then(Panel::pointer_on_card);
                    let on_thumb = on_card
                        .is_some_and(|(x, y)| state.scroll.on_pointer_down(x as f32, y as f32));
                    if state.pressed.is_none() && !on_thumb {
                        let item = state.item_at_pointer();
                        if let Some(panel) = state.panel.as_mut() {
                            state.pressed_item = item.map(|index| (index, panel.card_origin()));
                            panel.pointer_pressed();
                        }
                    }
                }
                _ => {
                    state.scroll.on_pointer_up();
                    if let Some(panel) = state.panel.as_mut() {
                        panel.pointer_released();
                    }
                    let pressed = state.pressed.take();
                    if let Some(hit) = pressed.filter(|&hit| state.hit_at_pointer() == Some(hit)) {
                        state.on_click(hit);
                    }
                    // Clicked, not dragged: the card is where it was.
                    let origin = state.panel.as_ref().map(Panel::card_origin);
                    if let Some((index, at)) = state.pressed_item.take() {
                        if state.item_at_pointer() == Some(index) && origin == Some(at) {
                            state.toggle_item(index);
                        }
                    }
                }
            },
            wl_pointer::Event::Leave { .. } => {
                panel.pointer_released();
                state.pressed = None;
                state.scroll.on_pointer_up();
                state.scroll.on_pointer_leave();
                state.pressed_item = None;
                if state.hovered_item.take().is_some() {
                    state.draw_panel();
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_shm!(State);
delegate_noop!(State: ignore wl_compositor::WlCompositor);
delegate_noop!(State: ignore wl_surface::WlSurface);
delegate_noop!(State: ignore wl_seat::WlSeat);
delegate_noop!(State: ignore ZwpInputMethodManagerV2);
delegate_noop!(State: ignore ZwlrDataControlManagerV1);
delegate_noop!(State: ignore ZwlrLayerShellV1);
delegate_noop!(State: ignore wl_region::WlRegion);
delegate_noop!(State: ignore wl_data_device_manager::WlDataDeviceManager);
delegate_noop!(State: ignore wl_subcompositor::WlSubcompositor);
delegate_noop!(State: ignore wl_subsurface::WlSubsurface);
delegate_noop!(State: ignore WpCursorShapeManagerV1);
delegate_noop!(State: ignore WpCursorShapeDeviceV1);
delegate_noop!(State: ignore otto_surface_style_manager_v1::OttoSurfaceStyleManagerV1);
delegate_noop!(State: ignore otto_surface_style_v1::OttoSurfaceStyleV1);
delegate_noop!(State: ignore otto_style_transaction_v1::OttoStyleTransactionV1);
delegate_noop!(State: ignore otto_timing_function_v1::OttoTimingFunctionV1);

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.on_frame();
        }
    }
}
