//! Raw browser dialog primitive (`window.confirm`).

pub use crate::telemetry::{ConfirmOutcome, DialogError};

/// Show a native confirm dialog.
///
/// Cancellation and an absent browser window are expected no-action outcomes;
/// only a thrown `window.confirm` call returns [`DialogError`].
///
/// # Errors
///
/// Returns [`DialogError`] when the browser throws while opening the dialog.
pub fn confirm(message: &str) -> Result<ConfirmOutcome, DialogError> {
    let Some(window) = web_sys::window() else {
        return Ok(ConfirmOutcome::Unavailable);
    };
    window
        .confirm_with_message(message)
        .map(|confirmed| {
            if confirmed {
                ConfirmOutcome::Confirmed
            } else {
                ConfirmOutcome::Cancelled
            }
        })
        .map_err(|_| DialogError)
}
