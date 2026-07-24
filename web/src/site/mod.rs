//! Site settings vertical: operator-gated site-identity endpoints + the
//! settings-page UI.
mod api;
#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{
    base_url_warning_visible, get_site_identity, update_site_identity, BaseUrlWarningVisible,
    GetSiteIdentity, UpdateSiteIdentity,
};
#[cfg(target_arch = "wasm32")]
pub use component::{SiteBaseUrlBanner, SiteSettingsPage};
