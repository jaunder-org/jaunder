//! Site settings vertical: operator-gated site-identity endpoints + the
//! settings-page UI.
mod api;
#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{get_site_identity, update_site_identity, GetSiteIdentity, UpdateSiteIdentity};
#[cfg(target_arch = "wasm32")]
pub use component::SiteSettingsPage;
