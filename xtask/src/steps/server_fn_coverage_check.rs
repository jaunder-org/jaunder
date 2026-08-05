//! Both lanes of the `#[server]` flow-coverage gate (#681).
//!
//! **The static lane** ([`run`]) belongs to `check` / `validate --no-e2e`: it reads
//! only committed files plus the syn inventory — no traces, no e2e run — so adding
//! a `#[server]` fn with no browser flow reddens the build immediately, without
//! waiting for the e2e matrix.
//!
//! **The e2e lane** ([`from_capture`], [`verify_after_combo`]) is the half that
//! keeps the snapshot honest: only a real run produces traces, so only there can
//! the gate notice that the committed snapshot no longer matches what the suite
//! exercises. `regenerate` rewrites it from a capture; `verify` fails on any
//! difference. Per the spec's D8 that runs on the per-combo
//! `cargo xtask e2e sqlite chromium` path **only**, never from the aggregate
//! `checks.e2e` join, where both sqlite combos emit a `capture-sqlite.tar.gz` and
//! collide unpredictably.
//!
//! Neither half is sufficient alone: traces exist only in the e2e lane, and fast
//! feedback only in the static one.

use std::path::Path;

use anyhow::Result;

use crate::result::{CommandResult, StepResult};
use crate::server_fn_coverage::io::{
    ALLOWLIST_PATH, CAPTURE_PATH, EVIDENCE_PATH, SNAPSHOT_PATH, WEB_SRC, coverage_from_capture,
    inventory, read_allowlist, read_artifact, write_artifact,
};
use crate::server_fn_coverage::{
    Evidence, REGENERATE_CMD, Snapshot, evidence_verdict, render, verdict,
};

/// The static lane's step name.
const STATIC_STEP: &str = "server-fn-coverage";
/// The e2e lane's step names. `pub` because `lib.rs` labels the `CommandResult`
/// and its `command_name()` arm with the same strings — spelling them again there
/// would let a rename desync the reported command from the step it ran.
pub const REGENERATE_STEP: &str = "server-fn-coverage-regenerate";
pub const VERIFY_STEP: &str = "server-fn-coverage-verify";

/// The one combo whose traces are authoritative (spec D6). `chromium` and
/// `chromium-admin` are exact complements over all spec files and no test is
/// browser- or backend-conditional, so picking one combo drops no coverage.
const AUTHORITATIVE: (&str, &str) = ("sqlite", "chromium");

/// The static-lane check, over explicit paths so it is testable without the repo.
///
/// A missing snapshot, an unscannable `web/src`, or an unparseable artifact is a
/// **failure**, never a pass: the failure mode this gate guards against and the
/// failure mode of its own plumbing would otherwise look identical.
/// The two generated artifacts are adjacent in this signature and in
/// [`regenerate_or_verify`]'s, so the pair reads the same way in both. They are
/// all `&Path`, so a transposition is a silent swap rather than a type error —
/// keeping one order is what makes it noticeable.
fn check(
    web_src: &Path,
    snapshot_path: &Path,
    evidence_path: &Path,
    allowlist_path: &Path,
) -> StepResult {
    let (inventory, snapshot, allowlist, evidence) = match (
        inventory(web_src),
        read_artifact::<Snapshot>(snapshot_path),
        read_allowlist(allowlist_path),
        read_artifact::<Evidence>(evidence_path),
    ) {
        (Ok(i), Ok(s), Ok(a), Ok(e)) => (i, s, a, e),
        (i, s, a, e) => {
            let detail = [i.err(), s.err(), a.err(), e.err()]
                .into_iter()
                .flatten()
                .map(|e| format!("{e:#}"))
                .collect::<Vec<_>>()
                .join("\n");
            return StepResult::fail(STATIC_STEP).detail(detail);
        }
    };

    // Both rules in one step, so a failing run reports every reason at once
    // rather than making an author fix them one gate run at a time.
    let mut violations = verdict(&inventory, &snapshot, &allowlist);
    violations.extend(evidence_verdict(&snapshot, &evidence));
    if violations.is_empty() {
        return StepResult::ok(STATIC_STEP).detail(format!(
            "{} server fn(s) accounted for ({} covered, {} allowlisted)",
            inventory.len(),
            snapshot.covered.len(),
            allowlist.len()
        ));
    }
    StepResult::fail(STATIC_STEP).detail(violations.join("\n"))
}

