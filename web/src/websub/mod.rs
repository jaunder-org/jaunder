//! Operator `WebSub` configuration and recovery vertical.
mod api;
#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(feature = "server")]
mod server;

pub use api::{
    DeadLetterCursor, DeadLetterPage, DeadLetterRow, GetWebsubSettings, ListDeadLetters,
    RedriveDeadLetters, UpdateWebsubHub, WebsubPhase, WebsubSettings, get_websub_settings,
    list_dead_letters, redrive_dead_letters, update_websub_hub,
};
#[cfg(target_arch = "wasm32")]
pub use component::WebsubPage;
#[cfg(feature = "server")]
pub use server::{WebsubPublisher, WebsubPublisherError};
