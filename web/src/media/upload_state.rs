//! Host-compiled, host-tested state and decisions for the media upload widget (#306,
//! ADR-0083).
//!
//! `MediaUpload` carried the vertical's heaviest component body: the picked-file
//! guard, the upload outcome fold, and the reset/notify sequencing all sat inside the
//! wasm-only component where nothing could assert them. Signals bundle into a `Copy`
//! state struct exercised under a reactive `Owner` (the `forms::Field` /
//! `tags::input_state` convention); only `Effect::new` and `spawn_local` stay on the
//! wasm side.
//!
//! Distinct from [`super::format`], which is pure byte/label formatting with no
//! reactive state.

use leptos::prelude::*;

use common::MutationOutcome;
use common::media::UploadedMedia;
use common::root_relative_url::RootRelativeUrl;

use super::MediaDeletion;
use crate::error::WebResult;

/// What one upload attempt resolved to — the fold of the multipart `#[server]` fn's
/// `Result` into the one value both consumers read.
///
/// Failure and indeterminate commits carry the rendered message rather than the typed
/// `WebError` (unlike `timeline::LoadStatus`, ADR-0083): both sinks here are already
/// stringly — the `on_error` prop is a `Callback<String>` and the inline banner paints
/// text. The distinct indeterminate variant lets the media page invalidate stale data
/// without ever treating an uncertain commit as a confirmed upload.
#[derive(Debug, PartialEq, Eq)]
pub enum UploadOutcome {
    Uploaded(RootRelativeUrl),
    Indeterminate(String),
    Failed(String),
}

impl UploadOutcome {
    /// Fold the server fn's result. Only the URL survives a confirmed success; this
    /// control does not need the rest of [`UploadedMedia`]'s stored-upload metadata.
    #[must_use]
    pub fn classify(result: WebResult<MutationOutcome<UploadedMedia>>) -> Self {
        match result {
            Ok(MutationOutcome::Confirmed(response)) => Self::Uploaded(response.url),
            Ok(MutationOutcome::CommitIndeterminate(_)) => Self::Indeterminate(
                "Upload status is unknown. Reload and verify whether the media was uploaded."
                    .to_owned(),
            ),
            Err(error) => Self::Failed(error.to_string()),
        }
    }
}

/// Whether a completed delete may have changed the media list and storage usage.
///
/// A confirmed refusal cannot change either resource, while a confirmed deletion or
/// an indeterminate commit must trigger revalidation.
#[must_use]
pub fn delete_invalidates_media_resources(outcome: &MutationOutcome<MediaDeletion>) -> bool {
    matches!(
        outcome,
        MutationOutcome::Confirmed(MediaDeletion::Deleted)
            | MutationOutcome::CommitIndeterminate(_)
    )
}

/// The caller notifications an upload fires, bundled so the component hands one
/// `Copy` value to [`UploadState::settle`] instead of threading optional props through
/// the spawned future.
///
/// All callbacks are optional and independent. A confirmed upload fires
/// `on_uploaded`; an indeterminate upload fires both `on_indeterminate` so callers
/// can revalidate uncertain resources and `on_error` for error reporting.
#[derive(Clone, Copy)]
pub struct UploadCallbacks {
    pub on_uploaded: Option<Callback<RootRelativeUrl>>,
    pub on_indeterminate: Option<Callback<()>>,
    pub on_error: Option<Callback<String>>,
}

impl UploadCallbacks {
    /// Fire callbacks matching `outcome`, when the caller supplied them.
    pub fn notify(&self, outcome: &UploadOutcome) {
        match outcome {
            UploadOutcome::Uploaded(url) => {
                if let Some(callback) = self.on_uploaded {
                    callback.run(url.clone());
                }
            }
            UploadOutcome::Indeterminate(message) => {
                if let Some(callback) = self.on_indeterminate {
                    callback.run(());
                }
                if let Some(callback) = self.on_error {
                    callback.run(message.clone());
                }
            }
            UploadOutcome::Failed(message) => {
                if let Some(callback) = self.on_error {
                    callback.run(message.clone());
                }
            }
        }
    }
}

/// The reactive state of one upload control: the in-flight flag the button reads and
/// the two inline-display signals.
///
/// Every field is an `RwSignal` (a `Copy` handle into the reactive runtime) plus one
/// plain `bool`, so the whole struct is `Copy` and moves into the picker closure and
/// the spawned upload future without per-signal capture.
#[derive(Clone, Copy)]
pub struct UploadState {
    /// True for the duration of the multipart POST — disables the button and
    /// switches its label.
    pub uploading: RwSignal<bool>,
    /// The last successful upload's URL, painted inline in `show_result` mode.
    pub last_media_url: RwSignal<Option<RootRelativeUrl>>,
    /// The last failure's message, painted inline in `show_result` mode.
    pub error: RwSignal<Option<String>>,
    /// Whether this control paints the outcome itself. Held here rather than
    /// consulted at each `set` site, so "record it?" is one host-tested branch
    /// instead of two branches in the component body.
    show_result: bool,
}

