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

#[cfg(test)]
mod tests {
    use super::DefaultPostFormatState;
    use crate::error::WebError;
    use common::render::PostFormat;

    #[test]
    fn default_post_format_loading_cannot_dispatch() {
        let state = DefaultPostFormatState::resolve(None);

        assert_eq!(state, DefaultPostFormatState::Loading);
        assert_eq!(state.format_to_save(), None);
    }

    #[test]
    fn default_post_format_failure_cannot_dispatch_or_fabricate_markdown() {
        let failed = Err(WebError::server_message("boom"));
        let state = DefaultPostFormatState::resolve(Some(&failed));

        assert_eq!(state, DefaultPostFormatState::Failed);
        assert_eq!(state.format_to_save(), None);
    }

    #[test]
    fn default_post_format_ready_dispatches_the_fetched_format() {
        let ready = Ok(PostFormat::Org);
        let state = DefaultPostFormatState::resolve(Some(&ready));

        assert_eq!(state, DefaultPostFormatState::Ready(PostFormat::Org));
        assert_eq!(state.format_to_save(), Some(PostFormat::Org));
    }
}
