//! Ancillary E2E checkout/input attribution, separate from the test verdict.
//!
//! Git lives on the requesting host, not in the filtered Nix guest. Only a
//! clean unchanged checkout joined to its selected output's marker can attest
//! committed inputs. Reusing an output never invents its producing runtime.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Checkout {
    head: String,
    tree: String,
    parents: Vec<String>,
    reference: Option<String>,
    clean: bool,
}

fn git_text(root: &Path, args: &[&str]) -> Result<String> {
    let operation = args.first().copied().unwrap_or("query");
    // Never export Git output: it may contain private paths or configured URLs.
    let output = crate::git::at(root)
        .args(args)
        .output()
        .map_err(|error| anyhow::anyhow!("git {operation}: spawn {:?}", error.kind()))?;
    if !output.status.success() {
        bail!("git {operation}: exit {:?}", output.status.code());
    }
    Ok(String::from_utf8(output.stdout)
        .map_err(|_| anyhow::anyhow!("git {operation}: invalid UTF-8"))?
        .trim()
        .to_owned())
}

fn raw_identity(
    head: String,
    commit: &str,
    reference: Option<String>,
    clean: bool,
) -> Result<Checkout> {
    let mut tree = None;
    let mut parents = Vec::new();
    for line in commit.lines().take_while(|line| !line.is_empty()) {
        if let Some(value) = line.strip_prefix("tree ") {
            tree = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("parent ") {
            parents.push(value.to_owned());
        }
    }
    Ok(Checkout {
        head,
        tree: tree.context("missing raw commit tree")?,
        parents,
        reference,
        clean,
    })
}

fn checkout(root: &Path) -> Result<Checkout> {
    // Freeze once: a second HEAD read must not select a different commit body.
    let head = git_text(root, &["rev-parse", "HEAD"])?;
    let commit = git_text(root, &["cat-file", "-p", &head])?;
    let output = crate::git::at(root)
        .args(["symbolic-ref", "-q", "HEAD"])
        .output()
        .map_err(|error| anyhow::anyhow!("git symbolic-ref: spawn {:?}", error.kind()))?;
    let reference = match output.status.code() {
        Some(0) => Some(
            String::from_utf8(output.stdout)
                .map_err(|_| anyhow::anyhow!("git symbolic-ref: invalid UTF-8"))?
                .trim()
                .to_owned(),
        ),
        Some(1) => None, // Detached checkouts are ordinary in CI.
        _ => bail!("git symbolic-ref: exit {:?}", output.status.code()),
    };
    let clean = git_text(root, &["status", "--porcelain", "--untracked-files=all"])?.is_empty();
    raw_identity(head, &commit, reference, clean)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

fn annotations() -> Value {
    let mut values = serde_json::Map::new();
    for key in [
        "GITHUB_RUN_ID",
        "GITHUB_RUN_ATTEMPT",
        "GITHUB_JOB",
        "GITHUB_EVENT_NAME",
        "GITHUB_REPOSITORY",
        "GITHUB_REF",
        "GITHUB_SHA",
    ] {
        if let Ok(value) = std::env::var(key) {
            values.insert(key.to_owned(), json!(value));
        }
    }
    Value::Object(values)
}

fn attestation(
    before: Option<&Checkout>,
    after: Option<&Checkout>,
    marker: Option<&Value>,
    lane: &str,
    selection: &Value,
    cleared: bool,
) -> &'static str {
    if !cleared {
        return "marker-clearing-failed";
    }
    let (Some(before), Some(after)) = (before, after) else {
        return "checkout-unavailable";
    };
    if !before.clean || !after.clean {
        return "dirty-committed-base-only";
    }
    if before != after {
        return "checkout-changed";
    }
    if selection["derivation"].as_str().is_none()
        || selection["outputs"].as_array().is_none_or(Vec::is_empty)
    {
        return "selection-unavailable";
    }
    let Some(marker) = marker else {
        return "selected-output-marker-unavailable";
    };
    if marker["schema_version"] != 1
        || marker["lane"].as_str() != Some(lane)
        || marker["revision"].as_str() != Some(before.head.as_str())
    {
        return "marker-revision-or-lane-mismatch";
    }
    "clean-revision-and-selected-output"
}

/// Keep bounded, payload-free error context while preserving the browser verdict.
fn acquisition<T>(operation: &str, result: Result<T>, errors: &mut Vec<String>) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            let detail = format!("{operation}: {error}");
            // Errors originate in the controlled operations below, not child stderr
            // or JSON payloads. A failed warning sink is ancillary too.
            let _ = writeln!(
                std::io::stderr(),
                "xtask: warning: xtask.e2e.provenance: {detail}"
            );
            if errors.len() < 16 {
                errors.push(detail.chars().take(256).collect());
            }
            None
        }
    }
}

