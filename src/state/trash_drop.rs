//! Files dropped on the dock's Trash icon.
//!
//! Here the compositor is the drop target itself. While a drag is over the
//! dock, a [`TrashOffer`] answers the source in place of a client's
//! `wl_data_offer`: it takes `text/uri-list` and asks for a move, but only
//! while the pointer is over the Trash. On the drop it reads the list down a
//! pipe, tells the source it is done, and moves the files into the trash can
//! on a thread of their own, since a folder on another disk is a copy.
//!
//! The move is the target's job. Otto's own file drags leave their files
//! where they are when a move finishes, as other file managers do, so nobody
//! else deletes anything.

use std::io::Read;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use otto_kit::clipboard::{parse_file_payload, URI_LIST, URI_LIST_WITH_ACTION};
use smithay::{
    input::dnd::{DndAction, OfferData, Source},
    reexports::calloop::{
        generic::Generic,
        timer::{TimeoutAction, Timer},
        Interest, Mode, PostAction,
    },
    utils::{Logical, Point},
};

use super::{Backend, Otto};

/// How long a source gets to write its list before the read is abandoned.
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(10);

/// The dock's side of a drag it could take.
pub struct TrashOffer<S: Source> {
    source: Arc<S>,
    /// The file list type the source offers, if it offers one.
    mime: Option<&'static str>,
    /// Move if the source allows it, since that is what a trip to the trash
    /// is, and copy otherwise: the files end up in the trash either way.
    action: DndAction,
    /// `None` until the first position is known, so the first one always
    /// answers the source.
    over_trash: Option<bool>,
}

impl<S: Source> TrashOffer<S> {
    pub fn new(source: Arc<S>) -> Self {
        let metadata = source.metadata().unwrap_or_default();
        let mime = [URI_LIST, URI_LIST_WITH_ACTION]
            .into_iter()
            .find(|mime| metadata.mime_types.iter().any(|m| m == mime));
        let action = [DndAction::Move, DndAction::Copy]
            .into_iter()
            .find(|action| metadata.dnd_actions.contains(action))
            .unwrap_or(DndAction::None);
        Self {
            source,
            mime,
            action,
            over_trash: None,
        }
    }

    /// Follow the pointer on or off the Trash, telling the source whether a
    /// drop here would be taken. Returns the action now chosen, when it
    /// changed.
    fn set_over_trash(&mut self, over_trash: bool) -> Option<DndAction> {
        if self.over_trash == Some(over_trash) {
            return None;
        }
        self.over_trash = Some(over_trash);
        let action = if self.accepts() {
            self.action
        } else {
            DndAction::None
        };
        self.source.choose_action(action);
        Some(action)
    }

    fn accepts(&self) -> bool {
        self.over_trash == Some(true) && self.mime.is_some() && self.action != DndAction::None
    }
}

impl<S: Source> OfferData for TrashOffer<S> {
    fn disable(&self) {
        self.source.choose_action(DndAction::None);
    }

    fn drop(&self) {}

    fn validated(&self) -> bool {
        self.accepts()
    }
}

