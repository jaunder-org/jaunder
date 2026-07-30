use leptos::prelude::*;
use leptos::server_fn::codec::Json;

// `TagLabel` is only named in the server-only `list` body (the client build
// strips it via the `#[server]` stub), so gate it to match — the wire `TagSummary`
// it builds now lives in `common::seed`.
#[cfg(feature = "server")]
use {common::tag::TagLabel, std::sync::Arc, storage::PostStorage};

use common::pagination::PageSize;
use common::seed::TagSummary;

use crate::error::WebResult;

/// Default number of suggestions returned to the autocomplete dropdown when
/// the caller doesn't specify a limit.
///
/// Expressed as a [`PageSize`] because that type already carries this bound: its
/// range is `1..=50`, exactly the clamp this endpoint used to apply by hand, and
/// `PageSize::clamped` is the coerce-rather-than-reject policy a public `limit=`
/// param wants (#696; the `AtomPub` default of 25 is recorded the same way).
pub const DEFAULT_TAG_LIMIT: u32 = 10;

/// Hard upper bound on the autocomplete result set; protects the database
/// against pathological requests.
///
/// Equal to `PageSize::MAX` — the hand-rolled `.clamp(1, MAX_TAG_LIMIT)` this
/// replaced was a re-implementation of [`PageSize::clamped`].
pub const MAX_TAG_LIMIT: u32 = PageSize::MAX;

/// Returns tag suggestions for the autocomplete dropdown.
///
/// `prefix` is a case-insensitive prefix match against the canonical slug;
/// `None` or whitespace-only returns the alphabetically-first tags. `limit`
/// defaults to [`DEFAULT_TAG_LIMIT`] and is clamped at [`MAX_TAG_LIMIT`].
///
/// `prefix` stays `String` (not `Tag`): it is a partial search fragment matched
/// with SQL `LIKE prefix%`, not a complete tag value — typing it `Tag` would
/// reject valid partials (ADR-0063 §4 boundary policy; #409 Decision 7).
#[server(endpoint = "/tags/list", input = Json)]
#[tracing::instrument(name = "web.tags.list", skip(prefix))]
pub async fn list(prefix: Option<String>, limit: Option<u32>) -> WebResult<Vec<TagSummary>> {
    boundary!("list", {
        let posts = expect_context::<Arc<dyn PostStorage>>();
        // `exact_limit`, not `fetch_limit`: the dropdown shows what it gets and has no
        // "load more", so an extra probing row would just be fetched and discarded.
        let resolved_limit = PageSize::clamped(limit.unwrap_or(DEFAULT_TAG_LIMIT)).exact_limit();
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

    /// The server-side surface. Gated as a group rather than per item because
    /// every name below reaches `storage`, a `feature = "server"` dependency —
    /// and nested here so the file keeps a single `tests` module.
    #[cfg(feature = "server")]
    mod server {
        use super::super::list;
        use leptos::prelude::provide_context;
        use leptos::reactive::owner::Owner;
        use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
        use storage::{MockPostStorage, PostStorage};
        use tracing::field::{Field, Visit};
        use tracing_subscriber::layer::{Context, Layer};
        use tracing_subscriber::prelude::*;
        use tracing_subscriber::registry::LookupSpan;

        /// One captured span: its name, and each recorded field as `name=value`.
        ///
        /// Values are kept, not just names, so the test can assert a skipped
        /// argument's *value* is absent however it might have been recorded — a
        /// `fields(x = %prefix)` leak would name the field `x`, not `prefix`.
        struct CapturedSpan {
            name: String,
            fields: Vec<(String, String)>,
        }

        #[derive(Default)]
        struct Captured {
            spans: Vec<CapturedSpan>,
        }

        struct CaptureLayer(Arc<Mutex<Captured>>);

        /// `#[instrument]` records each non-skipped argument through its `Debug` impl,
        /// so a skipped argument never reaches a visitor at all.
        struct FieldPairs(Vec<(String, String)>);

        impl Visit for FieldPairs {
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                self.0
                    .push((field.name().to_string(), format!("{value:?}")));
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
                let mut fields = FieldPairs(Vec::new());
                attrs.record(&mut fields);
                lock(&self.0).spans.push(CapturedSpan {
                    name: attrs.metadata().name().to_string(),
                    fields: fields.0,
                });
            }
        }

        /// A poisoned lock still holds the captures we want to assert on, and this
        /// mutex guards a plain `Vec` with no invariant a panic could break.
        fn lock(captured: &Mutex<Captured>) -> MutexGuard<'_, Captured> {
            captured.lock().unwrap_or_else(PoisonError::into_inner)
        }

        /// The value the test feeds as the skipped `prefix`, distinctive enough that
        /// finding it anywhere in the span's fields proves a leak.
        const SECRET_PREFIX: &str = "secret-fragment";

        /// #511: the source-static gate cannot tell whether `#[server]`'s expansion
        /// actually wraps the server-side body, so one site is pinned at runtime —
        /// the span exists, carries its derived name, records the recordable
        /// argument, and never carries the skipped one's value.
        // guard:no-backend — mock store
        #[tokio::test]
        async fn list_emits_its_derived_span_recording_limit_but_not_prefix() {
            let captured = Arc::new(Mutex::new(Captured::default()));
            let subscriber =
                tracing_subscriber::registry().with(CaptureLayer(Arc::clone(&captured)));
            let _guard = tracing::subscriber::set_default(subscriber);

            let owner = Owner::new();
            owner.set();
            let mut posts = MockPostStorage::new();
            posts
                .expect_list_tags()
                .returning(|_prefix, _limit| Ok(Vec::new()));
            provide_context(Arc::new(posts) as Arc<dyn PostStorage>);

            let result = list(Some(SECRET_PREFIX.to_string()), Some(5)).await;
            drop(owner);
            assert!(result.is_ok(), "list failed: {result:?}");

            // Asserted over collected values rather than an `expect`/`else panic!`, so
            // every line here executes on the passing path — a diagnostic-only branch
            // would read as uncovered to the coverage gate.
            let captured = lock(&captured);
            let names: Vec<&str> = captured.spans.iter().map(|s| s.name.as_str()).collect();
            assert!(
                names.contains(&"web.tags.list"),
                "list must emit a span named web.tags.list; saw {names:?}"
            );
            let fields: Vec<&(String, String)> = captured
                .spans
                .iter()
                .filter(|s| s.name == "web.tags.list")
                .flat_map(|s| s.fields.iter())
                .collect();
            assert!(
                fields.iter().any(|(name, _)| name == "limit"),
                "limit is recordable and must be recorded; got {fields:?}"
            );
            assert!(
                !fields
                    .iter()
                    .any(|(name, value)| name == "prefix" || value.contains(SECRET_PREFIX)),
                "prefix is an unbounded String and must never reach the span; got {fields:?}"
            );
        }
    }
}
