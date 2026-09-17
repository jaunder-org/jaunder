//! Site settings vertical: operator-gated site identity and media-upload-capability
//! endpoints plus the settings-page UI.
mod api;
#[cfg(feature = "server")]
mod server;
#[cfg(feature = "server")]
pub use server::{SiteIdentityPublisher, SiteIdentityPublisherError};
#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{
    GetIdentity, GetMediaUploadsEnabled, IsBaseUrlWarningVisible, UpdateIdentity,
    UpdateIdentityRequest, UpdateMediaUploadsEnabled, get_identity, get_media_uploads_enabled,
    is_base_url_warning_visible, update_identity, update_media_uploads_enabled,
};
#[cfg(target_arch = "wasm32")]
pub use component::{SiteBaseUrlBanner, SiteSettingsPage};