fn read_json(path: &Path) -> Result<Option<Value>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => bail!("read {:?}", error.kind()),
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        anyhow::anyhow!(
            "decode {:?} at line {} column {}",
            error.classify(),
            error.line(),
            error.column()
        )
    })
}

/// Own a per-lane sidecar through early returns as well as normal completion.
pub(crate) struct Request {
    root: PathBuf,
    lane: String,
    before: Option<Checkout>,
    started_ms: u128,
    selection: Value,
    cleared: bool,
    succeeded: Option<bool>,
    acquisition_errors: Vec<String>,
}

impl Request {
    pub(crate) fn start(root: &Path, lane: &str) -> Self {
        let mut acquisition_errors = Vec::new();
        let canonical = acquisition(
            "checkout-root",
            root.canonicalize()
                .map_err(|error| anyhow::anyhow!("canonicalize {:?}", error.kind())),
            &mut acquisition_errors,
        );
        let root = canonical.unwrap_or_else(|| root.to_owned());
        let mut request = Self {
            before: acquisition("checkout-before", checkout(&root), &mut acquisition_errors),
            root,
            lane: lane.to_owned(),
            started_ms: now_ms(),
            selection: Value::Null,
            cleared: true,
            succeeded: None,
            acquisition_errors,
        };
        let marker = request
            .directory()
            .join(format!("producer-input-{lane}.json"));
        for (operation, path) in [
            ("clear-marker", marker),
            ("clear-checkout-sidecar", request.path()),
        ] {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    request.cleared = false;
                    acquisition::<()>(
                        operation,
                        Err(anyhow::anyhow!("remove {:?}", error.kind())),
                        &mut request.acquisition_errors,
                    );
                }
            }
        }
        request.persist(None);
        request
    }

    fn directory(&self) -> PathBuf {
        self.root
            .join(".xtask/diagnostics")
            .join(format!("e2e-{}", self.lane))
    }

    pub(crate) fn path(&self) -> PathBuf {
        self.directory()
            .join(format!("producer-checkout-{}.json", self.lane))
    }

    pub(crate) fn selected(&mut self, selection: Value) {
        self.selection = selection;
        // Persist before spawn: failed builds have no ordinary successful NixReport.
        self.persist(None);
    }

    pub(crate) fn completed(&mut self, succeeded: bool) {
        self.succeeded = Some(succeeded);
    }

    fn persist(&mut self, after: Option<Checkout>) {
        // A write failure cannot be reported through its own broken sidecar.
        let mut errors = Vec::new();
        acquisition("persist-sidecar", self.write(after), &mut errors);
    }

    fn write(&mut self, after: Option<Checkout>) -> Result<()> {
        // Read ONLY the preselected immutable output, never a recovered/stale
        // destination or a heuristically discovered failed build directory.
        let output = self.selection["outputs"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(Value::as_str);
        let marker_path =
            output.map(|p| Path::new(p).join(format!("producer-input-{}.json", self.lane)));
        let mut errors = self.acquisition_errors.clone();
        let marker_read = marker_path
            .as_ref()
            .and_then(|path| acquisition("selected-marker", read_json(path), &mut errors));
        let marker_status = match &marker_read {
            Some(Some(_)) => "present",
            Some(None) => "missing",
            None if marker_path.is_none() => "not-selected",
            None => "unavailable",
        };
        let marker = marker_read.flatten();
        let reason = attestation(
            self.before.as_ref(),
            after.as_ref(),
            marker.as_ref(),
            &self.lane,
            &self.selection,
            self.cleared,
        );
        let reused = self.selection["outputs_valid_before"]
            .as_array()
            .is_some_and(|a| !a.is_empty() && a.iter().all(|v| v == true));
        let report = output
            .and_then(|p| {
                acquisition(
                    "selected-report",
                    read_json(&Path::new(p).join(format!("playwright-report-{}.json", self.lane))),
                    &mut errors,
                )
            })
            .flatten();
        let report_started = report.and_then(|value| {
            acquisition(
                "selected-report-start",
                (|| {
                    let raw = value["stats"]["startTime"]
                        .as_str()
                        .context("missing stats.startTime")?;
                    raw.parse::<jiff::Timestamp>()
                        .map_err(|_| anyhow::anyhow!("decode invalid timestamp"))
                })(),
                &mut errors,
            )
        });
        let report_during_request = report_started.is_some_and(|t| {
            t.as_millisecond() >= self.started_ms as i64 && t.as_millisecond() <= now_ms() as i64
        });
        let execution = if reused {
            "reused-output-original-runtime-unknown"
        } else if self.succeeded.is_some() && report_during_request {
            "selected-report-started-during-request"
        } else {
            "execution-not-attested"
        };
        // Keep early acquisition failures even if a later read succeeds or is absent.
        self.acquisition_errors.clone_from(&errors);
        let value = json!({
            "schema_version": 1, "lane": self.lane, "request_started_unix_ms": self.started_ms,
            "recorded_unix_ms": now_ms(), "requesting_checkout": self.before, "checkout_after": after,
            "ci_request_annotations": annotations(), "selection_before_build": self.selection,
            "selected_marker_path": marker_path, "selected_marker_status": marker_status,
            "acquisition_errors": errors, "guest_input": marker, "build_succeeded": self.succeeded,
            "input_attestation": reason, "execution_observation": execution,
            "original_producing_runtime": "unknown", "marker_destination_cleared": self.cleared,
        });
        std::fs::create_dir_all(self.directory())
            .map_err(|error| anyhow::anyhow!("create directory {:?}", error.kind()))?;
        let bytes = serde_json::to_vec_pretty(&value)
            .map_err(|error| anyhow::anyhow!("encode {:?}", error.classify()))?;
        std::fs::write(self.path(), bytes)
            .map_err(|error| anyhow::anyhow!("write {:?}", error.kind()))?;
        Ok(())
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        let after = acquisition(
            "checkout-after",
            checkout(&self.root),
            &mut self.acquisition_errors,
        );
        self.persist(after);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    #[test]
    fn e2e_provenance_acquisition_failures_are_context_not_absence() {
        let root = tempfile::tempdir().unwrap();
        let path;
        {
            let mut request = Request::start(root.path(), "test");
            path = request.path();
            request.completed(true);
        }
        let value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(value["build_succeeded"], true);
        assert_eq!(value["input_attestation"], "checkout-unavailable");
        let errors = value["acquisition_errors"].to_string();
        assert!(errors.contains("checkout-before"), "{errors}");
        assert!(errors.contains("git rev-parse"), "{errors}");
        assert!(errors.contains("checkout-after"), "{errors}");
    }

    #[test]
    fn e2e_provenance_corrupt_marker_is_distinct_from_missing() {
        let root = test_support::temp_repo("provenance", "corrupt");
        test_support::commit(&root, ".gitignore", ".xtask/\n");
        let output = tempfile::tempdir().unwrap();
        let marker = output.path().join("producer-input-test.json");
        let mut request = Request::start(&root, "test");
        request.selected(json!({"derivation":"selected.drv","outputs":[output.path()]}));
        let missing: Value =
            serde_json::from_slice(&std::fs::read(request.path()).unwrap()).unwrap();
        assert_eq!(missing["selected_marker_status"], "missing");
        std::fs::write(&marker, b"private-sentinel not JSON").unwrap();
        request.completed(true);
        let path = request.path();
        drop(request);
        let corrupt: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(corrupt["selected_marker_status"], "unavailable");
        assert!(
            corrupt["acquisition_errors"]
                .to_string()
                .contains("selected-marker: decode")
        );
        assert!(!corrupt.to_string().contains("private-sentinel"));
        assert_eq!(corrupt["build_succeeded"], true);
        assert_eq!(
            corrupt["input_attestation"],
            "selected-output-marker-unavailable"
        );
    }

    #[test]
    fn e2e_provenance_unreadable_marker_and_corrupt_report_keep_context() {
        let root = test_support::temp_repo("provenance", "unreadable");
        test_support::commit(&root, ".gitignore", ".xtask/\n");
        let output = tempfile::tempdir().unwrap();
        std::fs::create_dir(output.path().join("producer-input-test.json")).unwrap();
        std::fs::write(
            output.path().join("playwright-report-test.json"),
            b"private-report-sentinel",
        )
        .unwrap();
        let path;
        {
            let mut request = Request::start(&root, "test");
            path = request.path();
            request.selected(json!({"derivation":"selected.drv","outputs":[output.path()]}));
            request.completed(true);
        }
        let value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(value["selected_marker_status"], "unavailable");
        let errors = value["acquisition_errors"].to_string();
        assert!(errors.contains("selected-marker: read"), "{errors}");
        assert!(errors.contains("selected-report: decode"), "{errors}");
        assert!(!value.to_string().contains("private-report-sentinel"));
        assert_eq!(value["build_succeeded"], true);
        assert_eq!(value["execution_observation"], "execution-not-attested");
    }

    #[test]
    fn e2e_provenance_freezes_raw_parents_including_shallow_checkout() {
        let root = test_support::temp_repo("provenance", "raw");
        test_support::commit(&root, "a", "one");
        let parent = git_text(&root, &["rev-parse", "HEAD"]).unwrap();
        test_support::commit(&root, "a", "two");
        let second = git_text(&root, &["rev-parse", "HEAD"]).unwrap();
        let tree = git_text(&root, &["rev-parse", "HEAD^{tree}"]).unwrap();
        let head = git_text(
            &root,
            &[
                "commit-tree",
                &tree,
                "-p",
                &parent,
                "-p",
                &second,
                "-m",
                "synthetic merge",
            ],
        )
        .unwrap();
        test_support::git_ok(&root, &["checkout", "--detach", "-q", &head]);
        std::fs::write(root.join(".git/shallow"), format!("{head}\n")).unwrap();
        let record = checkout(&root).unwrap();
        assert_eq!(record.parents, [parent, second]);
        assert_eq!(record.reference, None);
        assert!(record.clean);
        std::fs::write(root.join("a"), "dirty").unwrap();
        assert!(!checkout(&root).unwrap().clean);
    }

    #[test]
    fn e2e_provenance_keeps_all_merge_parents_not_body_lookalikes() {
        let value = raw_identity(
            "frozen".into(),
            "tree tree-id\nparent first\nparent second\nauthor ignored\n\nparent not-a-header",
            None,
            true,
        )
        .unwrap();
        assert_eq!(value.tree, "tree-id");
        assert_eq!(value.parents, ["first", "second"]);
        assert_eq!(value.head, "frozen");
        assert!(raw_identity("head".into(), "parent a", None, true).is_err());
    }

    #[test]
    fn e2e_provenance_requires_clean_matching_selected_origin() {
        let c = Checkout {
            head: "head".into(),
            tree: "tree".into(),
            parents: vec!["parent".into()],
            reference: None,
            clean: true,
        };
        let selected =
            json!({"derivation":"/nix/store/example.drv", "outputs":["/nix/store/example"]});
        let marker = json!({"schema_version":1,"lane":"test","revision":"head"});
        assert_eq!(
            attestation(Some(&c), Some(&c), Some(&marker), "test", &selected, true),
            "clean-revision-and-selected-output"
        );
        assert_eq!(
            attestation(Some(&c), Some(&c), None, "test", &selected, true),
            "selected-output-marker-unavailable"
        );
        assert_eq!(
            attestation(Some(&c), Some(&c), Some(&marker), "other", &selected, true),
            "marker-revision-or-lane-mismatch"
        );
        assert_eq!(
            attestation(Some(&c), Some(&c), Some(&marker), "test", &selected, false),
            "marker-clearing-failed"
        );
        let mut dirty = c.clone();
        dirty.clean = false;
        assert_eq!(
            attestation(
                Some(&dirty),
                Some(&dirty),
                Some(&marker),
                "test",
                &selected,
                true
            ),
            "dirty-committed-base-only"
        );
        let mut changed = c.clone();
        changed.tree = "different".into();
        assert_eq!(
            attestation(
                Some(&c),
                Some(&changed),
                Some(&marker),
                "test",
                &selected,
                true
            ),
            "checkout-changed"
        );
        assert_eq!(
            attestation(
                Some(&c),
                Some(&c),
                Some(&marker),
                "test",
                &Value::Null,
                true
            ),
            "selection-unavailable"
        );
    }

    #[test]
    fn e2e_provenance_failed_marker_clearing_prevents_attestation() {
        let root = test_support::temp_repo("provenance", "clearing");
        test_support::commit(&root, ".gitignore", ".xtask/\n");
        let marker = root.join(".xtask/diagnostics/e2e-test/producer-input-test.json");
        std::fs::create_dir_all(&marker).unwrap();
        let path;
        {
            let request = Request::start(&root, "test");
            path = request.path();
        }
        let value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(value["input_attestation"], "marker-clearing-failed");
        assert_eq!(value["marker_destination_cleared"], false);
    }

    #[test]
    fn e2e_provenance_retains_preselection_on_failed_spawn_and_never_claims_cached_runtime() {
        let root = test_support::temp_repo("provenance", "failed");
        test_support::commit(&root, ".gitignore", ".xtask/\n");
        let output = tempfile::tempdir().unwrap();
        let head = checkout(&root).unwrap().head;
        std::fs::write(
            output.path().join("producer-input-test.json"),
            serde_json::to_vec(&json!({"schema_version":1,"lane":"test","revision":head})).unwrap(),
        )
        .unwrap();
        let path;
        {
            let mut request = Request::start(&root, "test");
            path = request.path();
            request.selected(json!({"derivation":"selected.drv","outputs":[output.path()],"outputs_valid_before":[true]}));
            request.completed(false);
        }
        let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(v["selection_before_build"]["derivation"], "selected.drv");
        assert_eq!(v["build_succeeded"], false);
        assert_eq!(
            v["execution_observation"],
            "reused-output-original-runtime-unknown"
        );
        assert_eq!(v["input_attestation"], "clean-revision-and-selected-output");
    }
}
