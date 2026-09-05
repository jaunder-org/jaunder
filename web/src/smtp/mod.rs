//! Operator-managed SMTP relay settings.

mod api;
#[cfg(target_arch = "wasm32")]
mod component;
mod state;
pub use api::{
    GetSettings, Settings, UpdateSettings, UpdateSettingsRequest, get_settings, update_settings,
};
#[cfg(target_arch = "wasm32")]
pub use component::SmtpSettingsPage;
pub use state::{SmtpFormState, SmtpPasswordIntent, SmtpUpdateDraft};
