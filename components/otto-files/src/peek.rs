//! Peek, embedded — the Space-bar preview.
//!
//! Peek used to be a separate application reached over `org.otto.Peek1`.
//! It is now a library this window embeds, because the preview wants to be a
//! subsurface of the file view and a subsurface's parent must be a `wl_surface`
//! owned by the same client: "parented to the browser" and "separate process"
//! are mutually exclusive. Being parented also dissolves the anchor problem —
//! the row's rect is already in this surface's coordinates — and hands
//! stacking, focus and dismissal to this window instead of leaving them to be
//! managed by hand.
//!
//! What is still a separate process is the *decoder*. Untrusted bytes are
//! parsed by a sandboxed worker that is this binary re-executed, which is why
//! `main` must call [`otto_peek::run_worker_if_requested`] before anything
//! else. This module never interprets file bytes; it receives a validated
//! [`Preview`] and draws it.

use std::path::Path;
use std::time::Instant;

use otto_kit::components::scroll::{Axis, ScrollState, ScrollView};
use otto_kit::preview::{self, Page, Pixels, Preview, Word, WordSelection, Zoom};
use otto_media_kit::transport::TransportHit;
use otto_media_kit::{Frame, Options, Playback, Player, State};
use otto_peek::decode::Request;
use otto_peek::opening;
use skia_safe::{Contains, Rect};

/// The title strip along the top of the panel: the file's name, and the
/// close button. The preview's content starts below it, so neither ever
/// draws over the other.
pub const TITLEBAR_H: f32 = 30.0;

/// The panel's share of the window, and the least it may shrink to. A preview
/// that filled the window would stop reading as something laid over the file
/// list; one that scaled without a floor would be useless in a small window.
pub const PANEL_FRACTION: f32 = 0.72;
pub const PANEL_MIN: (f32, f32) = (420.0, 320.0);

/// The pan of a zoomed picture, run as a pair of scroll views.
///
/// Panning a picture is scrolling: the box shows part of something bigger,
/// and a two-finger gesture moves which part. Everything the toolkit already
/// knows about that — a gesture that goes on gliding after the fingers lift,
/// a bar that says how much of the content is in view and can be grabbed to
/// move it — is in [`ScrollView`], and a pan written by hand has none of it.
///
/// The rubber band takes one more step than the rest. Where a picture may sit
/// is geometry [`otto_kit::preview::clamp_zoom`] owns, and it clamps — so the
/// overshoot a view is holding travels in [`Zoom::band`], which nothing
/// clamps and only the drawing reads. Pull past an edge and the picture
/// stretches with the fingers and springs home when they lift, while
/// everything that asks how far there is left to pan still gets an answer
/// inside the picture's own limits.
///
/// A scroll view is one-dimensional, so a picture takes two of them sharing
/// the same viewport: the box the image is drawn in. They own the pan while a
/// gesture or a fling is running; [`Session::zoom`] is where it is read back
/// from, because everything that draws or hit-tests a preview measures
/// against a `Zoom`. [`Session::pull_pan`] copies one into the other after
/// every step, and [`Session::push_pan`] goes the other way for the one thing
/// that moves the picture without scrolling it — a pinch.
pub struct Pan {
    x: ScrollView,
    y: ScrollView,
}

impl Pan {
    fn new() -> Self {
        Self {
            x: ScrollView::on_axis(Axis::Horizontal, Rect::new_empty()),
            y: ScrollView::on_axis(Axis::Vertical, Rect::new_empty()),
        }
    }
}

/// A video being played inside a session.
pub struct Video {
    pub player: Player,
    /// The card's own artwork, drawn until the first frame arrives.
    pub poster: Option<Pixels>,
    /// A drag along the scrubber in progress, as a fraction of the duration.
    pub scrubbing: Option<f32>,
    /// Whether the video was playing when the drag began, so it resumes
    /// when the drag ends.
    resume: bool,
}

/// Everything a drawing of the video needs, detached from the player: for a
/// host that records its drawing into a picture, which must own what it
/// draws from.
pub struct VideoSnapshot {
    pub frame: Option<Frame>,
    pub state: State,
    pub scrubbing: Option<f32>,
    /// The poster, carried only until the first frame makes it moot.
    pub poster: Option<Pixels>,
}

impl VideoSnapshot {
    /// The video's width over its height, once anything has said what that
    /// is — the frame first, then the stream's announced size, then the
    /// poster. `None` until the worker has answered, when the caller fills
    /// the space it has and corrects on the next frame.
    pub fn aspect(&self) -> Option<f32> {
        let (w, h) = self
            .frame
            .as_ref()
            .map(|f| (f.width, f.height))
            .or(self.state.size)
            .or_else(|| self.poster.as_ref().map(|p| (p.width, p.height)))?;
        (w > 0 && h > 0).then(|| w as f32 / h as f32)
    }
}

impl Video {
    /// Start playing `path`, if the decoded `preview` says it is a video
    /// and a player can be started.
    ///
    /// Only for a preview [`otto_peek::payload::is_video`] vouches for:
    /// the sandboxed decoder read the bytes and said video, which is the one
    /// opinion that counts before a demuxer is handed the file. A worker
    /// that cannot be found or started leaves the card as it was, and says
    /// why in the log rather than on the panel — the card is a complete
    /// preview on its own.
    pub fn open(
        preview: &Preview,
        path: &Path,
        options: Options,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Option<Video> {
        if !otto_peek::payload::is_video(preview) {
            return None;
        }
        match Player::open(path, options, wake) {
            Ok(player) => {
                let poster = match preview {
                    Preview::Card { hero, .. } => hero.clone(),
                    _ => None,
                };
                Some(Video {
                    player,
                    poster,
                    scrubbing: None,
                    resume: false,
                })
            }
            Err(err) => {
                tracing::info!("no video playback for {}: {err}", path.display());
                None
            }
        }
    }

    /// The playback's contribution to a repaint key: everything the player
    /// view draws that changes without anything around it moving.
    pub fn key(&self) -> u64 {
        let state = self.player.state();
        let playing = state.playback == Playback::Playing;
        let millis = state.position().as_millis() as u64;
        let scrub = self
            .scrubbing
            .map(|fraction| fraction.to_bits() as u64)
            .unwrap_or(0);
        state.frame_seq.rotate_left(29)
            ^ millis.rotate_left(11)
            ^ (playing as u64) << 3
            ^ (state.playback as u64) << 5
            ^ ((state.volume <= 0.0) as u64) << 9
            ^ scrub.rotate_left(41)
    }

    /// The newest frame's sequence number; 0 before any has arrived.
    pub fn frame_seq(&self) -> u64 {
        self.player.state().frame_seq
    }

    /// What to draw, detached from the player.
    pub fn snapshot(&self) -> VideoSnapshot {
        let frame = self.player.frame();
        VideoSnapshot {
            poster: if frame.is_none() {
                self.poster.clone()
            } else {
                None
            },
            frame,
            state: self.player.state(),
            scrubbing: self.scrubbing,
        }
    }

    /// The pointer over the video, drawn in `content`. Returns whether it
    /// was taken.
    pub fn pointer(&mut self, kind: VideoPointer, x: f32, y: f32, content: Rect) -> bool {
        let layout = otto_media_kit::view::transport_layout(content);
        let point = skia_safe::Point::new(x, y);
        let state = self.player.state();
        let seek_to = |fraction: f32| state.duration.map(|duration| duration.mul_f32(fraction));
        match kind {
            VideoPointer::Press => {
                match layout.hit(point) {
                    Some(TransportHit::PlayPause) => self.player.toggle(),
                    Some(TransportHit::Mute) => {
                        let volume = if state.volume <= 0.0 { 1.0 } else { 0.0 };
                        self.player.set_volume(volume);
                    }
                    Some(TransportHit::Scrub(fraction)) => {
                        self.scrubbing = Some(fraction);
                        self.resume = state.playback == Playback::Playing;
                        if self.resume {
                            self.player.pause();
                        }
                        if let Some(position) = seek_to(fraction) {
                            self.player.seek(position, false);
                        }
                    }
                    Some(TransportHit::Bar) => {}
                    // The picture itself is the biggest play/pause button
                    // there is, as in every player.
                    None if content.contains(point) => self.player.toggle(),
                    None => return false,
                }
                true
            }
            VideoPointer::Motion => {
                if self.scrubbing.is_none() {
                    return false;
                }
                let fraction = layout.fraction_at(x);
                self.scrubbing = Some(fraction);
                if let Some(position) = seek_to(fraction) {
                    self.player.seek(position, false);
                }
                true
            }
            VideoPointer::Release | VideoPointer::Leave => {
                let Some(fraction) = self.scrubbing.take() else {
                    return false;
                };
                if let Some(position) = seek_to(fraction) {
                    self.player.seek(position, true);
                }
                if self.resume {
                    self.player.play();
                }
                true
            }
        }
    }
}

/// What the pointer did over a video, in the host's own vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoPointer {
    Press,
    Motion,
    Release,
    Leave,
}

/// How to open a video for a box of `panel` points at the output's scale.
/// A frame larger than the pixels it is drawn into is decode work and copy
/// bandwidth for nothing.
pub fn video_options(panel: Rect, scale: f32, autoplay: bool) -> Options {
    Options {
        max_width: ((panel.width() * scale) as u32).clamp(64, 3840),
        max_height: ((panel.height() * scale) as u32).clamp(64, 2160),
        autoplay,
    }
}

