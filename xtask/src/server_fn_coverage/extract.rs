//! Derive server-fn flow coverage from a run's OTel spans (#681).
//!
//! Pure given its inputs — no I/O — so it is unit-testable from a fixture and
//! reusable by both the `regenerate` and `verify` paths.
//!
//! **Two signals, unioned.** A span identifies a `#[server]` fn when either:
//!
//! 1. its **name** is one of that fn's [`candidate_span_names`] *and* its
//!    `code.namespace` is that fn's module (the primary signal — needs no URL
//!    parsing and survives any endpoint rename); or
//! 2. its **`uri`** path resolves to an inventory fn's *derived endpoint* (the
//!    complement).
//!
//! **Signal 2 is now masked in practice, and that is worth being explicit about.**
//! `identify` returns as soon as signal 1 hits, falling through to `uri` only on a
//! miss — and since #714 *every* server fn carries a span, because the attribute
//! that declares it also emits the `#[tracing::instrument]`. Signal 2 was once the
//! only signal for a fn with no instrument attribute; that case can no longer
//! exist. So a wrong computed endpoint would **not** show up as lost coverage
//! here: signal 1 would have already claimed the span.
//!
//! It is kept anyway, for the reason ADR-0081 records — a single silently
//! unmatched signal is exactly how this module failed before, and a union of two
//! is what made that recoverable. What verifies the endpoint instead is
//! `server_fn_coverage_check`'s seed cross-check, which locates a fn by signal 1
//! and *then* compares the computed endpoint to the URI a real run requested.
//!
//! **The name is matched forward, never inverted**, because this repo has already
//! had two naming regimes and could have a third. `server-fn-tracing` writes
//! `web.<vertical>.<ident>` today (#511); omitting the explicit `name` derives
//! `__server_<ident>`, since `#[server]` relocates the annotated body into a
//! generated fn of that name. An earlier version of this module matched only the
//! bare ident, so signal 1 matched **nothing** — and did so *silently*, because
//! `uri` kept carrying every hit and the totals still looked plausible. Computing
//! the candidates from the inventory is what makes a regime change a code update
//! rather than a silent outage; the per-signal tests are what make it visible.
//!
//! **`code.namespace` is the disambiguator, not the name.**
//! `web.<vertical>.<ident>` takes the module's *first* segment, so `posts::api` and
//! `posts::api::listing` both render `web.posts.…` — the name alone cannot separate
//! a same-named fn in each, while `(module, ident)` cannot collide at all, since
//! Rust forbids two items of one name in one module. `code.namespace` is where
//! `tracing-opentelemetry` records a span's module; `target` exists only on
//! *events*, so matching that would find nothing on any span — and would fail
//! silently in the same way.
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
// `pub(crate)` so the seed cross-check in `server_fn_coverage_check` can locate a
// fn by the same signal this module uses, rather than restating the attribute name
// — a second copy is precisely the drift that check exists to catch (#714).
pub(crate) const MODULE_ATTR: &str = "code.namespace";
/// The crate prefix `code.namespace` carries but [`ServerFn::module`] does not.
const CRATE_PREFIX: &str = "web::";
/// The prefix `#[server]` gives the fn it relocates the annotated body into, so a
/// span whose name `#[tracing::instrument]` *derived* carries it.
const DERIVED_SPAN_PREFIX: &str = "__server_";
/// The prefix on the span names `server-fn-tracing` writes (#511, ADR-0011):
/// `web.<vertical>.<ident>`, where the vertical is the module's first segment.
const EXPLICIT_SPAN_PREFIX: &str = "web.";
/// The span name the e2e harness gives its per-test span.
const TEST_SPAN_NAME: &str = "e2e.test";
/// The attribute on an `e2e.test` span holding the test's title.
const TEST_TITLE_ATTR: &str = "e2e.test";

