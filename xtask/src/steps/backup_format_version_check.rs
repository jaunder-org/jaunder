//! Guards the explicit backup-format compatibility acknowledgement.
//!
//! The inventory freezes the representation-defining backup sources by their
//! working-tree Git blob OIDs. Changing one is deliberately not self-approving:
//! maintainers must review it and update the recorded inventory, and incompatible
//! changes must also advance the independent backup format version.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::Path,
};

use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, Visitor},
};
use syn::{Expr, ExprLit, Item, Lit};

use crate::{
    git,
    result::{CommandResult, StepResult},
};

const STEP: &str = "backup-format-version";
const INVENTORY: &str = "storage/backup-format-sources.json";
const FORMAT_SOURCE: &str = "storage/src/backup/format.rs";
const CURRENT_CONSTANT: &str = "CURRENT_BACKUP_FORMAT_VERSION";
const WATCHED_SOURCES: &[&str] = &[
    "storage/src/backup/archive.rs",
    "storage/src/backup/catalog.rs",
    FORMAT_SOURCE,
    "storage/src/backup/media.rs",
    "storage/src/backup/orchestration.rs",
    "storage/src/backup/restore_bind.rs",
    "storage/src/backup/restore_validation.rs",
    "storage/src/sqlite/backup.rs",
    "storage/src/postgres/backup.rs",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    format_version: u32,
    #[serde(deserialize_with = "deserialize_sources")]
    sources: BTreeMap<String, String>,
}

fn deserialize_sources<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct SourcesVisitor;

    impl<'de> Visitor<'de> for SourcesVisitor {
        type Value = BTreeMap<String, String>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a source-to-Git-blob-OID map with unique source paths")
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let mut sources = BTreeMap::new();
            while let Some((path, oid)) = map.next_entry::<String, String>()? {
                if sources.contains_key(&path) {
                    return Err(de::Error::custom(format!(
                        "duplicate backup format source `{path}`"
                    )));
                }
                sources.insert(path, oid);
            }
            Ok(sources)
        }
    }

    deserializer.deserialize_map(SourcesVisitor)
}

fn current_backup_format_version(source: &str) -> Result<u32, String> {
    let file = syn::parse_file(source)
        .map_err(|error| format!("cannot parse {FORMAT_SOURCE}: {error}"))?;
    let constants = file
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Const(item) if item.ident == CURRENT_CONSTANT => Some(item),
            _ => None,
        })
        .collect::<Vec<_>>();

    let [constant] = constants.as_slice() else {
        return Err(match constants.len() {
            0 => format!("missing `{CURRENT_CONSTANT}` in {FORMAT_SOURCE}"),
            _ => format!("duplicate `{CURRENT_CONSTANT}` declarations in {FORMAT_SOURCE}"),
        });
    };
    let Expr::Lit(ExprLit {
        lit: Lit::Int(value),
        ..
    }) = constant.expr.as_ref()
    else {
        return Err(format!(
            "`{CURRENT_CONSTANT}` in {FORMAT_SOURCE} must be an integer literal"
        ));
    };
    value.base10_parse::<u32>().map_err(|error| {
        format!(
            "`{CURRENT_CONSTANT}` in {FORMAT_SOURCE} is not a valid u32 integer literal: {error}"
        )
    })
}

fn expected_sources() -> BTreeSet<String> {
    WATCHED_SOURCES
        .iter()
        .map(|source| (*source).to_owned())
        .collect()
}