impl UploadState {
    /// A fresh, idle control. `show_result` is fixed at construction because it is a
    /// prop, not reactive state.
    #[must_use]
    pub fn new(show_result: bool) -> Self {
        Self {
            uploading: RwSignal::new(false),
            last_media_url: RwSignal::new(None),
            error: RwSignal::new(None),
            show_result,
        }
    }

    /// Mark an upload as started.
    pub fn begin(&self) {
        self.uploading.set(true);
    }

    /// Settle a finished upload: clear the in-flight flag, notify the caller, then —
    /// in `show_result` mode — record the outcome for the inline display.
    ///
    /// The ordering is the pre-extraction body's, verbatim: `uploading` clears
    /// first, the caller's callback runs next (so a caller that refetches sees the
    /// button already re-enabled), and the inline signals are written last.
    pub fn settle(
        &self,
        result: WebResult<MutationOutcome<UploadedMedia>>,
        callbacks: UploadCallbacks,
    ) {
        self.uploading.set(false);
        let outcome = UploadOutcome::classify(result);
        callbacks.notify(&outcome);
        self.record(&outcome);
    }

    /// Record `outcome` in the inline-display signals — a no-op unless this control
    /// was built with `show_result`. Only a confirmed upload stores a URL and clears a
    /// previous error; failures and indeterminate commits remain visibly error-like.
    fn record(&self, outcome: &UploadOutcome) {
        if !self.show_result {
            return;
        }
        match outcome {
            UploadOutcome::Uploaded(url) => {
                self.last_media_url.set(Some(url.clone()));
                self.error.set(None);
            }
            UploadOutcome::Indeterminate(message) => {
                self.last_media_url.set(None);
                self.error.set(Some(message.clone()));
            }
            UploadOutcome::Failed(message) => self.error.set(Some(message.clone())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::WebError;
    use common::test_support::{
        parse_byte_size, parse_content_hash, parse_content_type, parse_filename,
        parse_root_relative_url,
    };

    fn url() -> RootRelativeUrl {
        parse_root_relative_url("/media/upload/abc/cat.png")
    }

    fn response() -> UploadedMedia {
        UploadedMedia {
            sha256: parse_content_hash(&"a".repeat(64)),
            filename: parse_filename("cat.png"),
            content_type: parse_content_type("image/png"),
            size_bytes: parse_byte_size("1024"),
            url: url(),
        }
    }

    fn no_callbacks() -> UploadCallbacks {
        UploadCallbacks {
            on_uploaded: None,
            on_indeterminate: None,
            on_error: None,
        }
    }

    /// The sinks a caller's callbacks would write, plus the callbacks that write them.
    ///
    /// **One helper, not a fresh set of closures per test.** The no-callback case
    /// asserts these very sinks stay unwritten, which only says something because the
    /// same instrumented callbacks demonstrably write them a few lines earlier — the
    /// difference between "no callback fired" and "nothing here could ever fire". It
    /// also keeps the closure bodies covered by the positive cases rather than leaving
    /// an uncovered stub behind in the negative one.
    #[derive(Clone, Copy)]
    struct Sinks {
        uploaded: RwSignal<Option<RootRelativeUrl>>,
        indeterminate: RwSignal<u32>,
        failed: RwSignal<Option<String>>,
    }

    impl Sinks {
        fn new() -> Self {
            Self {
                uploaded: RwSignal::new(None),
                indeterminate: RwSignal::new(0),
                failed: RwSignal::new(None),
            }
        }

        /// All callbacks supplied.
        fn callbacks(self) -> UploadCallbacks {
            UploadCallbacks {
                on_uploaded: Some(Callback::new(move |url| self.uploaded.set(Some(url)))),
                on_indeterminate: Some(Callback::new(move |()| {
                    self.indeterminate.update(|count| *count += 1);
                })),
                on_error: Some(Callback::new(move |message| self.failed.set(Some(message)))),
            }
        }

        /// Only `on_error` supplied — the compose form's shape.
        fn error_only(self) -> UploadCallbacks {
            UploadCallbacks {
                on_uploaded: None,
                on_indeterminate: None,
                ..self.callbacks()
            }
        }

        fn clear(self) {
            self.uploaded.set(None);
            self.indeterminate.set(0);
            self.failed.set(None);
        }
    }

    #[test]
    fn classify_keeps_only_the_url_on_confirmed_success() {
        assert_eq!(
            UploadOutcome::classify(Ok(MutationOutcome::Confirmed(response()))),
            UploadOutcome::Uploaded(url())
        );
    }

    #[test]
    fn classify_preserves_an_indeterminate_commit() {
        assert_eq!(
            UploadOutcome::classify(Ok(MutationOutcome::CommitIndeterminate(response()))),
            UploadOutcome::Indeterminate(
                "Upload status is unknown. Reload and verify whether the media was uploaded."
                    .to_owned()
            )
        );
    }

    #[test]
    fn classify_renders_the_error_message_on_failure() {
        assert_eq!(
            UploadOutcome::classify(Err(WebError::validation("file too large"))),
            UploadOutcome::Failed("file too large".to_string())
        );
        assert_eq!(
            UploadOutcome::classify(Err(WebError::server_message("disk full"))),
            UploadOutcome::Failed("server error: disk full".to_string()),
            "the rendered message is the whole Display form, not the inner text"
        );
    }

    // `Debug` is invoked by `assert_eq!` only on FAILURE, so the assertions above
    // never reach the derive. Pin it here rather than leave an uncovered line.
    #[test]
    fn upload_outcome_is_debug_printable() {
        assert!(format!("{:?}", UploadOutcome::Uploaded(url())).contains("Uploaded"));
        assert_eq!(
            format!("{:?}", UploadOutcome::Indeterminate("unknown".to_string())),
            "Indeterminate(\"unknown\")"
        );
        assert_eq!(
            format!("{:?}", UploadOutcome::Failed("boom".to_string())),
            "Failed(\"boom\")"
        );
    }

    #[test]
    fn notify_fires_only_the_matching_callback() {
        Owner::new().with(|| {
            let sinks = Sinks::new();
            let callbacks = sinks.callbacks();

            callbacks.notify(&UploadOutcome::Uploaded(url()));
            assert_eq!(sinks.uploaded.get(), Some(url()));
            assert_eq!(sinks.indeterminate.get(), 0);
            assert_eq!(sinks.failed.get(), None, "a success must not fire on_error");

            callbacks.notify(&UploadOutcome::Indeterminate("unknown".to_string()));
            assert_eq!(sinks.indeterminate.get(), 1, "uncertain uploads revalidate");
            assert_eq!(sinks.failed.get(), Some("unknown".to_string()));

            callbacks.notify(&UploadOutcome::Failed("boom".to_string()));
            assert_eq!(sinks.failed.get(), Some("boom".to_string()));
            assert_eq!(
                sinks.uploaded.get(),
                Some(url()),
                "a failure must not disturb the last success"
            );
            assert_eq!(
                sinks.indeterminate.get(),
                1,
                "a failure is not indeterminate"
            );
        });
    }

    #[test]
    fn notify_without_callbacks_is_a_no_op_on_both_arms() {
        Owner::new().with(|| {
            let sinks = Sinks::new();
            // First prove the sinks are writable through `notify` itself, so the
            // assertions below distinguish "no callback fired" from "nothing here
            // could ever have been observed".
            sinks.callbacks().notify(&UploadOutcome::Uploaded(url()));
            sinks
                .callbacks()
                .notify(&UploadOutcome::Indeterminate("unknown".to_string()));
            sinks
                .callbacks()
                .notify(&UploadOutcome::Failed("boom".to_string()));
            assert_eq!(sinks.uploaded.get(), Some(url()));
            assert_eq!(sinks.indeterminate.get(), 1);
            assert_eq!(sinks.failed.get(), Some("boom".to_string()));

            sinks.clear();
            no_callbacks().notify(&UploadOutcome::Uploaded(url()));
            no_callbacks().notify(&UploadOutcome::Indeterminate("unknown".to_string()));
            no_callbacks().notify(&UploadOutcome::Failed("boom".to_string()));
            assert_eq!(
                sinks.uploaded.get(),
                None,
                "an absent on_uploaded writes nothing"
            );
            assert_eq!(
                sinks.indeterminate.get(),
                0,
                "an absent on_indeterminate writes nothing"
            );
            assert_eq!(
                sinks.failed.get(),
                None,
                "an absent on_error writes nothing"
            );
        });
    }

    #[test]
    fn notify_reports_an_indeterminate_upload_when_only_on_error_is_supplied() {
        Owner::new().with(|| {
            let sinks = Sinks::new();
            let callbacks = sinks.error_only();

            callbacks.notify(&UploadOutcome::Uploaded(url()));
            assert_eq!(sinks.failed.get(), None);
            assert_eq!(
                sinks.uploaded.get(),
                None,
                "the absent on_uploaded fires nothing"
            );

            callbacks.notify(&UploadOutcome::Indeterminate("unknown".to_string()));
            assert_eq!(sinks.failed.get(), Some("unknown".to_string()));
            assert_eq!(
                sinks.indeterminate.get(),
                0,
                "the absent on_indeterminate fires nothing"
            );
        });
    }

    #[test]
    fn delete_invalidation_covers_confirmed_and_indeterminate_mutations_only() {
        let deleted = MutationOutcome::Confirmed(MediaDeletion::Deleted);
        let refused =
            MutationOutcome::Confirmed(MediaDeletion::RefusedReferenced { post_ids: vec![] });
        let indeterminate = MutationOutcome::CommitIndeterminate(MediaDeletion::Deleted);

        assert!(delete_invalidates_media_resources(&deleted));
        assert!(!delete_invalidates_media_resources(&refused));
        assert!(delete_invalidates_media_resources(&indeterminate));
    }

    #[test]
    fn a_new_control_is_idle_and_empty() {
        Owner::new().with(|| {
            let state = UploadState::new(true);
            assert!(!state.uploading.get());
            assert_eq!(state.last_media_url.get(), None);
            assert_eq!(state.error.get(), None);
        });
    }

    #[test]
    fn begin_marks_the_upload_in_flight() {
        Owner::new().with(|| {
            let state = UploadState::new(false);
            state.begin();
            assert!(state.uploading.get());
        });
    }

    #[test]
    fn settle_clears_the_in_flight_flag_on_every_outcome() {
        Owner::new().with(|| {
            let state = UploadState::new(false);
            state.begin();
            state.settle(Ok(MutationOutcome::Confirmed(response())), no_callbacks());
            assert!(!state.uploading.get());

            state.begin();
            state.settle(
                Ok(MutationOutcome::CommitIndeterminate(response())),
                no_callbacks(),
            );
            assert!(!state.uploading.get());

            state.begin();
            state.settle(Err(WebError::validation("boom")), no_callbacks());
            assert!(!state.uploading.get());
        });
    }

    #[test]
    fn settle_records_the_url_and_clears_a_stale_error_when_showing_results() {
        Owner::new().with(|| {
            let state = UploadState::new(true);
            state.settle(Err(WebError::validation("boom")), no_callbacks());
            assert_eq!(state.error.get(), Some("boom".to_string()));
            assert_eq!(state.last_media_url.get(), None);

            state.settle(Ok(MutationOutcome::Confirmed(response())), no_callbacks());
            assert_eq!(state.last_media_url.get(), Some(url()));
            assert_eq!(
                state.error.get(),
                None,
                "a retry must not leave the stale banner up"
            );
        });
    }

    #[test]
    fn settle_renders_an_indeterminate_upload_as_reload_and_verify_error() {
        Owner::new().with(|| {
            let state = UploadState::new(true);
            state.settle(Ok(MutationOutcome::Confirmed(response())), no_callbacks());
            assert_eq!(state.last_media_url.get(), Some(url()));

            state.settle(
                Ok(MutationOutcome::CommitIndeterminate(response())),
                no_callbacks(),
            );
            assert_eq!(
                state.last_media_url.get(),
                None,
                "an uncertain retry must not paint a confirmed success"
            );
            assert_eq!(
                state.error.get(),
                Some(
                    "Upload status is unknown. Reload and verify whether the media was uploaded."
                        .to_owned()
                )
            );
        });
    }

    #[test]
    fn settle_records_nothing_when_not_showing_results() {
        Owner::new().with(|| {
            let state = UploadState::new(false);
            state.settle(Ok(MutationOutcome::Confirmed(response())), no_callbacks());
            assert_eq!(state.last_media_url.get(), None);

            state.settle(Err(WebError::validation("boom")), no_callbacks());
            assert_eq!(
                state.error.get(),
                None,
                "a caller that renders nothing inline keeps its signals untouched"
            );
        });
    }

    // The callback fires regardless of `show_result` — they are independent
    // outputs, and `MediaPage` relies on the callback while displaying nothing.
    #[test]
    fn settle_notifies_the_caller_even_when_not_showing_results() {
        Owner::new().with(|| {
            let sinks = Sinks::new();
            let callbacks = sinks.callbacks();
            let state = UploadState::new(false);

            state.settle(Ok(MutationOutcome::Confirmed(response())), callbacks);
            assert_eq!(sinks.uploaded.get(), Some(url()));

            state.settle(
                Ok(MutationOutcome::CommitIndeterminate(response())),
                callbacks,
            );
            assert_eq!(sinks.indeterminate.get(), 1);
            assert_eq!(
                sinks.failed.get(),
                Some(
                    "Upload status is unknown. Reload and verify whether the media was uploaded."
                        .to_owned()
                )
            );

            state.settle(Err(WebError::validation("boom")), callbacks);
            assert_eq!(sinks.failed.get(), Some("boom".to_string()));
        });
    }
}
