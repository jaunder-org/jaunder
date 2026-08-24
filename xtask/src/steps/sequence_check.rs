//! The `identifier-collisions` static check: scans the ADR and migration
//! directories for duplicate numeric prefixes (which git merges silently because
//! the filenames differ) and for sqlite/postgres backend parity. Read-only in
//! every mode. Tracked feature drafts avoid ADR collisions; the legacy
//! `adr renumber` compatibility command is deprecated pending #1169.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::ids;
use crate::result::{CommandResult, StepResult};

const ADR_DIR: &str = "docs/adr";
const SQLITE_DIR: &str = "storage/migrations/sqlite";
const PG_DIR: &str = "storage/migrations/postgres";

/// Filenames of regular files directly in `dir`. Missing/unreadable directories
/// and unreadable entries are correctness failures, never empty populations.
fn filenames(dir: &Path) -> Result<Vec<String>> {
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("reading directory {}", dir.display()))?;
    filenames_from_entries(
        dir,
        entries,
        std::fs::DirEntry::path,
        std::fs::DirEntry::file_type,
        |entry| entry.file_name(),
    )
}

fn filenames_from_entries<T>(
    dir: &Path,
    entries: impl IntoIterator<Item = std::io::Result<T>>,
    path_of: impl Fn(&T) -> PathBuf,
    file_type: impl Fn(&T) -> std::io::Result<std::fs::FileType>,
    file_name: impl Fn(T) -> OsString,
) -> Result<Vec<String>> {
    entries
        .into_iter()
        .map(|entry| entry.with_context(|| format!("reading entry under {}", dir.display())))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .map(|entry| {
            let path = path_of(&entry);
            let is_file = file_type(&entry)
                .with_context(|| format!("reading file type {}", path.display()))?
                .is_file();
            Ok(is_file.then(|| file_name(entry).to_string_lossy().into_owned()))
        })
        .collect::<Result<Vec<_>>>()
        .map(|names| names.into_iter().flatten().collect())
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
        lines.push(
            "  recovery: new ADRs must be tracked numberless drafts; diagnose why numbered files bypassed the serialized promoter"
                .to_string(),
        );
        lines.push(
            "  deprecated compatibility only: cargo xtask adr renumber (removal: https://github.com/jaunder-org/jaunder/issues/1169)"
                .to_string(),
        );
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
    let read = || -> Result<(Vec<String>, Vec<String>, Vec<String>)> {
        Ok((
            filenames(Path::new(ADR_DIR))?,
            filenames(Path::new(SQLITE_DIR))?,
            filenames(Path::new(PG_DIR))?,
        ))
    };
    let (adr, sqlite, postgres) = match read() {
        Ok(populations) => populations,
        Err(error) => {
            result.push(
                StepResult::fail("identifier-collisions")
                    .detail(format!("cannot enumerate identifier population: {error:#}")),
            );
            return;
        }
    };
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
    fn adr_collision_points_to_tracked_drafts_and_deprecates_legacy_recovery() {
        let adr = vec!["0034-foo.md".to_string(), "0034-bar.md".to_string()];
        let detail = problems(&adr, &[], &[]).expect("a problem");
        assert!(detail.contains("ADR number 0034"));
        assert!(detail.contains("0034-bar.md"));
        assert!(detail.contains("tracked numberless drafts"));
        assert!(detail.contains("deprecated compatibility only"));
        assert!(detail.contains("https://github.com/jaunder-org/jaunder/issues/1169"));
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
        // so the `drafts` subdirectory entry (and anything inside it) is excluded.
        // This is the numbered-gate boundary tracked feature drafts rely on.
        let dir =
            std::env::temp_dir().join(format!("jaunder-seqcheck-drafts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("drafts")).unwrap();
        std::fs::write(dir.join("0001-a.md"), "x").unwrap();
        std::fs::write(dir.join("drafts/some-decision.md"), "x").unwrap();

        let names = filenames(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(names, vec!["0001-a.md".to_string()]);
    }

    #[test]
    fn fail_closed_population_missing_adr_and_migration_directories() {
        for path in ["missing/docs/adr", "missing/storage/migrations/sqlite"] {
            let error = filenames(Path::new(path)).unwrap_err();
            assert!(format!("{error:#}").contains(path));
            assert_eq!(
                error
                    .downcast_ref::<std::io::Error>()
                    .map(std::io::Error::kind),
                Some(std::io::ErrorKind::NotFound)
            );
        }
    }

    #[test]
    fn fail_closed_population_unreadable_adr_and_migration_file_types() {
        struct Fake {
            path: PathBuf,
            name: OsString,
        }
        for dir in [Path::new(ADR_DIR), Path::new(SQLITE_DIR)] {
            let entry = Fake {
                path: dir.join("0001-entry"),
                name: OsString::from("0001-entry"),
            };
            let error = filenames_from_entries(
                dir,
                [Ok(entry)],
                |entry| entry.path.clone(),
                |_| {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "injected",
                    ))
                },
                |entry| entry.name,
            )
            .unwrap_err();
            assert!(format!("{error:#}").contains("0001-entry"));
            assert_eq!(
                error
                    .downcast_ref::<std::io::Error>()
                    .map(std::io::Error::kind),
                Some(std::io::ErrorKind::PermissionDenied)
            );
        }
    }
}