fn valid_blob_oid(oid: &str) -> bool {
    matches!(oid.len(), 40 | 64) && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate(
    inventory_json: &str,
    format_source: &str,
    actual_hashes: &BTreeMap<String, String>,
) -> Vec<String> {
    let inventory: Inventory = match serde_json::from_str(inventory_json) {
        Ok(inventory) => inventory,
        Err(error) => return vec![format!("cannot parse {INVENTORY}: {error}")],
    };
    let current_version = match current_backup_format_version(format_source) {
        Ok(version) => version,
        Err(error) => return vec![error],
    };
    let expected_sources = expected_sources();
    let inventory_sources = inventory.sources.keys().cloned().collect::<BTreeSet<_>>();
    if inventory_sources != expected_sources {
        return vec![format!(
            "{INVENTORY} source set must exactly match the watched backup sources; expected {expected_sources:?}, found {inventory_sources:?}"
        )];
    }
    if let Some((source, oid)) = inventory
        .sources
        .iter()
        .find(|(_, oid)| !valid_blob_oid(oid))
    {
        return vec![format!(
            "{INVENTORY} records invalid Git blob OID `{oid}` for `{source}`"
        )];
    }
    let actual_sources = actual_hashes.keys().cloned().collect::<BTreeSet<_>>();
    if actual_sources != expected_sources {
        return vec![format!(
            "backup source hashing did not cover the exact watched source set; expected {expected_sources:?}, found {actual_sources:?}"
        )];
    }
    if inventory.format_version != current_version {
        return vec![format!(
            "incompatible format change: `{CURRENT_CONSTANT}` is {current_version}, but {INVENTORY} records {}; bump the explicit version and update the inventory version and reviewed hashes together",
            inventory.format_version
        )];
    }

    let drift = WATCHED_SOURCES
        .iter()
        .filter_map(|source| {
            let expected = &inventory.sources[*source];
            let actual = &actual_hashes[*source];
            (expected != actual)
                .then(|| format!("{source}: inventory {expected}, working tree {actual}"))
        })
        .collect::<Vec<_>>();
    if drift.is_empty() {
        Vec::new()
    } else {
        vec![format!(
            "backup representation source drift:\n  {}\n  compatible acknowledgement: after review, refresh only the affected hashes in {INVENTORY}\n  incompatible format change: bump `{CURRENT_CONSTANT}` and update {INVENTORY}'s version and reviewed hashes together",
            drift.join("\n  ")
        )]
    }
}

fn step_for(
    inventory_json: &str,
    format_source: &str,
    actual_hashes: &BTreeMap<String, String>,
) -> StepResult {
    let problems = validate(inventory_json, format_source, actual_hashes);
    if problems.is_empty() {
        StepResult::ok(STEP)
    } else {
        StepResult::fail(STEP).detail(problems.join("\n"))
    }
}

fn working_tree_hashes(repo_root: &Path) -> Result<BTreeMap<String, String>, String> {
    WATCHED_SOURCES
        .iter()
        .map(|source| {
            git::output(repo_root, &["hash-object", source])
                .map(|hash| ((*source).to_owned(), hash))
                .map_err(|error| format!("cannot hash {source}: {error}"))
        })
        .collect()
}

/// Validate that backup format source changes have an explicit compatibility acknowledgement.
pub fn run(result: &mut CommandResult) {
    let step = (|| {
        let repo_root = git::toplevel(Path::new("."))
            .map_err(|error| format!("cannot locate repository root: {error}"))?;
        let repo_root = Path::new(&repo_root);
        let inventory = std::fs::read_to_string(repo_root.join(INVENTORY))
            .map_err(|error| format!("cannot read {INVENTORY}: {error}"))?;
        let format_source = std::fs::read_to_string(repo_root.join(FORMAT_SOURCE))
            .map_err(|error| format!("cannot read {FORMAT_SOURCE}: {error}"))?;
        let hashes = working_tree_hashes(repo_root)?;
        Ok::<_, String>(step_for(&inventory, &format_source, &hashes))
    })();
    result.push(match step {
        Ok(step) => step,
        Err(error) => StepResult::fail(STEP).detail(error),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hashes() -> BTreeMap<String, String> {
        WATCHED_SOURCES
            .iter()
            .enumerate()
            .map(|(index, source)| ((*source).to_owned(), format!("{index:040x}")))
            .collect()
    }

    fn inventory(version: u32, hashes: &BTreeMap<String, String>) -> String {
        serde_json::to_string(&serde_json::json!({
            "format_version": version,
            "sources": hashes,
        }))
        .expect("test inventory serializes")
    }

    fn format_source(value: &str) -> String {
        format!("pub(crate) const {CURRENT_CONSTANT}: u32 = {value};")
    }

    #[test]
    fn accepts_clean_inventory() {
        let hashes = hashes();
        assert!(validate(&inventory(1, &hashes), &format_source("1"), &hashes).is_empty());
    }

    #[test]
    fn rejects_source_drift_with_both_recovery_paths() {
        let hashes = hashes();
        let mut actual = hashes.clone();
        actual.insert(FORMAT_SOURCE.to_owned(), "changed".to_owned());
        let problem = validate(&inventory(1, &hashes), &format_source("1"), &actual).join("\n");
        assert!(problem.contains("compatible acknowledgement"));
        assert!(problem.contains("incompatible format change"));
    }

    #[test]
    fn rejects_inventory_version_mismatch() {
        let hashes = hashes();
        let problem = validate(&inventory(1, &hashes), &format_source("2"), &hashes).join("\n");
        assert!(problem.contains("incompatible format change"));
    }

    #[test]
    fn rejects_inventory_source_set_mismatch() {
        let hashes = hashes();
        let mut incomplete = hashes.clone();
        incomplete.remove(FORMAT_SOURCE);
        let problem = validate(&inventory(1, &incomplete), &format_source("1"), &hashes).join("\n");
        assert!(problem.contains("source set must exactly match"));
    }

    #[test]
    fn rejects_malformed_inventory() {
        let hashes = hashes();
        let problem = validate(
            r#"{"format_version":1,"sources":{"storage/src/backup/archive.rs":"0000000000000000000000000000000000000000","storage/src/backup/archive.rs":"0000000000000000000000000000000000000000"}}"#,
            &format_source("1"),
            &hashes,
        )
        .join("\n");
        assert!(problem.contains("cannot parse"));
    }

    #[test]
    fn rejects_missing_duplicate_and_nonliteral_current_constant() {
        assert!(current_backup_format_version("const OTHER: u32 = 1;").is_err());
        assert!(
            current_backup_format_version(&format!(
                "const {CURRENT_CONSTANT}: u32 = 1; const {CURRENT_CONSTANT}: u32 = 1;"
            ))
            .is_err()
        );
        assert!(
            current_backup_format_version(&format!(
                "const {CURRENT_CONSTANT}: u32 = LEGACY_BACKUP_FORMAT_VERSION;"
            ))
            .is_err()
        );
    }
}