/// Which tests exercised each server fn, plus hits attributable to no test.
///
/// Both maps are keyed by [`ServerFn::qualified`] — `<vertical>::<ident>`, never
/// the bare ident, which fifteen fns share across six verticals since #684.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    /// `<vertical>::<ident>` → the sorted, de-duplicated titles of the tests that
    /// drove it.
    pub covered: BTreeMap<String, BTreeSet<String>>,
    /// `<vertical>::<ident>` → the distinct [`OrphanReason`]s (rendered) its
    /// unattributed hits ended with. Reasons, not counts, and for two reasons.
    ///
    /// *Diagnosability:* "outside any test" and "attribution is broken" are the same
    /// shape of result but opposite in meaning, so the artifact has to say which
    /// (spec AC5).
    ///
    /// *Stability:* a **count** is proportional to how many tests ran — warmup
    /// orphans twice per test — so it changes whenever anyone adds or removes an
    /// e2e test anywhere in the suite. Since the snapshot is compared byte-for-byte,
    /// counts would make this artifact churn on unrelated PRs and go spuriously red.
    /// A reason set is a function of the code, which is what the gate is about.
    pub orphans: BTreeMap<String, BTreeSet<String>>,
}

/// Every span name that could denote `f`, under any naming regime this repo has
/// used or could revert to. Deliberately a *set of candidates computed from the
/// inventory* rather than a pattern inverted out of the span name: inversion has to
/// know which regime produced the name, and getting that wrong fails silently
/// (`uri` keeps carrying every hit, so the totals still look right).
///
/// - `web.<vertical>.<ident>` — what `server-fn-tracing` writes today (#511,
///   ADR-0011). The vertical is the module's first segment, so `posts::api::listing`
///   and `posts::api` both yield `web.posts.…`; that collapse is why the module
///   check below, not the name, is what actually disambiguates.
/// - `__server_<ident>` — the name `#[tracing::instrument]` *derives* when no
///   explicit `name` is given, because `#[server]` relocates the annotated body
///   into a generated `__server_<ident>` fn (`server_fn_macro`'s `to_dummy_ident`).
/// - `<ident>` — what derivation would yield if `#[server]` stopped relocating.
// `pub(crate)` for the same reason as [`MODULE_ATTR`]: the seed cross-check must
// locate a fn by *this* rule, not a paraphrase of it.
pub(crate) fn candidate_span_names(f: &ServerFn) -> [String; 3] {
    let vertical = f.vertical();
    [
        format!("{EXPLICIT_SPAN_PREFIX}{vertical}.{}", f.ident),
        format!("{DERIVED_SPAN_PREFIX}{}", f.ident),
        f.ident.clone(),
    ]
}

