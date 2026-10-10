//! New-Post format initialization without inventing a saved preference.
//!
//! The browser owns requests and retry wiring; this fold owns readiness and
//! adoption so a delayed preference cannot erase composition or explicit choices.

use common::render::PostFormat;
use leptos::prelude::*;
use strum::EnumMessage;

use crate::error::WebResult;
use crate::posts::{ComposeState, CreationComposerSnapshot};

/// Only a successful preference lookup authorizes new-Post submission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InitialFormatState {
    #[default]
    Loading,
    Ready,
    Failed,
}

impl InitialFormatState {
    #[must_use]
    pub const fn can_submit(self) -> bool {
        matches!(self, Self::Ready)
    }

    #[must_use]
    pub const fn failed(self) -> bool {
        matches!(self, Self::Failed)
    }
}

/// Adopt initialization once; later responses cannot replace an author's format.
pub fn settle_default_format(
    result: &WebResult<PostFormat>,
    initial: RwSignal<InitialFormatState>,
    state: ComposeState,
    baseline: RwSignal<CreationComposerSnapshot>,
) {
    if initial.get_untracked().can_submit() {
        return;
    }
    match result {
        Ok(format) => {
            state.format.set(*format);
            baseline.update(|snapshot| *snapshot = snapshot.clone().with_format(*format));
            initial.set(InitialFormatState::Ready);
        }
        Err(_) => initial.set(InitialFormatState::Failed),
    }
}

/// Never label an unresolved preference with the internal placeholder format.
#[must_use]
pub fn format_control_value(format: PostFormat, ready: bool) -> String {
    if ready {
        format.get_message().unwrap_or("HTML").to_owned()
    } else {
        "Unresolved".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::WebError;
    use crate::forms::Field;
    use common::slug::Slug;

    #[test]
    fn preference_resolution_preserves_body_edits_and_format_ownership() {
        Owner::new().with(|| {
            let state = ComposeState::new();
            let slug = Field::<Slug>::optional();
            let baseline = RwSignal::new(CreationComposerSnapshot::capture(
                state,
                slug,
                String::new(),
                String::new(),
            ));
            let initial = RwSignal::new(InitialFormatState::default());
            assert_eq!(initial.get(), InitialFormatState::Loading);
            assert!(!initial.get().can_submit());
            assert!(!initial.get().failed());
            state.body.set_value("draft in progress");
            settle_default_format(&Ok(PostFormat::Org), initial, state, baseline);
            assert_eq!(initial.get(), InitialFormatState::Ready);
            assert_eq!(state.format.get(), PostFormat::Org);
            assert_eq!(state.body.value(), "draft in progress");
            assert_ne!(
                baseline.get(),
                CreationComposerSnapshot::capture(state, slug, String::new(), String::new()),
            );
            state.body.reset();
            assert_eq!(
                baseline.get(),
                CreationComposerSnapshot::capture(state, slug, String::new(), String::new()),
            );
            state.switch_format(PostFormat::Markdown);
            settle_default_format(&Ok(PostFormat::Org), initial, state, baseline);
            settle_default_format(
                &Err(WebError::server_message("late failure")),
                initial,
                state,
                baseline,
            );
            state.reset();
            assert_eq!(state.format.get(), PostFormat::Markdown);
            assert!(initial.get().can_submit());
        });
    }

    #[test]
    fn failed_preference_blocks_until_retry_resolves_each_supported_format() {
        Owner::new().with(|| {
            for format in [PostFormat::Markdown, PostFormat::Org, PostFormat::Html] {
                let state = ComposeState::new();
                let baseline = RwSignal::new(CreationComposerSnapshot::capture(
                    state,
                    Field::<Slug>::optional(),
                    String::new(),
                    String::new(),
                ));
                let initial = RwSignal::new(InitialFormatState::Loading);
                settle_default_format(
                    &Err(WebError::server_message("offline")),
                    initial,
                    state,
                    baseline,
                );
                assert!(initial.get().failed());
                assert!(!initial.get().can_submit());
                initial.set(InitialFormatState::Loading);
                settle_default_format(&Ok(format), initial, state, baseline);
                assert_eq!(state.format.get(), format);
                assert!(initial.get().can_submit());
                assert!(!initial.get().failed());
            }
        });
    }

    #[test]
    fn control_labels_distinguish_unresolved_and_all_saved_formats() {
        assert_eq!(
            format_control_value(PostFormat::Markdown, false),
            "Unresolved"
        );
        assert_eq!(format_control_value(PostFormat::Markdown, true), "Markdown");
        assert_eq!(format_control_value(PostFormat::Org, true), "Org");
        assert_eq!(format_control_value(PostFormat::Html, true), "HTML");
    }
}
