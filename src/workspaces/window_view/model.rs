use core::fmt;
use smithay::{
    backend::renderer::utils::CommitCounter, reexports::wayland_server::backend::ObjectId,
    utils::Transform,
};
use std::hash::{Hash, Hasher};

#[derive(Clone)]
pub struct WindowViewSurface {
    pub(crate) id: ObjectId,
    pub(crate) parent_id: Option<ObjectId>, // Parent surface ID for hierarchy
    pub(crate) phy_src_x: f32,
    pub(crate) phy_src_y: f32,
    pub(crate) phy_src_w: f32,
    pub(crate) phy_src_h: f32,
    pub(crate) phy_dst_x: f32,
    pub(crate) phy_dst_y: f32,
    pub(crate) phy_dst_w: f32,
    pub(crate) phy_dst_h: f32,
    pub(crate) log_offset_x: f32,
    pub(crate) log_offset_y: f32,
    pub(crate) texture_id: Option<u32>,
    pub(crate) commit: CommitCounter,
    pub(crate) transform: Transform,
}
impl fmt::Debug for WindowViewSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowViewSurface")
            .field("id", &self.id)
            .field("parent_id", &self.parent_id)
            .field("src_x", &self.phy_src_x)
            .field("src_y", &self.phy_src_y)
            .field("src_w", &self.phy_src_w)
            .field("src_h", &self.phy_src_h)
            .field("dst_x", &self.phy_dst_x)
            .field("dst_y", &self.phy_dst_y)
            .field("dst_w", &self.phy_dst_w)
            .field("dst_h", &self.phy_dst_h)
            .field("offset_x", &self.log_offset_x)
            .field("offset_y", &self.log_offset_y)
            .field("commit", &self.commit)
            .field("transform", &self.transform)
            .finish()
    }
}

#[derive(Clone)]
pub struct WindowViewBaseModel {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub title: String,
    pub fullscreen: bool,
    pub active: bool,
}

/// State of a server-side decoration (the titlebar Otto draws above a client
/// surface). Everything here feeds otto-kit's `WindowDecoration`, which is the
/// component that actually paints it — the same one otto-kit clients use, so
/// server- and client-drawn titlebars stay identical.
#[derive(Clone, Debug, Default)]
pub struct WindowDecorationModel {
    /// Titlebar width in logical points
    pub width: f32,
    /// Titlebar height in logical points
    pub height: f32,
    pub title: String,
    pub active: bool,
    pub dark: bool,
    /// Window frame corner radius; 0 while maximized or tiled
    pub corner_radius: f32,
    /// Pointer is over the traffic lights
    pub controls_hovered: bool,
    /// Index of the control being held down, if any: 0 close, 1 minimize,
    /// 2 zoom. Kept as an index so the model stays free of otto-kit types.
    pub pressed: Option<u8>,
    /// The window is being screencast — the titlebar shows a sharing badge.
    pub sharing: bool,
    /// The window is pinned to one size (min == max), so it has no maximized
    /// form: its zoom control is drawn gray and does nothing.
    pub fixed_size: bool,
    pub scale: f32,
}

impl Hash for WindowDecorationModel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.width.to_bits().hash(state);
        self.height.to_bits().hash(state);
        self.title.hash(state);
        self.active.hash(state);
        self.dark.hash(state);
        self.corner_radius.to_bits().hash(state);
        self.controls_hovered.hash(state);
        self.pressed.hash(state);
        self.sharing.hash(state);
        self.fixed_size.hash(state);
        self.scale.to_bits().hash(state);
    }
}

impl Hash for WindowViewBaseModel {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.w.to_bits().hash(state);
        self.h.to_bits().hash(state);
        self.active.hash(state);
    }
}
impl Hash for WindowViewSurface {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let distance = self
            .commit
            .distance(Some(CommitCounter::default()))
            .unwrap_or(0);
        distance.hash(state);
        if let Some(tid) = self.texture_id {
            tid.hash(state);
        }
        self.id.hash(state);
        if let Some(ref parent_id) = self.parent_id {
            parent_id.hash(state);
        }
        self.phy_src_x.to_bits().hash(state);
        self.phy_src_y.to_bits().hash(state);
        self.phy_src_w.to_bits().hash(state);
        self.phy_src_h.to_bits().hash(state);
        self.phy_dst_x.to_bits().hash(state);
        self.phy_dst_y.to_bits().hash(state);
        self.phy_dst_w.to_bits().hash(state);
        self.phy_dst_h.to_bits().hash(state);
        self.log_offset_x.to_bits().hash(state);
        self.log_offset_y.to_bits().hash(state);
    }
}