/// An open preview.
pub struct Session {
    pub preview: Preview,
    /// The file's own name, shown in the panel's title strip. The preview
    /// itself does not carry one — a decoded image knows nothing about where
    /// it came from — so the host puts it here when it opens the session.
    pub name: String,
    /// Where in the content the panel is: the scroll offset into a listing or
    /// a text preview, and the frame of an animated image, which the toolkit
    /// reads the same way — it is which part of the content is on screen.
    /// The host owns it, as it owns every other piece of interaction state.
    pub first_row: usize,
    /// When the frame now showing went up, for an animation. Only read while
    /// the preview is one.
    frame_shown_at: Instant,
    /// How far an image preview is zoomed in, and how far it has been dragged
    /// about while it is. Lives here rather than in the toolkit for the same
    /// reason `first_row` does: the drawing half is canvas-pure and holds no
    /// interaction state. Reset by construction — a session is built afresh
    /// for every file, so changing file or closing the panel puts the picture
    /// back to fit without anything having to remember to.
    pub zoom: Zoom,
    /// The two scroll views the zoomed picture is panned by. Kept in step
    /// with `zoom` rather than replacing it: a scroll view holds a position
    /// and the feel of moving it, while the drawing wants the geometry a
    /// `Zoom` describes.
    pub pan: Pan,
    /// The row this grew out of, in surface-local coordinates. Empty means
    /// "open in place" — a row scrolled out of view, or a panned-away Miller
    /// column.
    pub anchor: Rect,
    /// When the entrance started.
    pub opened_at: Instant,
    /// When the exit started, once it has. A closing session is no longer the
    /// window's open preview — it is only still on screen, going home.
    pub closing: Option<Instant>,
    /// A playback, when the file is a video and a player could be started.
    /// The card in `preview` stays underneath it: its artwork is the poster
    /// until the first frame lands, and it is what is shown again if the
    /// player fails.
    pub video: Option<Video>,
    /// Whether the panel fills the space it is centred in — the display or
    /// the window — rather than taking its usual share of it. Toggled by the
    /// title strip's expand button and kept while arrow-keying between files.
    pub expanded: bool,
    /// How far the panel has been dragged from where it rests, by its title
    /// strip. In the same space as the resting rect the host works out — the
    /// display when the panel is centred on one, the window otherwise — so
    /// the host folds it into that rect and everything downstream follows.
    ///
    /// Kept while arrow-keying between files, like `expanded`: a panel moved
    /// out of the way of something stays where it was put. A panel closed and
    /// opened again starts at rest, because a session is built afresh.
    pub offset: (f32, f32),
    /// Whether `preview` is the waiting line rather than a decoded file.
    ///
    /// The panel's surface repaints only when its content key changes, and
    /// the key is built from the request's generation — which is bumped when
    /// the decode is *asked for*, not when it answers. Without this the panel
    /// draws once, while it is still waiting, and the content landing changes
    /// nothing the key can see: the card sits on "Opening preview…" until
    /// something else moves it.
    pub loading: bool,
    /// The words selected on a picture, when some are. Indices into the
    /// preview's words; the toolkit turns them into text and highlights.
    pub selection: Option<WordSelection>,
    /// Whether a press on a word is still down, extending the selection as
    /// the pointer moves.
    pub selecting: bool,
    /// Bumped when words land on the picture after it was drawn, so the
    /// panel's content key sees that there is now something to select.
    pub words_epoch: u64,
    /// Bumped when a page of a document is rasterised, or dropped again. The
    /// strip is where it was and is zoomed as it was; what changed is that a
    /// page that was blank is now paper with printing on it.
    pub pages_epoch: u64,
    /// When the recogniser was started on this picture, while it is still
    /// running. The badge in the panel's corner reads it: the picture is up
    /// before its words are, and a reader who cannot see that something is
    /// still coming reads "no text here" instead of "not yet".
    pub recognising_since: Option<Instant>,
}

impl Session {
    /// Open a session on a decoded file, at fit and unscrolled.
    pub fn new(preview: Preview, name: String, anchor: Rect, opened_at: Instant) -> Self {
        let zoom = match &preview {
            Preview::Pages { .. } => Zoom::TOP,
            _ => Zoom::FIT,
        };
        Self {
            preview,
            name,
            first_row: 0,
            frame_shown_at: Instant::now(),
            // Fit, whatever the last file was left at: a zoom belongs to the
            // picture it was made on, not to the panel. A document opens at
            // the top of its first page instead, which is the same thing —
            // the beginning of what there is to look at.
            zoom,
            pan: Pan::new(),
            anchor,
            opened_at,
            closing: None,
            video: None,
            expanded: false,
            offset: (0.0, 0.0),
            loading: false,
            selection: None,
            selecting: false,
            words_epoch: 0,
            pages_epoch: 0,
            recognising_since: None,
        }
    }

    /// The animation this preview is, if it is one.
    fn animation(&self) -> Option<&Pixels> {
        match &self.preview {
            Preview::Pixels { pixels, .. } if pixels.is_animated() => Some(pixels),
            _ => None,
        }
    }

    /// Whether the panel has an animation running — a GIF, an animated WEBP.
    /// The host needs a steady clock while it does, since nothing else is
    /// going to ask for the next frame.
    pub fn frames_running(&self) -> bool {
        self.animation().is_some()
    }

    /// Show the next frame if the one up has had its time. Returns whether
    /// the picture changed.
    ///
    /// An animation loops for as long as the preview is open: a GIF says how
    /// many times it would like to repeat, and a preview is looked at for as
    /// long as it is looked at — stopping partway through would leave the
    /// panel on whatever frame the author happened to end on.
    pub fn tick_animation(&mut self) -> bool {
        let Some(pixels) = self.animation() else {
            return false;
        };
        if self.frame_shown_at.elapsed() < pixels.delay(self.first_row) {
            return false;
        }
        // Counted up and wrapped when it is read, so a long-open preview
        // never needs the count reset and the frame after the last is the
        // first again.
        self.first_row = (self.first_row + 1) % pixels.frames();
        self.frame_shown_at = Instant::now();
        true
    }

    /// Fill the space, or go back to the usual share of it.
    pub fn toggle_expanded(&mut self) {
        self.expanded = !self.expanded;
    }

    /// Start playing `path` in this session, if the preview is a video and
    /// a player can be started. See [`Video::open`].
    pub fn attach_video(
        &mut self,
        path: &Path,
        options: Options,
        wake: impl Fn() + Send + Sync + 'static,
    ) {
        self.video = Video::open(&self.preview, path, options, wake);
    }

    /// The playback's contribution to the panel's repaint key.
    pub fn video_key(&self) -> u64 {
        self.video.as_ref().map(Video::key).unwrap_or(0)
    }

    /// The newest frame's sequence number, for a host that paints the panel
    /// into its own window and has no content key to notice a frame for it.
    pub fn video_frame_seq(&self) -> u64 {
        self.video.as_ref().map(Video::frame_seq).unwrap_or(0)
    }

    /// The pointer over a playing video. `None` when there is no video and
    /// the caller's own handling applies; `Some(handled)` otherwise.
    pub fn video_pointer(
        &mut self,
        kind: VideoPointer,
        x: f32,
        y: f32,
        content: Rect,
    ) -> Option<bool> {
        let video = self.video.as_mut()?;
        Some(video.pointer(kind, x, y, content))
    }

    /// A session on a file whose decode has only just been asked for: the
    /// panel, its title and a line saying it is working.
    ///
    /// Pressing Space and seeing nothing until a worker has been spawned,
    /// handed the file and answered reads as a keystroke that did not take.
    /// The panel goes up on the keystroke instead, and the content arrives
    /// into a card that is already there.
    pub fn waiting(name: String, is_dir: bool, anchor: Rect, opened_at: Instant) -> Self {
        let preview = waiting_preview(&name, is_dir);
        Self {
            loading: true,
            selection: None,
            selecting: false,
            words_epoch: 0,
            pages_epoch: 0,
            ..Self::new(preview, name, anchor, opened_at)
        }
    }

    /// Point an open panel at another file, whose decode is in flight.
    ///
    /// Everything that belonged to the old file goes now rather than when the
    /// new one lands — its content, its scroll, its zoom and its pan. A
    /// preview that does not match the selection is worse than no preview,
    /// because nothing on screen says it is the wrong file. `anchor` moves
    /// too, so an exit still flies home to the row that is selected.
    pub fn awaiting(&mut self, name: String, is_dir: bool, anchor: Rect) {
        let opened_at = self.opened_at;
        let expanded = self.expanded;
        let offset = self.offset;
        let anchor = if anchor.is_empty() {
            self.anchor
        } else {
            anchor
        };
        *self = Self::waiting(name, is_dir, anchor, opened_at);
        self.expanded = expanded;
        self.offset = offset;
    }

    /// The words on the picture, if it is one with any.
    pub fn words(&self) -> &[Word] {
        match &self.preview {
            Preview::Pixels { pixels, .. } => &pixels.words,
            Preview::Pages { words, .. } => words,
            _ => &[],
        }
    }

    /// The document's pages, if this preview is one.
    pub fn pages(&self) -> &[Page] {
        match &self.preview {
            Preview::Pages { pages, .. } => pages,
            _ => &[],
        }
    }

    /// Words recognised after the picture was shown. Ignored when the
    /// preview is not a picture any more; a selection that was somehow made
    /// on the old words is dropped with them.
    pub fn attach_words(&mut self, words: Vec<Word>) {
        match &mut self.preview {
            Preview::Pixels { pixels, .. } => pixels.words = words,
            Preview::Pages { words: held, .. } => *held = words,
            _ => {
                self.recognising_since = None;
                return;
            }
        }
        self.selection = None;
        self.selecting = false;
        self.words_epoch = self.words_epoch.wrapping_add(1);
        self.recognising_since = None;
    }

    /// Take the text layer a second pass read out of the document: its words,
    /// and the page geometry it measured them against.
    ///
    /// The sizes are adopted along with the words rather than only the words,
    /// because the two have to agree — a selection boxed against one idea of
    /// how big a page is, drawn against another, highlights the wrong line.
    /// Pages already rasterised keep their pixels; they are the same pages,
    /// measured again.
    pub fn attach_text_layer(&mut self, measured: Vec<Page>, words: Vec<Word>) -> bool {
        let Preview::Pages { pages, .. } = &mut self.preview else {
            return false;
        };
        // A text layer read from a different document than the one on screen
        // — the file changed under the second pass — is not merged.
        if measured.len() != pages.len() {
            return false;
        }
        for (page, measured) in pages.iter_mut().zip(measured) {
            page.width = measured.width;
            page.height = measured.height;
        }
        self.attach_words(words);
        true
    }

    /// Say that the recogniser is running on this picture, from `now`.
    pub fn start_recognising(&mut self, now: Instant) {
        self.recognising_since = Some(now);
    }

