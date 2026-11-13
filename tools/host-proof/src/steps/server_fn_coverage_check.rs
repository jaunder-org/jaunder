//! Explicit producer-artifact flow verification; static inventory lives in repository-policy.
//! The workflow, never this consumer, selects/builds the authoritative output.

use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::server_fn_coverage::io::{
    SNAPSHOT_PATH, WEB_SRC, coverage_from_capture, inventory, write_snapshot,
};
use crate::server_fn_coverage::{REGENERATE_CMD, Snapshot, render};

/// The e2e lane's core, over explicit paths so it is testable without the repo.
///
/// Comparison is on the rendered bytes rather than the parsed value, so the
/// committed file is checked to be exactly what regeneration would produce —
/// a hand-edit that happens to parse equal is still drift.
fn regenerate_or_verify(
    web_src: &Path,
    capture: &Path,
    snapshot_path: &Path,
    regenerate: bool,
) -> Result<String> {
    let inventory = inventory(web_src)?;
    let snapshot = coverage_from_capture(capture, &inventory)?.into_snapshot();
    let covered = snapshot.covered.len();

    if regenerate {
        return regenerate_snapshot(snapshot_path, &snapshot);
    }

    // The deterministic snapshot is the sole generated artifact and the sole
    // value compared by verification.
    let committed = std::fs::read_to_string(snapshot_path).unwrap_or_default();
    compare_rendered(&committed, &render(&snapshot)?, snapshot_path, covered)
}

/// Write the one generated artifact from an already-derived snapshot. Keeping
/// this edge separate makes the one-output regeneration contract testable
/// without manufacturing a capture tarball.
fn regenerate_snapshot(snapshot_path: &Path, snapshot: &Snapshot) -> Result<String> {
    let covered = snapshot.covered.len();
    let orphans = snapshot.orphans.len();
    write_snapshot(snapshot_path, snapshot)?;
    Ok(format!(
        "{covered} covered, {orphans} fn(s) with unattributed hits → {}",
        snapshot_path.display()
    ))
}

/// The verify verdict for bytes already derived — pure over its inputs, so the
/// drift branch is testable without a capture tarball.
///
/// A missing snapshot reaches here as an empty `committed`, which never equals
/// rendered output and so fails — the strict reading, not a lenient one.
fn compare_rendered(
    committed: &str,
    rendered: &str,
    snapshot_path: &Path,
    covered: usize,
) -> Result<String> {
    if committed == rendered {
        return Ok(format!("{covered} covered; snapshot current"));
    }
    bail!(
        "{} is out of date with this run's traces — regenerate it with `{REGENERATE_CMD}` and commit the result",
        snapshot_path.display()
    )
}

/// Consume only the selected successful immutable producer output. Nix owns
/// writer quiescence, retention and archive member hash/size checks; the existing
/// population and trace verdicts corroborate the artifacts, not hidden omissions.
pub fn from_producer(inputs: &crate::e2e::ProducerCapture, regenerate: bool) -> Result<String> {
    validate_selected_capture(&inputs.capture, &inputs.producer_output, &inputs.lane)?;
    crate::e2e_evidence::verify_lane(&inputs.lane, &inputs.catalog, &inputs.producer_output)
        .map_err(anyhow::Error::msg)?;
    regenerate_or_verify(
        Path::new(WEB_SRC),
        &inputs.capture,
        Path::new(SNAPSHOT_PATH),
        regenerate,
    )
}

