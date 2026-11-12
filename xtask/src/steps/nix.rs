use std::{fs, io, path::Path, process::Command};

use crate::result::{CommandResult, StepResult};
use anyhow::{Context, Result, bail};

/// The flake checks are Linux-only (`optionalAttrs isLinux` in `nix/checks.nix`);
/// the project's CI host is x86_64-linux.
const SYSTEM: &str = "x86_64-linux";

/// Lift and consume the existing producer's fixed artifact set, without building it.
pub(crate) fn consume_elisp_coverage(result: &mut CommandResult) {
    let artifacts = Path::new(".xtask/gcroots/elisp-coverage-producer/elisp-coverage");
    result.push(lift_elisp_coverage_artifacts(
        artifacts,
        Path::new(".xtask/diagnostics/elisp-coverage"),
    ));

    let step = match crate::elisp_coverage::consume(Path::new("."), artifacts) {
        Ok(report) => StepResult::ok("elisp-coverage").detail(format!(
            "covered {} point(s); ignored {} point(s)",
            report.covered_points, report.ignored_points
        )),
        Err(error) => StepResult::fail("elisp-coverage").detail(elisp_coverage_detail(error)),
    };
    result.push(step);
}

fn lift_elisp_coverage_artifacts(source: &Path, destination: &Path) -> StepResult {
    let names = ["lcov.info", "summary.txt", "status.json"];
    if let Err(error) = fs::create_dir_all(destination) {
        return StepResult::fail("elisp-coverage-artifacts")
            .detail(format!("creating {}: {error}", destination.display()));
    }
    let mut failures = Vec::new();
    for name in names {
        let target = destination.join(name);
        if let Err(error) = fs::remove_file(&target)
            && error.kind() != io::ErrorKind::NotFound
        {
            failures.push(format!("{name}: removing prior artifact: {error}"));
            continue;
        }
        if let Err(error) = fs::copy(source.join(name), target) {
            failures.push(format!("{name}: {error}"));
        }
    }
    if failures.is_empty() {
        StepResult::ok("elisp-coverage-artifacts")
            .detail("lifted lcov.info, summary.txt, status.json")
    } else {
        StepResult::fail("elisp-coverage-artifacts").detail(failures.join("; "))
    }
}

fn elisp_coverage_detail(error: crate::elisp_coverage::CoverageError) -> String {
    use crate::elisp_coverage::CoverageError;

    match error {
        CoverageError::Artifact { path, message } | CoverageError::Source { path, message } => {
            format!("{}: {message}", path.display())
        }
        CoverageError::Status { message }
        | CoverageError::Census { message }
        | CoverageError::Lcov { message } => message,
        CoverageError::Verdict { failures } => failures
            .into_iter()
            .map(|failure| format!("{}:{} {}", failure.path, failure.line, failure.message))
            .collect::<Vec<_>>()
            .join("; "),
    }
}