    /// Say that it has stopped, with nothing to attach.
    pub fn stop_recognising(&mut self) {
        self.recognising_since = None;
    }

    /// How long the recogniser has been running, in seconds; `None` when it
    /// is not. What the badge animates on, and what puts a moving panel in
    /// the content key while it moves.
    pub fn recognising_phase(&self) -> Option<f32> {
        self.recognising_since
            .map(|since| since.elapsed().as_secs_f32())
    }

    /// The word under a panel point, in the picture as it is drawn in
    /// `content`.
    pub fn word_at(&self, x: f32, y: f32, content: Rect) -> Option<usize> {
        if self.words().is_empty() {
            return None;
        }
        let layout = preview::layout(content, &self.preview, self.first_row, self.zoom);
        preview::word_at(&layout, &self.preview, x, y)
    }

    /// A press over the picture: on a word it starts a selection and is
    /// taken; anywhere else it clears one and is not. Returns whether the
    /// press was on a word.
    pub fn select_pointer_down(&mut self, x: f32, y: f32, content: Rect) -> bool {
        let had = self.selection.take().is_some();
        self.selecting = false;
        match self.word_at(x, y, content) {
            Some(index) => {
                self.selection = Some(WordSelection::word(index));
                self.selecting = true;
                true
            }
            None => had,
        }
    }

    /// The pointer moved with a selection being dragged. Returns whether the
    /// selection changed.
    pub fn select_pointer_move(&mut self, x: f32, y: f32, content: Rect) -> bool {
        if !self.selecting {
            return false;
        }
        let Some(mut selection) = self.selection else {
            return false;
        };
        let layout = preview::layout(content, &self.preview, self.first_row, self.zoom);
        let Some(head) = preview::word_near(&layout, &self.preview, x, y) else {
            return false;
        };
        if head == selection.head {
            return false;
        }
        selection.head = head;
        self.selection = Some(selection);
        true
    }

    /// The button came up: the selection stays, the drag ends.
    pub fn select_pointer_up(&mut self) {
        self.selecting = false;
    }

    /// Select every word on the picture. Returns whether there were any.
    pub fn select_all_words(&mut self) -> bool {
        let count = self.words().len();
        if count == 0 {
            return false;
        }
        self.selection = Some(WordSelection::new(0, count - 1));
        true
    }

    /// Drop the selection. Returns whether there was one.
    pub fn clear_selection(&mut self) -> bool {
        self.selecting = false;
        self.selection.take().is_some()
    }

    /// The selected words as text, or `None` with nothing selected.
    pub fn selected_text(&self) -> Option<String> {
        let selection = self.selection?;
        let text = preview::selection_text(self.words(), selection);
        (!text.is_empty()).then_some(text)
    }

    /// How far into the entrance the panel is, 0 → 1.
    pub fn entrance_t(&self) -> f32 {
        let elapsed = self.opened_at.elapsed().as_secs_f32();
        (elapsed / opening::geometry_in().as_secs_f32()).clamp(0.0, 1.0)
    }

    /// How far into the exit the panel is, 0 → 1. Always 0 while open.
    pub fn exit_t(&self) -> f32 {
        let Some(started) = self.closing else {
            return 0.0;
        };
        (started.elapsed().as_secs_f32() / opening::geometry_out().as_secs_f32()).clamp(0.0, 1.0)
    }

    /// Whether this session still has frames to run — arriving, or leaving.
    pub fn animating(&self) -> bool {
        match self.closing {
            Some(_) => self.exit_t() < 1.0,
            None => self.entrance_t() < 1.0,
        }
    }

    /// Where the panel is now: partway in, at rest, or partway back to the
    /// item it came from.
    pub fn panel(&self, resting: Rect) -> Rect {
        match self.closing {
            Some(_) => exit_at(self.anchor, resting, self.exit_t()),
            None => entrance_at(self.anchor, resting, self.entrance_t()),
        }
    }

    /// Scroll a listing or a text preview by `rows`, stopping at both ends.
    ///
    /// Images and cards do not scroll: they are laid out to fit, so there is
    /// nothing under the fold to reach. A zoomed image *does* have something
    /// under the fold, but that is a pan rather than a scroll — see
    /// [`Session::pan_wheel`], which the host reaches for first.
    pub fn scroll_by(&mut self, rows: i32, panel: Rect) {
        let geometry = otto_kit::preview::layout(panel, &self.preview, self.first_row, self.zoom);
        let total = match &self.preview {
            Preview::Text { lines, .. } => lines.len(),
            Preview::Rows { rows, .. } => rows.len(),
            // A document's rows are its *wrapped* lines, which only the layout
            // knows: the same blocks are more lines in a narrow panel than in
            // a wide one, so the count has to come from the geometry rather
            // than from the payload.
            Preview::Document { .. } => geometry.doc_lines.len(),
            _ => return,
        };
        let visible = geometry.visible_rows;
        let max = total.saturating_sub(visible);
        let next = self.first_row as i64 + rows as i64;
        self.first_row = next.clamp(0, max as i64) as usize;
    }

    /// Which page of a paginated preview is showing, and how many there are —
    /// `None` for everything that is not paginated, which is everything but a
    /// PDF of more than one page.
    ///
    /// A page is not scroll state and not zoom state: it is *a different
    /// decode*. Turning one is asking the worker for another picture, which is
    /// why this only reports and the host does the turning.
    pub fn paged(&self, content: Rect) -> Option<(u32, u32)> {
        match &self.preview {
            Preview::Pixels { pages, page, .. } if *pages > 1 => Some((*page, *pages)),
            Preview::Pages { pages, .. } if pages.len() > 1 => {
                Some((self.showing_page(content) as u32 + 1, pages.len() as u32))
            }
            _ => None,
        }
    }

    /// The page the panel is showing: the one with most of the viewport.
    ///
    /// A scrolled document is usually showing two pages at once, and "which
    /// page is this" has one answer. Most of the box is that answer — it is
    /// what a reader would say they were on.
    pub fn showing_page(&self, content: Rect) -> usize {
        let layout = preview::layout(content, &self.preview, self.first_row, self.zoom);
        let mut best = (0usize, 0.0f32);
        for (index, rect) in layout.page_rects.iter().enumerate() {
            let top = rect.top.max(layout.inner.top);
            let bottom = rect.bottom.min(layout.inner.bottom);
            let shown = bottom - top;
            if shown > best.1 {
                best = (index, shown);
            }
        }
        best.0
    }

    /// Which pages are on screen, or near enough to be worth having ready.
    /// Empty for everything that is not a document.
    pub fn pages_in_view(&self, content: Rect) -> Vec<usize> {
        let layout = preview::layout(content, &self.preview, self.first_row, self.zoom);
        layout
            .page_rects
            .iter()
            .enumerate()
            .filter(|(_, rect)| rect.bottom >= layout.inner.top && rect.top <= layout.inner.bottom)
            .map(|(index, _)| index)
            .collect()
    }

    /// The pages worth holding pixels for: the ones on screen, and a couple
    /// either side so scrolling back a page does not start a rasteriser.
    ///
    /// Wider than what is asked for, so a page that has just been passed is
    /// kept rather than dropped and immediately wanted again.
    fn pages_kept(&self, content: Rect) -> std::ops::RangeInclusive<usize> {
        /// How far either side of the screen a page is kept.
        const KEEP: usize = 2;
        let visible = self.pages_in_view(content);
        let first = visible.first().copied().unwrap_or(0);
        let last = visible.last().copied().unwrap_or(first);
        first.saturating_sub(KEEP)..=last + KEEP
    }

    /// What the panel wants rasterised: the 1-based page, and how wide to
    /// rasterise it, nearest to the eye first.
    ///
    /// Reading goes downwards, so this looks further ahead than behind: a
    /// page is asked for before it comes into view, and a scroll that arrives
    /// on one that is already there arrives on a page rather than on paper.
    /// A page whose pixels are too narrow for the size it is now drawn at —
    /// the reader zoomed in — is asked for again at the size it needs.
    pub fn pages_wanted(&self, content: Rect, scale: f32) -> Vec<PageRequest> {
        /// How far ahead of the screen pages are fetched, and how far behind.
        const AHEAD: usize = 2;
        const BEHIND: usize = 1;
        /// How much wider than its pixels a page may be drawn before it is
        /// worth rasterising again. A page drawn a little larger than it was
        /// decoded is resampled and looks it; one drawn a little smaller is
        /// supersampled, which looks better than the decode did.
        const TOO_SOFT: f32 = 1.25;

        let pages = self.pages();
        if pages.is_empty() {
            return Vec::new();
        }
        let layout = preview::layout(content, &self.preview, self.first_row, self.zoom);
        let visible = self.pages_in_view(content);
        let first = visible.first().copied().unwrap_or(0);
        let last = visible.last().copied().unwrap_or(first);
        let showing = self.showing_page(content) as i64;

        let mut wanted: Vec<(usize, u32)> = (first.saturating_sub(BEHIND)
            ..=(last + AHEAD).min(pages.len() - 1))
            .filter_map(|index| {
                let drawn = layout.page_rects.get(index)?.width() * scale;
                let width = (drawn.ceil() as u32).clamp(MIN_PAGE_WIDTH, MAX_PAGE_WIDTH);
                let enough = match &pages[index].pixels {
                    // Never asked for, or asked for at a size the reader has
                    // since zoomed past.
                    Some(pixels) => (pixels.width as f32 * TOO_SOFT) >= drawn,
                    None => false,
                };
                (!enough).then_some((index, width))
            })
            .collect();
        wanted.sort_by_key(|(index, _)| (*index as i64 - showing).abs());
        wanted
            .into_iter()
            .map(|(index, width)| PageRequest {
                page: index as u32 + 1,
                width,
            })
            .collect()
    }

