//! Site settings vertical: operator-gated site-identity endpoints + the
//! settings-page UI.
mod api;
#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{
    get_identity, is_base_url_warning_visible, update_identity, GetIdentity,
    IsBaseUrlWarningVisible, UpdateIdentity,
};
#[cfg(target_arch = "wasm32")]
pub use component::{SiteBaseUrlBanner, SiteSettingsPage};