/// The static lane over the repo's real artifacts. Runs in `check` and
/// `validate --no-e2e`.
pub fn run(result: &mut CommandResult) {
    result.push(check(
        Path::new(WEB_SRC),
        Path::new(SNAPSHOT_PATH),
        Path::new(EVIDENCE_PATH),
        Path::new(ALLOWLIST_PATH),
    ));
}

/// The e2e lane's core, over explicit paths so it is testable without the repo.
///
/// Comparison is on the rendered bytes rather than the parsed value, so the
/// committed file is checked to be exactly what regeneration would produce —
/// a hand-edit that happens to parse equal is still drift.
fn regenerate_or_verify(
    web_src: &Path,
    capture: &Path,
    snapshot_path: &Path,
    evidence_path: &Path,
    regenerate: bool,
) -> Result<StepResult> {
    let name = if regenerate {
        REGENERATE_STEP
    } else {
        VERIFY_STEP
    };
    let inventory = inventory(web_src)?;
    let coverage = coverage_from_capture(capture, &inventory)?;
    let (snapshot, evidence) = coverage.split();
    let covered = snapshot.covered.len();

    if regenerate {
        let orphans = snapshot.orphans.len();
        // Both, always: `evidence_verdict` fails on a key-set disagreement, so
        // writing one without the other would redden the very next static check.
        write_artifact(snapshot_path, &snapshot)?;
        write_artifact(evidence_path, &evidence)?;
        return Ok(StepResult::ok(name).detail(format!(
            "{covered} covered, {orphans} fn(s) with unattributed hits → {} + {}",
            snapshot_path.display(),
            evidence_path.display()
        )));
    }

    // Only the snapshot is compared. The evidence file is a timing-dependent
    // observation (see `snapshot.rs`'s module docs) and comparing it is exactly
    // the bug #745 fixed.
    let committed = std::fs::read_to_string(snapshot_path).unwrap_or_default();
    compare_rendered(
        name,
        &committed,
        &render(&snapshot)?,
        snapshot_path,
        covered,
    )
}

/// The verify verdict for bytes already derived — pure over its inputs, so the
/// drift branch is testable without a capture tarball.
///
/// A missing snapshot reaches here as an empty `committed`, which never equals
/// rendered output and so fails — the strict reading, not a lenient one.
fn compare_rendered(
    name: &'static str,
    committed: &str,
    rendered: &str,
    snapshot_path: &Path,
    covered: usize,
) -> Result<StepResult> {
    if committed == rendered {
        return Ok(StepResult::ok(name).detail(format!("{covered} covered; snapshot current")));
    }
    Ok(StepResult::fail(name).detail(format!(
        "{} is out of date with this run's traces — regenerate it with `{REGENERATE_CMD}` and \
         commit the result",
        snapshot_path.display()
    )))
}

/// Derive coverage from an e2e capture over the repo's real roots, and either
/// rewrite the committed artifacts (`regenerate`) or fail on any difference from
/// the committed snapshot (`verify`).
pub fn from_capture(capture: &Path, regenerate: bool) -> Result<StepResult> {
    regenerate_or_verify(
        Path::new(WEB_SRC),
        capture,
        Path::new(SNAPSHOT_PATH),
        Path::new(EVIDENCE_PATH),
        regenerate,
    )
}