    /// Put a rasterised page in its place in the strip, and let go of the
    /// pages that have been scrolled well past.
    ///
    /// Holding every page a long scroll went over would be hundreds of
    /// megabytes of images nobody is looking at; they are cheap to ask for
    /// again, and asking again is what scrolling back does anyway.
    pub fn attach_page(&mut self, page: u32, mut pixels: Pixels, content: Rect) -> bool {
        let keep = self.pages_kept(content);
        let Preview::Pages { pages, words } = &mut self.preview else {
            return false;
        };
        let Some(index) = (page as usize).checked_sub(1).filter(|i| *i < pages.len()) else {
            return false;
        };

        // Words recognised on the page image, for a document that has no text
        // layer of its own. They are boxed in the page's pixels and the
        // selection is made in the strip's coordinates, so they are moved
        // before they are merged, and they replace whatever this page
        // contributed before rather than piling up beside it.
        if !pixels.words.is_empty() {
            let mut recognised =
                preview::words_in_strip(pages, index, &pixels.words, pixels.width, pixels.height);
            for word in &mut recognised {
                word.block = index as u32;
            }
            words.retain(|word| word.block != index as u32);
            let at = words
                .iter()
                .position(|word| word.block > index as u32)
                .unwrap_or(words.len());
            words.splice(at..at, recognised);
            self.selection = None;
            self.selecting = false;
            self.words_epoch = self.words_epoch.wrapping_add(1);
        }
        pixels.words = Vec::new();
        pages[index].pixels = Some(pixels);
        for (other, page) in pages.iter_mut().enumerate() {
            if !keep.contains(&other) {
                page.pixels = None;
            }
        }
        self.pages_epoch = self.pages_epoch.wrapping_add(1);
        true
    }

    /// Scroll a document to `page`, 1-based, putting its top edge at the top
    /// of the box. Returns whether anything moved.
    pub fn scroll_to_page(&mut self, page: u32, content: Rect) -> bool {
        let index = (page.max(1) - 1) as usize;
        let layout = preview::layout(content, &self.preview, self.first_row, self.zoom);
        let Some(rect) = layout.page_rects.get(index) else {
            return false;
        };
        self.pan_by(0.0, layout.inner.top - rect.top, content)
    }

    /// The page a turn of `delta` lands on, or `None` when there is nothing to
    /// turn to: an unpaginated preview, or an end already reached. Stopping at
    /// both ends rather than wrapping, like every other end in the panel.
    pub fn page_turn(&self, delta: i32, content: Rect) -> Option<u32> {
        let (page, pages) = self.paged(content)?;
        let next = (page as i64 + delta as i64).clamp(1, pages as i64) as u32;
        (next != page).then_some(next)
    }

    /// Whether a two-finger gesture over `panel` should move the picture
    /// rather than scroll the content.
    ///
    /// False for everything but an image, and false for an image at fit: one
    /// that fills no more than its box has nothing to pan to, so the gesture
    /// must go on meaning exactly what it meant before there was a zoom.
    /// Asked against the panel's content rect because a zoom clamped for one
    /// box is not clamped for another — resizing the window can leave a
    /// stored zoom with no slack left.
    pub fn pannable(&self, panel: Rect) -> bool {
        !otto_kit::preview::clamp_zoom(panel, &self.preview, self.zoom).is_fit()
    }

    /// Drag a zoomed image by `dx`, `dy` in the panel's own pixels, stopping
    /// where its edge reaches the edge of the content box.
    ///
    /// A placement rather than a gesture: nothing is thrown and nothing
    /// bounces, which is what a caller moving the picture itself — a keyboard
    /// pan, a test — means. A two-finger scroll goes through
    /// [`Session::pan_wheel`] instead.
    ///
    /// Returns whether anything moved, so a host that repaints on demand does
    /// not repaint for a gesture that was already against the stop.
    pub fn pan_by(&mut self, dx: f32, dy: f32, content: Rect) -> bool {
        // From where the content actually is, not from what is stored: a
        // document opens asking to be further up than it can go (see
        // [`Zoom::TOP`]), and adding to that would move nothing.
        let from = otto_kit::preview::clamp_zoom(content, &self.preview, self.zoom);
        let asked = Zoom {
            scale: from.scale,
            offset: (from.offset.0 + dx, from.offset.1 + dy),
            // A placement is not a stretch: whatever band was in flight ends
            // here rather than being carried along by it.
            band: (0.0, 0.0),
        };
        let next = otto_kit::preview::clamp_zoom(content, &self.preview, asked);
        let moved = next != self.zoom;
        self.zoom = next;
        self.sync_pan(content);
        self.push_pan();
        moved
    }

    /// Feed a two-finger scroll to a zoomed picture's pan, with the momentum,
    /// the rubber band and the bars a scrolled list gets.
    ///
    /// `dx`/`dy` are the deltas the compositor reported, unscaled: the scroll
    /// views apply the same [`wheel_scale`](otto_kit::components::scroll::wheel_scale)
    /// every other scroll in the toolkit does, so one gesture covers the same
    /// ground over a picture as over a listing. `stop` is the fingers
    /// lifting, which throws; `discrete` a notched wheel, which does not.
    ///
    /// Both axes are fed. Unlike the browser's own panes there is no leading
    /// axis to pick: a picture is panned about in two dimensions at once, and
    /// an axis with no slack simply has nothing to move.
    ///
    /// Returns whether anything moved.
    pub fn pan_wheel(
        &mut self,
        dx: f32,
        dy: f32,
        content: Rect,
        stop: bool,
        discrete: bool,
    ) -> bool {
        self.sync_pan(content);
        // An axis with no slack is left out rather than fed a delta it would
        // rubber-band against: a picture that fits its box across is not
        // stretched sideways by a gesture meant for the axis that does have
        // somewhere to go. That is also what makes this safe to ask first for
        // any preview — a listing, or an image at fit, pans nothing and says
        // so, and the host goes on to scroll it by rows.
        let mut axes = [(&mut self.pan.x, dx), (&mut self.pan.y, dy)];
        let mut moved = false;
        for (view, delta) in axes.iter_mut() {
            if !view.state.scrollable() {
                continue;
            }
            moved |= if stop {
                // Fingers off the touchpad: what the gesture was carrying
                // becomes a fling, and anything pulled past an end springs
                // back.
                view.on_wheel_end();
                true
            } else if discrete {
                view.on_wheel_discrete(*delta)
            } else {
                // The picture follows the fingers the way the content of a
                // scrolled view does: pushing down brings what is below into
                // view, which moves the image up. That is a scroll view's own
                // sign convention, so the deltas go in as they arrived.
                view.on_wheel(*delta)
            };
        }
        self.pull_pan(content) | moved
    }

    /// A pointer press over the panel, in the panel's coordinates. Returns
    /// whether it landed on one of the pan's scrollbars and started dragging
    /// it — a host must not treat such a press as a click on the picture, or
    /// as the click-outside that dismisses the panel.
    ///
    /// A press anywhere over the picture also catches an in-flight fling.
    pub fn pan_pointer_down(&mut self, x: f32, y: f32, content: Rect) -> bool {
        self.sync_pan(content);
        self.pan.x.on_pointer_down(x, y) | self.pan.y.on_pointer_down(x, y)
    }

    /// The pointer moved to `(x, y)`. Continues a scrollbar drag if one is
    /// running, and otherwise only tracks which bar is hovered — so this is
    /// safe to call for every motion event over the panel. Returns whether
    /// anything changed and a repaint is needed.
    pub fn pan_pointer_move(&mut self, x: f32, y: f32, content: Rect) -> bool {
        self.sync_pan(content);
        let dragged = self.pan.x.on_pointer_drag(x, y) | self.pan.y.on_pointer_drag(x, y);
        if dragged {
            return self.pull_pan(content) | true;
        }
        self.pan.x.on_pointer_move(x, y) | self.pan.y.on_pointer_move(x, y)
    }

    /// The pointer left the panel: no bar is hovered any more.
    pub fn pan_pointer_leave(&mut self) {
        self.pan.x.on_pointer_leave();
        self.pan.y.on_pointer_leave();
    }

    /// The pointer button came up, ending any scrollbar drag.
    pub fn pan_pointer_up(&mut self) {
        self.pan.x.on_pointer_up();
        self.pan.y.on_pointer_up();
    }

    /// Whether the pan still has frames to run — a fling, a bounce, or a bar
    /// fading out.
    pub fn pan_animating(&self) -> bool {
        self.pan.x.is_animating() || self.pan.y.is_animating()
    }

    /// Advance the pan by one frame. Returns whether anything moved.
    pub fn tick_pan(&mut self, content: Rect) -> bool {
        if !self.pan_animating() {
            return false;
        }
        self.sync_pan(content);
        let moved = self.pan.x.tick() | self.pan.y.tick();
        self.pull_pan(content) | moved
    }

    /// The two bars' states, for drawing: horizontal first.
    pub fn pan_bars(&self) -> (&ScrollState, &ScrollState) {
        (&self.pan.x.state, &self.pan.y.state)
    }

    /// Lay the pan's views against the picture as it is drawn in `content`
    /// now: the box they scroll inside, and how much picture there is to
    /// scroll along each axis.
    ///
    /// Cheap to call before every step, and it has to be: the box a stored
    /// pan was clamped against is not the box it is drawn in after a resize,
    /// and the content is a different size after every pinch. Both setters
    /// are no-ops when nothing changed, so a gesture in flight keeps its
    /// momentum across the calls that change nothing.
    fn sync_pan(&mut self, content: Rect) {
        // Only a picture pans. Everything else is laid out to fit, so it has
        // no length past its box — which leaves the views unscrollable and
        // draws no bars, exactly as if they were not there.
        let (viewport, length) = match &self.preview {
            // A document's strip is longer than its box by construction, so
            // this is what scrolls one: the same two views, the same
            // momentum, the same bars as a zoomed picture.
            Preview::Pixels { .. } | Preview::Pages { .. } => {
                let layout =
                    otto_kit::preview::layout(content, &self.preview, self.first_row, self.zoom);
                (
                    layout.inner,
                    (layout.content.width(), layout.content.height()),
                )
            }
            _ => (Rect::new_empty(), (0.0, 0.0)),
        };
        self.pan.x.set_viewport(viewport);
        self.pan.y.set_viewport(viewport);
        self.pan.x.set_content_length(length.0);
        self.pan.y.set_content_length(length.1);
    }

