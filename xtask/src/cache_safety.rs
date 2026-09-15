//! Fail-closed structural proof for the coverage and e2e cache boundary.
//!
//! The catalog is deliberately checked against a Nix-derived inventory. Names
//! identify the population, but closure membership is the safety evidence.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::result::StepResult;

const POLICY_PATH: &str = "nix/cache-policy.json";
const CI_SETUP_PATH: &str = ".github/actions/setup-ci/action.yml";
const INVENTORY_ATTR: &str = "packages.x86_64-linux.cache-safety-inventory";

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

fn eval_path(attr: &str, field: &str) -> Result<String> {
    let output = run(
        "nix",
        &[
            "eval",
            "--raw",
            "--accept-flake-config",
            &format!(".#{attr}.{field}"),
        ],
    )?;
    let path = output.trim();
    if !path.starts_with("/nix/store/") {
        bail!("{attr}.{field} did not evaluate to a store path: {path:?}");
    }
    Ok(path.to_owned())
}

fn broad_filter_fragments(raw: &str) -> Result<Vec<&str>> {
    let filter = raw
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("pushFilter: "))
        .context("reading Cachix pushFilter from .github/actions/setup-ci/action.yml")?;
    let filter = filter
        .strip_prefix('"')
        .and_then(|filter| filter.strip_suffix('"'))
        .context("Cachix pushFilter must be a double-quoted literal")?;
    let fragments = filter.split('|').collect::<Vec<_>>();
    if fragments.is_empty()
        || fragments.iter().any(|fragment| {
            fragment.is_empty()
                || !fragment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        bail!("Cachix pushFilter must contain nonempty ASCII literal fragments");
    }
    Ok(fragments)
}

fn verify_broad_filter_membership(
    resolved: &[(PolicyOutput, String, String)],
    fragments: &[&str],
) -> Result<()> {
    for (output, out_path, drv_path) in resolved {
        for identity in [out_path, drv_path] {
            if !fragments.iter().any(|fragment| identity.contains(fragment)) {
                bail!(
                    "{} is cataloged as {} but its evaluated identity is not excluded by the current Cachix pushFilter: {identity}",
                    output.attr,
                    match output.classification {
                        Classification::Support => "support",
                        Classification::Final => "final",
                    }
                );
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

fn verify(policy: &Policy) -> Result<()> {
    let inventory = build_inventory()?;
    reconcile(policy, &inventory)?;
    let mut resolved = Vec::new();
    for output in &policy.outputs {
        resolved.push((
            output.clone(),
            eval_path(&output.attr, "outPath")?,
            eval_path(&output.attr, "drvPath")?,
        ));
    }
    let ci_setup =
        fs::read_to_string(CI_SETUP_PATH).context("reading .github/actions/setup-ci/action.yml")?;
    let broad_filter = broad_filter_fragments(&ci_setup)?;
    verify_broad_filter_membership(&resolved, &broad_filter)?;
    let finals = resolved
        .iter()
        .filter(|(output, _, _)| output.classification == Classification::Final)
        .map(|(output, out, drv)| (output.attr.clone(), out.clone(), drv.clone()))
        .collect::<Vec<_>>();
    for (output, out_path, drv_path) in resolved {
        if output.classification == Classification::Support {
            // Realize support only. Final verdicts are never built just to inspect
            // their closure; their evaluated identities are enough to reject them.
            run(
                "nix",
                &[
                    "build",
                    "--no-link",
                    "--accept-flake-config",
                    &format!(".#{}", output.attr),
                ],
            )?;
            reject_final_membership(&closure(&out_path, false)?, &finals)?;
            reject_final_membership(&closure(&drv_path, true)?, &finals)?;
        }
    }
    Ok(())
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
            "all admitted support closures exclude every final and lifted coverage/e2e output",
        ),
        Err(error) => StepResult::fail("cache-safety-probe").detail(format!("{error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(outputs: &[(&str, Classification)]) -> Policy {
        Policy {
            schema_version: 1,
            outputs: outputs
                .iter()
                .map(|(attr, classification)| PolicyOutput {
                    attr: (*attr).into(),
                    classification: classification.clone(),
                    equivalent_to: None,
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
    fn accepts_actual_filter_matched_support_and_final_identities() {
        let filter =
            broad_filter_fragments(include_str!("../../.github/actions/setup-ci/action.yml"))
                .unwrap();
        let resolved = vec![
            (
                PolicyOutput {
                    attr: "packages.e2e-support".into(),
                    classification: Classification::Support,
                    equivalent_to: None,
                },
                "/nix/store/support-jaunder-e2e".into(),
                "/nix/store/support-jaunder-e2e.drv".into(),
            ),
            (
                PolicyOutput {
                    attr: "checks.coverage".into(),
                    classification: Classification::Final,
                    equivalent_to: None,
                },
                "/nix/store/final-jaunder-coverage".into(),
                "/nix/store/final-jaunder-coverage.drv".into(),
            ),
        ];
        assert!(verify_broad_filter_membership(&resolved, &filter).is_ok());
    }

    #[test]
    fn rejects_attr_spelling_when_actual_identity_escapes_filter() {
        let resolved = vec![(
            PolicyOutput {
                attr: "packages.e2e-support".into(),
                classification: Classification::Support,
                equivalent_to: None,
            },
            "/nix/store/innocuous-support".into(),
            "/nix/store/innocuous-support.drv".into(),
        )];
        assert!(verify_broad_filter_membership(&resolved, &["jaunder-e2e"]).is_err());
    }
}
