//! Site settings vertical: operator-gated site-identity endpoints + the
//! settings-page UI.
mod api;
#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{
    base_url_warning_visible, get_identity, update_identity, BaseUrlWarningVisible, GetIdentity,
    UpdateIdentity,
};
#[cfg(target_arch = "wasm32")]
pub use component::{SiteBaseUrlBanner, SiteSettingsPage};
