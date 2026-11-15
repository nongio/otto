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
//! The functions are async on the caller's tokio runtime. [`blocking`] runs
//! them from a plain thread, as Settings does.
//!
//! The contract is `specs/network.md`.

#![warn(unreachable_pub)]

pub mod blocking;
mod client;
pub mod model;
pub mod profile;

pub use client::{
    activate, connect_hidden, connect_wifi, connect_wifi_with_password, deactivate,
    disconnect_device, follow, forget, is_running, read, request_scan, set_autoconnect,
    set_ip_config, set_wifi_enabled, watch,
};
pub use model::*;
pub use profile::*;
