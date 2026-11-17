//! Backup vertical: operator-gated settings and back-up-now endpoints + the
//! banner/settings-page UI.
mod api;
#[cfg(feature = "server")]
mod server;
#[cfg(feature = "server")]
pub use server::{ManualBackupError, ManualBackupRunner};
#[cfg(target_arch = "wasm32")]
mod component;

pub use api::{
    BackUpNow, GetSettings, IsWarningVisible, UpdateSettings, back_up_now, get_settings,
    is_warning_visible, update_settings,
};
#[cfg(target_arch = "wasm32")]
pub use component::{BackupBanner, BackupSettingsPage};