impl<B: Backend + 'static> Otto<B> {
    /// A drag moved over the dock at `location`, global and logical.
    pub(crate) fn trash_drag_motion<S: Source>(
        &mut self,
        offer: Option<&mut TrashOffer<S>>,
        location: Point<f64, Logical>,
    ) {
        let over_trash = self
            .dock_local_px(location)
            .is_some_and(|local_px| self.workspaces.dock.file_drag_motion(location, local_px));
        // The cursor says what a drop would do, as it does over a client
        // that negotiated an action.
        if let Some(action) = offer.and_then(|offer| offer.set_over_trash(over_trash)) {
            self.load_cursor_for_action(action.into());
        }
    }

    /// The drag was released over the dock. A drop on the Trash starts the
    /// transfer, once the grab has told the source the drop happened.
    pub(crate) fn trash_drop<S: Source>(&mut self, offer: Option<&mut TrashOffer<S>>) {
        self.workspaces.dock.file_drag_leave();
        let Some(offer) = offer.filter(|offer| offer.accepts()) else {
            return;
        };
        let (Some(mime), source) = (offer.mime, offer.source.clone()) else {
            return;
        };
        self.handle
            .insert_idle(move |state| state.receive_trash_drop(source, mime));
    }

    /// Where `location` falls on the dock's output, in its physical pixels:
    /// the space the dock's layers are laid out in.
    fn dock_local_px(&self, location: Point<f64, Logical>) -> Option<(f32, f32)> {
        let output = self.workspaces.primary_output()?;
        let origin = self.workspaces.output_geometry(output)?.loc;
        let scale = output.current_scale().fractional_scale();
        let local = Point::<f64, Logical>::from((
            location.x - origin.x as f64,
            location.y - origin.y as f64,
        ))
        .to_physical(scale);
        Some((local.x as f32, local.y as f32))
    }

    /// Ask the source for its file list and read it without blocking the
    /// compositor. The source hears it is finished once the list is in.
    fn receive_trash_drop<S: Source>(&mut self, source: Arc<S>, mime: &'static str) {
        // Otto's own can, the one Files trashes into. `[dock] trash_path`
        // only says which directory the icon watches.
        let Some(trash) = otto_kit::trash::home_trash_dir() else {
            source.finished();
            return;
        };
        let (reader, writer) = match std::io::pipe() {
            Ok(pipe) => pipe,
            Err(err) => {
                tracing::warn!("trash drop: no pipe for the transfer: {err}");
                source.finished();
                return;
            }
        };
        let reader = OwnedFd::from(reader);
        // SAFETY: fcntl on a descriptor this function owns.
        unsafe {
            let fd = std::os::fd::AsRawFd::as_raw_fd(&reader);
            let flags = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        // The write end goes to the source, and is closed here once sent, so
        // the source closing its copy is the end of the list.
        source.send(mime, OwnedFd::from(writer));

        let mut payload = Vec::new();
        let finished_source = source.clone();
        let read = self.handle.insert_source(
            Generic::new(std::fs::File::from(reader), Interest::READ, Mode::Level),
            move |_, file, _state| {
                let mut chunk = [0u8; 4096];
                loop {
                    // SAFETY: the file stays owned by this source; it is only read.
                    match unsafe { file.get_mut() }.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => payload.extend_from_slice(&chunk[..n]),
                        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                            return Ok(PostAction::Continue);
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(err) => {
                            tracing::warn!("trash drop: reading the file list failed: {err}");
                            break;
                        }
                    }
                }
                finished_source.finished();
                let (paths, _) = parse_file_payload(mime, &payload);
                throw_away(paths, trash.clone());
                Ok(PostAction::Remove)
            },
        );
        let token = match read {
            Ok(token) => token,
            Err(err) => {
                tracing::warn!("trash drop: cannot watch the transfer: {err}");
                source.finished();
                return;
            }
        };
        // A source that never closes its end would hold the pipe open for
        // good; give up on it after a while. Removing a source that already
        // finished does nothing.
        let _ = self.handle.insert_source(
            Timer::from_duration(TRANSFER_TIMEOUT),
            move |_, _, state| {
                state.handle.remove(token);
                TimeoutAction::Drop
            },
        );
    }
}

/// Move `paths` into the trash can at `trash`, off the compositor's thread.
fn throw_away(paths: Vec<PathBuf>, trash: PathBuf) {
    if paths.is_empty() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("otto-trash-drop".into())
        .spawn(move || {
            for path in paths {
                if let Err(err) = otto_kit::trash::trash_into(&path, &trash) {
                    tracing::warn!("trash drop: {}: {err}", path.display());
                }
            }
        });
    if let Err(err) = spawned {
        tracing::warn!("trash drop: cannot start the move: {err}");
    }
}
