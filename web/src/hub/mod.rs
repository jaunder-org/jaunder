//! Hub — the Settings and Account configuration hubs and the page-header corner
//! controls that reach them (#1733). A server-less vertical (ADR-0070): the pure
//! host-tested catalog plus wasm-only UI.

mod catalog;
#[cfg(target_arch = "wasm32")]
mod component;

pub use catalog::{Hub, HubGroup, HubLink, account_groups, settings_groups};
#[cfg(target_arch = "wasm32")]
pub use component::{AccountPage, HubControls, SettingsPage};
