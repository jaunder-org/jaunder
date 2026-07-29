//! Backup settings vertical: operator-gated settings endpoints + the banner/
//! settings-page UI.
mod api;
#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{
    get_settings, update_settings, warning_visible, GetSettings, UpdateSettings, WarningVisible,
};
#[cfg(target_arch = "wasm32")]
pub use component::{BackupBanner, BackupSettingsPage};