    /// Copy the views' offsets into the zoom. Returns whether it moved.
    ///
    /// A scroll view measures from the content's leading edge, where 0 shows
    /// the left of the picture; a zoom offset measures from the centre of the
    /// box and points the other way. The two are mirror images about the
    /// slack — half of what a view calls its maximum offset.
    fn pull_pan(&mut self, content: Rect) -> bool {
        let (slack_x, slack_y) = self.pan_slack();
        let asked = Zoom {
            scale: self.zoom.scale,
            offset: (slack_x - self.pan.x.offset(), slack_y - self.pan.y.offset()),
            // What a view holds past its own end is the stretch, and the only
            // part of a pan the clamp is not allowed to take away: the offset
            // above already carries it, the clamp cuts exactly it off, and
            // this puts exactly it back. Negated for the same reason the
            // offset is — a view measures from the content's leading edge and
            // a zoom from the box's centre, so pulling *before* the start
            // drags the picture right.
            band: (
                -self.pan.x.state.overscroll(),
                -self.pan.y.state.overscroll(),
            ),
        };
        let next = otto_kit::preview::clamp_zoom(content, &self.preview, asked);
        let moved = next != self.zoom;
        self.zoom = next;
        moved
    }

    /// Copy the zoom's offset into the views, dropping whatever they were
    /// carrying: something outside them has placed the picture, and a fling
    /// still running would drag it straight back off that spot.
    fn push_pan(&mut self) {
        let (slack_x, slack_y) = self.pan_slack();
        self.pan.x.scroll_to(slack_x - self.zoom.offset.0);
        self.pan.y.scroll_to(slack_y - self.zoom.offset.1);
    }

    /// How far the picture can be dragged from centred, along each axis.
    fn pan_slack(&self) -> (f32, f32) {
        (
            self.pan.x.state.max_offset() / 2.0,
            self.pan.y.state.max_offset() / 2.0,
        )
    }

    /// Zoom an image to `scale` about `focus`, a point in the same
    /// coordinates as `panel`.
    ///
    /// Returns whether anything moved. The clamping — the range, the snap
    /// back to fit and the pan limits — all happens in the toolkit, so the
    /// panel and the file picker cannot end up with different ideas of how
    /// far a picture zooms.
    pub fn zoom_to(&mut self, scale: f32, focus: (f32, f32), panel: Rect) -> bool {
        let next = otto_kit::preview::zoom_about(panel, &self.preview, self.zoom, scale, focus);
        let moved = next != self.zoom;
        self.zoom = next;
        // The pinch, not the pan, has just placed the picture: re-measure the
        // views against the size it is now and put them where it left it.
        self.sync_pan(panel);
        self.push_pan();
        // And bring the bars up. Zooming in is when they have most to say —
        // it is the moment the picture stops fitting, and how much of it is
        // now off the sides is exactly what a bar reports — so waiting for a
        // pan to reveal them tells the user last what they needed first.
        if moved {
            for view in [&mut self.pan.x, &mut self.pan.y] {
                if view.state.scrollable() {
                    view.flash_scrollbar();
                }
            }
        }
        moved
    }
}

/// A page to rasterise, and how wide to rasterise it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRequest {
    /// 1-based, as everything that names a page is.
    pub page: u32,
    /// In physical pixels: the width the page is drawn at now.
    pub width: u32,
}

/// The narrowest a page is ever rasterised — below this the text on it is not
/// text — and the widest, which is the rasteriser's own ceiling.
const MIN_PAGE_WIDTH: u32 = 320;
const MAX_PAGE_WIDTH: u32 = 2_048;

/// What a panel shows while its decode is still running: the file's own icon,
/// and a line saying the preview is opening.
///
/// Said the same way an undecodable file says why it cannot be shown, because
/// it is the same thing from the panel's side — there is nothing decoded to
/// draw. The icon is the one the browser's listing was already showing for the
/// file, so the card is about the row it grew out of rather than a blank
/// waiting on a worker.
fn waiting_preview(name: &str, is_dir: bool) -> Preview {
    Preview::Unavailable {
        reason: otto_kit::t_owned!("files-status-opening-preview"),
        icon: otto_peek::payload::icon_names_for(name, is_dir),
    }
}

/// Where the panel rests, centred in a window of `width` × `height`.
pub fn panel_rect(width: f32, height: f32) -> Rect {
    resting_in(Rect::from_wh(width, height), false)
}

/// What an expanded panel leaves around itself. Enough to still read as a
/// card laid over the desktop rather than a window that replaced it.
pub const EXPANDED_MARGIN: f32 = 12.0;

/// Where the panel rests inside `space` — a window or a display, in points.
///
/// At rest it takes [`PANEL_FRACTION`] of the space, floored at
/// [`PANEL_MIN`] and never quite touching the edges. Expanded it takes all
/// of it bar [`EXPANDED_MARGIN`], which is what the title strip's expand
/// button asks for.
pub fn resting_in(space: Rect, expanded: bool) -> Rect {
    let (width, height) = (space.width(), space.height());
    let (w, h) = if expanded {
        (
            (width - EXPANDED_MARGIN * 2.0).max(1.0),
            (height - EXPANDED_MARGIN * 2.0).max(1.0),
        )
    } else {
        (
            (width * PANEL_FRACTION).max(PANEL_MIN.0).min(width - 32.0),
            (height * PANEL_FRACTION)
                .max(PANEL_MIN.1)
                .min(height - 32.0),
        )
    };
    Rect::from_xywh(
        space.left + (width - w) / 2.0,
        space.top + (height - h) / 2.0,
        w.max(1.0),
        h.max(1.0),
    )
}

/// Room around the panel in its own surface. The card has no shadow to spill
/// past its edge any more — only the antialiasing on its border — so this is
/// a single point, and the surface is the card.
pub const SURFACE_MARGIN: f32 = 1.0;

/// The panel's rect partway through the entrance.
///
/// Runs the same curve `opening` describes, in this process rather than through
/// a compositor transaction — the panel is drawn into this window's own
/// surface, so there is no separate surface for the compositor to transform.
pub fn entrance_at(anchor: Rect, resting: Rect, t: f32) -> Rect {
    let rect = opening::sample(to_opening(anchor), to_opening(resting), t);
    Rect::from_xywh(rect.x, rect.y, rect.width, rect.height)
}

/// The reverse: the panel on its way back to the item it grew out of.
pub fn exit_at(anchor: Rect, resting: Rect, t: f32) -> Rect {
    let rect = opening::sample_out(to_opening(anchor), to_opening(resting), t);
    Rect::from_xywh(rect.x, rect.y, rect.width, rect.height)
}

fn to_opening(rect: Rect) -> opening::Rect {
    if rect.is_empty() {
        return opening::Rect::new(0.0, 0.0, 0.0, 0.0);
    }
    opening::Rect::new(rect.left, rect.top, rect.width(), rect.height())
}

/// Decode one file. **Blocks** until the sandboxed worker answers or its
/// deadline expires, so it must never be called on the UI thread.
///
/// `panel` is the resting rect in logical pixels and `scale` the output's
/// scale; the worker is asked for roughly twice that, so a scaled decode still
/// has detail to show when the panel is looked at closely. `page` is 1-based
/// and only means anything to paginated content.
pub fn decode(path: &Path, panel: Rect, scale: f32, page: u32) -> Preview {
    decode_with(path, panel, scale, page, false, false)
}

/// Rasterise one page of a document at exactly `width` physical pixels.
///
/// The width is the page's own drawn width rather than the panel's — a page
/// rests whole in the panel with a gutter either side, and rasterising is
/// superlinear in width, so asking for the panel's width is several times the
/// work for pixels nothing draws.
pub fn decode_page(path: &Path, page: u32, width: u32) -> Preview {
    let request = Request {
        page: page.max(1),
        width,
        // The rasteriser keeps the page's aspect ratio and takes its height
        // from the width, so this only has to be out of the way.
        height: width * 4,
        oversample: 1.0,
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ..Request::default()
    };
    otto_peek::decode_path(path, &request)
}

/// Decode a file for a host that **scrolls documents**: a paginated file comes
/// back as the whole strip — every page's geometry, and the pixels of `page`
/// — rather than as a picture of one page.
///
/// Everything else decodes exactly as it does for [`decode`], so the caller
/// does not have to know which files are documents.
pub fn decode_document(path: &Path, panel: Rect, scale: f32, page: u32) -> Preview {
    decode_with(path, panel, scale, page, true, false)
}

/// The panel's content box in physical pixels, which is what a document is
/// fitted into. Not oversampled: a page rests at a fraction of the panel's
/// width, and asking for twice the panel is several times the rasterising for
/// detail nothing shows.
fn document_box(panel: Rect, scale: f32) -> (u32, u32) {
    (
        ((panel.width() * scale) as u32).clamp(64, 4096),
        ((panel.height() * scale) as u32).clamp(64, 4096),
    )
}

/// Read a document's own text, with a box for every word, in the strip's
/// coordinates. A second pass over a file already on screen: it costs about a
/// millisecond a page and carries no pixels at all.
///
/// `None` for a file with no text layer — a scan, or anything that is not a
/// document — which is where the recogniser comes in instead.
pub fn text_layer(path: &Path, panel: Rect, scale: f32) -> Option<(Vec<Page>, Vec<Word>)> {
    match decode_with(path, panel, scale, 1, true, true) {
        Preview::Pages { pages, words } if !words.is_empty() => Some((pages, words)),
        _ => None,
    }
}

fn decode_with(
    path: &Path,
    panel: Rect,
    scale: f32,
    page: u32,
    document: bool,
    text: bool,
) -> Preview {
    /// The headroom the worker is asked for over the panel's own pixels, so a
    /// picture looked at closely has detail to show before the zoom asks
    /// again. Told to the worker as well as folded into the size, because an
    /// animation spends it on frames instead.
    const OVERSAMPLE: f32 = 2.0;

    let (width, height) = if document {
        document_box(panel, scale)
    } else {
        (
            ((panel.width() * scale * OVERSAMPLE) as u32).clamp(64, 4096),
            ((panel.height() * scale * OVERSAMPLE) as u32).clamp(64, 4096),
        )
    };
    let request = Request {
        page: page.max(1),
        document,
        text,
        width,
        height,
        oversample: if document { 1.0 } else { OVERSAMPLE },
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        ..Request::default()
    };
    otto_peek::decode_path(path, &request)
}

