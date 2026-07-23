//! Backup settings vertical: operator-gated settings endpoints + the banner/
//! settings-page UI.
mod api;
#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(feature = "server")]
pub(crate) mod server;

pub use api::{
    backup_warning_visible, get_backup_settings, update_backup_settings, BackupWarningVisible,
    GetBackupSettings, UpdateBackupSettings,
};
#[cfg(target_arch = "wasm32")]
pub use component::{BackupBanner, BackupSettingsPage};