fn validate_selected_capture(capture: &Path, output: &Path, lane: &str) -> Result<()> {
    if lane != "sqlite-chromium-unsplit" {
        bail!("only sqlite-chromium-unsplit is authoritative for flow coverage");
    }
    let output = output
        .canonicalize()
        .context("resolving selected producer output")?;
    if output.parent() != Some(Path::new("/nix/store")) {
        bail!("producer output is not an immutable Nix output");
    }
    let capture = capture.canonicalize().context("resolving capture")?;
    if capture != output.join(format!("capture-{lane}.tar.gz")) {
        bail!("capture does not belong to the selected producer output");
    }
    // A kept failed output is not registered as a valid store path. Merely
    // finding its capture (or a stale success-only gcroot) cannot grant authority.
    let status = std::process::Command::new("nix")
        .args(["path-info", "--offline"])
        .arg(&output)
        // Only validity matters; stdout carries this command's JSON report.
        .stdout(std::process::Stdio::null())
        .status()
        .context("checking selected producer output validity")?;
    if !status.success() {
        bail!("selected producer output is not a successful registered Nix output");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server_fn_coverage::io::read_snapshot;
    use crate::test_support::retired_server_fn;

    /// The vertical the fixture tree writes its fns in.
    const VERTICAL: &str = "posts";

    /// A `web/src`-shaped tree with one `#[macros::server]` fn per ident given, all
    /// in the [`VERTICAL`] vertical — a fn at the crate root has no vertical, so its
    /// coverage key would be a degenerate `::<ident>` and pin nothing about the
    /// real snapshot. The endpoints are the `<vertical>/<ident>` the gate derives
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

    #[test]
    fn regeneration_writes_only_the_snapshot() {
        // The retired file is a sentinel: regeneration must neither rewrite it
        // nor create any replacement attribution artifact beside the snapshot.
        let tmp = tempfile::tempdir().expect("tempdir");
        let coverage_dir = tmp.path().join("docs/coverage");
        std::fs::create_dir_all(&coverage_dir).expect("coverage directory");
        let snapshot_path = coverage_dir.join("server-fns.json");
        let retired_path = coverage_dir.join("server-fns-evidence.json");
        write_json(&retired_path, "retired sentinel");
        let snapshot = Snapshot {
            covered: ["posts::create_post".to_string()].into_iter().collect(),
            orphans: Default::default(),
        };

        regenerate_snapshot(&snapshot_path, &snapshot).expect("regenerates");
        assert_eq!(
            read_snapshot(&snapshot_path).expect("snapshot parses"),
            snapshot
        );
        assert_eq!(
            std::fs::read_to_string(&retired_path).expect("sentinel remains"),
            "retired sentinel"
        );

        let mut generated: Vec<String> = std::fs::read_dir(&coverage_dir)
            .expect("read coverage directory")
            .map(|entry| {
                entry
                    .expect("directory entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        generated.sort();
        assert_eq!(
            generated,
            vec![
                "server-fns-evidence.json".to_string(),
                "server-fns.json".to_string(),
            ],
            "regeneration created an unexpected artifact"
        );
    }

    // ── The e2e lane's byte-compare ─────────────────────────────────────────
    //
    // These reach `compare_rendered` directly. Before it was extracted the only
    // way in was `regenerate_or_verify`, which needs a real ~2 MB capture
    // tarball, so the drift branch — the thing the whole e2e lane exists to do —
    // had no test at all.

    #[test]
    fn identical_bytes_verify_clean() {
        let detail = compare_rendered(
            "same\n",
            "same\n",
            Path::new("docs/coverage/server-fns.json"),
            54,
        )
        .unwrap();
        assert!(detail.contains("54 covered"));
    }

    #[test]
    fn any_byte_difference_is_drift() {
        // Byte equality, not parsed equality: a hand-edit that happens to parse
        // equal is still drift, which is what makes the committed artifact
        // provably what regeneration produces.
        let detail = compare_rendered("a\n", "b\n", Path::new("docs/coverage/server-fns.json"), 54)
            .unwrap_err()
            .to_string();
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
        assert!(
            compare_rendered(
                "",
                "anything\n",
                Path::new("docs/coverage/server-fns.json"),
                0
            )
            .is_err()
        );
    }

    #[test]
    fn whitespace_only_difference_is_still_drift() {
        // The failure mode a parsed comparison would wave through: same value,
        // different bytes. This is exactly what byte-comparison is for.
        assert!(
            compare_rendered(
                "{\n  \"covered\": []\n}\n",
                "{\n    \"covered\": []\n}\n",
                Path::new("docs/coverage/server-fns.json"),
                0
            )
            .is_err(),
            "reformatted-but-equal must still be drift"
        );
    }

    fn write_sample_capture(path: &Path) {
        let file = std::fs::File::create(path).expect("create capture");
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut archive = tar::Builder::new(encoder);
        let jsonl = include_str!("../server_fn_coverage/testdata/coverage-sample.jsonl");
        let mut header = tar::Header::new_gnu();
        header.set_size(jsonl.len().try_into().expect("fixture length fits"));
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, "capture/otel-traces.jsonl", jsonl.as_bytes())
            .expect("add trace");
        archive
            .into_inner()
            .expect("finish archive")
            .finish()
            .expect("finish compression");
    }

    fn write_sample_inventory(dir: &Path) {
        for (vertical, idents) in [
            ("posts", &["create_post", "update_post", "get_post"][..]),
            ("tags", &["list_tags"][..]),
            ("registration", &["register"][..]),
        ] {
            let vertical_dir = dir.join(vertical);
            std::fs::create_dir_all(&vertical_dir).expect("create vertical dir");
            let source: String = idents
                .iter()
                .map(|ident| format!("#[macros::server]\npub async fn {ident}() {{}}\n"))
                .collect();
            std::fs::write(vertical_dir.join("api.rs"), source).expect("write source");
        }
    }

    fn write_empty_capture(path: &Path) {
        let file = std::fs::File::create(path).expect("create capture");
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        tar::Builder::new(encoder)
            .into_inner()
            .expect("finish archive")
            .finish()
            .expect("finish compression");
    }

    fn current_snapshot_for(capture: &Path, web_src: &Path, snapshot: &Path) {
        let inventory = inventory(web_src).expect("inventory");
        let derived = coverage_from_capture(capture, &inventory)
            .expect("fixture capture derives coverage")
            .into_snapshot();
        write_snapshot(snapshot, &derived).expect("write matching snapshot");
    }

    #[test]
    fn an_explicit_capture_with_a_matching_snapshot_verifies_cleanly() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let capture = tmp.path().join("capture-sqlite.tar.gz");
        let snapshot = tmp.path().join("server-fns.json");
        write_sample_inventory(tmp.path());
        write_sample_capture(&capture);
        current_snapshot_for(&capture, tmp.path(), &snapshot);

        assert!(regenerate_or_verify(tmp.path(), &capture, &snapshot, false).is_ok());
    }

    #[test]
    fn an_explicit_capture_rejects_a_stale_snapshot() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let capture = tmp.path().join("capture-sqlite.tar.gz");
        let snapshot = tmp.path().join("server-fns.json");
        write_sample_inventory(tmp.path());
        write_sample_capture(&capture);
        write_json(&snapshot, "stale\n");

        let error = regenerate_or_verify(tmp.path(), &capture, &snapshot, false).unwrap_err();
        let detail = error.to_string();
        assert!(detail.contains("server-fns.json"), "{detail}");
        assert!(detail.contains(REGENERATE_CMD), "{detail}");
    }

    #[test]
    fn explicit_capture_failures_are_named_fail_closed_results() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let empty = tmp.path().join("empty.tar.gz");
        let malformed = tmp.path().join("malformed.tar.gz");
        let directory = tmp.path().join("not-a-tarball");
        write_sample_inventory(tmp.path());
        write_empty_capture(&empty);
        write_json(&malformed, "not a gzip archive");
        std::fs::create_dir(&directory).expect("create directory");

        for capture in [
            tmp.path().join("missing.tar.gz"),
            empty,
            malformed,
            directory,
        ] {
            assert!(
                regenerate_or_verify(
                    tmp.path(),
                    &capture,
                    &tmp.path().join("server-fns.json"),
                    false
                )
                .is_err(),
                "{} must fail, not skip",
                capture.display()
            );
        }
    }

    // ── Assertions against the real seeding capture (spec AC2/AC11) ──────────
    //
    // The fixture is the `sqlite × chromium` capture the committed allowlist was
    // derived from, reduced by `testdata/reduce-otel-capture.mjs` to the spans the
    // extractor actually reads: one hit-chain per (span name + URI path, test)
    // pair, at most two orphan examples per key, eight non-`/api/` spans, and only
    // the handful of attributes `parse_spans`/`extract` consume. That keeps it
    // ~1.2 MiB instead of the full capture while preserving its (fn, test) hit
    // set exactly. That preservation is what AC11
    // rests on, which is why the reduction is committed and re-runnable rather than
    // described: a reader can regenerate the fixture and diff instead of taking it
    // on trust. Per-fn orphan *counts* are NOT preserved by the dedup — the
    // committed snapshot's counts come from the full capture.

    /// xtask's tests run with the crate dir as cwd, so repo-relative artifact
    /// paths have to be resolved explicitly.
    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
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
    fn seed_capture_covers_the_committed_snapshots_fns() {
        // The fixture must still underwrite the committed snapshot: every fn it
        // calls covered is present in the reduced capture too.
        let coverage = seed_coverage();
        let snapshot = read_snapshot(&repo_root().join(SNAPSHOT_PATH)).expect("snapshot parses");
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
            // Signal 1 — span name plus module. Independent of the URI, which is
            // the value under test. `span_module` owns current-then-legacy attribute
            // precedence so this contract cannot drift from extraction.
            let module = crate::server_fn_coverage::extract::span_module(span);
            if module.is_empty() {
                continue;
            }
            let relative = module.strip_prefix("web::").unwrap_or(module.as_str());
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
        // spans but dropped their module attributes would silently fall back to `uri`
        // for everything — and, per the test above, look identical while doing it.
        let inv = inventory(&repo_root().join(WEB_SRC)).expect("inventory enumerates");
        let verticals: std::collections::BTreeSet<&str> = inv
            .iter()
            .map(repository_policy::server_fns::ServerFn::vertical)
            .collect();
        let mut checked = 0;
        for span in seed_spans().iter().filter(|s| {
            verticals
                .iter()
                .any(|v| s.name.starts_with(&format!("web.{v}.")))
        }) {
            let module = crate::server_fn_coverage::extract::span_module(span);
            assert!(
                module.starts_with("web::"),
                "{} lost its web:: module attribute: {module:?}",
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
        // that silently dropped every fn would make the whole gate vacuous, so this
        // pins the narrowing here while the committed coverage snapshot keeps the
        // real-tree inventory from becoming an empty silent green.
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
    #[test]
    fn selected_immutable_output_rejects_a_capture_from_another_output() {
        fn store_capture(bytes: &str) -> std::path::PathBuf {
            let temp = tempfile::tempdir().unwrap();
            std::fs::write(
                temp.path().join("capture-sqlite-chromium-unsplit.tar.gz"),
                bytes,
            )
            .unwrap();
            let output = std::process::Command::new("nix")
                .args(["store", "add-path", "--name", "coverage-input-fixture"])
                .arg(temp.path())
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().into()
        }
        let selected = store_capture("selected");
        let other = store_capture("stale");
        let name = "capture-sqlite-chromium-unsplit.tar.gz";
        assert!(
            validate_selected_capture(&selected.join(name), &selected, "sqlite-chromium-unsplit")
                .is_ok()
        );
        let error =
            validate_selected_capture(&other.join(name), &selected, "sqlite-chromium-unsplit")
                .unwrap_err();
        assert!(error.to_string().contains("does not belong"));
        assert!(
            validate_selected_capture(
                &selected.join("missing"),
                &selected,
                "sqlite-chromium-unsplit"
            )
            .is_err()
        );
    }

    #[test]
    fn capture_authority_rejects_other_lanes_missing_outputs_and_mutable_copies() {
        let temp = tempfile::tempdir().unwrap();
        let capture = temp.path().join("capture-sqlite-chromium-unsplit.tar.gz");
        write_sample_capture(&capture);
        for (lane, output) in [
            ("postgres-chromium-unsplit", temp.path().to_owned()),
            ("sqlite-chromium-unsplit", temp.path().join("missing")),
            ("sqlite-chromium-unsplit", temp.path().to_owned()),
        ] {
            assert!(validate_selected_capture(&capture, &output, lane).is_err());
        }
    }
}
