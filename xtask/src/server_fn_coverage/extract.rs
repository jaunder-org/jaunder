//! Derive server-fn flow coverage from a run's OTel spans (#681).
//!
//! Pure given its inputs — no I/O — so it is unit-testable from a fixture and
//! reusable by both the `regenerate` and `verify` paths.
//!
//! **Two signals, unioned.** A span identifies a `#[server]` fn when either:
//!
//! 1. its **name** is `__server_<ident>` for an inventory fn ident *and* its
//!    `code.namespace` is that fn's module (the primary signal — a derived span
//!    name needs no URL parsing and survives any endpoint rename); or
//! 2. its **`uri`** path resolves to an inventory fn's *declared endpoint* (the
//!    complement, covering the fns #511 has not instrumented yet).
//!
//! The `__server_` prefix is not decoration: `#[server]` relocates the annotated
//! body — carrying its `#[tracing::instrument]` — into a generated
//! `__server_<ident>` fn (`server_fn_macro`'s `to_dummy_ident`), and the attribute
//! derives its span name from the fn it lands on. Matching the bare ident finds
//! nothing in a real capture, and finds it *silently*: `uri` goes on carrying
//! every hit, so the result still looks plausible. That is exactly what happened
//! here until the union was measured signal-by-signal against a real capture.
//!
//! The module check is what stops a same-named non-`#[server]` fn elsewhere in
//! `web` from counting. It reads **`code.namespace`**, which is where
//! `tracing-opentelemetry` records a span's module; `target` exists only on
//! *events*, so matching it would find nothing on any span — and would fail
//! silently, leaving `uri` to carry everything while the result still looked
//! plausible.
//!
//! **Attribution is an ancestor walk, not a parent check.** Only the *request*
//! span carries the test's span id as its parent; an instrument span's parent is
//! that request span. So a hit is walked upward through `parent_span_id` until it
//! reaches a known `e2e.test` span. `uri` hits resolve in one hop, span-name hits
//! in two. A walk that terminates without reaching a test is an **orphan** — kept
//! and reported rather than dropped, because a silent drop would understate
//! coverage and hide a broken harness.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::server_fns::ServerFn;
use crate::traces::parse::Span;

/// The `#[server]` route prefix every server-fn request lands under.
const API_PREFIX: &str = "/api/";
/// The span attribute `tracing-opentelemetry` records a span's module in.
const MODULE_ATTR: &str = "code.namespace";
/// The crate prefix `code.namespace` carries but [`ServerFn::module`] does not.
const CRATE_PREFIX: &str = "web::";
/// The prefix `#[server]` gives the fn it relocates the annotated body into, and
/// therefore the prefix on every span that body's `#[tracing::instrument]` derives.
/// Required, not merely tolerated: the bare-ident form does not occur, and
/// insisting on the prefix means a match can only have come from a server fn's
/// generated body.
const DERIVED_SPAN_PREFIX: &str = "__server_";
/// The span name the e2e harness gives its per-test span.
const TEST_SPAN_NAME: &str = "e2e.test";
/// The attribute on an `e2e.test` span holding the test's title.
const TEST_TITLE_ATTR: &str = "e2e.test";

/// Which tests exercised each server fn, plus hits attributable to no test.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    /// fn ident → the sorted, de-duplicated titles of the tests that drove it.
    pub covered: BTreeMap<String, BTreeSet<String>>,
    /// fn ident → [`OrphanReason`] (rendered) → how many hits ended that way. Keyed
    /// by reason rather than a bare total so the artifact says *why* attribution
    /// stopped — see [`OrphanReason`].
    pub orphans: BTreeMap<String, BTreeMap<String, usize>>,
}

/// The fn a span identifies, by either signal, or `None` if it identifies none.
fn identify<'a>(span: &Span, inventory: &'a [ServerFn]) -> Option<&'a ServerFn> {
    // Primary: derived span name + module. `code.namespace` on these spans holds
    // the plain module the fn was declared in (`web::auth::api` for `session`) —
    // verified against a real capture, not assumed — so the existing comparison
    // against `ServerFn::module` is the right disambiguator and needs no widening.
    if let Some(ident) = span.name.strip_prefix(DERIVED_SPAN_PREFIX) {
        if let Some(f) = inventory.iter().find(|f| f.ident == ident) {
            let namespace = crate::traces::parse::get_attr(&span.raw, MODULE_ATTR);
            let relative = namespace.strip_prefix(CRATE_PREFIX).unwrap_or(&namespace);
            // An empty namespace cannot be confirmed to be the right module, so it
            // does not count — better to fall through to `uri` than to guess.
            if !namespace.is_empty() && relative == f.module {
                return Some(f);
            }
        }
    }

    // Complement: the request URI's path, matched against the DECLARED endpoint.
    // Never `"/api/" + ident` — the coincidence that they agree today is not
    // load-bearing (#698 may drop the explicit attributes entirely).
    let endpoint = api_endpoint_of(&span.uri)?;
    inventory
        .iter()
        .find(|f| f.endpoint.as_deref() == Some(endpoint))
}

