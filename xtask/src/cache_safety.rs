//! Fail-closed structural proof for the coverage and e2e cache boundary.
//!
//! The versioned catalog is the only classification authority. It is reconciled
//! with Nix's derived inventory, validates Cachix's actual store-name filter,
//! and carries the paired source probes for every admitted support output.

use std::collections::BTreeSet;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::Deserialize;

use crate::git;
use crate::result::StepResult;

const POLICY_PATH: &str = "nix/cache-policy.json";
const CI_SETUP_PATH: &str = ".github/actions/setup-ci/action.yml";
const INVENTORY_ATTR: &str = "packages.x86_64-linux.cache-safety-inventory";
const WORKTREE_DIR: &str = ".xtask/cache-safety-source-probe.worktree";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Policy {
    schema_version: u32,
    outputs: Vec<PolicyOutput>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct PolicyOutput {
    attr: String,
    classification: Classification,
    #[serde(default)]
    equivalent_to: Option<String>,
    #[serde(default)]
    source_probe: Option<SourceProbe>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
struct SourceProbe {
    relevant: String,
    unrelated: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Classification {
    Support,
    Final,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NixInventory {
    schema_version: u32,
    outputs: Vec<String>,
}

fn parse_policy(raw: &str) -> Result<Policy> {
    let policy: Policy = serde_json::from_str(raw).context("parsing nix/cache-policy.json")?;
    if policy.schema_version != 1 || policy.outputs.is_empty() {
        bail!("cache policy must have schemaVersion 1 and at least one output");
    }
    let mut attrs = BTreeSet::new();
    for output in &policy.outputs {
        if output.attr.is_empty() || !attrs.insert(&output.attr) {
            bail!("cache policy has an empty or duplicate output attr");
        }
        match (&output.classification, &output.source_probe) {
            (Classification::Support, Some(probe))
                if !probe.relevant.is_empty()
                    && !probe.unrelated.is_empty()
                    && probe.relevant != probe.unrelated => {}
            (Classification::Support, _) => {
                bail!(
                    "support output {} needs distinct sourceProbe paths",
                    output.attr
                );
            }
            (Classification::Final, None) => {}
            (Classification::Final, Some(_)) => {
                bail!("final output {} must not define a sourceProbe", output.attr);
            }
        }
        if let Some(equivalent) = &output.equivalent_to
            && (equivalent.is_empty() || equivalent == &output.attr)
        {
            bail!(
                "cache policy has an invalid lifted equivalent for {}",
                output.attr
            );
        }
    }
    Ok(policy)
}

fn reconcile(policy: &Policy, inventory: &NixInventory) -> Result<()> {
    if inventory.schema_version != 1 || inventory.outputs.is_empty() {
        bail!("Nix cache-safety inventory is malformed or empty");
    }
    let actual = inventory.outputs.iter().collect::<BTreeSet<_>>();
    if actual.len() != inventory.outputs.len() {
        bail!("Nix cache-safety inventory contains duplicate output attrs");
    }
    let classified = policy
        .outputs
        .iter()
        .map(|output| &output.attr)
        .collect::<BTreeSet<_>>();
    if classified != actual {
        let missing = actual.difference(&classified).collect::<Vec<_>>();
        let unreachable = classified.difference(&actual).collect::<Vec<_>>();
        bail!(
            "cache policy does not reconcile with Nix inventory; missing={missing:?} unreachable={unreachable:?}"
        );
    }
    for output in &policy.outputs {
        if let Some(equivalent) = &output.equivalent_to {
            let equivalent_output = policy
                .outputs
                .iter()
                .find(|candidate| candidate.attr == *equivalent)
                .with_context(|| {
                    format!(
                        "{} names unknown lifted equivalent {equivalent}",
                        output.attr
                    )
                })?;
            if output.classification != Classification::Final
                || equivalent_output.classification != Classification::Final
            {
                bail!("lifted equivalents must both be final outputs");
            }
            if equivalent_output.equivalent_to.as_deref() != Some(&output.attr) {
                bail!("lifted equivalent declarations must be exactly reciprocal");
            }
        }
    }
    Ok(())
}

fn run(command: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(command)
        .args(args)
        .output()
        .with_context(|| format!("spawning `{command} {}`", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "`{command} {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8(output.stdout).context("Nix command output was not UTF-8")
}

fn build_inventory() -> Result<NixInventory> {
    let out = run(
        "nix",
        &[
            "build",
            "--no-link",
            "--print-out-paths",
            "--accept-flake-config",
            &format!(".#{}", INVENTORY_ATTR),
        ],
    )?;
    let path = out
        .lines()
        .next()
        .filter(|path| !path.is_empty())
        .context("inventory build returned no output path")?;
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading Nix cache-safety inventory {path}"))?;
    serde_json::from_str(&raw).context("parsing Nix cache-safety inventory")
}

fn eval_path_in(dir: Option<&Path>, attr: &str, field: &str) -> Result<String> {
    let mut command = Command::new("nix");
    if let Some(dir) = dir {
        command.current_dir(dir);
    }
    let output = command
        .args([
            "eval",
            "--raw",
            "--accept-flake-config",
            &format!(".#{}.{field}", attr),
        ])
        .output()
        .with_context(|| format!("spawning nix eval for {attr}.{field}"))?;
    if !output.status.success() {
        bail!(
            "nix eval for {attr}.{field} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let path = String::from_utf8(output.stdout).context("Nix eval output was not UTF-8")?;
    let path = path.trim();
    if !path.starts_with("/nix/store/") {
        bail!("{attr}.{field} did not evaluate to a store path: {path:?}");
    }
    Ok(path.to_owned())
}

fn push_filter(raw: &str) -> Result<Regex> {
    let filter = raw
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("pushFilter: "))
        .context("reading Cachix pushFilter from .github/actions/setup-ci/action.yml")?;
    let filter = filter
        .strip_prefix('"')
        .and_then(|filter| filter.strip_suffix('"'))
        .context("Cachix pushFilter must be a double-quoted literal")?;
    // The action receives the YAML-decoded scalar. This compact parser accepts
    // the only escape used by the checked-in double-quoted literal, so probe
    // matching has the same regex that Cachix receives rather than YAML text.
    let filter = filter.replace("\\\\", "\\");
    Regex::new(&filter).context("Cachix pushFilter must be a valid regular expression")
}

/// Cachix applies `pushFilter` to actual store paths, not flake attribute names.
/// The candidate admits only cataloged support identities and rejects every final
/// identity, including both output and derivation paths.
fn verify_filter_membership(
    resolved: &[(PolicyOutput, String, String)],
    filter: &Regex,
) -> Result<()> {
    for (output, out_path, drv_path) in resolved {
        for identity in [out_path, drv_path] {
            let matched = filter.is_match(identity);
            match output.classification {
                Classification::Support if matched => bail!(
                    "{} is cataloged support but Cachix pushFilter excludes its actual identity: {identity}",
                    output.attr
                ),
                Classification::Final if !matched => bail!(
                    "{} is cataloged final but Cachix pushFilter admits its actual identity: {identity}",
                    output.attr
                ),
                _ => {}
            }
        }
    }
    Ok(())
}

fn closure(path: &str, include_outputs: bool) -> Result<BTreeSet<String>> {
    let mut args = vec!["--query", "--requisites"];
    if include_outputs {
        args.push("--include-outputs");
    }
    args.push(path);
    Ok(run("nix-store", &args)?
        .lines()
        .map(ToOwned::to_owned)
        .collect())
}

fn reject_final_membership(
    closure: &BTreeSet<String>,
    finals: &[(String, String, String)],
) -> Result<()> {
    for (attr, out_path, drv_path) in finals {
        for path in [out_path, drv_path] {
            if closure.contains(path) {
                bail!("support closure contains final output {attr}: {path}");
            }
        }
    }
    Ok(())
}

fn git_run(dir: &Path, args: &[&str]) -> Result<()> {
    let mut full = vec!["-c", "core.hooksPath="];
    full.extend_from_slice(args);
    git::run(dir, &full)
}

struct WorktreeGuard {
    repo_root: PathBuf,
    path: PathBuf,
}

impl Drop for WorktreeGuard {
    fn drop(&mut self) {
        let _ = git::at(&self.repo_root)
            .args(["-c", "core.hooksPath=", "worktree", "remove", "--force"])
            .arg(&self.path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn dirty_probe_tree(dir: &Path) -> Result<()> {
    let readme = dir.join("README.md");
    let mut bytes =
        fs::read(&readme).context("reading README.md to dirty source probe worktree")?;
    bytes.push(b'\n');
    fs::write(readme, bytes).context("dirtying source probe worktree")
}

fn stage_change(dir: &Path, path: &str) -> Result<()> {
    OpenOptions::new()
        .append(true)
        .open(dir.join(path))
        .with_context(|| format!("opening {path} for cache-safety source probe"))?
        .write_all(b"\n")
        .with_context(|| format!("changing {path} for cache-safety source probe"))?;
    git_run(dir, &["add", path])
}

fn source_identity(dir: &Path, attr: &str) -> Result<String> {
    eval_path_in(Some(dir), attr, "drvPath")
}

fn verify_source_probes(policy: &Policy) -> Result<()> {
    let repo_root = std::env::current_dir().context("resolving cwd")?;
    let path = repo_root.join(WORKTREE_DIR);
    fs::create_dir_all(repo_root.join(".xtask")).context("creating .xtask")?;
    let _ = git::at(&repo_root)
        .args(["-c", "core.hooksPath=", "worktree", "remove", "--force"])
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let path_str = path
        .to_str()
        .context("source probe worktree path is not UTF-8")?;
    git_run(
        &repo_root,
        &["worktree", "add", "--detach", path_str, "HEAD"],
    )?;
    let _guard = WorktreeGuard {
        repo_root,
        path: path.clone(),
    };

    for output in policy
        .outputs
        .iter()
        .filter(|output| output.classification == Classification::Support)
    {
        let probe = output
            .source_probe
            .as_ref()
            .with_context(|| format!("{} has no validated sourceProbe", output.attr))?;
        dirty_probe_tree(&path)?;
        let base = source_identity(&path, &output.attr)?;

        stage_change(&path, &probe.relevant)?;
        let relevant = source_identity(&path, &output.attr)?;
        if relevant == base {
            bail!(
                "{}: relevant source {} did not change derivation identity {base}",
                output.attr,
                probe.relevant
            );
        }

        git_run(&path, &["reset", "--hard", "HEAD"])?;
        dirty_probe_tree(&path)?;
        stage_change(&path, &probe.unrelated)?;
        let unrelated = source_identity(&path, &output.attr)?;
        if unrelated != base {
            bail!(
                "{}: unrelated source {} changed derivation identity {base} -> {unrelated}",
                output.attr,
                probe.unrelated
            );
        }
        git_run(&path, &["reset", "--hard", "HEAD"])?;
    }
    Ok(())
}

fn verify(policy: &Policy) -> Result<()> {
    let inventory = build_inventory()?;
    reconcile(policy, &inventory)?;
    let mut resolved = Vec::new();
    for output in &policy.outputs {
        resolved.push((
            output.clone(),
            eval_path_in(None, &output.attr, "outPath")?,
            eval_path_in(None, &output.attr, "drvPath")?,
        ));
    }
    let ci_setup =
        fs::read_to_string(CI_SETUP_PATH).context("reading .github/actions/setup-ci/action.yml")?;
    verify_filter_membership(&resolved, &push_filter(&ci_setup)?)?;
    let finals = resolved
        .iter()
        .filter(|(output, _, _)| output.classification == Classification::Final)
        .map(|(output, out, drv)| (output.attr.clone(), out.clone(), drv.clone()))
        .collect::<Vec<_>>();
    for (output, out_path, drv_path) in &resolved {
        if output.classification == Classification::Support {
            // Realize support only. Final verdicts are never built just to inspect
            // their closure; evaluated final identities reject closure leakage.
            run(
                "nix",
                &[
                    "build",
                    "--no-link",
                    "--accept-flake-config",
                    &format!(".#{}", output.attr),
                ],
            )?;
            reject_final_membership(&closure(out_path, false)?, &finals)?;
            reject_final_membership(&closure(drv_path, true)?, &finals)?;
        }
    }
    verify_source_probes(policy)
}

pub fn probe() -> StepResult {
    let result = (|| {
        let policy = parse_policy(
            &fs::read_to_string(Path::new(POLICY_PATH)).context("reading nix/cache-policy.json")?,
        )?;
        verify(&policy)
    })();
    match result {
        Ok(()) => StepResult::ok("cache-safety-probe").detail(
            "cataloged support closures exclude finals, Cachix admits only support identities, and paired source probes hold",
        ),
        Err(error) => StepResult::fail("cache-safety-probe").detail(format!("{error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn support_probe() -> Option<SourceProbe> {
        Some(SourceProbe {
            relevant: "relevant".into(),
            unrelated: "unrelated".into(),
        })
    }

    fn policy(outputs: &[(&str, Classification)]) -> Policy {
        Policy {
            schema_version: 1,
            outputs: outputs
                .iter()
                .map(|(attr, classification)| PolicyOutput {
                    attr: (*attr).into(),
                    classification: classification.clone(),
                    equivalent_to: None,
                    source_probe: (classification == &Classification::Support)
                        .then(support_probe)
                        .flatten(),
                })
                .collect(),
        }
    }

    #[test]
    fn admits_classified_support() {
        let policy = policy(&[
            ("support", Classification::Support),
            ("final", Classification::Final),
        ]);
        let inventory = NixInventory {
            schema_version: 1,
            outputs: vec!["support".into(), "final".into()],
        };
        assert!(reconcile(&policy, &inventory).is_ok());
        assert!(
            reject_final_membership(
                &BTreeSet::from(["support-out".into()]),
                &[("final".into(), "final-out".into(), "final.drv".into())]
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_transitive_final_leakage() {
        let error = reject_final_membership(
            &BTreeSet::from(["final.drv".into()]),
            &[("final".into(), "final-out".into(), "final.drv".into())],
        )
        .unwrap_err();
        assert!(error.to_string().contains("final output final"));
    }

    #[test]
    fn rejects_incomplete_inventory() {
        let policy = policy(&[("support", Classification::Support)]);
        let inventory = NixInventory {
            schema_version: 1,
            outputs: vec!["support".into(), "unclassified".into()],
        };
        assert!(reconcile(&policy, &inventory).is_err());
    }

    #[test]
    fn rejects_malformed_policy_arms() {
        assert!(parse_policy(r#"{"schemaVersion":2,"outputs":[]}"#).is_err());
        assert!(
            parse_policy(
                r#"{"schemaVersion":1,"outputs":[{"attr":"x","classification":"unknown"}]}"#
            )
            .is_err()
        );
        assert!(parse_policy(r#"{"schemaVersion":1,"outputs":[{"attr":"x","classification":"final"},{"attr":"x","classification":"support"}]}"#).is_err());
    }

    #[test]
    fn rejects_support_without_paired_source_probe() {
        assert!(
            parse_policy(
                r#"{"schemaVersion":1,"outputs":[{"attr":"support","classification":"support"}]}"#
            )
            .is_err()
        );
        assert!(parse_policy(r#"{"schemaVersion":1,"outputs":[{"attr":"final","classification":"final","sourceProbe":{"relevant":"a","unrelated":"b"}}]}"#).is_err());
    }

    #[test]
    fn rejects_one_way_or_mismatched_lifted_equivalents() {
        let mut one_way = policy(&[
            ("check", Classification::Final),
            ("package", Classification::Final),
        ]);
        one_way.outputs[0].equivalent_to = Some("package".into());
        let inventory = NixInventory {
            schema_version: 1,
            outputs: vec!["check".into(), "package".into()],
        };
        assert!(reconcile(&one_way, &inventory).is_err());
        let mut mismatched = one_way;
        mismatched.outputs[1].equivalent_to = Some("another-final".into());
        assert!(reconcile(&mismatched, &inventory).is_err());
    }

    #[test]
    fn admits_only_actual_cataloged_support_identities() {
        let filter = Regex::new(
            r"(?:^|/)[0-9a-z]{32}-jaunder-coverage(?:\.drv)?$|(?:^|/)[0-9a-z]{32}-jaunder-e2e-(?:checks|sqlite-chromium)(?:\.drv)?$",
        )
        .unwrap();
        let resolved = vec![
            (
                PolicyOutput {
                    attr: "packages.e2e-support".into(),
                    classification: Classification::Support,
                    equivalent_to: None,
                    source_probe: support_probe(),
                },
                "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-e2e".into(),
                "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-e2e.drv".into(),
            ),
            (
                PolicyOutput {
                    attr: "checks.coverage".into(),
                    classification: Classification::Final,
                    equivalent_to: None,
                    source_probe: None,
                },
                "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-coverage".into(),
                "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-coverage.drv".into(),
            ),
        ];
        assert!(verify_filter_membership(&resolved, &filter).is_ok());
    }

    #[test]
    fn rejects_actual_final_identity_that_escapes_filter() {
        let resolved = vec![(
            PolicyOutput {
                attr: "checks.coverage".into(),
                classification: Classification::Final,
                equivalent_to: None,
                source_probe: None,
            },
            "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-coverage".into(),
            "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-coverage.drv".into(),
        )];
        assert!(verify_filter_membership(&resolved, &Regex::new("jaunder-e2e").unwrap()).is_err());
    }

    #[test]
    fn rejects_actual_support_identity_that_filter_excludes() {
        let resolved = vec![(
            PolicyOutput {
                attr: "packages.e2e-support".into(),
                classification: Classification::Support,
                equivalent_to: None,
                source_probe: support_probe(),
            },
            "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-e2e".into(),
            "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-e2e.drv".into(),
        )];
        assert!(verify_filter_membership(&resolved, &Regex::new("jaunder-e2e").unwrap()).is_err());
    }
}
