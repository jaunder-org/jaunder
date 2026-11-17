//! Settings areas — the Settings and Account pages that collect configuration
//! destinations, and the page-header corner controls that reach them (#1733). A
//! server-less vertical (ADR-0070): the pure host-tested catalog plus wasm-only
//! UI.

mod catalog;
#[cfg(target_arch = "wasm32")]
mod component;

pub use catalog::{AreaGroup, AreaLink, SettingsArea, account_groups, settings_groups};
#[cfg(target_arch = "wasm32")]
pub use component::{AccountPage, AreaControls, SettingsPage};