/// The server-fn endpoint named by a request `uri`, or `None` when the URI is not
/// a server-fn call. Handles both the origin form the server actually records
/// (`/api/get_post?id=7`) and an absolute form (`https://host/api/list_tags`),
/// and strips the query string — Leptos GET server fns encode their args there.
fn api_endpoint_of(uri: &str) -> Option<&str> {
    let path = uri.split(['?', '#']).next().unwrap_or(uri);
    // Absolute form: drop scheme://host by finding the prefix anywhere; origin
    // form: it is at the start. Requiring the prefix (rather than taking the last
    // segment) is what keeps static assets and feeds out.
    let rest = path.split_once(API_PREFIX)?.1;
    if rest.is_empty() {
        return None;
    }
    Some(rest)
}

/// Why a hit reached no test. Recorded rather than collapsed into a bare count:
/// "outside any test" and "attribution is broken" are the same *shape* of result
/// but opposite in meaning, and a gate that cannot tell them apart hides the
/// failure it exists to catch (spec AC5).
///
/// The expected, benign value is [`Self::UnknownParent`] naming the run-wide
/// traceparent's span id — the `_autoPerfSpan` warmup load, which runs before the
/// per-test traceparent is applied and so is deliberately unattributed. Any
/// *other* reason, or an unfamiliar parent id, means a context lost its
/// traceparent or the capture is truncated.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum OrphanReason {
    /// The chain ran off the end of the capture: this parent id has no span. The
    /// id is kept — it is the one piece of evidence that identifies the source.
    UnknownParent(String),
    /// The span declared no parent at all, so it was never in a trace context.
    NoParent,
    /// `MAX_DEPTH` hops without reaching a test — a cycle or a pathological chain.
    DepthExceeded,
}

impl std::fmt::Display for OrphanReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownParent(id) => write!(f, "unknown-parent:{id}"),
            Self::NoParent => write!(f, "no-parent"),
            Self::DepthExceeded => write!(f, "depth-exceeded"),
        }
    }
}

/// Walk `parent_span_id` upward from `start` until a known `e2e.test` span id is
/// reached, returning that test's title — or the [`OrphanReason`] the walk ended
/// without one. `MAX_DEPTH` is a cycle guard: a malformed capture must not hang a
/// gate.
fn attribute(
    start: &Span,
    by_id: &HashMap<&str, &Span>,
    tests: &HashMap<&str, String>,
) -> Result<String, OrphanReason> {
    const MAX_DEPTH: usize = 32;
    let mut current = start;
    for _ in 0..MAX_DEPTH {
        if current.parent_span_id.is_empty() {
            return Err(OrphanReason::NoParent);
        }
        if let Some(title) = tests.get(current.parent_span_id.as_str()) {
            return Ok(title.clone());
        }
        match by_id.get(current.parent_span_id.as_str()) {
            Some(parent) => current = parent,
            None => return Err(OrphanReason::UnknownParent(current.parent_span_id.clone())),
        }
    }
    Err(OrphanReason::DepthExceeded)
}