/// Run `nix eval --raw --accept-flake-config <installable>`, optionally in a
/// supplied flake directory.
fn nix_eval_raw(dir: Option<&Path>, installable: &str) -> Result<String> {
    let mut cmd = Command::new("nix");
    if let Some(dir) = dir {
        cmd.current_dir(dir);
    }
    let out = cmd
        .args(["eval", "--raw", "--accept-flake-config", installable])
        .output()
        .with_context(|| format!("spawning `nix eval {installable}`"))?;
    if !out.status.success() {
        bail!(
            "`nix eval {installable}` failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let path = String::from_utf8(out.stdout)
        .with_context(|| format!("`nix eval {installable}` output was not UTF-8"))?
        .trim()
        .to_owned();
    if path.is_empty() {
        bail!("`nix eval {installable}` returned an empty path");
    }
    Ok(path)
}

/// Evaluate the probe-only identity of the filtered coverage source.
pub(crate) fn eval_coverage_source_probe_drvpath(flake_dir: &Path) -> Result<String> {
    nix_eval_raw(
        Some(flake_dir),
        &format!(".#checks.{SYSTEM}.coverage-source-probe.drvPath"),
    )
}

/// Realize the coverage producer and return its output directory.
pub(crate) fn build_coverage_out_path(flake_dir: &Path) -> Result<String> {
    let installable = format!(".#checks.{SYSTEM}.coverage");
    let out = Command::new("nix")
        .current_dir(flake_dir)
        .args([
            "build",
            "--no-link",
            "--print-out-paths",
            "--accept-flake-config",
            &installable,
        ])
        .output()
        .with_context(|| format!("spawning `nix build {installable}`"))?;
    if !out.status.success() {
        bail!(
            "`nix build {installable}` failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let path = String::from_utf8(out.stdout)
        .with_context(|| format!("`nix build {installable}` output was not UTF-8"))?
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    if path.is_empty() {
        bail!("`nix build {installable}` returned an empty output path");
    }
    Ok(path)
}

/// Named derivation identities guarded by `nix probe-source`.
pub(crate) struct SourceProbeDrvPaths {
    pub(crate) static_docs: String,
    pub(crate) static_code: String,
    pub(crate) static_elisp: String,
    pub(crate) static_e2e_census: String,
    pub(crate) site: String,
    pub(crate) wasm_tests: String,
    pub(crate) xtask: String,
    pub(crate) devtool: String,
    pub(crate) tooling_dependencies: String,
    pub(crate) csr_debug_bundle: String,
}

/// Evaluate the derivation identities guarded by `nix probe-source`.
/// This deliberately performs only serial `nix eval` calls: the probe mutates
/// one isolated worktree between arms, so concurrent evaluation would obscure
/// which tracked perturbation produced a failed identity.
pub(crate) fn eval_source_probe_drvpaths(flake_dir: &Path) -> Result<SourceProbeDrvPaths> {
    Ok(SourceProbeDrvPaths {
        static_docs: nix_eval_raw(Some(flake_dir), ".#checks.x86_64-linux.static-docs.drvPath")?,
        static_code: nix_eval_raw(Some(flake_dir), ".#checks.x86_64-linux.static-code.drvPath")?,
        static_elisp: nix_eval_raw(
            Some(flake_dir),
            ".#checks.x86_64-linux.static-elisp.drvPath",
        )?,
        static_e2e_census: nix_eval_raw(
            Some(flake_dir),
            ".#checks.x86_64-linux.static-e2e-census.drvPath",
        )?,
        site: nix_eval_raw(Some(flake_dir), ".#site.drvPath")?,
        wasm_tests: nix_eval_raw(Some(flake_dir), ".#checks.x86_64-linux.wasm-tests.drvPath")?,
        xtask: nix_eval_raw(Some(flake_dir), ".#xtask.drvPath")?,
        devtool: nix_eval_raw(Some(flake_dir), ".#devtool.drvPath")?,
        tooling_dependencies: nix_eval_raw(
            Some(flake_dir),
            ".#devtool.toolingDependencies.drvPath",
        )?,
        csr_debug_bundle: nix_eval_raw(Some(flake_dir), ".#csrDebugBundle.drvPath")?,
    })
}

#[cfg(test)]
mod tests {
    use super::lift_elisp_coverage_artifacts;
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};
    #[test]
    fn elisp_coverage_artifact_lift_replaces_prior_read_only_outputs() {
        // Nix outputs are read-only; repeated validate runs must replace the
        // prior lifted diagnostics rather than failing on their mode bits.
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        for name in ["lcov.info", "summary.txt", "status.json"] {
            std::fs::write(source.path().join(name), format!("fresh-{name}")).unwrap();
            let existing = destination.path().join(name);
            std::fs::write(&existing, "stale").unwrap();
            let mut permissions = std::fs::metadata(&existing).unwrap().permissions();
            permissions.set_mode(0o444);
            std::fs::set_permissions(&existing, permissions).unwrap();
        }

        let step = lift_elisp_coverage_artifacts(source.path(), destination.path());

        assert!(step.ok, "{:?}", step.detail);
        for name in ["lcov.info", "summary.txt", "status.json"] {
            assert_eq!(
                std::fs::read_to_string(destination.path().join(name)).unwrap(),
                format!("fresh-{name}")
            );
        }
    }

    #[test]
    fn e2e_capture_host_transport_behavior() {
        let output = std::process::Command::new("python3")
            .args([
                "-B",
                "-m",
                "unittest",
                "discover",
                "-s",
                "nix",
                "-p",
                "test_e2e_capture.py",
            ])
            .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join(".."))
            .output()
            .expect("run host capture transport tests");
        assert!(
            output.status.success(),
            "capture transport tests failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn evaluate_e2e_expression(result: &str) -> std::process::Output {
        // Keep the real combo -> check -> execution path, replacing only NixOS
        // realization and store inputs so this remains a primitive evaluation.
        let checks_path =
            serde_json::to_string(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../nix/checks.nix"))
                .expect("checks path");
        let expression = r#"
          let
            lib = rec {
              inherit (builtins) listToAttrs genList concatLists concatStringsSep;
              concatStrings = concatStringsSep "";
              concatMap = f: xs: concatLists (map f xs);
              concatMapStringsSep = sep: f: xs: concatStringsSep sep (map f xs);
              unique = xs: builtins.foldl' (acc: x: if builtins.elem x acc then acc else acc ++ [ x ]) [] xs;
              optionalString = condition: value: if condition then value else "";
              optionalAttrs = condition: value: if condition then value else {};
              removeSuffix = suffix: value: builtins.substring 0 (builtins.stringLength value - builtins.stringLength suffix) value;
            };
            outputs = import (builtins.toPath CHECKS_PATH) {
              self = {};
              system = "x86_64-linux";
              nixpkgs = {};
              nixosInternals = {};
              packageInternals = {
                visualFontConfig = "/primitive/fonts";
                jaunderBin = "/primitive/jaunder";
                e2ePackage = "/primitive/e2e";
              };
              pkgs = {
                inherit lib;
                stdenv.hostPlatform.isLinux = true;
                testers.nixosTest = args: args // { overrideTestDerivation = _: args; };
                writeText = _: _: "/primitive/input";
                nodejs = "/primitive/node";
                playwright-driver.browsers = "/primitive/browsers";
              };
            };
          in RESULT
        "#
        .replace("CHECKS_PATH", &checks_path)
        .replace("RESULT", result);
        // Materialize interpolated source files (not derivation outputs) so the
        // executed Python imports the same capture module as the VM driver.
        std::process::Command::new("nix-instantiate")
            .args([
                "--eval",
                "--read-write-mode",
                "--strict",
                "--json",
                "--expr",
                &expression,
            ])
            .output()
            .expect("primitive Nix lane evaluation")
    }

    fn evaluated_e2e_lanes() -> serde_json::Value {
        let output = evaluate_e2e_expression(
            r#"builtins.listToAttrs (map (name: {
              inherit name;
              value = { inherit (outputs.checks.${name}) globalTimeout testScript; };
            }) (builtins.filter (name: builtins.match "e2e-.*" name != null)
              (builtins.attrNames outputs.checks)))"#,
        );
        assert!(
            output.status.success(),
            "lane evaluation: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("evaluated lanes")
    }

    #[test]
    fn e2e_vm_deadline_accounts_for_the_full_lifecycle() {
        let lanes = evaluated_e2e_lanes();
        let lanes = lanes.as_object().expect("lane map");
        assert_eq!(lanes.len(), 8);
        for (name, lane) in lanes {
            let (suite, deadline) = if name == "e2e-postgres-chromium-unsplit" {
                (1800, 2220)
            } else {
                (1500, 1920)
            };
            assert_eq!(lane["globalTimeout"], deadline, "{name}");
            let script = lane["testScript"].as_str().expect("VM test script");
            let syntax = std::process::Command::new("python3")
                .args([
                    "-B",
                    "-c",
                    r#"
import ast
import json
import sys

module = ast.parse(sys.argv[1])
timeouts = []
for call in ast.walk(module):
    if not isinstance(call, ast.Call) or not call.args:
        continue
    if not isinstance(call.func, ast.Attribute) or call.func.attr != "execute":
        continue
    if not any(isinstance(node, ast.Constant) and isinstance(node.value, str)
               and "node_modules/.bin/playwright test" in node.value
               for node in ast.walk(call.args[0])):
        continue
    timeout = next(keyword.value for keyword in call.keywords if keyword.arg == "timeout")
    timeouts.append(ast.literal_eval(timeout))
print(json.dumps(sorted(timeouts)))
"#,
                    script,
                ])
                .output()
                .expect("parse evaluated VM script");
            assert!(
                syntax.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&syntax.stderr)
            );
            let timeouts: Vec<u64> =
                serde_json::from_slice(&syntax.stdout).expect("browser execution timeouts");
            let expected = if name.contains("firefox") {
                vec![1500, suite]
            } else {
                vec![suite]
            };
            assert_eq!(timeouts, expected, "{name}");
            assert!(timeouts.iter().all(|timeout| *timeout < deadline), "{name}");
        }
        for (producer, suite, deadline, accepted) in [
            ("null", 1500, 1920, true),
            ("null", 1500, 1919, false),
            ("null", 1500, 1680, false),
            ("null", 1800, 2220, true),
            ("null", 1800, 2219, false),
            ("{}", 1500, 6000, true),
        ] {
            let expression = format!(
                r#"(outputs.internals.mkE2eCheck {{
                  backend = "sqlite";
                  checkName = "deadline-boundary";
                  browser = "chromium";
                  traceId = "";
                  traceParent = "";
                  producer = {producer};
                  lane = if {producer} == null then {{}} else null;
                  playwrightTimeout = {suite};
                  vmGlobalTimeout = {deadline};
                }}).globalTimeout"#
            );
            let output = evaluate_e2e_expression(&expression);
            assert_eq!(
                output.status.success(),
                accepted,
                "producer={producer} suite={suite} deadline={deadline}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            if accepted {
                let actual: u64 =
                    serde_json::from_slice(&output.stdout).expect("evaluated VM deadline");
                assert_eq!(actual, deadline);
            }
        }
    }

    #[test]
    fn e2e_panic_gate_verifies_the_lane_qualified_journal() {
        let checks = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("nix")
                .join("checks.nix"),
        )
        .expect("nix/checks.nix");
        assert!(checks.contains("--server-log /tmp/jaunder-journal-${identity}.log"));
        assert!(!checks.contains("--server-log /tmp/jaunder-journal-${backend}.log"));
    }

    #[test]
    fn e2e_result_capture_helper_exercises_source_transport_and_failure_precedence() {
        let lanes = evaluated_e2e_lanes();
        let helper = lanes["e2e-sqlite-chromium-unsplit"]["testScript"]
            .as_str()
            .expect("evaluated VM script");
        let node_binary = std::process::Command::new("which")
            .arg("node")
            .output()
            .expect("locate the pinned Node runtime");
        assert!(node_binary.status.success(), "Node runtime is required");
        let node_binary = String::from_utf8(node_binary.stdout).unwrap();
        let helper = helper.replace("/primitive/node/bin/node", node_binary.trim());
        let script = r#"
import ast
import json
import os
import shlex
import shutil
import subprocess
import tempfile

namespace = {"json": json, "shlex": shlex}
module = ast.parse(__HELPER__)
exec(compile(ast.Module(body=[node for node in module.body if isinstance(node, ast.FunctionDef)], type_ignores=[]), "<evaluated VM helpers>", "exec"), namespace)
capture = namespace["capture_result_file"]
assert_capture_outcome = namespace["assert_capture_outcome"]

class Machine:
  def __init__(self, root, fail_inspection=False, fail_stage=False, fail_output=False, empty_inspection_failure=False):
    self.root = root
    self.output = os.path.join(root, "retained")
    os.makedirs(self.output, exist_ok=True)
    self.fail_inspection = fail_inspection
    self.fail_stage = fail_stage
    self.fail_output = fail_output
    self.empty_inspection_failure = empty_inspection_failure
  def execute(self, command):
    if " -e " in command and self.fail_inspection:
      return (2, "injected inspect failure")
    if " -e " in command and self.empty_inspection_failure:
      return (127, "")
    if command.startswith("cp -- ") and self.fail_stage:
      return (1, "injected stage failure")
    result = subprocess.run(command, shell=True, capture_output=True, text=True)
    return (result.returncode, result.stdout if result.returncode == 0 else result.stderr)
  def copy_from_machine(self, path, target):
    if self.fail_output:
      raise OSError("injected output copy failure")
    shutil.copyfile(path, os.path.join(self.output, os.path.basename(path)))

with tempfile.TemporaryDirectory() as root:
  def paths(name, payload):
    source = os.path.join(root, name + ".source")
    guest = os.path.join(root, name + ".guest")
    with open(source, "wb") as stream:
      stream.write(payload)
    return source, guest

  source, guest = paths("empty-ok", b"")
  errors = []
  result = capture(Machine(root), "sqlite-firefox-ordinary-1-of-2", "results.json", source, guest, errors)
  assert result["source"]["condition"] == "empty"
  assert result["source_to_guest"]["outcome"] == "copied"
  assert result["guest_to_output"]["outcome"] == "copied", result
  assert os.path.getsize(os.path.join(root, "retained", os.path.basename(guest))) == 0
  assert not errors

  source, guest = paths("empty-stage-fail", b"")
  errors = []
  result = capture(Machine(root, fail_stage=True), "sqlite-chromium-unsplit", "results.json", source, guest, errors)
  assert result["source"]["condition"] == "empty"
  assert result["source_to_guest"]["outcome"] == "copy-error"
  assert result["guest_to_output"]["outcome"] == "not-applicable"
  assert "lane sqlite-chromium-unsplit file results.json operation source-to-guest copy" in errors[0]

  malformed = b"{\"partial\":"
  source, guest = paths("malformed", malformed)
  errors = []
  result = capture(Machine(root), "sqlite-firefox-ordinary-1-of-2", "results.json", source, guest, errors)
  assert result["source"]["condition"] == "nonempty"
  assert open(os.path.join(root, "retained", os.path.basename(guest)), "rb").read() == malformed

  missing = os.path.join(root, "does-not-exist")
  result = capture(Machine(root), "sqlite-firefox-ordinary-1-of-2", "results.json", missing, guest, [])
  assert result["source"]["condition"] == "missing"
  assert result["source_to_guest"]["outcome"] == "not-applicable"
  required_errors = []
  capture(Machine(root), "sqlite-firefox-ordinary-1-of-2", "e2e-expected-census.json", missing, guest, required_errors, required=True)
  assert "required source is missing" in required_errors[0]

  source, guest = paths("empty-output-fail", b"")
  errors = []
  result = capture(Machine(root, fail_output=True), "sqlite-firefox-ordinary-1-of-2", "results.json", source, guest, errors)
  assert result["source"]["condition"] == "empty"
  assert result["guest_to_output"]["outcome"] == "copy-error"

  source, guest = paths("inspection-error", b"data")
  errors = []
  result = capture(Machine(root, fail_inspection=True), "sqlite-firefox-ordinary-1-of-2", "results.json", source, guest, errors)
  assert result["source"]["condition"] == "inspection-error"
  assert "operation source inspection" in errors[0]
  errors = []
  result = capture(Machine(root, empty_inspection_failure=True), "sqlite-firefox-ordinary-1-of-2", "results.json", source, guest, errors)
  assert result["source"]["condition"] == "inspection-error"
  assert "exited with status 127" in result["source"]["detail"], result

  source, guest = paths("output-fail", b"{\"ok\":true}")
  errors = []
  result = capture(Machine(root, fail_output=True), "sqlite-firefox-ordinary-1-of-2", "results.json", source, guest, errors)
  assert result["source_to_guest"]["outcome"] == "copied"
  assert result["guest_to_output"]["outcome"] == "copy-error"
  assert "operation guest-to-output copy" in errors[0]

  try:
    assert_capture_outcome(9, ["ancillary lift failed"], "sqlite-chromium-unsplit")
  except AssertionError as error:
    assert "Playwright failed (exit 9)" in str(error)
    assert "capture retention failed" not in str(error)
  else:
    raise AssertionError("Playwright failure was lost")
  try:
    assert_capture_outcome(0, ["required lift failed"], "sqlite-chromium-unsplit")
  except AssertionError as error:
    assert "capture retention failed" in str(error)
  else:
    raise AssertionError("successful Playwright hid required lift failure")
print("actual result-capture helper cases passed")
"#
        .replace("__HELPER__", &format!("{helper:?}"));
        let output = std::process::Command::new("python3")
            .args(["-B", "-c", &script])
            .output()
            .expect("launch capture helper harness");
        assert!(
            output.status.success(),
            "capture helper harness failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains("actual result-capture helper cases passed")
        );
    }

    #[test]
    fn e2e_vm_routes_result_files_through_capture_receipt_before_verdict() {
        let checks =
            fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../nix/checks.nix"))
                .expect("nix/checks.nix");
        let result_helper = checks
            .find("# BEGIN result capture helper")
            .expect("actual capture helper");
        let census_capture = result_helper
            + checks[result_helper..]
                .find("/tmp/e2e/test-results/e2e-expected-census.json")
                .expect("configured census source");
        let gate_start = checks
            .find("gate_started_at = time.monotonic()")
            .expect("Playwright starts");
        let report_capture = gate_start
            + checks[gate_start..]
                .find("/tmp/e2e/test-results/results.json")
                .expect("Playwright report capture");
        let duration_capture = report_capture
            + checks[report_capture..]
                .find("/tmp/e2e/test-results/duration-budget-manifest.json")
                .expect("duration report capture");
        let assertion = gate_start
            + checks[gate_start..]
                .find("assert_capture_outcome(pw_status, retention_errors")
                .expect("Playwright/capture verdict policy");
        assert!(result_helper < census_capture && census_capture < gate_start);
        assert!(gate_start < report_capture && report_capture < duration_capture);
        assert!(duration_capture < assertion);
        assert!(checks.contains("\"result_captures\": e2e_result_captures"));
        assert!(checks.contains("\"condition\": \"not-applicable\""));
        assert!(checks.contains("/tmp/playwright-report-${identity}.json"));
        assert!(checks.contains("/tmp/duration-budget-manifest-${identity}.json"));
        assert!(checks.contains("/tmp/e2e-census-${identity}.json"));
        assert!(checks.contains("retention_errors.extend(retain_e2e_capture())"));
        assert!(checks.contains("assert panic_status == 0"));
    }

    fn run_seed_timeout_harness(timed_out_command: &str, identity: &str) -> serde_json::Value {
        let lanes = evaluated_e2e_lanes();
        let helper = lanes[format!("e2e-{identity}")]["testScript"]
            .as_str()
            .expect("evaluated VM script");
        let script = format!(
            r#"
import ast
import contextlib
import json
from pathlib import Path
import sys
import tarfile
import tempfile

temporary = tempfile.TemporaryDirectory()

class FakeMachine:
  def __init__(self):
    self.actions = []
    self.state_dir = Path(temporary.name) / "vm-state-machine"
    self.out_dir = Path(temporary.name) / "output"
    (self.state_dir / "xchg").mkdir(parents=True)
    self.out_dir.mkdir()
    (self.state_dir / "xchg" / "diag.log").write_bytes(b"abc")

  def execute(self, command, timeout=None):
    self.actions.append(["execute", command, timeout])
    if command == {timed_out_command:?}:
      return (124, "")
    if "sha256sum" in command:
      return (0, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  /var/lib/jaunder/capture/diag.log\\n")
    if "-printf" in command:
      return (0, "3 /var/lib/jaunder/capture/diag.log\\n")
    return (0, "")

  def copy_from_machine(self, path, target):
    self.actions.append(["copy_from_machine", path, target])

  def succeed(self, command):
    self.actions.append(["succeed", command])
    return ""

  def wait_for_unit(self, unit, timeout=None):
    self.actions.append(["wait_for_unit", unit, timeout])

  def wait_for_open_port(self, port, timeout=None):
    self.actions.append(["wait_for_open_port", port, timeout])

machine = FakeMachine()
module = ast.parse({helper:?})
exec(compile(ast.Module(body=[node for node in module.body if isinstance(node, ast.FunctionDef)], type_ignores=[]), "<evaluated VM helpers>", "exec"))
try:
  with contextlib.redirect_stdout(sys.stderr):
    assert_seed_storage_spans()
except AssertionError as error:
  with tarfile.open(machine.out_dir / "capture-{identity}.tar.gz") as archive:
    payload = archive.extractfile("capture/diag.log").read().decode()
  print(json.dumps({{"error": str(error), "actions": machine.actions, "capture_bytes": payload}}))
else:
  raise AssertionError("seed timeout harness unexpectedly succeeded")
finally:
  temporary.cleanup()
"#
        );
        let output = std::process::Command::new("python3")
            .arg("-B")
            .arg("-c")
            .arg(script)
            .output()
            .expect("launching Python seed timeout harness");
        assert!(
            output.status.success(),
            "seed timeout harness failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("capture-hash exit=0"),
            "capture-retention diagnostics must remain on stderr: {:?}",
            output.stderr
        );
        serde_json::from_slice(&output.stdout).expect("seed timeout harness JSON")
    }

    fn assert_seed_timeout_harness(
        timed_out_command: &str,
        expected_token: &str,
        forbidden_stage: &str,
    ) {
        for identity in ["sqlite-chromium-unsplit", "postgres-chromium-unsplit"] {
            assert_seed_timeout_result(
                run_seed_timeout_harness(timed_out_command, identity),
                timed_out_command,
                expected_token,
                forbidden_stage,
            );
        }
    }

    fn assert_seed_timeout_result(
        result: serde_json::Value,
        timed_out_command: &str,
        expected_token: &str,
        forbidden_stage: &str,
    ) {
        let expected_error = if expected_token == "seed-collector-stop-timeout" {
            "seed-collector-stop-timeout: collector stop exceeded 30s"
        } else {
            "seed-trace-verifier-timeout: verification exceeded 30s"
        };
        assert_eq!(result["error"], expected_error);
        let actions = result["actions"].as_array().expect("harness actions");
        let timeout = actions
            .iter()
            .find(|action| action[0] == "execute" && action[1] == timed_out_command)
            .expect("timed out command");
        assert_eq!(timeout[2], 30);
        assert_eq!(result["capture_bytes"], "abc");
        assert!(
            !actions.iter().any(|action| {
                action[0] == "copy_from_machine"
                    || action[1]
                        .as_str()
                        .is_some_and(|command| command.contains("tar czf"))
            }),
            "capture must be archived directly from the host exchange: {actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|action| action.to_string().contains("systemctl start ")),
            "timeout path must not restart a service: {actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|action| action.to_string().contains(forbidden_stage)),
            "timeout path must not reach {forbidden_stage}: {actions:?}"
        );
        assert!(
            !actions
                .iter()
                .any(|action| action.to_string().contains("playwright")),
            "timeout path must not reach Playwright: {actions:?}"
        );
    }

    #[test]
    fn e2e_seed_collector_stop_timeout_retains_capture_before_failing() {
        assert_seed_timeout_harness(
            "systemctl stop otel-collector.service",
            "seed-collector-stop-timeout",
            "test-support verify-seed-trace",
        );
    }

    #[test]
    fn e2e_seed_verifier_timeout_retains_capture_before_failing() {
        assert_seed_timeout_harness(
            "test-support verify-seed-trace --capture-dir /var/lib/jaunder/capture 2>&1",
            "seed-trace-verifier-timeout",
            "systemctl start otel-collector.service",
        );
    }

    #[test]
    fn e2e_seed_verifier_merges_stderr_into_the_reported_failure() {
        let checks = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("nix")
                .join("checks.nix"),
        )
        .expect("nix/checks.nix");
        let command = "test-support verify-seed-trace --capture-dir /var/lib/jaunder/capture 2>&1";
        let command_at = checks.find(command).expect("verifier stderr is captured");
        let assertion_at = checks[command_at..]
            .find("raise AssertionError(verify_out)")
            .map(|offset| command_at + offset)
            .expect("captured verifier output is reported");
        assert!(command_at < assertion_at);
    }
}