/// The fn a span identifies, by either signal, or `None` if it identifies none.
fn identify<'a>(span: &Span, inventory: &'a [ServerFn]) -> Option<&'a ServerFn> {
    // Primary: span name + module. `code.namespace` holds the plain module the fn
    // was declared in (`web::auth::api` for `session`) — verified against a real
    // capture, not assumed — and it is the disambiguator, since the name alone can
    // collapse two modules of one vertical.
    let namespace = crate::traces::parse::get_attr(&span.raw, MODULE_ATTR);
    // An empty namespace cannot be confirmed to be the right module, so it does not
    // count — better to fall through to `uri` than to guess.
    if !namespace.is_empty() {
        let relative = namespace.strip_prefix(CRATE_PREFIX).unwrap_or(&namespace);
        if let Some(f) = inventory
            .iter()
            .find(|f| relative == f.module && candidate_span_names(f).contains(&span.name))
        {
            return Some(f);
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
                    .entry(f.qualified())
                    .or_default()
                    .insert(title);
            }
            Err(reason) => {
                coverage
                    .orphans
                    .entry(f.qualified())
                    .or_default()
                    .insert(reason.to_string());
            }
        }
    }
    coverage
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traces::parse::{Filters, parse_spans};

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

    /// The tests recorded against a qualified key (`<vertical>::<ident>`).
    fn titles(c: &Coverage, qualified: &str) -> Vec<String> {
        c.covered
            .get(qualified)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    #[test]
    fn uri_hit_attributes_to_its_test() {
        let c = extract(&sample_spans(), &sample_inventory());
        assert_eq!(titles(&c, "posts::create_post"), vec!["creates a post"]);
    }

    #[test]
    fn instrument_span_attributes_through_its_request_parent() {
        // Two hops: instrument span -> request span -> test span. The signal-1 case
        // in isolation: `__server_update_post` carries no `uri`, and the request it
        // hangs under is a `create_post` call, so only the span name can find it.
        let c = extract(&sample_spans(), &sample_inventory());
        assert_eq!(titles(&c, "posts::update_post"), vec!["creates a post"]);
    }

    #[test]
    fn query_string_is_stripped() {
        let c = extract(&sample_spans(), &sample_inventory());
        assert!(c.covered.contains_key("posts::get_post"));
    }

    #[test]
    fn both_origin_form_and_absolute_uris_resolve() {
        // Real captures carry origin-form (`/api/…`); other fixtures carry
        // absolute URLs. Both must work or a real run finds zero hits.
        let c = extract(&sample_spans(), &sample_inventory());
        assert!(c.covered.contains_key("posts::create_post"), "origin form");
        assert!(c.covered.contains_key("tags::list_tags"), "absolute form");
    }

    #[test]
    fn unattributable_hit_lands_in_orphans_not_covered() {
        let c = extract(&sample_spans(), &sample_inventory());
        assert!(!c.covered.contains_key("registration::register"));
        // Recorded by REASON, not as a bare count: AC5's point is that a deliberate
        // non-attribution and a broken chain must not look alike. A count would also
        // track how many tests ran, which would churn this byte-compared artifact.
        let reasons = c
            .orphans
            .get("registration::register")
            .expect("register is orphaned");
        let reason = reasons.iter().next().expect("one reason");
        assert_eq!(reasons.len(), 1, "one distinct reason: {reasons:?}");
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
        assert!(!c.covered.contains_key("storage::update_post"));
        assert!(!c.orphans.contains_key("storage::update_post"));
    }

    #[test]
    fn code_namespace_crate_prefix_is_stripped_before_comparing() {
        // code.namespace is `web::posts::api`; ServerFn.module is `posts::api`.
        // Comparing them raw would reject every span-name hit — silently, since
        // `uri` would still carry the fn.
        let c = extract(&sample_spans(), &[fnf("update_post", "posts::api")]);
        assert_eq!(titles(&c, "posts::update_post"), vec!["creates a post"]);
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
        assert_eq!(titles(&c, "posts::create_post").len(), 1);
    }

    /// One `e2e.test` span with a single child span of `name`, declaring `namespace`.
    fn one_named_span(name: &str, namespace: &str) -> Vec<Span> {
        let line = format!(
            concat!(
                r#"{{"resourceSpans":[{{"scopeSpans":[{{"spans":["#,
                r#"{{"traceId":"aa","spanId":"t1","name":"e2e.test","#,
                r#""attributes":[{{"key":"e2e.test","value":{{"stringValue":"t"}}}}]}},"#,
                r#"{{"traceId":"aa","spanId":"i1","parentSpanId":"t1","name":"{name}","#,
                r#""attributes":[{{"key":"code.namespace","#,
                r#""value":{{"stringValue":"{namespace}"}}}}]}}"#,
                r#"]}}]}}]}}"#,
            ),
            name = name,
            namespace = namespace,
        );
        parse_spans(&line, &Filters::default(), "t").expect("parses")
    }

    #[test]
    fn every_naming_regime_is_a_hit_given_the_right_module() {
        // The gate must not depend on which naming regime is in force. Today
        // `server-fn-tracing` writes `web.<vertical>.<ident>` (#511); omitting the
        // explicit name derives `__server_<ident>`; and were `#[server]` to stop
        // relocating the body, derivation would yield the bare ident. All three
        // denote the same fn, so all three count — matching one shape only is how
        // the signal silently died once already.
        for name in [
            "web.posts.update_post",
            "__server_update_post",
            "update_post",
        ] {
            let c = extract(
                &one_named_span(name, "web::posts::api"),
                &[fnf("update_post", "posts::api")],
            );
            assert_eq!(
                titles(&c, "posts::update_post"),
                vec!["t"],
                "span name `{name}` must identify the fn"
            );
        }
    }

    #[test]
    fn the_module_not_the_name_is_what_disambiguates() {
        // `web.<vertical>.<ident>` collapses every module of a vertical
        // (`posts::api` and `posts::api::listing` both yield `web.posts.…`), so the
        // name cannot be the discriminator — `code.namespace` is.
        for name in [
            "web.posts.update_post",
            "__server_update_post",
            "update_post",
        ] {
            let c = extract(
                &one_named_span(name, "web::elsewhere::api"),
                &[fnf("update_post", "posts::api")],
            );
            assert!(
                c.covered.is_empty(),
                "`{name}` in the wrong module must not count"
            );
        }
    }

    #[test]
    fn endpoint_is_matched_by_declared_value_not_by_fn_name() {
        // The fn is `fetch_post` but its declared endpoint is `get_post`, which is
        // what the URI carries. Assuming "/api/" + ident would miss it.
        let renamed = ServerFn {
            ident: "fetch_post".into(),
            endpoint: Some("get_post".into()),
            module: "posts::api".into(),
            line: 1,
        };
        let c = extract(&sample_spans(), &[renamed]);
        assert_eq!(titles(&c, "posts::fetch_post"), vec!["creates a post"]);
    }

    #[test]
    fn a_bare_server_fn_with_no_endpoint_matches_no_uri() {
        // `module` is deliberately NOT the fixture's `web::posts::api`: that keeps
        // the `__server_create_post` span from matching by name, so what is left to
        // observe is purely whether a `None` endpoint can match a URI.
        let bare = ServerFn {
            ident: "create_post".into(),
            endpoint: None,
            module: "elsewhere::api".into(),
            line: 1,
        };
        let c = extract(&sample_spans(), &[bare]);
        assert!(c.covered.is_empty(), "None endpoint must not match /api/…");
    }

    /// The two fns #684 makes indistinguishable by ident: one `create` per
    /// vertical, each with the endpoint its vertical declares.
    fn two_creates() -> Vec<ServerFn> {
        vec![
            ServerFn {
                ident: "create".into(),
                endpoint: Some("posts/create".into()),
                module: "posts::api".into(),
                line: 1,
            },
            ServerFn {
                ident: "create".into(),
                endpoint: Some("audiences/create".into()),
                module: "audiences::api".into(),
                line: 1,
            },
        ]
    }

    #[test]
    fn one_verticals_hit_does_not_cover_another_verticals_same_named_fn() {
        // The whole point of the qualified key. #684 dropped the vertical noun from
        // these idents, so `create` now names three fns; keying coverage on the
        // ident would have marked all three covered the moment any one of them ran,
        // and the gate would have gone green over two entirely untested flows.
        //
        // Both signals are exercised, because either one keying on the ident would
        // reopen the hole on its own: a span-name hit (module `web::posts::api`,
        // no `uri`) and a `uri` hit (`/api/posts/create`, no `code.namespace`).
        let by_name = one_named_span("web.posts.create", "web::posts::api");
        let mut by_uri = one_named_span("masked", "");
        by_uri[1].uri = "/api/posts/create".into();

        for spans in [by_name, by_uri] {
            let c = extract(&spans, &two_creates());
            assert_eq!(titles(&c, "posts::create"), vec!["t"]);
            assert!(
                !c.covered.contains_key("audiences::create"),
                "audiences::create was never driven and must report as uncovered: {:?}",
                c.covered
            );
        }
    }
}
