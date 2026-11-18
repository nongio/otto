//! Otto's view of the machine's network, through NetworkManager.
//!
//! [`model`] is the snapshot every network UI draws from (the devices, the
//! Wi-Fi networks in range folded one per name, the saved networks, the VPNs
//! and the connection in use) with the decisions they share: list order,
//! signal bars, what the top bar's indicator shows. [`read`] fills a snapshot
//! from the system bus, the action functions ask NetworkManager to change
//! something, and [`watch`] says when to read again. NetworkManager keeps all
//! the state; nothing here caches it.
//!
//! The calls that touch the radios' kill switches use the machine's own;
//! each has a `_with` variant taking [`rfkill::Radios`], for a test that must
//! leave the host's radios alone.
//!
//! The functions are async on the caller's tokio runtime. [`blocking`] runs
//! them from a plain thread, as Settings does.
//!
//! The contract is `specs/network.md`.

#![warn(unreachable_pub)]

pub mod blocking;
mod client;
pub mod model;
pub mod profile;
pub mod reveal;
pub mod rfkill;

pub use client::{
    activate, connect_hidden, connect_wifi, connect_wifi_with_password, deactivate,
    disconnect_device, follow, follow_with, forget, read, read_with, request_scan, set_autoconnect,
    set_ip_config, set_mac_policy, set_metered, set_wifi_enabled, set_wifi_enabled_with, watch,
    watch_with, wifi_share,
};
pub use model::*;
pub use profile::*;
pub use reveal::ShareError;