/// Derive [`Coverage`] from a run's spans and the syn-derived inventory.
pub fn extract(spans: &[Span], inventory: &[ServerFn]) -> Coverage {
    let by_id: HashMap<&str, &Span> = spans
        .iter()
        .filter(|s| !s.span_id.is_empty())
        .map(|s| (s.span_id.as_str(), s))
        .collect();

    let tests: HashMap<&str, String> = spans
        .iter()
        .filter(|s| s.name == TEST_SPAN_NAME && !s.span_id.is_empty())
        .map(|s| {
            (
                s.span_id.as_str(),
                crate::traces::parse::get_attr(&s.raw, TEST_TITLE_ATTR),
            )
        })
        .collect();

    let mut coverage = Coverage::default();
    for span in spans {
        let Some(f) = identify(span, inventory) else {
            continue;
        };
        match attribute(span, &by_id, &tests) {
            Ok(title) => {
                coverage
                    .covered
                    .entry(f.ident.clone())
                    .or_default()
                    .insert(title);
            }
            Err(reason) => {
                *coverage
                    .orphans
                    .entry(f.ident.clone())
                    .or_default()
                    .entry(reason.to_string())
                    .or_default() += 1;
            }
        }
    }
    coverage
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traces::parse::{parse_spans, Filters};

    // The hand-authored fixture mirrors the span shapes a real capture contains —
    // notably `__server_<ident>` instrument spans, not bare idents. It once carried
    // the bare form, which occurs nowhere, so every span-name assertion below was
    // pinning a fiction while the signal was dead on real data. The reduced real
    // capture (`otel-traces-seed.jsonl`, exercised from
    // `steps::server_fn_coverage_check`) is what keeps these shapes honest.
    const SAMPLE: &str = include_str!("testdata/coverage-sample.jsonl");

    fn sample_spans() -> Vec<Span> {
        parse_spans(SAMPLE, &Filters::default(), "sample").expect("fixture parses")
    }

    fn fnf(ident: &str, module: &str) -> ServerFn {
        ServerFn {
            name: crate::server_fns::pascal_case(ident),
            ident: ident.to_string(),
            endpoint: Some(ident.to_string()),
            module: module.to_string(),
            line: 1,
        }
    }

    fn sample_inventory() -> Vec<ServerFn> {
        vec![
            fnf("create_post", "posts::api"),
            fnf("update_post", "posts::api"),
            fnf("get_post", "posts::api"),
            fnf("list_tags", "tags::api"),
            fnf("register", "registration::api"),
        ]
    }

    fn titles(c: &Coverage, ident: &str) -> Vec<String> {
        c.covered
            .get(ident)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    #[test]
    fn uri_hit_attributes_to_its_test() {
        let c = extract(&sample_spans(), &sample_inventory());
        assert_eq!(titles(&c, "create_post"), vec!["creates a post"]);
    }

    #[test]
    fn instrument_span_attributes_through_its_request_parent() {
        // Two hops: instrument span -> request span -> test span. The signal-1 case
        // in isolation: `__server_update_post` carries no `uri`, and the request it
        // hangs under is a `create_post` call, so only the span name can find it.
        let c = extract(&sample_spans(), &sample_inventory());
        assert_eq!(titles(&c, "update_post"), vec!["creates a post"]);
    }

    #[test]
    fn query_string_is_stripped() {
        let c = extract(&sample_spans(), &sample_inventory());
        assert!(c.covered.contains_key("get_post"));
    }

    #[test]
    fn both_origin_form_and_absolute_uris_resolve() {
        // Real captures carry origin-form (`/api/…`); other fixtures carry
        // absolute URLs. Both must work or a real run finds zero hits.
        let c = extract(&sample_spans(), &sample_inventory());
        assert!(c.covered.contains_key("create_post"), "origin form");
        assert!(c.covered.contains_key("list_tags"), "absolute form");
    }

    #[test]
    fn unattributable_hit_lands_in_orphans_not_covered() {
        let c = extract(&sample_spans(), &sample_inventory());
        assert!(!c.covered.contains_key("register"));
        // Bucketed by REASON, not a bare count — the whole point of AC5 is that a
        // deliberate non-attribution and a broken chain must not look alike.
        let reasons = c.orphans.get("register").expect("register is orphaned");
        assert_eq!(reasons.values().sum::<usize>(), 1);
        let reason = reasons.keys().next().expect("one reason");
        assert!(
            reason.starts_with("unknown-parent:"),
            "the fixture's orphan has a parent absent from the capture: {reason}"
        );
    }

    #[test]
    fn orphan_reasons_are_distinguishable() {
        use super::OrphanReason;
        // Each variant renders to its own key, so the snapshot can be read for
        // cause. `unknown-parent` carries the id because that id is the evidence
        // (the run-wide traceparent's id means "warmup"; anything else does not).
        assert_eq!(
            OrphanReason::UnknownParent("1111111111111111".into()).to_string(),
            "unknown-parent:1111111111111111"
        );
        assert_eq!(OrphanReason::NoParent.to_string(), "no-parent");
        assert_eq!(OrphanReason::DepthExceeded.to_string(), "depth-exceeded");
    }

    #[test]
    fn non_api_traffic_is_ignored() {
        let c = extract(&sample_spans(), &sample_inventory());
        // The static asset resolves to no fn in either map.
        assert!(!c.covered.keys().any(|k| k.contains("wasm")));
        assert!(!c.orphans.keys().any(|k| k.contains("wasm")));
    }

    #[test]
    fn span_name_in_the_wrong_module_is_not_counted() {
        // A span named `__server_update_post` whose code.namespace is a different
        // module is a different fn; and it carries no `uri`, so nothing else
        // matches it.
        let c = extract(&sample_spans(), &[fnf("update_post", "storage::posts")]);
        assert!(!c.covered.contains_key("update_post"));
        assert!(!c.orphans.contains_key("update_post"));
    }

    #[test]
    fn code_namespace_crate_prefix_is_stripped_before_comparing() {
        // code.namespace is `web::posts::api`; ServerFn.module is `posts::api`.
        // Comparing them raw would reject every span-name hit — silently, since
        // `uri` would still carry the fn.
        let c = extract(&sample_spans(), &[fnf("update_post", "posts::api")]);
        assert_eq!(titles(&c, "update_post"), vec!["creates a post"]);
    }

    #[test]
    fn a_span_name_hit_with_no_code_namespace_is_not_counted() {
        // One JSON object per line — this is JSONL, so the literal must not wrap.
        let line = concat!(
            r#"{"resourceSpans":[{"scopeSpans":[{"spans":["#,
            r#"{"traceId":"aa","spanId":"t1","name":"e2e.test","#,
            r#""attributes":[{"key":"e2e.test","value":{"stringValue":"t"}}]},"#,
            r#"{"traceId":"aa","spanId":"i1","parentSpanId":"t1","#,
            r#""name":"__server_update_post","attributes":[]}"#,
            r#"]}]}]}"#,
        );
        let spans = parse_spans(line, &Filters::default(), "t").expect("parses");
        let c = extract(&spans, &[fnf("update_post", "posts::api")]);
        assert!(
            c.covered.is_empty(),
            "an unconfirmable module must not count"
        );
    }

    #[test]
    fn a_fn_hit_by_both_signals_is_counted_once_per_test() {
        // `create_post` is reached twice over: the `/api/create_post` request span
        // and the `__server_create_post` instrument span beneath it. The union is a
        // set of test titles, so the same test must not be listed twice.
        let c = extract(&sample_spans(), &sample_inventory());
        assert_eq!(titles(&c, "create_post").len(), 1);
    }

    #[test]
    fn a_bare_ident_span_name_is_not_a_hit() {
        // The shape `#[server]` never produces. Pinned deliberately: this fixture
        // once used it, which made the span-name signal *look* exercised while it
        // matched nothing a real capture contains.
        let line = concat!(
            r#"{"resourceSpans":[{"scopeSpans":[{"spans":["#,
            r#"{"traceId":"aa","spanId":"t1","name":"e2e.test","#,
            r#""attributes":[{"key":"e2e.test","value":{"stringValue":"t"}}]},"#,
            r#"{"traceId":"aa","spanId":"i1","parentSpanId":"t1","name":"update_post","#,
            r#""attributes":[{"key":"code.namespace","#,
            r#""value":{"stringValue":"web::posts::api"}}]}"#,
            r#"]}]}]}"#,
        );
        let spans = parse_spans(line, &Filters::default(), "t").expect("parses");
        let c = extract(&spans, &[fnf("update_post", "posts::api")]);
        assert!(
            c.covered.is_empty(),
            "the derived span name is `__server_update_post`, not `update_post`"
        );
    }

    #[test]
    fn endpoint_is_matched_by_declared_value_not_by_fn_name() {
        // The fn is `fetch_post` but its declared endpoint is `get_post`, which is
        // what the URI carries. Assuming "/api/" + ident would miss it.
        let renamed = ServerFn {
            name: "FetchPost".into(),
            ident: "fetch_post".into(),
            endpoint: Some("get_post".into()),
            module: "posts::api".into(),
            line: 1,
        };
        let c = extract(&sample_spans(), &[renamed]);
        assert_eq!(titles(&c, "fetch_post"), vec!["creates a post"]);
    }

    #[test]
    fn a_bare_server_fn_with_no_endpoint_matches_no_uri() {
        // `module` is deliberately NOT the fixture's `web::posts::api`: that keeps
        // the `__server_create_post` span from matching by name, so what is left to
        // observe is purely whether a `None` endpoint can match a URI.
        let bare = ServerFn {
            name: "CreatePost".into(),
            ident: "create_post".into(),
            endpoint: None,
            module: "elsewhere::api".into(),
            line: 1,
        };
        let c = extract(&sample_spans(), &[bare]);
        assert!(c.covered.is_empty(), "None endpoint must not match /api/…");
    }
}