/// The size a picture is decoded at for `panel`, so a second decode of the
/// same file lands on the same pixels as the first.
pub fn decode_size(panel: Rect, scale: f32) -> (u32, u32) {
    (
        ((panel.width() * scale * 2.0) as u32).clamp(64, 4096),
        ((panel.height() * scale * 2.0) as u32).clamp(64, 4096),
    )
}

/// Who is waiting for a recognition, which decides whose CPU it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// The panel is up and the badge is breathing: the user is watching.
    Interactive,
    /// The pass over the folder, which nobody asked for. Runs down the queue
    /// so it fills idle cores and yields to an interactive one the moment the
    /// scheduler has to choose.
    Background,
}

impl Priority {
    /// The `nice` value a worker at this priority runs at.
    fn nice(self) -> i32 {
        match self {
            // Whatever the app itself runs at: the panel is up and the badge
            // is breathing, so this is as urgent as anything else the window
            // is doing.
            Priority::Interactive => 0,
            // Enough to lose every contest with the desktop without being so
            // far down that a machine with something else running never
            // finishes the pass.
            Priority::Background => 10,
        }
    }
}

/// Recognise the text in a picture with `recogniser` — a command line, see
/// `otto_peek::ocr` — at the size [`decode`] shows it, and remember
/// it. Returns the words in the coordinates of that decode, or
/// `None` when the file is not a picture or nothing could be recognised
/// (in which case nothing is remembered either, so a transient failure is
/// retried next time).
///
/// Costs a second sandboxed decode of the file with the recogniser run
/// inside it. Blocks; never on the UI thread.
pub fn recognise(
    path: &Path,
    panel: Rect,
    scale: f32,
    page: u32,
    recogniser: &str,
    priority: Priority,
) -> Option<Vec<Word>> {
    let (width, height) = decode_size(panel, scale);
    let request = Request {
        page: page.max(1),
        width,
        height,
        ocr: true,
        languages: otto_peek::ocr::languages(),
        recogniser: recogniser.to_string(),
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        budget: otto_peek::sandbox::Budget {
            nice: priority.nice(),
            ..Default::default()
        },
        ..Request::default()
    };
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    match otto_peek::decode_path(path, &request) {
        Preview::Pixels { pixels, .. } => {
            if let Err(err) = crate::ocrcache::store(
                path,
                page,
                modified,
                &request.languages,
                request.recogniser_command(),
                (pixels.width, pixels.height),
                &pixels.words,
            ) {
                tracing::debug!(path = %path.display(), %err, "could not remember recognised text");
            }
            Some(pixels.words)
        }
        _ => None,
    }
}

/// Words already remembered for `path`, scaled to the decode `panel` gets.
pub fn remembered_words(path: &Path, panel: Rect, scale: f32, page: u32) -> Option<Vec<Word>> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let entry = crate::ocrcache::lookup(path, page, modified)?;
    // The decode is fitted inside the requested box at the picture's own
    // aspect, so it has the entry's aspect too; scaling by width alone would
    // do, but both are recorded and both are used.
    let (width, height) = fitted_size((entry.width, entry.height), decode_size(panel, scale));
    Some(crate::ocrcache::scale_words(
        &entry.words,
        (entry.width, entry.height),
        (width, height),
    ))
}

