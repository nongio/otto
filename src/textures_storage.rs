use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

use crate::renderer::SkiaTextureImage;
use smithay::reexports::wayland_server::backend::ObjectId;

static TEXTURES_STORAGE: OnceLock<Mutex<HashMap<ObjectId, SkiaTextureImage>>> = OnceLock::new();

fn store() -> &'static Mutex<HashMap<ObjectId, SkiaTextureImage>> {
    TEXTURES_STORAGE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Best-effort insert/update. Returns false if lock could not be acquired.
pub fn set(id: &ObjectId, tex: SkiaTextureImage) -> bool {
    if let Ok(mut map) = store().try_lock() {
        map.insert(id.clone(), tex);
        true
    } else {
        false
    }
}

/// Best-effort snapshot. Returns empty if lock could not be acquired.
pub fn snapshot() -> Vec<SkiaTextureImage> {
    if let Ok(map) = store().try_lock() {
        map.values().cloned().collect()
    } else {
        Vec::new()
    }
}

/// Best-effort snapshot with keys. Returns empty if lock could not be acquired.
pub fn snapshot_kv() -> Vec<(ObjectId, SkiaTextureImage)> {
    if let Ok(map) = store().try_lock() {
        map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    } else {
        Vec::new()
    }
}

/// Returns a snapshot of the texture for a specific id if available.
pub fn get(id: &ObjectId) -> Option<SkiaTextureImage> {
    store().try_lock().ok().and_then(|map| map.get(id).cloned())
}

/// Best-effort remove. Returns false if lock could not be acquired.
pub fn remove(id: &ObjectId) -> bool {
    if let Ok(mut map) = store().try_lock() {
        map.remove(id);
        true
    } else {
        false
    }
}
