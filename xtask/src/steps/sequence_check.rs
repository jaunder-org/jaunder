//! The `identifier-collisions` static check: scans the ADR and migration
//! directories for duplicate numeric prefixes (which git merges silently because
//! the filenames differ) and for sqlite/postgres backend parity. Read-only in
//! every mode — resolution for ADRs is the separate `adr renumber` command.

use std::path::Path;

use crate::ids;
use crate::result::{CommandResult, StepResult};

const ADR_DIR: &str = "docs/adr";
const SQLITE_DIR: &str = "storage/migrations/sqlite";
const PG_DIR: &str = "storage/migrations/postgres";

/// Filenames of regular files directly in `dir`. A missing directory yields an
/// empty list (the check is a no-op rather than an error).
fn filenames(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// The failure detail for all collisions/parity problems, or `None` when clean.
/// Pure given the three filename lists, so it is unit-tested directly.
pub fn problems(adr: &[String], sqlite: &[String], postgres: &[String]) -> Option<String> {
    let mut lines = Vec::new();

    let adr_dups = ids::duplicate_prefixes(adr);
    for (number, files) in &adr_dups {
        lines.push(format!(
            "ADR number {number:04} is used by multiple files: {}",
            files.join(", ")
        ));
    }
    if !adr_dups.is_empty() {
        lines.push("  recovery: cargo xtask adr renumber".to_string());
    }

    for (number, files) in ids::duplicate_prefixes(sqlite) {
        lines.push(format!(
            "sqlite migration {number:04} is used by multiple files: {}",
            files.join(", ")
        ));
    }
    for (number, files) in ids::duplicate_prefixes(postgres) {
        lines.push(format!(
            "postgres migration {number:04} is used by multiple files: {}",
            files.join(", ")
        ));
    }
    for mismatch in ids::parity_mismatch(sqlite, postgres) {
        lines.push(format!("migration backend parity: {mismatch}"));
    }

    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Scan the repo's identifier directories and push the result step.
pub fn run(result: &mut CommandResult) {
    let adr = filenames(Path::new(ADR_DIR));
    let sqlite = filenames(Path::new(SQLITE_DIR));
    let postgres = filenames(Path::new(PG_DIR));
    let step = match problems(&adr, &sqlite, &postgres) {
        None => StepResult::ok("identifier-collisions"),
        Some(detail) => StepResult::fail("identifier-collisions").detail(detail),
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_dirs_report_no_problems() {
        let adr = vec!["0001-a.md".to_string(), "0002-b.md".to_string()];
        let mig = vec!["0001_x.sql".to_string()];
        assert_eq!(problems(&adr, &mig, &mig), None);
    }

    #[test]
    fn adr_collision_includes_recovery_command() {
        let adr = vec!["0034-foo.md".to_string(), "0034-bar.md".to_string()];
        let detail = problems(&adr, &[], &[]).expect("a problem");
        assert!(detail.contains("ADR number 0034"));
        assert!(detail.contains("0034-bar.md"));
        assert!(detail.contains("cargo xtask adr renumber"));
    }

    #[test]
    fn migration_collision_has_no_adr_recovery_line() {
        let mig = vec!["0007_a.sql".to_string(), "0007_b.sql".to_string()];
        let detail = problems(&[], &mig, &mig).expect("a problem");
        assert!(detail.contains("sqlite migration 0007"));
        assert!(!detail.contains("cargo xtask adr renumber"));
    }

    #[test]
    fn parity_gap_is_reported() {
        let sqlite = vec!["0001_a.sql".to_string()];
        let postgres = vec!["0001_a.sql".to_string(), "0002_b.sql".to_string()];
        let detail = problems(&[], &sqlite, &postgres).expect("a problem");
        assert!(detail.contains("backend parity"));
        assert!(detail.contains("0002_b (postgres only)"));
    }

    #[test]
    fn filenames_skips_the_drafts_subdir() {
        // A numberless ADR draft under `docs/adr/drafts/` must be invisible to the
        // `identifier-collisions` scan: `filenames` is non-recursive and file-only,
        // so the `drafts` subdirectory entry (and anything inside it) is excluded
        // (#219). Locks the enumeration rule the drafts-out-of-git flow relies on.
        let dir =
            std::env::temp_dir().join(format!("jaunder-seqcheck-drafts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("drafts")).unwrap();
        std::fs::write(dir.join("0001-a.md"), "x").unwrap();
        std::fs::write(dir.join("drafts/some-decision.md"), "x").unwrap();

        let names = filenames(&dir);
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(names, vec!["0001-a.md".to_string()]);
    }
}