/// After the authoritative combo, confirm the committed snapshot still matches
/// what the suite exercised. A no-op for every other combo (D8/D6).
///
/// Skipped when the combo itself failed: a failed run's capture is partial or
/// absent, so drift against it would be noise reported on top of the real
/// failure. `result` carries the combo's verdict at this point — the only other
/// step run before this one is the informational flaky scan, which never fails.
pub fn verify_after_combo(result: &mut CommandResult, backend: &str, browser: &str) {
    if (backend, browser) != AUTHORITATIVE {
        return;
    }
    if !result.ok {
        result.push(StepResult::skip(VERIFY_STEP).detail("combo failed — no trustworthy capture"));
        return;
    }
    let step = from_capture(Path::new(CAPTURE_PATH), false)
        .unwrap_or_else(|e| StepResult::fail(VERIFY_STEP).detail(format!("{e:#}")));
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::retired_server_fn;

    /// The vertical the fixture tree writes its fns in.
    const VERTICAL: &str = "posts";

    /// A `web/src`-shaped tree with one `#[macros::server]` fn per ident given, all
    /// in the [`VERTICAL`] vertical — a fn at the crate root has no vertical, so its
    /// coverage key would be a degenerate `::<ident>` and pin nothing about the
    /// real artifacts. The endpoints are the `<vertical>/<ident>` the gate derives
    /// from that placement.
    fn web_src_with(dir: &Path, idents: &[&str]) {
        let src: String = idents
            .iter()
            .map(|i| format!("#[macros::server]\npub async fn {i}() {{}}\n"))
            .collect();
        let vertical = dir.join(VERTICAL);
        std::fs::create_dir_all(&vertical).expect("create the vertical's dir");
        std::fs::write(vertical.join("api.rs"), src).expect("write source");
    }

    fn write_json(path: &Path, json: &str) {
        std::fs::write(path, json).expect("write json");
    }

    /// Write an evidence file agreeing with `names`, so a test that is not about
    /// evidence drift does not trip over `evidence_verdict`.
    fn evidence_for(dir: &Path, names: &[&str]) -> std::path::PathBuf {
        let path = dir.join("evidence.json");
        let entries: Vec<String> = names
            .iter()
            .map(|n| format!(r#""{n}":["a test"]"#))
            .collect();
        write_json(
            &path,
            &format!(r#"{{"covered":{{{}}}}}"#, entries.join(",")),
        );
        path
    }

    #[test]
    fn static_lane_passes_when_every_fn_is_covered() {
        let tmp = tempfile::tempdir().expect("tempdir");
        web_src_with(tmp.path(), &["create_post"]);
        let snap = tmp.path().join("snap.json");
        write_json(&snap, r#"{"covered":["posts::create_post"],"orphans":{}}"#);
        let ev = evidence_for(tmp.path(), &["posts::create_post"]);

        let step = check(
            tmp.path(),
            &snap,
            &ev,
            &tmp.path().join("absent-allowlist.json"),
        );
        assert!(step.ok, "{:?}", step.detail);
        assert!(step.detail.unwrap_or_default().contains("1 server fn"));
    }

    #[test]
    fn static_lane_bites_on_an_uncovered_fn() {
        // The wired-up half of AC12: `verdict`'s own bite test proves the rule,
        // this proves the lane applies it to the real artifacts.
        let tmp = tempfile::tempdir().expect("tempdir");
        web_src_with(tmp.path(), &["create_post", "brand_new_uncovered_fn"]);
        let snap = tmp.path().join("snap.json");
        write_json(&snap, r#"{"covered":["posts::create_post"],"orphans":{}}"#);
        let ev = evidence_for(tmp.path(), &["posts::create_post"]);

        let step = check(
            tmp.path(),
            &snap,
            &ev,
            &tmp.path().join("absent-allowlist.json"),
        );
        assert!(!step.ok);
        let detail = step.detail.unwrap_or_default();
        assert!(detail.contains("brand_new_uncovered_fn"), "{detail}");
    }

    #[test]
    fn static_lane_accepts_a_substantive_allowlist_entry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        web_src_with(tmp.path(), &["no_flow_yet"]);
        let snap = tmp.path().join("snap.json");
        write_json(&snap, r#"{"covered":[],"orphans":{}}"#);
        let ev = evidence_for(tmp.path(), &[]);
        let allow = tmp.path().join("allow.json");
        write_json(
            &allow,
            r##"[{"server_fn":"posts::no_flow_yet","reason":"no UI surface yet","issue":"#700"}]"##,
        );

        let step = check(tmp.path(), &snap, &ev, &allow);
        assert!(step.ok, "{:?}", step.detail);
    }

    #[test]
    fn static_lane_fails_when_the_evidence_is_missing_a_covered_fn() {
        let tmp = tempfile::tempdir().expect("tempdir");
        web_src_with(tmp.path(), &["create_post"]);
        let snap = tmp.path().join("snap.json");
        write_json(&snap, r#"{"covered":["posts::create_post"],"orphans":{}}"#);
        let ev = evidence_for(tmp.path(), &[]);

        let step = check(
            tmp.path(),
            &snap,
            &ev,
            &tmp.path().join("absent-allowlist.json"),
        );
        assert!(!step.ok);
        let detail = step.detail.unwrap_or_default();
        assert!(detail.contains("posts::create_post"), "{detail}");
        assert!(detail.contains("missing from the evidence"), "{detail}");
    }

    #[test]
    fn static_lane_fails_when_the_evidence_names_an_uncovered_fn() {
        // The other direction, at the fixture-file level: stale evidence naming a
        // fn the snapshot no longer covers.
        let tmp = tempfile::tempdir().expect("tempdir");
        web_src_with(tmp.path(), &["create_post"]);
        let snap = tmp.path().join("snap.json");
        write_json(&snap, r#"{"covered":["posts::create_post"],"orphans":{}}"#);
        let ev = evidence_for(tmp.path(), &["posts::create_post", "posts::ghost"]);

        let step = check(
            tmp.path(),
            &snap,
            &ev,
            &tmp.path().join("absent-allowlist.json"),
        );
        assert!(!step.ok);
        let detail = step.detail.unwrap_or_default();
        assert!(detail.contains("posts::ghost"), "{detail}");
        assert!(detail.contains("stale evidence"), "{detail}");
        // The remedy travels with BOTH directions, not just the missing-key one:
        // an author who hits the stale direction needs the same two steps.
        assert!(detail.contains(REGENERATE_CMD), "{detail}");
        assert!(
            detail.contains("cargo xtask e2e sqlite chromium"),
            "{detail}"
        );
    }

    #[test]
    fn static_lane_fails_closed_on_a_missing_evidence_file() {
        // The plumbing's own failure must not look like "nothing uncovered".
        let tmp = tempfile::tempdir().expect("tempdir");
        web_src_with(tmp.path(), &["create_post"]);
        let snap = tmp.path().join("snap.json");
        write_json(&snap, r#"{"covered":["posts::create_post"],"orphans":{}}"#);

        let step = check(
            tmp.path(),
            &snap,
            &tmp.path().join("absent-evidence.json"),
            &tmp.path().join("absent-allowlist.json"),
        );
        assert!(!step.ok);
        let detail = step.detail.unwrap_or_default();
        assert!(
            detail.contains(REGENERATE_CMD),
            "names the remedy: {detail}"
        );
    }

    #[test]
    fn static_lane_fails_closed_on_a_missing_snapshot() {
        // The plumbing's own failure must not look like "nothing uncovered".
        let tmp = tempfile::tempdir().expect("tempdir");
        web_src_with(tmp.path(), &["create_post"]);
        let ev = evidence_for(tmp.path(), &["posts::create_post"]);

        let step = check(
            tmp.path(),
            &tmp.path().join("absent-snapshot.json"),
            &ev,
            &tmp.path().join("absent-allowlist.json"),
        );
        assert!(!step.ok);
        let detail = step.detail.unwrap_or_default();
        assert!(
            detail.contains(REGENERATE_CMD),
            "names the remedy: {detail}"
        );
    }

    #[test]
    fn static_lane_fails_closed_on_an_unparseable_snapshot() {
        let tmp = tempfile::tempdir().expect("tempdir");
        web_src_with(tmp.path(), &["create_post"]);
        let snap = tmp.path().join("snap.json");
        write_json(&snap, "{not json");
        let ev = evidence_for(tmp.path(), &["posts::create_post"]);

        let step = check(
            tmp.path(),
            &snap,
            &ev,
            &tmp.path().join("absent-allowlist.json"),
        );
        assert!(!step.ok);
    }

    // ── The e2e lane's byte-compare ─────────────────────────────────────────
    //
    // These reach `compare_rendered` directly. Before it was extracted the only
    // way in was `regenerate_or_verify`, which needs a real ~2 MB capture
    // tarball, so the drift branch — the thing the whole e2e lane exists to do —
    // had no test at all.

    #[test]
    fn identical_bytes_verify_clean() {
        let step = compare_rendered(
            VERIFY_STEP,
            "same\n",
            "same\n",
            Path::new("docs/coverage/server-fns.json"),
            54,
        )
        .expect("compares");
        assert!(step.ok, "{:?}", step.detail);
        assert!(step.detail.unwrap_or_default().contains("54 covered"));
    }

    #[test]
    fn any_byte_difference_is_drift() {
        // Byte equality, not parsed equality: a hand-edit that happens to parse
        // equal is still drift, which is what makes the committed artifact
        // provably what regeneration produces.
        let step = compare_rendered(
            VERIFY_STEP,
            "a\n",
            "b\n",
            Path::new("docs/coverage/server-fns.json"),
            54,
        )
        .expect("compares");
        assert!(!step.ok);
        let detail = step.detail.unwrap_or_default();
        assert!(detail.contains("docs/coverage/server-fns.json"), "{detail}");
        assert!(
            detail.contains(REGENERATE_CMD),
            "names the remedy: {detail}"
        );
    }

    #[test]
    fn a_missing_committed_file_reads_as_empty_and_therefore_drifts() {
        // `regenerate_or_verify` passes `unwrap_or_default()` for an unreadable
        // file; empty never equals rendered output, so it fails — the strict
        // reading, not a lenient one.
        let step = compare_rendered(
            VERIFY_STEP,
            "",
            "anything\n",
            Path::new("docs/coverage/server-fns.json"),
            0,
        )
        .expect("compares");
        assert!(!step.ok);
    }

    #[test]
    fn whitespace_only_difference_is_still_drift() {
        // The failure mode a parsed comparison would wave through: same value,
        // different bytes. This is exactly what byte-comparison is for.
        let step = compare_rendered(
            VERIFY_STEP,
            "{\n  \"covered\": []\n}\n",
            "{\n    \"covered\": []\n}\n",
            Path::new("docs/coverage/server-fns.json"),
            0,
        )
        .expect("compares");
        assert!(!step.ok, "reformatted-but-equal must still be drift");
    }

    #[test]
    fn e2e_lane_is_a_no_op_for_a_non_authoritative_combo() {
        // D8: only the sqlite × chromium combo has an uncollided capture.
        for (backend, browser) in [
            ("sqlite", "firefox"),
            ("postgres", "chromium"),
            ("postgres", "firefox"),
        ] {
            let mut result = CommandResult::new("e2e");
            verify_after_combo(&mut result, backend, browser);
            assert!(
                result.steps.is_empty(),
                "{backend} × {browser} must not touch the snapshot"
            );
        }
    }

    #[test]
    fn e2e_lane_skips_when_the_combo_failed() {
        // A failed combo's capture is partial or absent; drift against it would be
        // noise stacked on the real failure.
        let mut result = CommandResult::new("e2e-sqlite-chromium");
        result.push(StepResult::fail("nix-e2e-sqlite-chromium"));
        verify_after_combo(&mut result, "sqlite", "chromium");

        let last = result.steps.last().expect("a step");
        assert_eq!(last.name, VERIFY_STEP);
        assert!(last.skipped, "reported as skipped, not as a second failure");
    }

    // ── Assertions against the real seeding capture (spec AC2/AC11) ──────────
    //
    // The fixture is the `sqlite × chromium` capture the committed allowlist was
    // derived from, reduced by `testdata/reduce-otel-capture.mjs` to the spans the
    // extractor actually reads: one hit-chain per (span name + URI path, test)
    // pair, at most two orphan examples per key, eight non-`/api/` spans, and only
    // the handful of attributes `parse_spans`/`extract` consume. That keeps it
    // ~610 KiB instead of 25 MB while preserving the hit set exactly — the same 911
    // (fn, test) pairs the full capture yields. That preservation is what AC11
    // rests on, which is why the reduction is committed and re-runnable rather than
    // described: a reader can regenerate the fixture and diff instead of taking it
    // on trust. Per-fn orphan *counts* are NOT preserved by the dedup — the
    // committed snapshot's counts come from the full capture.

    /// xtask's tests run with the crate dir as cwd, so repo-relative artifact
    /// paths have to be resolved explicitly.
    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
    }

    const SEED_FIXTURE: &str = "src/server_fn_coverage/testdata/otel-traces-seed.jsonl";

    fn seed_spans() -> Vec<crate::traces::parse::Span> {
        crate::traces::parse::read_spans(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join(SEED_FIXTURE),
            &crate::traces::parse::Filters::default(),
        )
        .expect("the seed fixture parses")
    }

    fn seed_coverage() -> crate::server_fn_coverage::Coverage {
        let inv = inventory(&repo_root().join(WEB_SRC)).expect("inventory enumerates");
        crate::server_fn_coverage::extract(&seed_spans(), &inv)
    }

    #[test]
    fn every_allowlist_entry_is_absent_from_the_seed_captures_hit_set() {
        // AC11. Without this, an evidence-seeded allowlist and a guessed one are
        // byte-identical and nothing in the repo can tell them apart.
        let coverage = seed_coverage();
        let allowlist =
            read_allowlist(&repo_root().join(ALLOWLIST_PATH)).expect("allowlist parses");
        assert!(!allowlist.is_empty(), "the assertion is vacuous when empty");
        for entry in allowlist {
            assert!(
                !coverage.covered.contains_key(&entry.server_fn),
                "{} is allowlisted but the seed capture shows it covered — the allowlist was \
                 not derived from evidence",
                entry.server_fn
            );
        }
    }

    #[test]
    fn seed_capture_covers_the_committed_snapshots_fns() {
        // The fixture must still be the evidence behind the committed snapshot:
        // every fn the snapshot calls covered is covered in the reduced capture too.
        let coverage = seed_coverage();
        let snapshot =
            read_artifact::<Snapshot>(&repo_root().join(SNAPSHOT_PATH)).expect("snapshot parses");
        let missing: Vec<&String> = snapshot
            .covered
            .iter()
            .filter(|fnname| !coverage.covered.contains_key(*fnname))
            .collect();
        assert!(
            missing.is_empty(),
            "the snapshot claims these covered but the seed capture does not: {missing:?}"
        );
    }

    /// The seed capture with every span's `uri` erased, so whatever `extract` still
    /// finds got there by the span-name signal alone. Erased rather than dropped:
    /// removing the request spans would break the ancestor chains an instrument span
    /// is attributed through, and the lane would look dead for the wrong reason.
    /// The derived endpoint matches what a real run actually requested (#714).
    ///
    /// This is the only live verification that the macro's derivation and xtask's
    /// copy of the same rule agree. A mechanical cross-check is impossible: xtask
    /// is its own workspace with no `web` dependency, so it can never read
    /// `ServerFn::PATH`. What it *can* do is compare its computed value against
    /// URIs a browser actually requested, which is ground truth produced by the
    /// macro's real expansion rather than a second statement of the rule.
    ///
    /// **Presence is established by span name + module, never by the endpoint
    /// being checked.** Looking a computed endpoint up among the seed's URIs and
    /// skipping the misses would make the drift case *identical to* the skipped
    /// case — a wrong derivation would simply be "absent" and pass in silence.
    /// That exact failure is on the record: `extract.rs`'s module doc describes a
    /// matcher that matched nothing and did so silently. So the fn is located by
    /// signal 1, and only then is its URI compared.
    ///
    /// The matched count is asserted too, so the check cannot quietly shrink to
    /// nothing and keep passing.
    #[test]
    fn the_derived_endpoint_matches_the_uri_a_real_run_requested() {
        let inv = inventory(&repo_root().join(WEB_SRC)).expect("inventory enumerates");
        let spans = seed_spans();
        let by_id: std::collections::HashMap<&str, &crate::traces::parse::Span> =
            spans.iter().map(|s| (s.span_id.as_str(), s)).collect();

        let mut matched = 0usize;
        let mut mismatches: Vec<String> = Vec::new();

        for span in &spans {
            // Signal 1 — span name plus `code.namespace`. Independent of the URI,
            // which is the value under test.
            let namespace = crate::traces::parse::get_attr(
                &span.raw,
                crate::server_fn_coverage::extract::MODULE_ATTR,
            );
            if namespace.is_empty() {
                continue;
            }
            let relative = namespace
                .strip_prefix("web::")
                .unwrap_or(namespace.as_str());
            let Some(f) = inv.iter().find(|f| {
                relative == f.module
                    && crate::server_fn_coverage::extract::candidate_span_names(f)
                        .contains(&span.name)
            }) else {
                continue;
            };

            // The instrument span carries the module; the *request* span carries the
            // URI — two different spans, the fn's being a descendant of the HTTP
            // one. So walk up to the nearest ancestor that has a URI, the same
            // two-hop shape `extract` attributes through.
            let mut cursor = span;
            let mut hops = 0;
            let observed = loop {
                if !cursor.uri.is_empty() {
                    break Some(cursor.uri.clone());
                }
                hops += 1;
                if hops > 8 {
                    break None;
                }
                match by_id.get(cursor.parent_span_id.as_str()) {
                    Some(parent) => cursor = parent,
                    None => break None,
                }
            };
            let Some(observed) = observed else { continue };

            let expected = format!("/api/{}", f.endpoint.as_deref().unwrap_or_default());
            let without_query = observed.split('?').next().unwrap_or(&observed);
            let path = without_query
                .find("/api/")
                .map_or(without_query, |at| &without_query[at..]);
            if path != expected {
                mismatches.push(format!(
                    "{}: derived `{expected}` but a real run requested `{path}`",
                    f.qualified()
                ));
            }
            matched += 1;
        }

        assert!(
            mismatches.is_empty(),
            "the computed endpoint disagrees with the captured traffic: {mismatches:?}"
        );
        assert!(
            matched >= 50,
            "only {matched} span(s) were located by name+module and carried a URI — the \
             cross-check has gone nearly vacuous, which is how it would silently stop \
             verifying anything"
        );
    }

    fn seed_spans_without_uris() -> Vec<crate::traces::parse::Span> {
        let mut spans = seed_spans();
        for span in &mut spans {
            span.uri.clear();
        }
        spans
    }

    /// The seed capture with every non-test span's name mangled, so whatever
    /// `extract` still finds got there by the `uri` signal alone. Renamed rather
    /// than dropped, for the same reason `seed_spans_without_uris` erases instead
    /// of removing: the spans are load-bearing links in the ancestor chains.
    fn seed_spans_without_span_names() -> Vec<crate::traces::parse::Span> {
        let mut spans = seed_spans();
        for span in &mut spans {
            if span.name != "e2e.test" {
                span.name = format!("masked.{}", span.name);
            }
        }
        spans
    }

    #[test]
    fn each_signal_finds_fns_on_its_own_in_the_real_capture() {
        // AC2's "both signals" clause, measured signal-by-signal against real data
        // rather than inferred from the union. This is the assertion whose absence
        // let the span-name signal sit dead for an entire cycle: it matched the bare
        // `<ident>` while every real span was named `__server_<ident>`, and since
        // `uri` covered the same fns the union looked perfectly healthy.
        //
        // Since #511 instrumented all 55 fns, the two signals now cover the SAME
        // set — which is the redundancy the design wants, and is why each must be
        // measured alone. Asserting set equality both ways means a regression in
        // either one fails here instead of hiding behind the other.
        let inv = inventory(&repo_root().join(WEB_SRC)).expect("inventory enumerates");
        let by_name = crate::server_fn_coverage::extract(&seed_spans_without_uris(), &inv);
        let by_uri = crate::server_fn_coverage::extract(&seed_spans_without_span_names(), &inv);
        let union = crate::server_fn_coverage::extract(&seed_spans(), &inv);

        assert!(
            !union.covered.is_empty(),
            "the fixture must cover something, or every assertion below is vacuous"
        );
        let names: Vec<&String> = by_name.covered.keys().collect();
        let uris: Vec<&String> = by_uri.covered.keys().collect();
        let both: Vec<&String> = union.covered.keys().collect();
        assert_eq!(
            names, both,
            "the span-name signal alone must cover everything the union does"
        );
        assert_eq!(
            uris, both,
            "the uri signal alone must cover everything the union does"
        );

        // `media::upload` is the sharpest single case for the uri signal: it declares
        // `#[server(input = MultipartFormData, endpoint = "/media/upload")]`, so
        // anything reading `endpoint` as the attribute's FIRST argument loses it
        // silently and the fn drops out of URI matching altogether.
        assert!(
            by_uri.covered.contains_key("media::upload"),
            "media::upload must be covered by the uri signal alone"
        );

        // No query-string assertion here: every server fn this suite drives is a
        // POST, so not one `/api/` URI in the capture carries a `?`. Query stripping
        // stays pinned on the hand-authored `coverage-sample.jsonl` — asserting it
        // against real data would only pin its absence.
    }

    #[test]
    fn the_span_names_carry_the_module_the_check_compares() {
        // Guards the reduction as much as the extractor: signal 1 refuses a hit it
        // cannot place in the right module, so a fixture that kept the instrument
        // spans but dropped their `code.namespace` would silently fall back to `uri`
        // for everything — and, per the test above, look identical while doing it.
        let inv = inventory(&repo_root().join(WEB_SRC)).expect("inventory enumerates");
        let verticals: std::collections::BTreeSet<&str> = inv
            .iter()
            .map(crate::server_fns::ServerFn::vertical)
            .collect();
        let mut checked = 0;
        for span in seed_spans().iter().filter(|s| {
            verticals
                .iter()
                .any(|v| s.name.starts_with(&format!("web.{v}.")))
        }) {
            let namespace = crate::traces::parse::get_attr(&span.raw, "code.namespace");
            assert!(
                namespace.starts_with("web::"),
                "{} lost its web:: code.namespace: {namespace:?}",
                span.name
            );
            checked += 1;
        }
        assert!(
            checked > 0,
            "no instrument spans in the fixture — the reduction dropped the span-name \
             signal's evidence entirely"
        );
    }

    #[test]
    fn the_fixture_keeps_non_api_traffic_as_the_negative_case() {
        // Static assets and feeds must be present and attributed to no fn, or
        // "nothing spurious is counted" is untested against real traffic.
        let spans = seed_spans();
        assert!(
            spans
                .iter()
                .any(|s| !s.uri.is_empty() && !s.uri.contains("/api/")),
            "no non-/api/ traffic in the fixture — the negative case is untested"
        );
    }

    // ── AC16: the committed artifacts must not invalidate the e2e checks ─────────

    /// Every string literal the app source filter (`flake.nix`'s top-level `src`,
    /// the tree the e2e VM checks build from) matches paths against. Pinned as a
    /// whole set rather than probed for the absence of "docs": a filter that began
    /// admitting the coverage artifacts would do it through a *new* literal —
    /// `".json"`, say — that a search for "docs" would sail straight past.
    const APP_SRC_FILTER_LITERALS: &[&str] =
        &["/xtask/", ".sql", ".css", "csr/index.html", "scripts/.*"];

    /// The `flake.nix` block whose filter decides the app source tree, delimited by
    /// its opening `cleanSourceWith` and the closing brace at that indent.
    fn app_src_filter_block(flake: &str) -> &str {
        let (_, rest) = flake
            .split_once("        src = pkgs.lib.cleanSourceWith {")
            .expect("flake.nix declares the app source filter");
        let (block, _) = rest
            .split_once("\n        };")
            .expect("the filter block closes at its own indent");
        block
    }

    #[test]
    fn the_e2e_checks_source_filter_still_excludes_the_coverage_artifacts() {
        // AC16: regenerating `docs/coverage/*.json` must leave the four e2e VM
        // checks' input hashes untouched. It does because the app source filter is an
        // ALLOWLIST — a path enters only by matching one of the literals above, and
        // no `docs/…json` path matches any of them — and because `e2ePackage` roots
        // at `./end2end`, which cannot contain `docs/`.
        //
        // This is a PROXY and does not prove AC16. It asserts the *reason* the drv
        // hashes are stable, not the hashes: only comparing
        // `nix eval --raw .#checks.x86_64-linux.e2e-sqlite-chromium.drvPath` across
        // the change proves that, and a `nix eval` is far too slow for the
        // per-commit gate. What it does buy is that no filter edit can quietly widen
        // the admitted set — a new clause reddens here and has to be argued against
        // AC16 explicitly.
        let flake = std::fs::read_to_string(repo_root().join("flake.nix")).expect("flake.nix");
        let block = app_src_filter_block(&flake);

        let literals: Vec<&str> = block
            .split('"')
            .skip(1)
            .step_by(2)
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(
            literals, APP_SRC_FILTER_LITERALS,
            "the app source filter's match literals changed — if the new one can \
             admit a docs/ path, regenerating the coverage snapshot now rebuilds the \
             four e2e VMs (AC16)"
        );

        assert!(
            flake.contains("src = ./end2end;"),
            "e2ePackage must stay rooted at ./end2end; a wider root could pull in docs/"
        );
    }

    #[test]
    fn verify_from_a_missing_capture_is_an_error() {
        // Not `Ok(fail)`: a broken capture must reach the exit-2 path, so it can
        // never be confused with "the suite covers everything".
        let tmp = tempfile::tempdir().expect("tempdir");
        web_src_with(tmp.path(), &["create_post"]);
        let err = regenerate_or_verify(
            tmp.path(),
            Path::new("/nonexistent-capture.tar.gz"),
            &tmp.path().join("snap.json"),
            &tmp.path().join("evidence.json"),
            false,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("capture"), "{err:#}");
    }

    #[test]
    fn an_unscannable_web_src_is_an_error_not_an_empty_inventory() {
        // A moved/renamed `web/src` must not quietly derive "no fns, all covered".
        let tmp = tempfile::tempdir().expect("tempdir");
        let err = regenerate_or_verify(
            &tmp.path().join("nonexistent"),
            Path::new("/nonexistent-capture.tar.gz"),
            &tmp.path().join("snap.json"),
            &tmp.path().join("evidence.json"),
            false,
        )
        .unwrap_err();
        let chain = format!("{err:#}");
        assert!(chain.contains("nonexistent"), "{chain}");
        assert!(chain.contains("#[macros::server]"), "{chain}");
    }

    #[test]
    fn a_fn_in_the_retired_spelling_is_not_in_the_inventory() {
        // #714 narrowed the shared enumerator to `#[macros::server]`. An inventory
        // that silently dropped every fn would make the whole gate vacuous, which is
        // why `enumeration_of_web_src_matches_the_registrar` (in the registrar gate)
        // asserts the real tree still enumerates; here we only pin the narrowing.
        let tmp = tempfile::tempdir().expect("tempdir");
        let vertical = tmp.path().join(VERTICAL);
        std::fs::create_dir_all(&vertical).expect("create the vertical's dir");
        std::fs::write(
            vertical.join("api.rs"),
            retired_server_fn("(endpoint = \"/posts/create\")", "pub async fn create() {}"),
        )
        .expect("write source");
        assert!(inventory(tmp.path()).expect("inventory scans").is_empty());
    }
}
