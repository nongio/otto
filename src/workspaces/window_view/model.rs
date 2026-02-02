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
