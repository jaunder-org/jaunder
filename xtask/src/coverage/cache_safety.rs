//! Fail-closed cache policy over Nix's authoritative cache-class inventory.

use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::steps::nix;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Output {
    pub attr: String,
    pub name: String,
    #[serde(rename = "output")]
    pub out_path: String,
    #[serde(rename = "derivation")]
    pub drv_path: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct CachePolicy {
    #[serde(rename = "excludedNameFragments")]
    pub excluded_name_fragments: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct SupportInputs {
    #[serde(rename = "rustToolchain")]
    pub rust_toolchain: String,
    #[serde(rename = "cargoLlvmCov")]
    pub cargo_llvm_cov: String,
    #[serde(rename = "cargoNextest")]
    pub cargo_nextest: String,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Inventory {
    pub policy: CachePolicy,
    pub eligible: Vec<Output>,
    #[serde(rename = "final")]
    pub final_outputs: Vec<Output>,
    #[serde(rename = "supportInputs")]
    pub support_inputs: SupportInputs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheSafetyError {
    InvalidInventory(String),
    SupportClosureContainsFinal { final_attr: String, path: String },
    SupportClosureMissingInput { input: &'static str, path: String },
}

impl fmt::Display for CacheSafetyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInventory(detail) => write!(f, "invalid cache-safety inventory: {detail}"),
            Self::SupportClosureContainsFinal { final_attr, path } => write!(
                f,
                "cache-eligible support closure contains final {final_attr} input {path}"
            ),
            Self::SupportClosureMissingInput { input, path } => {
                write!(
                    f,
                    "cache-eligible support closure omits {input} input {path}"
                )
            }
        }
    }
}

impl std::error::Error for CacheSafetyError {}

pub fn parse_inventory(raw: &str) -> Result<Inventory, CacheSafetyError> {
    let inventory: Inventory = serde_json::from_str(raw)
        .map_err(|error| CacheSafetyError::InvalidInventory(error.to_string()))?;
    verify_inventory(&inventory)?;
    Ok(inventory)
}

pub fn verify_inventory(inventory: &Inventory) -> Result<(), CacheSafetyError> {
    if inventory.eligible.is_empty() {
        return Err(CacheSafetyError::InvalidInventory(
            "cache-eligible support output set is empty".into(),
        ));
    }
    let mut fragments = BTreeSet::new();
    for fragment in &inventory.policy.excluded_name_fragments {
        if fragment.is_empty()
            || !fragment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || !fragments.insert(fragment)
        {
            return Err(CacheSafetyError::InvalidInventory(
                "Cachix exclusion fragments must be distinct nonempty ASCII literals".into(),
            ));
        }
    }
    if inventory.final_outputs.is_empty() {
        return Err(CacheSafetyError::InvalidInventory(
            "final coverage/e2e output set is empty".into(),
        ));
    }
    let mut support_input_paths = BTreeSet::new();
    for (name, path) in [
        ("Rust toolchain", &inventory.support_inputs.rust_toolchain),
        ("cargo-llvm-cov", &inventory.support_inputs.cargo_llvm_cov),
        ("cargo-nextest", &inventory.support_inputs.cargo_nextest),
    ] {
        if path.is_empty() || !support_input_paths.insert(path) {
            return Err(CacheSafetyError::InvalidInventory(format!(
                "{name} support input path is empty or duplicated"
            )));
        }
    }
    let mut attrs = BTreeSet::new();
    for output in inventory.eligible.iter().chain(&inventory.final_outputs) {
        if output.attr.is_empty()
            || output.name.is_empty()
            || output.out_path.is_empty()
            || output.drv_path.is_empty()
        {
            return Err(CacheSafetyError::InvalidInventory(
                "output has an empty attr, name, outPath, or drvPath".into(),
            ));
        }
        if !attrs.insert(output.attr.as_str()) {
            return Err(CacheSafetyError::InvalidInventory(format!(
                "duplicate output attr {}",
                output.attr
            )));
        }
    }
    verify_policy_equivalence(inventory)?;
    Ok(())
}

pub fn upload_eligible(inventory: &Inventory, attr: &str) -> bool {
    inventory.eligible.iter().any(|output| output.attr == attr)
}

pub fn cache_only_substitution_allowed(inventory: &Inventory, attr: &str) -> bool {
    upload_eligible(inventory, attr)
}

pub fn verify_policy_equivalence(inventory: &Inventory) -> Result<(), CacheSafetyError> {
    let excluded = |output: &Output| {
        inventory
            .policy
            .excluded_name_fragments
            .iter()
            .any(|fragment| output.name.contains(fragment))
    };
    if inventory.eligible.iter().any(excluded)
        || inventory
            .final_outputs
            .iter()
            .any(|output| !excluded(output))
    {
        return Err(CacheSafetyError::InvalidInventory(
            "Cachix exclusion policy does not exactly classify every inventoried output".into(),
        ));
    }
    Ok(())
}

pub fn verify_support_closure(
    closure: &BTreeSet<String>,
    final_outputs: &[Output],
) -> Result<(), CacheSafetyError> {
    for output in final_outputs {
        for path in [&output.out_path, &output.drv_path] {
            if closure.contains(path) {
                return Err(CacheSafetyError::SupportClosureContainsFinal {
                    final_attr: output.attr.clone(),
                    path: path.clone(),
                });
            }
        }
    }
    Ok(())
}

const CACHE_SAFETY_INVENTORY_ATTR: &str = "packages.x86_64-linux.cache-safety-inventory";

fn store_requisites(path: &str, include_outputs: bool) -> Result<BTreeSet<String>> {
    let mut command = Command::new("nix-store");
    command.args(["--query", "--requisites"]);
    if include_outputs {
        command.arg("--include-outputs");
    }
    let output = command
        .arg(path)
        .output()
        .with_context(|| format!("querying recursive store closure for {path}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "querying recursive store closure for {path} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8(output.stdout)
        .context("store closure was not UTF-8")
        .map(|raw| raw.lines().map(ToOwned::to_owned).collect())
}
pub fn verify_support_inputs(
    closure: &BTreeSet<String>,
    inputs: &SupportInputs,
) -> Result<(), CacheSafetyError> {
    for (input, path) in [
        ("Rust toolchain", &inputs.rust_toolchain),
        ("cargo-llvm-cov", &inputs.cargo_llvm_cov),
        ("cargo-nextest", &inputs.cargo_nextest),
    ] {
        if !closure.contains(path) {
            return Err(CacheSafetyError::SupportClosureMissingInput {
                input,
                path: path.clone(),
            });
        }
    }
    Ok(())
}

fn final_derivation_rejects_substitution(raw: &str) -> Result<(), CacheSafetyError> {
    let document: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| CacheSafetyError::InvalidInventory(error.to_string()))?;
    let derivations = document
        .get("derivations")
        .and_then(serde_json::Value::as_object)
        .filter(|derivations| derivations.len() == 1)
        .ok_or_else(|| {
            CacheSafetyError::InvalidInventory(
                "final derivation query did not return exactly one derivation".into(),
            )
        })?;
    let environment = derivations
        .values()
        .next()
        .and_then(|derivation| derivation.get("env"))
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            CacheSafetyError::InvalidInventory(
                "final derivation query omitted its environment".into(),
            )
        })?;
    if environment
        .get("allowSubstitutes")
        .and_then(serde_json::Value::as_str)
        != Some("")
        || environment
            .get("preferLocalBuild")
            .and_then(serde_json::Value::as_str)
            != Some("1")
    {
        return Err(CacheSafetyError::InvalidInventory(
            "final derivation permits substitution or does not prefer a local build".into(),
        ));
    }
    Ok(())
}

fn verify_final_derivation(repo: &Path, output: &Output) -> Result<()> {
    let evaluated_drv_path = nix::eval_installable_drvpath(repo, &output.attr)?;
    if evaluated_drv_path != output.drv_path {
        anyhow::bail!(
            "Nix cache-safety inventory recorded {} for {}, but evaluation returned {}",
            output.drv_path,
            output.attr,
            evaluated_drv_path
        );
    }
    let result = Command::new("nix")
        .args(["derivation", "show", &evaluated_drv_path])
        .output()
        .with_context(|| format!("querying final derivation {}", output.attr))?;
    if !result.status.success() {
        anyhow::bail!(
            "querying final derivation {} failed: {}",
            output.attr,
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let raw = String::from_utf8(result.stdout)
        .with_context(|| format!("final derivation {} was not UTF-8", output.attr))?;
    final_derivation_rejects_substitution(&raw)
        .with_context(|| format!("final derivation {} is cache-substitutable", output.attr))
}

pub fn verify(repo: &Path) -> Result<()> {
    let inventory_path = nix::build_installable_out_path(repo, CACHE_SAFETY_INVENTORY_ATTR)?;
    let inventory = parse_inventory(
        &fs::read_to_string(&inventory_path)
            .with_context(|| format!("reading Nix cache-safety inventory {inventory_path}"))?,
    )?;
    if inventory.eligible.iter().any(|support| {
        !upload_eligible(&inventory, &support.attr)
            || !cache_only_substitution_allowed(&inventory, &support.attr)
    }) || inventory.final_outputs.iter().any(|output| {
        upload_eligible(&inventory, &output.attr)
            || cache_only_substitution_allowed(&inventory, &output.attr)
    }) {
        anyhow::bail!("Nix-derived cache-safety policy does not fail closed");
    }

    for support in &inventory.eligible {
        let realized_support = nix::build_installable_out_path(repo, &support.attr)?;
        verify_support_closure(
            &store_requisites(&realized_support, false)?,
            &inventory.final_outputs,
        )?;
        let derivation_closure = store_requisites(&support.drv_path, true)?;
        verify_support_closure(&derivation_closure, &inventory.final_outputs)?;
        verify_support_inputs(&derivation_closure, &inventory.support_inputs)?;
    }
    for output in &inventory.final_outputs {
        verify_final_derivation(repo, output)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(attr: &str) -> Output {
        Output {
            attr: attr.into(),
            name: if attr.starts_with("packages.") {
                "jaunder-instrumented-test-archive".into()
            } else {
                "jaunder-coverage".into()
            },
            out_path: format!("/nix/store/{attr}-out"),
            drv_path: format!("/nix/store/{attr}.drv"),
        }
    }

    fn inventory() -> Inventory {
        Inventory {
            policy: CachePolicy {
                excluded_name_fragments: vec![
                    "jaunder-coverage".into(),
                    "jaunder-e2e".into(),
                    "jaunder-wasm-coverage".into(),
                    "jaunder-elisp-coverage".into(),
                ],
            },
            support_inputs: SupportInputs {
                rust_toolchain: "/nix/store/rust-toolchain".into(),
                cargo_llvm_cov: "/nix/store/cargo-llvm-cov".into(),
                cargo_nextest: "/nix/store/cargo-nextest".into(),
            },
            eligible: vec![output("packages.x86_64-linux.coverage-support")],
            final_outputs: vec![output("checks.x86_64-linux.coverage")],
        }
    }

    #[test]
    fn only_nix_enumerated_support_can_upload_or_substitute() {
        let inventory = inventory();
        assert!(upload_eligible(
            &inventory,
            "packages.x86_64-linux.coverage-support"
        ));
        assert!(cache_only_substitution_allowed(
            &inventory,
            "packages.x86_64-linux.coverage-support"
        ));
        assert!(!upload_eligible(&inventory, "checks.x86_64-linux.coverage"));
        assert!(!cache_only_substitution_allowed(
            &inventory,
            "checks.x86_64-linux.coverage"
        ));
    }

    #[test]
    fn policy_must_classify_each_inventory_member_exactly() {
        assert!(verify_policy_equivalence(&inventory()).is_ok());

        let mut support_excluded = inventory();
        support_excluded.eligible[0].name = "jaunder-coverage-support".into();
        assert!(verify_policy_equivalence(&support_excluded).is_err());

        let mut final_allowed = inventory();
        final_allowed.final_outputs[0].name = "jaunder-final-verdict".into();
        assert!(verify_policy_equivalence(&final_allowed).is_err());
    }

    #[test]
    fn inventory_requires_nonempty_disjoint_output_sets() {
        assert!(verify_inventory(&inventory()).is_ok());

        let mut missing_support = inventory();
        missing_support.eligible.clear();
        assert!(verify_inventory(&missing_support).is_err());

        let mut missing_final = inventory();
        missing_final.final_outputs.clear();
        assert!(verify_inventory(&missing_final).is_err());

        let mut two_support_outputs = inventory();
        two_support_outputs
            .eligible
            .push(output("packages.x86_64-linux.e2e-support"));
        assert!(verify_inventory(&two_support_outputs).is_ok());

        let mut duplicate = inventory();
        duplicate
            .final_outputs
            .push(output("checks.x86_64-linux.coverage"));
        assert!(verify_inventory(&duplicate).is_err());

        let mut regex_fragment = inventory();
        regex_fragment.policy.excluded_name_fragments[0] = "jaunder.*coverage".into();
        assert!(verify_inventory(&regex_fragment).is_err());
    }

    #[test]
    fn support_closure_rejects_final_runtime_and_derivation_inputs() {
        let inventory = inventory();
        assert!(
            verify_support_closure(
                &BTreeSet::from(["/nix/store/support".into()]),
                &inventory.final_outputs
            )
            .is_ok()
        );
        for path in [
            inventory.final_outputs[0].out_path.clone(),
            inventory.final_outputs[0].drv_path.clone(),
        ] {
            assert!(
                verify_support_closure(&BTreeSet::from([path]), &inventory.final_outputs).is_err()
            );
        }
    }

    #[test]
    fn support_derivation_closure_requires_compiler_and_coverage_tools() {
        let inventory = inventory();
        let complete = BTreeSet::from([
            inventory.support_inputs.rust_toolchain.clone(),
            inventory.support_inputs.cargo_llvm_cov.clone(),
            inventory.support_inputs.cargo_nextest.clone(),
        ]);
        assert!(verify_support_inputs(&complete, &inventory.support_inputs).is_ok());

        for missing in [
            &inventory.support_inputs.rust_toolchain,
            &inventory.support_inputs.cargo_llvm_cov,
            &inventory.support_inputs.cargo_nextest,
        ] {
            let mut incomplete = complete.clone();
            incomplete.remove(missing);
            assert!(verify_support_inputs(&incomplete, &inventory.support_inputs).is_err());
        }
    }

    #[test]
    fn final_derivation_must_disable_substitution() {
        let protected = r#"{"version":4,"derivations":{"final.drv":{"env":{"allowSubstitutes":"","preferLocalBuild":"1"}}}}"#;
        assert!(final_derivation_rejects_substitution(protected).is_ok());

        let substitutable = r#"{"version":4,"derivations":{"final.drv":{"env":{"allowSubstitutes":"1","preferLocalBuild":"1"}}}}"#;
        assert!(final_derivation_rejects_substitution(substitutable).is_err());

        let remote_preferred = r#"{"version":4,"derivations":{"final.drv":{"env":{"allowSubstitutes":"","preferLocalBuild":""}}}}"#;
        assert!(final_derivation_rejects_substitution(remote_preferred).is_err());
    }
}