/// The size a `source`-shaped picture comes out at when decoded to fit in
/// `target`, never upscaled.
fn fitted_size(source: (u32, u32), target: (u32, u32)) -> (u32, u32) {
    if source.0 == 0 || source.1 == 0 {
        return source;
    }
    let scale = (target.0 as f32 / source.0 as f32)
        .min(target.1 as f32 / source.1 as f32)
        .min(1.0);
    (
        ((source.0 as f32 * scale).round() as u32).max(1),
        ((source.1 as f32 * scale).round() as u32).max(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use otto_kit::preview::Pixels;

    /// A session on a picture wide and tall enough that it is scaled *down*
    /// to fit any panel these tests use, so the fit rect fills one axis of
    /// the content box exactly and the zoom arithmetic has something to bite
    /// on.
    #[test]
    fn an_expanded_panel_fills_its_space_bar_the_margin() {
        let space = Rect::from_xywh(100.0, 50.0, 1600.0, 900.0);
        let rest = resting_in(space, false);
        let full = resting_in(space, true);
        assert!(rest.width() < full.width() && rest.height() < full.height());
        assert!((full.left - (space.left + EXPANDED_MARGIN)).abs() < 0.01);
        assert!((full.right - (space.right - EXPANDED_MARGIN)).abs() < 0.01);
        assert!((full.top - (space.top + EXPANDED_MARGIN)).abs() < 0.01);
        assert!((full.bottom - (space.bottom - EXPANDED_MARGIN)).abs() < 0.01);
        // Both are centred in the same space.
        assert!((rest.center_x() - full.center_x()).abs() < 0.01);
        assert!((rest.center_y() - full.center_y()).abs() < 0.01);
    }

    #[test]
    fn expanding_survives_moving_to_the_next_file() {
        let mut session = image_session(100, 100);
        session.toggle_expanded();
        session.awaiting("next.png".into(), false, Rect::new_empty());
        assert!(session.expanded);
        session.toggle_expanded();
        assert!(!session.expanded);
    }

    #[test]
    fn dragging_over_words_selects_them_in_reading_order_and_copies_as_text() {
        let word = |text: &str, left: u32, top: u32, line: u32| Word {
            text: text.into(),
            left,
            top,
            width: 40,
            height: 16,
            confidence: 90,
            block: 1,
            paragraph: 1,
            line,
        };
        let mut session = image_session(400, 120);
        session.attach_words(vec![
            word("Hello", 20, 40, 1),
            word("Otto", 80, 40, 1),
            word("42", 140, 40, 1),
            word("Next", 20, 80, 2),
        ]);
        assert_eq!(session.words_epoch, 1);
        // Words landing end the wait, whoever set it going.
        assert!(session.recognising_phase().is_none());
        // Drawn one-to-one: a 400×120 picture in a 400×120 box.
        let content = Rect::from_wh(400.0, 120.0);
        let layout = preview::layout(content, &session.preview, 0, session.zoom);
        let at = |x: f32, y: f32| {
            (
                layout.content.left + x * layout.content.width() / 400.0,
                layout.content.top + y * layout.content.height() / 120.0,
            )
        };

        // A press on a gap clears and takes nothing.
        let (x, y) = at(200.0, 10.0);
        assert!(!session.select_pointer_down(x, y, content));
        assert!(session.selection.is_none());

        // Press on "Otto", drag to below "Next", release.
        let (x, y) = at(90.0, 48.0);
        assert!(session.select_pointer_down(x, y, content));
        let (x, y) = at(300.0, 110.0);
        assert!(session.select_pointer_move(x, y, content));
        session.select_pointer_up();
        assert_eq!(session.selected_text().as_deref(), Some("Otto 42\nNext"));

        // Backwards drag reads forwards.
        let (x, y) = at(30.0, 88.0);
        assert!(session.select_pointer_down(x, y, content));
        let (x, y) = at(25.0, 48.0);
        assert!(session.select_pointer_move(x, y, content));
        assert_eq!(
            session.selected_text().as_deref(),
            Some("Hello Otto 42\nNext")
        );

        assert!(session.select_all_words());
        assert!(session.clear_selection());
        assert!(session.selected_text().is_none());
    }

    fn image_session(width: u32, height: u32) -> Session {
        Session::new(
            Preview::Pixels {
                pixels: Pixels {
                    width,
                    height,
                    intrinsic_width: width,
                    intrinsic_height: height,
                    data: vec![0; (width * height * 4) as usize],
                    frame_delays: Vec::new(),
                    words: Vec::new(),
                },
                pages: 1,
                page: 1,
            },
            "photo.png".into(),
            Rect::new_empty(),
            Instant::now(),
        )
    }

    /// A three-frame animation whose frames are as short as a delay may be
    /// before it is taken as unset, so the test's own waiting is brief.
    fn animated_session() -> Session {
        Session::new(
            Preview::Pixels {
                pixels: Pixels {
                    width: 2,
                    height: 2,
                    intrinsic_width: 2,
                    intrinsic_height: 2,
                    data: vec![0; 2 * 2 * 4 * 3],
                    frame_delays: vec![Pixels::MIN_DELAY_MS; 3],
                    words: Vec::new(),
                },
                pages: 1,
                page: 1,
            },
            "loop.gif".into(),
            Rect::new_empty(),
            Instant::now(),
        )
    }

    #[test]
    fn an_animation_runs_and_starts_over() {
        let mut session = animated_session();
        assert!(session.frames_running());
        // Nothing moves before the frame has had its time.
        assert!(!session.tick_animation());
        assert_eq!(session.first_row, 0);

        let frame = std::time::Duration::from_millis(Pixels::MIN_DELAY_MS as u64);
        for expected in [1, 2, 0] {
            std::thread::sleep(frame + std::time::Duration::from_millis(5));
            assert!(session.tick_animation(), "the frame was due");
            assert_eq!(session.first_row, expected);
        }
    }

    #[test]
    fn a_still_picture_has_no_clock_to_run() {
        let mut session = image_session(100, 100);
        assert!(!session.frames_running());
        assert!(!session.tick_animation());
        assert_eq!(session.first_row, 0);
    }

    fn text_session() -> Session {
        Session::new(
            Preview::Text {
                lines: (0..200).map(|n| format!("line {n}")).collect(),
                truncated: false,
                language: String::new(),
            },
            "notes.txt".into(),
            Rect::new_empty(),
            Instant::now(),
        )
    }

    /// A session on a `pages`-page document, laid out as the strip the panel
    /// scrolls: every page's geometry known, none of them rasterised.
    fn pdf_session(pages: u32) -> Session {
        let strip: Vec<Page> = (0..pages).map(|_| Page::blank(612.0, 792.0)).collect();
        Session::new(
            Preview::Pages {
                pages: strip,
                words: Vec::new(),
            },
            "report.pdf".into(),
            Rect::new_empty(),
            Instant::now(),
        )
    }

    fn document_session() -> Session {
        use otto_kit::preview::{Block, Span};

        let paragraph = |n: usize| Block::Paragraph {
            spans: vec![Span::plain(format!(
                "Paragraph {n}. {}",
                "Words that wrap. ".repeat(20)
            ))],
        };
        Session::new(
            Preview::Document {
                blocks: (0..80).map(paragraph).collect(),
                truncated: false,
            },
            "notes.md".into(),
            Rect::new_empty(),
            Instant::now(),
        )
    }

    /// The content box of a panel resting in a window of a comfortable size.
    fn content() -> Rect {
        crate::view::peek_content_rect(panel_rect(1100.0, 700.0))
    }

    /// Arrow-keying on must not leave the last file's picture on screen under
    /// the new file's name: what the panel shows always belongs to what is
    /// selected, even while the decode for it is still running.
    #[test]
    fn moving_to_another_file_drops_the_previous_preview() {
        let content = content();
        let mut session = image_session(2000, 1500);
        session.zoom_to(4.0, (content.center_x(), content.center_y()), content);

        session.awaiting("notes.txt".into(), false, Rect::new_empty());

        assert!(matches!(session.preview, Preview::Unavailable { .. }));
        assert_eq!(session.name, "notes.txt");
        // And the zoom and pan of the picture that is gone go with it, so the
        // preview that lands opens at fit like any other.
        assert_eq!(session.zoom, Zoom::FIT);
        assert!(!session.pannable(content));
    }

    /// The waiting panel is inert: there is nothing on it to scroll, and a
    /// gesture over it must not move the content that is on its way.
    #[test]
    fn a_waiting_panel_does_not_scroll() {
        let content = content();
        let mut session = text_session();
        session.awaiting("big.log".into(), false, Rect::new_empty());
        session.scroll_by(20, content);
        assert_eq!(session.first_row, 0);
    }

    /// A session opened before its decode keeps the panel's entrance running
    /// rather than restarting it when the content lands.
    #[test]
    fn waiting_and_landing_are_one_entrance() {
        let opened_at = Instant::now();
        let waiting = Session::waiting("photo.png".into(), false, Rect::new_empty(), opened_at);
        assert!(matches!(waiting.preview, Preview::Unavailable { .. }));
        let mut session = waiting;
        session.awaiting("other.png".into(), false, Rect::new_empty());
        assert_eq!(session.opened_at, opened_at);
    }

    #[test]
    fn a_pinch_zooms_between_fit_and_the_maximum() {
        let content = content();
        let mut session = image_session(2000, 1500);
        let centre = (content.center_x(), content.center_y());

        session.zoom_to(200.0, centre, content);
        assert_eq!(session.zoom.scale, Zoom::MAX);

        // Pinching back out lands on fit exactly, rather than a hair above it.
        session.zoom_to(1.005, centre, content);
        assert!(session.zoom.is_fit(), "{:?}", session.zoom);
    }

    /// A picture no larger than its box has nothing to pan to, so a
    /// two-finger scroll over one must go on meaning what it always meant.
    #[test]
    fn a_fitted_image_does_not_pan() {
        let content = content();
        let mut session = image_session(2000, 1500);
        assert!(!session.pannable(content));
        assert!(!session.pan_by(-120.0, -90.0, content));
        assert_eq!(session.zoom, Zoom::FIT);
    }

    /// A two-finger scroll over a zoomed picture pans it, and covers the
    /// same ground per point of finger travel as a scroll over anything else
    /// — the deltas go in raw and the scroll views scale them, so the
    /// amplification cannot be applied twice or not at all.
    #[test]
    fn a_two_finger_scroll_pans_a_zoomed_picture() {
        let content = content();
        let mut session = image_session(2000, 1500);
        session.zoom_to(3.0, (content.center_x(), content.center_y()), content);
        // Zooming about the centre leaves the picture centred.
        assert_eq!(session.zoom.offset, (0.0, 0.0));

        assert!(session.pan_wheel(1.0, 2.0, content, false, false));
        let speed = otto_kit::components::scroll::wheel_scale();
        let (x, y) = session.zoom.offset;
        assert!((x + speed).abs() < 0.01, "{x} vs {speed}");
        assert!((y + speed * 2.0).abs() < 0.01, "{y} vs {speed}");
    }

    /// The scroll views and the zoom are two accounts of one pan, and a
    /// pinch writes only the second — so the next gesture has to carry on
    /// from where the pinch left the picture rather than from where the
    /// views last had it.
    #[test]
    fn a_pinch_leaves_the_pan_where_it_put_the_picture() {
        let content = content();
        let mut session = image_session(2000, 1500);
        // Pan first, so the views hold an offset of their own …
        session.pan_wheel(1.0, 0.0, content, true, false);
        session.zoom_to(4.0, (content.center_x(), content.center_y()), content);
        // … which the pinch about the centre has just overruled.
        assert_eq!(session.zoom.offset, (0.0, 0.0));

        session.pan_wheel(1.0, 0.0, content, false, false);
        let speed = otto_kit::components::scroll::wheel_scale();
        let x = session.zoom.offset.0;
        assert!((x + speed).abs() < 0.01, "{x} vs {speed}");
    }

    /// A pinch brings the bars up by itself. Zooming in is the moment the
    /// picture stops fitting, which is exactly what a bar is there to say.
    #[test]
    fn zooming_in_brings_the_bars_up() {
        let content = content();
        let mut session = image_session(2000, 1500);
        // Nothing to show and nothing to run, at fit.
        assert!(!session.pan_animating());

        session.zoom_to(3.0, (content.center_x(), content.center_y()), content);
        assert!(session.pan_animating());

        session.tick_pan(content);
        let (horizontal, vertical) = session.pan_bars();
        assert!(horizontal.scrollbar_opacity() > 0.0);
        assert!(vertical.scrollbar_opacity() > 0.0);
    }

    /// Pulling past an edge stretches the picture with the fingers, and
    /// letting go leaves a spring to run — the rubber band every other
    /// scroll in the toolkit has, reaching the picture through the one part
    /// of a zoom the clamp does not touch.
    #[test]
    fn pulling_past_the_edge_stretches_the_picture() {
        let content = content();
        let mut session = image_session(2000, 1500);
        session.zoom_to(3.0, (content.center_x(), content.center_y()), content);

        // One long gesture, well past the stop.
        for _ in 0..40 {
            session.pan_wheel(-40.0, 0.0, content, false, false);
        }
        assert!(session.zoom.band.0 > 0.0, "{:?}", session.zoom);

        let inner = otto_kit::preview::layout(content, &session.preview, 0, Zoom::FIT).inner;
        let drawn = otto_kit::preview::layout(content, &session.preview, 0, session.zoom).content;
        assert!(drawn.left > inner.left, "{drawn:?} {inner:?}");

        // Fingers up: the spring has somewhere to bring it back from.
        session.pan_wheel(0.0, 0.0, content, true, false);
        assert!(session.pan_animating());
    }

    /// Bars over a picture with something under the fold, and none over one
    /// that fits — the same rule every other scroll view in the toolkit
    /// follows.
    #[test]
    fn only_a_zoomed_picture_gets_bars() {
        use otto_kit::components::scroll::ScrollRenderer;

        let content = content();
        let mut session = image_session(2000, 1500);
        session.pan_wheel(0.0, 0.0, content, false, false);
        let (horizontal, vertical) = session.pan_bars();
        assert!(!ScrollRenderer::scrollbar_visible(horizontal));
        assert!(!ScrollRenderer::scrollbar_visible(vertical));

        session.zoom_to(3.0, (content.center_x(), content.center_y()), content);
        let (horizontal, vertical) = session.pan_bars();
        assert!(ScrollRenderer::scrollbar_visible(horizontal));
        assert!(ScrollRenderer::scrollbar_visible(vertical));
    }

    /// Zoomed in, panning stops with the picture still covering the box —
    /// it can never be dragged off the panel and left showing nothing.
    #[test]
    fn a_zoomed_image_pans_but_cannot_be_dragged_off_the_panel() {
        let content = content();
        let mut session = image_session(2000, 1500);
        session.zoom_to(3.0, (content.center_x(), content.center_y()), content);
        assert!(session.pannable(content));
        assert!(session.pan_by(-40.0, -30.0, content));

        // Far past any stop, in both directions, and the picture still
        // reaches both edges of the box it is looked at through.
        session.pan_by(-100_000.0, -100_000.0, content);
        let drawn = otto_kit::preview::layout(content, &session.preview, 0, session.zoom).content;
        let inner = otto_kit::preview::layout(content, &session.preview, 0, Zoom::FIT).inner;
        assert!(drawn.right >= inner.right, "{drawn:?} {inner:?}");
        assert!(drawn.bottom >= inner.bottom, "{drawn:?} {inner:?}");
        assert!(drawn.left <= inner.left, "{drawn:?} {inner:?}");
    }

    /// Zoom is an image affordance. A text preview keeps scrolling, and a
    /// pinch over one changes nothing.
    #[test]
    fn text_previews_scroll_and_do_not_zoom() {
        let content = content();
        let mut session = text_session();
        session.zoom_to(4.0, (content.center_x(), content.center_y()), content);
        assert!(session.zoom.is_fit());
        assert!(!session.pannable(content));

        // And a scroll fed to the pan moves nothing, so a host that asks it
        // first still ends up scrolling by rows.
        assert!(!session.pan_wheel(0.0, 3.0, content, false, false));

        session.scroll_by(5, content);
        assert_eq!(session.first_row, 5);
    }

    /// A Markdown document is longer than its panel like any other text, and
    /// scrolls the same way — by its *wrapped* lines, which only the layout
    /// counts. Missing that arm left a document pinned to its first screen.
    #[test]
    fn a_document_scrolls_by_its_wrapped_lines() {
        let content = content();
        let mut session = document_session();

        session.scroll_by(5, content);
        assert_eq!(session.first_row, 5);

        // And it stops at the end rather than scrolling off it: the last
        // screenful stays on screen.
        session.scroll_by(100_000, content);
        let lines = otto_kit::preview::layout(content, &session.preview, 0, session.zoom)
            .doc_lines
            .len();
        assert!(
            session.first_row > 0 && session.first_row < lines,
            "{lines}"
        );
    }

    /// The page keys move a document to the next page's top edge, and stop
    /// at both ends — where they report that they did nothing, which is what
    /// lets the host hand the keystroke back to the listing rather than
    /// swallowing it.
    #[test]
    fn a_pdf_turns_its_pages_and_stops_at_both_ends() {
        let content = content();
        let mut session = pdf_session(15);
        assert_eq!(session.paged(content), Some((1, 15)));
        assert_eq!(session.page_turn(1, content), Some(2));
        assert_eq!(session.page_turn(-1, content), None);

        assert!(session.scroll_to_page(2, content));
        assert_eq!(session.paged(content), Some((2, 15)));
        assert_eq!(session.page_turn(-1, content), Some(1));

        session.scroll_to_page(15, content);
        assert_eq!(session.paged(content), Some((15, 15)));
        assert_eq!(session.page_turn(1, content), None);

        // A single-page document is not paginated at all: its page keys were
        // never the preview's to take.
        assert_eq!(pdf_session(1).paged(content), None);
        assert_eq!(pdf_session(1).page_turn(1, content), None);
        // Neither is anything that is not a document.
        assert_eq!(text_session().page_turn(1, content), None);
    }

    /// A document is one long strip that scrolls with the same views,
    /// momentum and bars a zoomed picture is panned with — not a picture that
    /// is replaced a page at a time.
    #[test]
    fn a_document_scrolls_its_pages_as_one_strip() {
        let content = content();
        let mut session = pdf_session(15);
        assert!(
            session.pannable(content),
            "fifteen pages are longer than the box"
        );

        // A two-finger scroll moves it, and goes on moving it when the
        // fingers lift.
        let before = preview::clamp_zoom(content, &session.preview, session.zoom)
            .offset
            .1;
        assert!(session.pan_wheel(0.0, 40.0, content, false, false));
        let after = session.zoom.offset.1;
        assert!(after < before, "the strip moved up: {after} from {before}");
        session.pan_wheel(0.0, 0.0, content, true, false);
        assert!(session.pan_animating(), "the fling is still running");

        // And it stops at the end rather than scrolling past it.
        session.pan_by(0.0, -1_000_000.0, content);
        let layout = preview::layout(content, &session.preview, 0, session.zoom);
        assert!(
            layout.content.bottom <= layout.inner.bottom + 1.0,
            "{:?} past {:?}",
            layout.content,
            layout.inner
        );
        assert_eq!(session.paged(content), Some((15, 15)));
    }

    /// Pages arrive as they are scrolled to, with the next ones asked for
    /// before they are on screen — and the ones left far behind are let go
    /// of: a three-hundred-page document is not three hundred images.
    #[test]
    fn pages_are_rasterised_before_they_are_reached_and_dropped_after() {
        let content = content();
        let mut session = pdf_session(40);
        let wanted = session.pages_wanted(content, 2.0);
        let pages: Vec<u32> = wanted.iter().map(|request| request.page).collect();
        assert_eq!(pages.first(), Some(&1), "the page being read comes first");
        assert!(
            pages.contains(&2) && pages.contains(&3),
            "and the pages about to be reached: {pages:?}"
        );
        assert!(!pages.contains(&40), "not one forty pages down");

        // Asked for at the width it is drawn at, not the panel's.
        let drawn = preview::layout(content, &session.preview, 0, session.zoom).page_rects[0];
        assert_eq!(wanted[0].width, (drawn.width() * 2.0).ceil() as u32);

        assert!(session.attach_page(1, flat_pixels(wanted[0].width, 792), content));
        assert!(session.pages()[0].pixels.is_some());
        assert!(
            !session
                .pages_wanted(content, 2.0)
                .iter()
                .any(|request| request.page == 1),
            "a page that is there is not asked for again"
        );

        // Scrolled far away, the pages held for the top of the document are
        // dropped when the next one lands.
        session.scroll_to_page(30, content);
        assert!(session.attach_page(30, flat_pixels(600, 792), content));
        assert!(
            session.pages()[0].pixels.is_none(),
            "page one is behind us now"
        );
        assert!(session.pages()[29].pixels.is_some());
    }

    /// Zooming in past what a page was rasterised at asks for it again, wider
    /// — and zooming back out does not, because a page drawn smaller than its
    /// pixels is supersampled rather than soft.
    #[test]
    fn a_page_is_rasterised_again_when_it_is_zoomed_past() {
        let content = content();
        let mut session = pdf_session(4);
        let width = session.pages_wanted(content, 1.0)[0].width;
        session.attach_page(1, flat_pixels(width, 792), content);
        assert!(!session
            .pages_wanted(content, 1.0)
            .iter()
            .any(|request| request.page == 1));

        session.zoom_to(3.0, (content.center_x(), content.center_y()), content);
        let again = session.pages_wanted(content, 1.0);
        let first = again
            .iter()
            .find(|request| request.page == session.showing_page(content) as u32 + 1)
            .expect("the page being read is asked for again");
        assert!(
            first.width > width,
            "{} is no wider than {width}",
            first.width
        );
    }

    /// The text layer lands on the strip it was measured against, and a drag
    /// down the page selects the words in it.
    #[test]
    fn a_document_selects_the_text_the_file_carries() {
        let content = content();
        let mut session = pdf_session(2);
        let measured = vec![Page::blank(612.0, 792.0), Page::blank(612.0, 792.0)];
        let words = vec![
            doc_word("Quarterly", 72, 96, 0, 0),
            doc_word("report", 150, 96, 0, 0),
            // On the second page, which starts one page and one gap down.
            doc_word("Appendix", 72, 96 + 792 + 14, 1, 1),
        ];
        assert!(session.attach_text_layer(measured, words));
        assert_eq!(session.words().len(), 3);

        let layout = preview::layout(content, &session.preview, 0, session.zoom);
        let word = layout.content.left + layout.content.width() * (72.0 + 20.0) / 612.0;
        let line = layout.content.top
            + layout.content.height() * (96.0 + 6.0) / preview::strip_size(session.pages()).1;
        assert_eq!(
            session.word_at(word, line, content),
            Some(0),
            "the first word is under the pointer"
        );
        assert!(session.select_pointer_down(word, line, content));
        assert_eq!(session.selected_text().as_deref(), Some("Quarterly"));
    }

    /// One word of a document's text layer, boxed in the strip's points.
    fn doc_word(text: &str, left: u32, top: u32, block: u32, line: u32) -> Word {
        Word {
            text: text.into(),
            left,
            top,
            width: 60,
            height: 12,
            confidence: 100,
            block,
            paragraph: line,
            line,
        }
    }

    /// A page's worth of blank pixels.
    fn flat_pixels(width: u32, height: u32) -> Pixels {
        Pixels {
            width,
            height,
            intrinsic_width: width,
            intrinsic_height: height,
            data: vec![255; (width * height * 4) as usize],
            frame_delays: Vec::new(),
            words: Vec::new(),
        }
    }

    #[test]
    fn the_panel_is_centred_and_inside_the_window() {
        let panel = panel_rect(1100.0, 700.0);
        assert!(panel.left > 0.0 && panel.right < 1100.0, "{panel:?}");
        assert!((panel.center_x() - 550.0).abs() < 0.5, "{panel:?}");
        assert!((panel.center_y() - 350.0).abs() < 0.5, "{panel:?}");
    }

    #[test]
    fn a_small_window_still_gets_a_panel_with_area() {
        let panel = panel_rect(320.0, 240.0);
        assert!(panel.width() > 0.0 && panel.height() > 0.0, "{panel:?}");
    }

    /// The entrance starts at the row and ends at the resting rect — the whole
    /// point of carrying the anchor.
    #[test]
    fn the_panel_grows_out_of_the_row() {
        let resting = panel_rect(1100.0, 700.0);
        let row = Rect::from_xywh(240.0, 180.0, 260.0, 24.0);

        let start = entrance_at(row, resting, 0.0);
        assert!(start.width() < resting.width() / 2.0, "{start:?}");
        // Near the row it came from, not the middle of the window.
        assert!((start.center_x() - row.center_x()).abs() < 1.0, "{start:?}");

        let end = entrance_at(row, resting, 1.0);
        assert!((end.width() - resting.width()).abs() < 1.0, "{end:?}");
    }

    /// A row scrolled out of view has no anchor; the panel swells in place
    /// rather than growing out of nowhere.
    #[test]
    fn no_anchor_opens_in_place() {
        let resting = panel_rect(1100.0, 700.0);
        let start = entrance_at(Rect::new_empty(), resting, 0.0);
        assert!(
            (start.center_x() - resting.center_x()).abs() < 1.0,
            "{start:?}"
        );
        assert!(start.width() < resting.width(), "{start:?}");
        assert!(start.width() > resting.width() * 0.9, "{start:?}");
    }

    #[test]
    fn the_panel_says_the_recogniser_is_running_until_it_is_not() {
        let mut session = image_session(400, 120);
        // Nothing running, nothing to say.
        assert!(session.recognising_phase().is_none());

        session.start_recognising(Instant::now());
        assert!(session.recognising_phase().is_some());

        // Finishing with nothing found puts the badge away rather than
        // leaving it promising text that is not there.
        session.stop_recognising();
        assert!(session.recognising_phase().is_none());
        assert!(session.words().is_empty());
    }

    /// The pass over the folder runs below whatever the desktop is doing; a
    /// preview somebody is waiting for runs alongside it. Both at the same
    /// priority is the case this exists to prevent — two recognisers sharing
    /// the cores while the badge breathes.
    #[test]
    fn work_nobody_asked_for_runs_below_work_somebody_did() {
        assert_eq!(Priority::Interactive.nice(), 0);
        assert!(Priority::Background.nice() > Priority::Interactive.nice());
    }
}
