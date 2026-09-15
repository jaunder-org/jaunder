//! Fail-closed structural proof for the coverage and e2e cache boundary.
//!
//! Nix emits the classification and source-boundary inventory from the
//! concern-owned attrsets. The checked catalog reconciles to that authority,
//! validates Cachix's actual store-name filter, and runs finite source-boundary
//! regression arms without mistaking them for the complete Nix input graph.

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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Policy {
    schema_version: u32,
    cache_boundary: CacheBoundary,
    outputs: Vec<PolicyOutput>,
    source_families: std::collections::BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum CacheBoundary {
    Broad,
    Narrow,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PolicyOutput {
    attr: String,
    classification: Classification,
    #[serde(default)]
    equivalent_to: Option<String>,
    #[serde(default)]
    source_family: Option<String>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SourceFamily {
    categories: Vec<SourceCategory>,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SourceCategory {
    name: String,
    relevant: String,
    excluded: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct SupportFamily {
    attr: String,
    family: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum Classification {
    Support,
    Final,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NixInventory {
    schema_version: u32,
    final_attrs: Vec<String>,
    support_attrs: Vec<String>,
    support_families: Vec<SupportFamily>,
    source_families: std::collections::BTreeMap<String, SourceFamily>,
}

fn parse_policy(raw: &str) -> Result<Policy> {
    let policy: Policy = serde_json::from_str(raw).context("parsing nix/cache-policy.json")?;
    if policy.schema_version != 1 || policy.outputs.is_empty() || policy.source_families.is_empty()
    {
        bail!("cache policy must have schemaVersion 1 and at least one output");
    }
    let mut attrs = BTreeSet::new();
    for output in &policy.outputs {
        if output.attr.is_empty() || !attrs.insert(&output.attr) {
            bail!("cache policy has an empty or duplicate output attr");
        }
        match (&output.classification, &output.source_family) {
            (Classification::Support, Some(family)) if !family.is_empty() => {}
            (Classification::Support, _) => {
                bail!("support output {} needs a sourceFamily", output.attr)
            }
            (Classification::Final, None) => {}
            (Classification::Final, Some(_)) => bail!(
                "final output {} must not define a sourceFamily",
                output.attr
            ),
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
    for (family, categories) in &policy.source_families {
        if family.is_empty()
            || categories.is_empty()
            || categories.iter().any(String::is_empty)
            || categories.iter().collect::<BTreeSet<_>>().len() != categories.len()
        {
            bail!("cache policy has malformed source-family categories");
        }
    }
    Ok(policy)
}

fn reconcile(policy: &Policy, inventory: &NixInventory) -> Result<()> {
    let finals = inventory.final_attrs.iter().collect::<BTreeSet<_>>();
    let supports = inventory.support_attrs.iter().collect::<BTreeSet<_>>();
    if inventory.schema_version != 1
        || finals.is_empty()
        || supports.is_empty()
        || !finals.is_disjoint(&supports)
    {
        bail!("Nix cache-safety inventory is malformed or has overlapping classifications");
    }
    if finals.len() != inventory.final_attrs.len()
        || supports.len() != inventory.support_attrs.len()
    {
        bail!("Nix cache-safety inventory contains duplicate output attrs");
    }
    let policy_finals = policy
        .outputs
        .iter()
        .filter(|output| output.classification == Classification::Final)
        .map(|output| &output.attr)
        .collect::<BTreeSet<_>>();
    let policy_supports = policy
        .outputs
        .iter()
        .filter(|output| output.classification == Classification::Support)
        .map(|output| &output.attr)
        .collect::<BTreeSet<_>>();
    if policy_finals != finals || policy_supports != supports {
        bail!("cache policy classifications do not reconcile with generated Nix inventory");
    }
    let generated_families = inventory
        .support_families
        .iter()
        .map(|entry| (&entry.attr, &entry.family))
        .collect::<std::collections::BTreeMap<_, _>>();
    if generated_families.len() != inventory.support_families.len()
        || generated_families.keys().copied().collect::<BTreeSet<_>>() != supports
    {
        bail!("Nix cache-safety support families are incomplete or duplicate");
    }
    for output in policy
        .outputs
        .iter()
        .filter(|output| output.classification == Classification::Support)
    {
        let family = output
            .source_family
            .as_ref()
            .with_context(|| format!("{} has no validated source family", output.attr))?;
        if generated_families.get(&output.attr) != Some(&family)
            || !inventory.source_families.contains_key(family)
        {
            bail!(
                "{} does not reconcile with its generated source family",
                output.attr
            );
        }
    }
    if policy.source_families.len() != inventory.source_families.len() {
        bail!("cache policy source-family categories do not reconcile with Nix inventory");
    }
    for (name, family) in &inventory.source_families {
        let generated_categories = family
            .categories
            .iter()
            .map(|category| &category.name)
            .collect::<BTreeSet<_>>();
        let declared_categories = policy
            .source_families
            .get(name)
            .with_context(|| format!("cache policy omits generated source family {name}"))?
            .iter()
            .collect::<BTreeSet<_>>();
        if generated_categories != declared_categories
            || generated_categories.len() != family.categories.len()
            || name.is_empty()
            || family.categories.is_empty()
            || family.categories.iter().any(|category| {
                category.name.is_empty()
                    || category.relevant.is_empty()
                    || category.excluded.is_empty()
                    || category.relevant == category.excluded
            })
        {
            bail!("generated source family {name} has malformed categories");
        }
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

fn run_in(dir: Option<&Path>, command: &str, args: &[&str]) -> Result<String> {
    let mut process = Command::new(command);
    if let Some(dir) = dir {
        process.current_dir(dir);
    }
    let output = process
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

fn run(command: &str, args: &[&str]) -> Result<String> {
    run_in(None, command, args)
}

fn build_inventory(dir: &Path) -> Result<NixInventory> {
    let out = run_in(
        Some(dir),
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
    boundary: CacheBoundary,
) -> Result<()> {
    for (output, out_path, drv_path) in resolved {
        for identity in [out_path, drv_path] {
            let matched = filter.is_match(identity);
            match output.classification {
                Classification::Support if boundary == CacheBoundary::Narrow && matched => {
                    bail!(
                        "{} is cataloged support but the narrow Cachix pushFilter excludes its actual identity: {identity}",
                        output.attr
                    )
                }
                Classification::Support if boundary == CacheBoundary::Broad && !matched => {
                    bail!(
                        "{} escapes the selected broad Cachix exclusion: {identity}",
                        output.attr
                    )
                }
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

fn non_substitutable_metadata(raw: &str, attr: &str) -> Result<()> {
    let derivations: serde_json::Value =
        serde_json::from_str(raw).context("parsing Nix derivation metadata")?;
    let derivation = derivations
        .get("derivations")
        .and_then(serde_json::Value::as_object)
        .and_then(|derivations| derivations.values().next())
        .context("Nix derivation metadata was empty")?;
    let env = derivation
        .get("env")
        .and_then(serde_json::Value::as_object)
        .context("Nix derivation metadata has no environment")?;
    // Nix's JSON derivation encoding serializes `allowSubstitutes = false`
    // as an empty string and `preferLocalBuild = true` as `"1"`.
    if env
        .get("allowSubstitutes")
        .and_then(serde_json::Value::as_str)
        != Some("")
        || env
            .get("preferLocalBuild")
            .and_then(serde_json::Value::as_str)
            != Some("1")
    {
        bail!(
            "final output {attr} is not marked non-substitutable in its actual Nix derivation metadata"
        );
    }
    Ok(())
}

fn verify_non_substitutable(attr: &str, drv_path: &str) -> Result<()> {
    // Read the evaluated derivation, rather than trusting a Nix expression or
    // policy declaration. These are the flags the Nix daemon receives when it
    // decides whether this final result may be substituted.
    non_substitutable_metadata(&run("nix", &["show-derivation", drv_path])?, attr)
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

fn git_output(dir: &Path, args: &[&str]) -> Result<String> {
    let output = git::at(dir)
        .args(["-c", "core.hooksPath="])
        .args(args)
        .output()
        .context("running git for cache-safety probe")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout)
        .context("git output is not UTF-8")
        .map(|value| value.trim().to_owned())
}

fn staged_snapshot_commit(repo_root: &Path) -> Result<String> {
    let tree = git_output(repo_root, &["write-tree"])?;
    let output = git::at(repo_root)
        .args(["-c", "core.hooksPath=", "commit-tree"])
        .arg(&tree)
        .args(["-m", "cache-safety staged snapshot"])
        .env("GIT_AUTHOR_NAME", "cache-safety probe")
        .env("GIT_AUTHOR_EMAIL", "cache-safety@invalid")
        .env("GIT_COMMITTER_NAME", "cache-safety probe")
        .env("GIT_COMMITTER_EMAIL", "cache-safety@invalid")
        .output()
        .context("creating staged cache-safety snapshot")?;
    if !output.status.success() {
        bail!(
            "git commit-tree failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout)
        .context("snapshot commit id is not UTF-8")
        .map(|value| value.trim().to_owned())
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
    let path = dir.join(path);
    if path.ends_with("nix/checks.nix") {
        // A whitespace-only Nix edit does not alter a derivation. Exercise the
        // declared test-definition category through its explicit e2e salt,
        // which every generated driver interpolates into its input graph.
        let source = fs::read_to_string(&path).context("reading Nix source probe")?;
        if !source.contains("e2eSalt = \"\";") {
            bail!("locating e2e salt source-probe arm");
        }
        let source = source.replace("e2eSalt = \"\";", "e2eSalt = \"cache-safety-probe\";");
        fs::write(&path, source).context("changing Nix source probe")?;
    } else {
        OpenOptions::new()
            .append(true)
            .open(&path)
            .with_context(|| format!("opening {} for cache-safety source probe", path.display()))?
            .write_all(b"\n")
            .with_context(|| {
                format!("changing {} for cache-safety source probe", path.display())
            })?;
    }
    let relative = path
        .strip_prefix(dir)
        .context("resolving source probe path")?;
    git_run(
        dir,
        &[
            "add",
            relative
                .to_str()
                .context("source probe path is not UTF-8")?,
        ],
    )
}

fn source_identity(dir: &Path, attr: &str) -> Result<String> {
    eval_path_in(Some(dir), attr, "drvPath")
}

fn family_outputs<'a>(inventory: &'a NixInventory, family: &str) -> Result<Vec<&'a str>> {
    let outputs = inventory
        .support_families
        .iter()
        .filter(|entry| entry.family == family)
        .map(|entry| entry.attr.as_str())
        .collect::<Vec<_>>();
    if outputs.is_empty() {
        bail!("generated source family {family} has no support output");
    }
    Ok(outputs)
}

fn snapshot_worktree() -> Result<(PathBuf, WorktreeGuard)> {
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
    // The flake ignores unstaged files. Freeze the index once, then derive all
    // inventory, identities, closures, and mutation baselines from that commit.
    let snapshot = staged_snapshot_commit(&repo_root)?;
    git_run(
        &repo_root,
        &["worktree", "add", "--detach", path_str, &snapshot],
    )?;
    let guard = WorktreeGuard {
        repo_root,
        path: path.clone(),
    };
    Ok((path, guard))
}

fn verify_source_probes(path: &Path, inventory: &NixInventory) -> Result<()> {
    for (family_name, family) in &inventory.source_families {
        for output in family_outputs(inventory, family_name)? {
            dirty_probe_tree(path)?;
            let base = source_identity(path, output)?;
            // These are regression mutations. The complete boundary is the Nix
            // source predicate/input graph that generated this family, not this
            // finite representative set.
            for category in &family.categories {
                stage_change(path, &category.relevant)?;
                let relevant = source_identity(path, output)?;
                if relevant == base {
                    bail!(
                        "{family_name}/{output}/{}: relevant source {} did not change derivation identity {base}",
                        category.name,
                        category.relevant
                    );
                }
                git_run(path, &["reset", "--hard", "HEAD"])?;
                dirty_probe_tree(path)?;
                stage_change(path, &category.excluded)?;
                let excluded = source_identity(path, output)?;
                if excluded != base {
                    bail!(
                        "{family_name}/{output}/{}: excluded source {} changed derivation identity {base} -> {excluded}",
                        category.name,
                        category.excluded
                    );
                }
                git_run(path, &["reset", "--hard", "HEAD"])?;
                dirty_probe_tree(path)?;
            }
            git_run(path, &["reset", "--hard", "HEAD"])?;
        }
    }
    Ok(())
}

fn verify() -> Result<()> {
    let (snapshot, _guard) = snapshot_worktree()?;
    let policy = parse_policy(
        &fs::read_to_string(snapshot.join(POLICY_PATH)).context("reading nix/cache-policy.json")?,
    )?;
    let inventory = build_inventory(&snapshot)?;
    reconcile(&policy, &inventory)?;
    let mut resolved = Vec::new();
    for output in &policy.outputs {
        resolved.push((
            output.clone(),
            eval_path_in(Some(&snapshot), &output.attr, "outPath")?,
            eval_path_in(Some(&snapshot), &output.attr, "drvPath")?,
        ));
    }
    let ci_setup = fs::read_to_string(snapshot.join(CI_SETUP_PATH))
        .context("reading .github/actions/setup-ci/action.yml")?;
    verify_filter_membership(&resolved, &push_filter(&ci_setup)?, policy.cache_boundary)?;
    let finals = resolved
        .iter()
        .filter(|(output, _, _)| output.classification == Classification::Final)
        .map(|(output, out, drv)| (output.attr.clone(), out.clone(), drv.clone()))
        .collect::<Vec<_>>();
    for (attr, _, drv_path) in &finals {
        verify_non_substitutable(attr, drv_path)?;
    }
    for (output, out_path, drv_path) in &resolved {
        if output.classification == Classification::Support {
            // Realize support only. Final verdicts are never built just to inspect
            // their closure; evaluated final identities reject closure leakage.
            run_in(
                Some(&snapshot),
                "nix",
                &[
                    "build",
                    "--no-link",
                    "--accept-flake-config",
                    &format!(".#{}", output.attr),
                ],
            )?;
            let output_closure = closure(out_path, false)?;
            reject_final_membership(&output_closure, &finals)?;
            let derivation_closure = closure(drv_path, true)?;
            reject_final_membership(&derivation_closure, &finals)?;
        }
    }
    verify_source_probes(&snapshot, &inventory)
}

pub fn probe() -> StepResult {
    let result = verify();
    match result {
        Ok(()) => StepResult::ok("cache-safety-probe").detail(
            "generated support closures exclude generated finals, Cachix admits only support identities, and generated source-family regression arms hold",
        ),
        Err(error) => StepResult::fail("cache-safety-probe").detail(format!("{error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inventory(final_attrs: &[&str], support_attrs: &[&str]) -> NixInventory {
        let support_families = support_attrs
            .iter()
            .map(|attr| SupportFamily {
                attr: (*attr).into(),
                family: "family".into(),
            })
            .collect();
        NixInventory {
            schema_version: 1,
            final_attrs: final_attrs.iter().map(|attr| (*attr).into()).collect(),
            support_attrs: support_attrs.iter().map(|attr| (*attr).into()).collect(),
            support_families,
            source_families: std::collections::BTreeMap::from([(
                "family".into(),
                SourceFamily {
                    categories: vec![SourceCategory {
                        name: "category".into(),
                        relevant: "relevant".into(),
                        excluded: "excluded".into(),
                    }],
                },
            )]),
        }
    }

    fn policy(outputs: &[(&str, Classification)]) -> Policy {
        Policy {
            schema_version: 1,
            cache_boundary: CacheBoundary::Narrow,
            source_families: std::collections::BTreeMap::from([(
                "family".into(),
                vec!["category".into()],
            )]),
            outputs: outputs
                .iter()
                .map(|(attr, classification)| PolicyOutput {
                    attr: (*attr).into(),
                    classification: classification.clone(),
                    equivalent_to: None,
                    source_family: (classification == &Classification::Support)
                        .then(|| "family".into()),
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
        let inventory = inventory(&["final"], &["support"]);
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
    fn admits_unrelated_nixpkgs_closure_path() {
        assert!(
            reject_final_membership(
                &BTreeSet::from(
                    ["/nix/store/0123456789abcdefghijklmnopqrstuv-bash-5.3.drv".into()]
                ),
                &[("final".into(), "final-out".into(), "final.drv".into())]
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_missing_generated_source_family() {
        let policy = policy(&[("support", Classification::Support)]);
        let mut inventory = inventory(&[], &["support"]);
        inventory.source_families.clear();
        assert!(reconcile(&policy, &inventory).is_err());
    }

    #[test]
    fn rejects_malformed_generated_category_arm() {
        let policy = policy(&[("support", Classification::Support)]);
        let mut inventory = inventory(&[], &["support"]);
        inventory
            .source_families
            .get_mut("family")
            .unwrap()
            .categories[0]
            .excluded = "relevant".into();
        assert!(reconcile(&policy, &inventory).is_err());
    }

    #[test]
    fn rejects_missing_extra_or_duplicate_source_categories() {
        assert!(parse_policy(r#"{"schemaVersion":1,"sourceFamilies":{"family":[]},"outputs":[{"attr":"support","classification":"support","sourceFamily":"family"}]}"#).is_err());
        assert!(parse_policy(r#"{"schemaVersion":1,"sourceFamilies":{"family":["category","category"]},"outputs":[{"attr":"support","classification":"support","sourceFamily":"family"}]}"#).is_err());

        let mut missing = policy(&[("support", Classification::Support)]);
        missing
            .source_families
            .insert("family".into(), vec!["other".into()]);
        assert!(reconcile(&missing, &inventory(&[], &["support"])).is_err());

        let mut extra = policy(&[("support", Classification::Support)]);
        extra
            .source_families
            .insert("extra".into(), vec!["category".into()]);
        assert!(reconcile(&extra, &inventory(&[], &["support"])).is_err());
    }

    #[test]
    fn selects_every_support_output_in_a_family() {
        let inventory = inventory(&[], &["one", "two"]);
        assert_eq!(
            family_outputs(&inventory, "family").unwrap(),
            vec!["one", "two"]
        );
    }

    #[test]
    fn rejects_final_without_actual_non_substitution_metadata() {
        let error = non_substitutable_metadata(
            r#"{"derivations":{"drv":{"env":{"allowSubstitutes":"1"}}}}"#,
            "final",
        )
        .unwrap_err();
        assert!(error.to_string().contains("not marked non-substitutable"));
    }

    #[test]
    fn admits_actual_non_substitution_metadata() {
        assert!(
            non_substitutable_metadata(
                r#"{"derivations":{"drv":{"env":{"allowSubstitutes":"","preferLocalBuild":"1"}}}}"#,
                "final"
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
    fn rejects_generated_classification_mismatch() {
        let policy = policy(&[("verdict", Classification::Final)]);
        let inventory = inventory(&[], &["verdict"]);
        assert!(reconcile(&policy, &inventory).is_err());
    }

    #[test]
    fn rejects_incomplete_inventory() {
        let policy = policy(&[("support", Classification::Support)]);
        let inventory = inventory(&[], &["support", "unclassified"]);
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
    fn rejects_unknown_policy_fields() {
        assert!(
            parse_policy(
                r#"{"schemaVersion":1,"cacheBoundary":"broad","sourceFamilies":{"family":["category"]},"outputs":[{"attr":"final","classification":"final"}],"unexpected":true}"#
            )
            .is_err()
        );
        assert!(
            parse_policy(
                r#"{"schemaVersion":1,"cacheBoundary":"broad","sourceFamilies":{"family":["category"]},"outputs":[{"attr":"final","classification":"final","equivalantTo":"other"}]}"#
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_unknown_inventory_fields() {
        let inventory = || {
            serde_json::json!({
                "schemaVersion": 1,
                "finalAttrs": ["final"],
                "supportAttrs": ["support"],
                "supportFamilies": [{"attr": "support", "family": "family"}],
                "sourceFamilies": {"family": {"categories": [
                    {"name": "category", "relevant": "relevant", "excluded": "excluded"}
                ]}}
            })
        };

        let mut top = inventory();
        top.as_object_mut()
            .unwrap()
            .insert("unexpected".into(), true.into());
        assert!(serde_json::from_value::<NixInventory>(top).is_err());

        let mut support = inventory();
        support["supportFamilies"][0]["unexpected"] = true.into();
        assert!(serde_json::from_value::<NixInventory>(support).is_err());

        let mut family = inventory();
        family["sourceFamilies"]["family"]["unexpected"] = true.into();
        assert!(serde_json::from_value::<NixInventory>(family).is_err());

        let mut category = inventory();
        category["sourceFamilies"]["family"]["categories"][0]["unexpected"] = true.into();
        assert!(serde_json::from_value::<NixInventory>(category).is_err());
    }

    #[test]
    fn rejects_support_without_source_family() {
        assert!(
            parse_policy(
                r#"{"schemaVersion":1,"outputs":[{"attr":"support","classification":"support"}]}"#
            )
            .is_err()
        );
        assert!(parse_policy(r#"{"schemaVersion":1,"outputs":[{"attr":"final","classification":"final","sourceFamily":"family"}]}"#).is_err());
    }

    #[test]
    fn rejects_one_way_or_mismatched_lifted_equivalents() {
        let mut one_way = policy(&[
            ("check", Classification::Final),
            ("package", Classification::Final),
        ]);
        one_way.outputs[0].equivalent_to = Some("package".into());
        let inventory = inventory(&["check", "package"], &[]);
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
                    source_family: Some("family".into()),
                },
                "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-e2e".into(),
                "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-e2e.drv".into(),
            ),
            (
                PolicyOutput {
                    attr: "checks.coverage".into(),
                    classification: Classification::Final,
                    equivalent_to: None,
                    source_family: None,
                },
                "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-coverage".into(),
                "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-coverage.drv".into(),
            ),
        ];
        assert!(verify_filter_membership(&resolved, &filter, CacheBoundary::Narrow).is_ok());
    }

    #[test]
    fn rejects_actual_final_identity_that_escapes_filter() {
        let resolved = vec![(
            PolicyOutput {
                attr: "checks.coverage".into(),
                classification: Classification::Final,
                equivalent_to: None,
                source_family: None,
            },
            "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-coverage".into(),
            "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-coverage.drv".into(),
        )];
        assert!(
            verify_filter_membership(
                &resolved,
                &Regex::new("jaunder-e2e").unwrap(),
                CacheBoundary::Narrow
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_actual_support_identity_that_filter_excludes() {
        let resolved = vec![(
            PolicyOutput {
                attr: "packages.e2e-support".into(),
                classification: Classification::Support,
                equivalent_to: None,
                source_family: Some("family".into()),
            },
            "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-e2e".into(),
            "/nix/store/0123456789abcdefghijklmnopqrstuv-jaunder-e2e.drv".into(),
        )];
        assert!(
            verify_filter_membership(
                &resolved,
                &Regex::new("jaunder-e2e").unwrap(),
                CacheBoundary::Narrow
            )
            .is_err()
        );
        assert!(
            verify_filter_membership(
                &resolved,
                &Regex::new("does-not-match").unwrap(),
                CacheBoundary::Broad
            )
            .is_err()
        );
        assert!(
            verify_filter_membership(
                &resolved,
                &Regex::new("jaunder-e2e").unwrap(),
                CacheBoundary::Broad
            )
            .is_ok()
        );
    }
}
