//! Host-compiled decision state for the profile page's default-post-format control.
//!
//! The persisted preference is loaded asynchronously by the wasm-only component, but
//! deciding whether a Save may carry a format is pure. Keeping that decision here makes
//! the loading and failure arms explicit and prevents either from inventing a format.

use common::render::PostFormat;

use crate::error::WebError;

/// Resolution state for the persisted default post format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultPostFormatState {
    /// The preference request has not settled.
    Loading,
    /// The server returned the persisted format.
    Ready(PostFormat),
    /// The preference request failed.
    Failed,
}

impl DefaultPostFormatState {
    /// Fold the resource's unresolved/resolved shape into the page state without
    /// taking ownership of a returned server error.
    #[must_use]
    pub fn resolve(result: Option<&Result<PostFormat, WebError>>) -> Self {
        match result {
            None => Self::Loading,
            Some(Ok(format)) => Self::Ready(*format),
            Some(Err(_)) => Self::Failed,
        }
    }

    /// The format a Save action may dispatch, if the load succeeded.
    #[must_use]
    pub const fn format_to_save(self) -> Option<PostFormat> {
        match self {
            Self::Loading | Self::Failed => None,
            Self::Ready(format) => Some(format),
        }
    }
}
