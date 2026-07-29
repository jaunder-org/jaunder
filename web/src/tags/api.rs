use leptos::prelude::*;
use leptos::server_fn::codec::Json;

// `TagLabel` is only named in the server-only `list_tags` body (the client build
// strips it via the `#[server]` stub), so gate it to match — the wire `TagSummary`
// it builds now lives in `common::seed`.
#[cfg(feature = "server")]
use {common::tag::TagLabel, std::sync::Arc, storage::PostStorage};

use common::seed::TagSummary;

use crate::error::WebResult;

/// Default number of suggestions returned to the autocomplete dropdown when
/// the caller doesn't specify a limit.
pub const DEFAULT_TAG_LIMIT: u32 = 10;

/// Hard upper bound on the autocomplete result set; protects the database
/// against pathological requests.
pub const MAX_TAG_LIMIT: u32 = 50;

/// Returns tag suggestions for the autocomplete dropdown.
///
/// `prefix` is a case-insensitive prefix match against the canonical slug;
/// `None` or whitespace-only returns the alphabetically-first tags. `limit`
/// defaults to [`DEFAULT_TAG_LIMIT`] and is clamped at [`MAX_TAG_LIMIT`].
///
/// `prefix` stays `String` (not `Tag`): it is a partial search fragment matched
/// with SQL `LIKE prefix%`, not a complete tag value — typing it `Tag` would
/// reject valid partials (ADR-0063 §4 boundary policy; #409 Decision 7).
#[server(endpoint = "/list_tags", input = Json)]
#[tracing::instrument(name = "web.tags.list_tags", skip(prefix))]
pub async fn list_tags(prefix: Option<String>, limit: Option<u32>) -> WebResult<Vec<TagSummary>> {
    boundary!("list_tags", {
        let posts = expect_context::<Arc<dyn PostStorage>>();
        let resolved_limit = limit.unwrap_or(DEFAULT_TAG_LIMIT).clamp(1, MAX_TAG_LIMIT);
        let records = posts.list_tags(prefix.as_deref(), resolved_limit).await?;
        Ok(records
            .into_iter()
            .map(|rec| TagSummary {
                slug: rec.tag_slug.clone(),
                display: TagLabel::from(rec.tag_slug),
            })
            .collect())
    })
}

#[cfg(test)]
mod tests {
    use common::seed::TagSummary;
    use common::tag::TagLabel;

    /// #416 agreement: the `TagInput` commit path validates the raw token with
    /// `TagLabel::from_str` (the same rule the server applies at arg-decode), so
    /// client and server accept/reject identically — no re-implemented validator
    /// can drift. A trimmed, mixed-case token is accepted with its casing kept.
    #[test]
    fn tag_label_validation_agrees_client_and_server() {
        assert!("Rust".parse::<TagLabel>().is_ok());
        assert_eq!(" ab ".parse::<TagLabel>().unwrap().as_ref(), "ab");
        assert!("bad tag".parse::<TagLabel>().is_err());
    }

    /// Committing "Rust" yields a `TagSummary` whose `slug` is the canonical
    /// lowercase and whose `display` preserves the author's casing (Decision 4).
    #[test]
    fn tag_summary_preserves_casing_with_canonical_slug() {
        let label: TagLabel = "Rust".parse().unwrap();
        let summary = TagSummary {
            slug: label.slug(),
            display: label,
        };
        assert_eq!(summary.slug, "rust");
        assert_eq!(summary.display, "Rust");
    }
}

#[cfg(all(test, feature = "server"))]
mod server_tests {
    // Helper fns in this feature-gated test module aren't covered by clippy's
    // allow-{unwrap,expect}-in-tests, so allow the test-scaffolding panics.
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::list_tags;
    use leptos::prelude::provide_context;
    use leptos::reactive::owner::Owner;
    use std::sync::{Arc, Mutex};
    use storage::{MockPostStorage, PostStorage};
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::registry::LookupSpan;

    /// Every span created while the layer is installed: its name plus the names
    /// of the fields actually recorded on it at creation.
    #[derive(Default)]
    struct Captured {
        spans: Vec<(String, Vec<String>)>,
    }

    struct CaptureLayer(Arc<Mutex<Captured>>);

    /// Collects the *names* of recorded fields. `#[instrument]` records each
    /// non-skipped argument through its `Debug` impl, so a skipped argument
    /// never reaches a visitor at all — absence is the assertion.
    struct FieldNames(Vec<String>);

    impl Visit for FieldNames {
        fn record_debug(&mut self, field: &Field, _value: &dyn std::fmt::Debug) {
            self.0.push(field.name().to_string());
        }
    }

    impl<S> Layer<S> for CaptureLayer
    where
        S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    {
        fn on_new_span(
            &self,
            attrs: &tracing::span::Attributes<'_>,
            _id: &tracing::span::Id,
            _ctx: Context<'_, S>,
        ) {
            let mut names = FieldNames(Vec::new());
            attrs.record(&mut names);
            self.0
                .lock()
                .unwrap()
                .spans
                .push((attrs.metadata().name().to_string(), names.0));
        }
    }

    /// #511: the source-static gate cannot tell whether `#[server]`'s expansion
    /// actually wraps the server-side body, so one site is pinned at runtime —
    /// the span exists, carries its derived name, and records the recordable
    /// argument while the skipped one never reaches the subscriber.
    // guard:no-backend — mock store
    #[tokio::test]
    async fn list_tags_emits_its_derived_span_recording_limit_but_not_prefix() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let subscriber = tracing_subscriber::registry().with(CaptureLayer(Arc::clone(&captured)));
        let _guard = tracing::subscriber::set_default(subscriber);

        let owner = Owner::new();
        owner.set();
        let mut posts = MockPostStorage::new();
        posts
            .expect_list_tags()
            .returning(|_prefix, _limit| Ok(Vec::new()));
        provide_context(Arc::new(posts) as Arc<dyn PostStorage>);

        let result = list_tags(Some("secret-fragment".to_string()), Some(5)).await;
        drop(owner);
        assert!(result.is_ok(), "list_tags failed: {result:?}");

        let captured = captured.lock().unwrap();
        let (_, fields) = captured
            .spans
            .iter()
            .find(|(name, _)| name == "web.tags.list_tags")
            .expect("list_tags must emit a span named web.tags.list_tags");
        assert!(
            fields.iter().any(|f| f == "limit"),
            "limit is recordable and must be recorded; got {fields:?}"
        );
        assert!(
            !fields.iter().any(|f| f == "prefix"),
            "prefix is an unbounded String and must be skipped; got {fields:?}"
        );
    }
}
