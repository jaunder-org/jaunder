//! Host-compiled decision state for the profile page's default-post-format control.
//!
//! The persisted preference is loaded asynchronously by the wasm-only component, but
//! deciding whether a Save may carry a format is pure. Keeping that decision here makes
//! the loading and failure arms explicit and prevents either from inventing a format.

use common::{MutationOutcome, render::PostFormat};

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

/// One selected value in an author theme control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeSelection {
    /// No author override is stored; public pages inherit the site theme.
    SiteDefault,
    /// The author override stored for their public pages.
    Theme(common::theme::Theme),
}

impl ThemeSelection {
    #[must_use]
    pub const fn from_override(theme: Option<common::theme::Theme>) -> Self {
        match theme {
            Some(theme) => Self::Theme(theme),
            None => Self::SiteDefault,
        }
    }

    #[must_use]
    pub fn button_class(self, selected: Option<ThemeSelection>) -> &'static str {
        if selected == Some(self) {
            "j-btn is-selected"
        } else {
            "j-btn"
        }
    }

    #[must_use]
    pub fn aria_pressed(self, selected: Option<ThemeSelection>) -> &'static str {
        if selected == Some(self) {
            "true"
        } else {
            "false"
        }
    }
}

/// The persisted selection shown by a theme control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeControlState {
    /// The persisted setting has not been read.
    Loading,
    /// The persisted setting was read successfully.
    Ready(ThemeSelection),
    /// A read failed; retain a confirmed selection when one exists.
    Failed(Option<ThemeSelection>),
}

/// Whether a completed write requires the control to reread persisted state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMutationDecision {
    /// The write is rollback-confirmed failed and requires no reread.
    Error,
    /// A confirmed write must reread before reporting its selection.
    RevalidateConfirmed,
    /// An uncertain commit must reread but remains visibly error-like.
    RevalidateIndeterminate,
}

impl ThemeControlState {
    /// Converts a persisted read into visible state, preserving `last_confirmed`
    /// when the reread fails.
    #[must_use]
    pub fn resolve(
        last_confirmed: Option<ThemeSelection>,
        result: Option<&Result<Option<common::theme::Theme>, WebError>>,
    ) -> Self {
        match result {
            None => Self::Loading,
            Some(Ok(override_theme)) => Self::Ready(ThemeSelection::from_override(*override_theme)),
            Some(Err(_)) => Self::Failed(last_confirmed),
        }
    }

    /// Classifies a mutation acknowledgement without ever treating an
    /// indeterminate commit as confirmed.
    #[must_use]
    pub fn mutation_decision(
        result: &Result<MutationOutcome<()>, WebError>,
    ) -> ThemeMutationDecision {
        match result {
            Ok(MutationOutcome::Confirmed(())) => ThemeMutationDecision::RevalidateConfirmed,
            Ok(MutationOutcome::CommitIndeterminate(())) => {
                ThemeMutationDecision::RevalidateIndeterminate
            }
            Err(_) => ThemeMutationDecision::Error,
        }
    }

    #[must_use]
    pub const fn is_loading(self) -> bool {
        matches!(self, Self::Loading)
    }

    #[must_use]
    pub const fn selection(self) -> Option<ThemeSelection> {
        match self {
            Self::Loading => None,
            Self::Ready(selection) => Some(selection),
            Self::Failed(selection) => selection,
        }
    }
}

#[cfg(test)]
mod theme_tests {
    use super::{ThemeControlState, ThemeMutationDecision, ThemeSelection};
    use crate::error::WebError;
    use common::{MutationOutcome, theme::Theme};

    #[test]
    fn confirmed_mutation_rereads_and_adopts_the_persisted_selection() {
        let outcome = Ok(MutationOutcome::Confirmed(()));
        let reread = Ok(Some(Theme::Reader));

        assert_eq!(
            ThemeControlState::mutation_decision(&outcome),
            ThemeMutationDecision::RevalidateConfirmed
        );
        assert_eq!(
            ThemeControlState::resolve(Some(ThemeSelection::Theme(Theme::Studio)), Some(&reread)),
            ThemeControlState::Ready(ThemeSelection::Theme(Theme::Reader))
        );
    }

    #[test]
    fn indeterminate_mutation_rereads_but_remains_error_like() {
        let outcome = Ok(MutationOutcome::CommitIndeterminate(()));

        assert_eq!(
            ThemeControlState::mutation_decision(&outcome),
            ThemeMutationDecision::RevalidateIndeterminate
        );
    }

    #[test]
    fn failed_reread_retains_last_confirmed_selection() {
        let reread = Err(WebError::server_message("database unavailable"));

        assert_eq!(
            ThemeControlState::resolve(Some(ThemeSelection::Theme(Theme::Terminal)), Some(&reread)),
            ThemeControlState::Failed(Some(ThemeSelection::Theme(Theme::Terminal)))
        );
    }

    #[test]
    fn initial_read_failure_does_not_invent_a_selection() {
        let read = Err(WebError::server_message("database unavailable"));
        let state = ThemeControlState::resolve(None, Some(&read));

        assert_eq!(state, ThemeControlState::Failed(None));
        assert_eq!(state.selection(), None);
    }

    #[test]
    fn missing_override_is_site_default() {
        let reread = Ok(None);

        assert_eq!(
            ThemeControlState::resolve(Some(ThemeSelection::Theme(Theme::Reader)), Some(&reread)),
            ThemeControlState::Ready(ThemeSelection::SiteDefault)
        );
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
