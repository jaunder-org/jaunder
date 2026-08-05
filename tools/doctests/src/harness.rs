//! Materializes a one-file fixture crate into a tempdir and runs its doctests.
//!
//! The rest of this crate's tests feed inline `r#"…"#` sources to the pure seams,
//! which is the repo's convention. These cannot: they assert what **rustdoc and
//! cargo actually do** — that an unrecognized info string is dropped silently, that
//! `cfg(test)` is not set for doc runs, that a bin-only crate yields no doctests —
//! and the only honest way to establish that is to compile a real crate and look.
//!
//! Fixtures live in `testdata/` and are `include_str!`'d, following the
//! `xtask/src/pr/testdata/` convention. They are dependency-free so this needs no
//! registry access.

use std::process::Command;

/// Write `source` as a standalone crate under a fresh tempdir and run its
/// doctests, returning the combined output.
///
/// `entry` is `src/lib.rs` or `src/main.rs`; a `main.rs`-only crate has no lib
/// target, which is itself one of the things under test.
fn run_crate(name: &str, entry: &str, source: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).expect("src dir");
    std::fs::write(root.join("src").join(entry), source).expect("write source");
    // `[workspace]` keeps the fixture from being adopted by any ancestor
    // workspace; `[features] off` exists so the cfg-feature fixture has a real,
    // declared, NOT-enabled feature to hide behind.
    std::fs::write(
        root.join("Cargo.toml"),
        format!(
            "[workspace]\n\n\
             [package]\n\
             name = \"{name}\"\n\
             version = \"0.0.0\"\n\
             edition = \"2021\"\n\n\
             [features]\n\
             off = []\n"
        ),
    )
    .expect("write manifest");

    let output = Command::new("cargo")
        .args(["test", "--doc"])
        .current_dir(root)
        .output()
        .expect("spawn cargo");
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (dir, combined)
}

/// Run `source` as a lib crate's doctests.
pub fn run_fixture(name: &str, source: &str) -> (tempfile::TempDir, String) {
    run_crate(name, "lib.rs", source)
}

/// Run `source` as a crate with **only** a bin target — no lib, so cargo collects
/// no doctests from it at all.
pub fn run_bin_fixture(name: &str, source: &str) -> (tempfile::TempDir, String) {
    run_crate(name, "main.rs", source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::{Kind, ScannedFile, problems};
    use crate::libtest::run_entries;

    // `.rs.txt`, not `.rs`: these are test DATA, not any crate's source. Under a
    // `.rs` name the doctest gate would (correctly) see their deliberately-broken
    // fences as part of the population it polices and report every one as never
    // run — and the recovery it suggests, marking them ```text, would destroy the
    // very thing they exist to pin. Keeping them out of the population
    // structurally beats carving a `testdata/` hole in it, and matches the repo's
    // other fixture trees, which are never `.rs` either.
    const ORDERING_CONTROL: &str = include_str!("../testdata/ordering_control.rs.txt");
    const CFG_FEATURE: &str = include_str!("../testdata/cfg_feature.rs.txt");
    const CFG_TEST_MODULE: &str = include_str!("../testdata/cfg_test_module.rs.txt");
    const UNKNOWN_TAG: &str = include_str!("../testdata/unknown_tag.rs.txt");
    const FAILING: &str = include_str!("../testdata/failing.rs.txt");
    const BIN_ONLY: &str = include_str!("../testdata/bin_only.rs.txt");

    fn scanned(source: &str) -> Vec<ScannedFile> {
        vec![ScannedFile {
            path: "src/lib.rs".to_string(),
            run_path: "src/lib.rs".to_string(),
            source: source.to_string(),
        }]
    }

    fn kinds(source: &str, output: &str) -> Vec<Kind> {
        problems(&scanned(source), output)
            .into_iter()
            .map(|v| v.kind)
            .collect()
    }

    #[test]
    fn the_compiler_fact_the_ordering_proofs_rest_on_holds() {
        // With `PartialEq + Eq` present, `a < b` can only fail for the missing
        // `PartialOrd` — which is what makes Task 12's three proofs discriminate
        // rather than merely document intent. The control fence proves the
        // un-suppressed shape DOES order, so the negative is not vacuous.
        let (_dir, out) = run_fixture("ordering_control", ORDERING_CONTROL);
        let e = run_entries(&out);
        assert_eq!(e.len(), 2, "{out}");
        assert!(e.iter().all(|x| !x.ignored && !x.failed), "{out}");
        assert!(kinds(ORDERING_CONTROL, &out).is_empty(), "{out}");
    }

    #[test]
    fn vector_1_a_fence_behind_an_unenabled_feature_is_not_run() {
        // The `sanitize` case that made the issue's own measurement wrong.
        let (_dir, out) = run_fixture("cfg_feature", CFG_FEATURE);
        assert!(run_entries(&out).is_empty(), "{out}");
        assert_eq!(kinds(CFG_FEATURE, &out), vec![Kind::NotRun], "{out}");
    }

    #[test]
    fn vector_2_a_fence_in_a_cfg_test_module_is_not_run() {
        // rustdoc sets cfg(doctest), not cfg(test) — web/src/reactive/scope.rs.
        let (_dir, out) = run_fixture("cfg_test_module", CFG_TEST_MODULE);
        assert!(run_entries(&out).is_empty(), "{out}");
        assert_eq!(kinds(CFG_TEST_MODULE, &out), vec![Kind::NotRun], "{out}");
    }

    #[test]
    fn vector_3_an_unrecognized_info_string_is_silently_uncollected() {
        // Both halves must catch it: the vocabulary rejects the tag, and the
        // reconciler notices the fence never ran. Either alone lets it through —
        // and rustdoc emits no warning, so nothing else would.
        let (_dir, out) = run_fixture("unknown_tag", UNKNOWN_TAG);
        assert!(run_entries(&out).is_empty(), "{out}");
        let ks = kinds(UNKNOWN_TAG, &out);
        assert!(ks.contains(&Kind::BannedAttribute), "{ks:?}\n{out}");
        assert!(ks.contains(&Kind::NotRun), "{ks:?}\n{out}");
    }

    #[test]
    fn vector_5_a_crate_with_no_lib_target_yields_no_doctests() {
        // cargo collects doctests from lib targets only — tools/devtool has
        // src/main.rs and no src/lib.rs, so a fence there could never run.
        let (_dir, out) = run_bin_fixture("bin_only", BIN_ONLY);
        assert!(run_entries(&out).is_empty(), "{out}");
    }

    #[test]
    fn a_failing_doctest_is_reported_as_failed_against_a_real_run() {
        let (_dir, out) = run_fixture("failing", FAILING);
        let v = problems(&scanned(FAILING), &out);
        assert_eq!(v.len(), 1, "{v:?}\n{out}");
        assert_eq!(v[0].kind, Kind::Failed);
    }
}
