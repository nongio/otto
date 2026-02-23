/// Shared abstraction for foreign toplevel management across different protocols
///
/// This module provides a unified interface for both:
/// - ext-foreign-toplevel-list-v1 (newer, Smithay built-in)
/// - wlr-foreign-toplevel-management-unstable-v1 (older, wlroots protocol)
use smithay::wayland::foreign_toplevel_list::ForeignToplevelHandle as ExtHandle;

use super::wlr_foreign_toplevel::WlrForeignToplevelHandle;

/// Combined handle that manages both protocol handles
pub struct ForeignToplevelHandles {
    /// ext-foreign-toplevel-list handle (newer protocol)
    pub ext: Option<ExtHandle>,
    /// wlr-foreign-toplevel-management handle (older wlroots protocol)
    pub wlr: Option<WlrForeignToplevelHandle>,
}

impl ForeignToplevelHandles {
    pub fn new(ext: ExtHandle, wlr: WlrForeignToplevelHandle) -> Self {
        Self {
            ext: Some(ext),
            wlr: Some(wlr),
        }
    }

    pub fn send_title(&self, title: &str) {
        if let Some(ext) = &self.ext {
            ext.send_title(title);
        }
        if let Some(wlr) = &self.wlr {
            wlr.send_title(title.to_string());
        }
    }

    pub fn send_app_id(&self, app_id: &str) {
        if let Some(ext) = &self.ext {
            ext.send_app_id(app_id);
        }
        if let Some(wlr) = &self.wlr {
            wlr.send_app_id(app_id.to_string());
        }
    }

    pub fn send_done(&self) {
        if let Some(ext) = &self.ext {
            ext.send_done();
        }
        // wlr protocol doesn't have a done event
    }

    pub fn send_closed(&self) {
        if let Some(ext) = &self.ext {
            ext.send_closed();
        }
        if let Some(wlr) = &self.wlr {
            wlr.send_closed();
        }
    }

    /// Broadcast window state (activated, minimized, maximized, fullscreen) via wlr protocol.
    /// The ext-foreign-toplevel-list protocol does not carry state events.
    pub fn send_state(&self, activated: bool, minimized: bool, maximized: bool, fullscreen: bool) {
        if let Some(wlr) = &self.wlr {
            wlr.send_state(activated, minimized, maximized, fullscreen);
        }
    }

    /// Get title from ext handle (they should be in sync)
    pub fn title(&self) -> String {
        self.ext
            .as_ref()
            .map(|h| h.title().to_string())
            .or_else(|| self.wlr.as_ref().map(|h| h.title().clone()))
            .unwrap_or_default()
    }

    /// Get app_id from ext handle (they should be in sync)
    pub fn app_id(&self) -> String {
        self.ext
            .as_ref()
            .map(|h| h.app_id().to_string())
            .or_else(|| self.wlr.as_ref().map(|h| h.app_id().clone()))
            .unwrap_or_default()
    }
}
