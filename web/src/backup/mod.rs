//! Backup settings vertical: operator-gated settings endpoints + the banner/
//! settings-page UI.
mod api;
#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{
    get_settings, is_warning_visible, update_settings, GetSettings, IsWarningVisible,
    UpdateSettings,
};
#[cfg(target_arch = "wasm32")]
pub use component::{BackupBanner, BackupSettingsPage};
