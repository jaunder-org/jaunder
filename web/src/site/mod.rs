//! Site settings vertical: operator-gated site-identity endpoints + the
//! settings-page UI.
mod api;
#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{
    GetIdentity, IsBaseUrlWarningVisible, UpdateIdentity, get_identity,
    is_base_url_warning_visible, update_identity,
};
#[cfg(target_arch = "wasm32")]
pub use component::{SiteBaseUrlBanner, SiteSettingsPage};
