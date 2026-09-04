//! Compiler-backed boundary check for `common::render::RenderedHtml`.
//!
//! A temporary crate is deliberately outside every workspace. Cargo therefore resolves
//! `common` as a downstream production dependency with its default features disabled,
//! rather than inheriting test-support through another workspace member. The positive
//! fixture proves that ordinary downstream use resolves; the negative fixtures prove
//! privacy rejects both raw construction and the test-only fixture helper.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::result::{CommandResult, StepResult};

const STEP: &str = "rendered-html-compiler-boundary";

const POSITIVE: &str = r#"
pub fn read(html: &common::render::RenderedHtml) -> &str {
    html.as_ref()
}
"#;

const RAW_CONSTRUCTION: &str = r#"
pub fn forge() {
    let _ = common::render::RenderedHtml("<p>untrusted</p>".to_owned());
}
"#;

const TEST_HELPER: &str = r#"
pub fn fixture() {
    let _ = common::test_support::rendered_html("<p>fixture</p>");
}
"#;

struct Fixture {
    label: &'static str,
    source: &'static str,
    expected: fn(&str) -> bool,
}

fn succeeds(_: &str) -> bool {
    true
}

fn rejects_raw_construction(diagnostic: &str) -> bool {
    diagnostic.contains("RenderedHtml") && diagnostic.contains("private")
}

fn rejects_test_helper(diagnostic: &str) -> bool {
    diagnostic.contains("test_support")
        && (diagnostic.contains("could not find") || diagnostic.contains("gated"))
}

fn manifest(common: &Path) -> String {
    format!(
        "[package]\nname = \"rendered-html-boundary-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[dependencies]\ncommon = {{ path = {common:?}, default-features = false }}\n# tinyvec 1.13.0 breaks alloc without std (upstream #225).\n# Jaunder #1362 tracks removal after upstream tinyvec PR #226 ships.\ntinyvec = \"=1.11.0\"\n",
        common = common.display().to_string(),
    )
}

fn cargo_check(manifest: &Path, target: &Path) -> std::io::Result<(bool, String)> {
    let output = Command::new("cargo")
        .args(["check", "--offline", "--quiet", "--manifest-path"])
        .arg(manifest)
        .env("CARGO_TARGET_DIR", target)
        .output()?;
    let mut diagnostic = String::from_utf8_lossy(&output.stdout).into_owned();
    diagnostic.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok((output.status.success(), diagnostic))
}

fn diagnostic_tail(diagnostic: &str) -> &str {
    const LIMIT: usize = 4_000;
    let start = diagnostic
        .char_indices()
        .rev()
        .nth(LIMIT)
        .map_or(0, |(index, _)| index);
    &diagnostic[start..]
}

fn prepare_fixture(root: &Path, temporary: &Path) -> std::io::Result<(PathBuf, PathBuf)> {
    let manifest_path = temporary.join("Cargo.toml");
    let source_path = temporary.join("src/lib.rs");
    fs::create_dir(temporary.join("src"))?;
    fs::write(&manifest_path, manifest(&root.join("common")))?;
    // Keep downstream feature resolution independent while anchoring transitive
    // versions to the repository's reviewed dependency graph.
    fs::copy(root.join("Cargo.lock"), temporary.join("Cargo.lock"))?;
    Ok((manifest_path, source_path))
}

fn check() -> std::result::Result<(), String> {
    let root =
        PathBuf::from(crate::git::toplevel(Path::new(".")).map_err(|error| error.to_string())?);
    let temporary = tempfile::tempdir().map_err(|error| error.to_string())?;
    let (manifest_path, source_path) =
        prepare_fixture(&root, temporary.path()).map_err(|error| error.to_string())?;

    let fixtures = [
        Fixture {
            label: "ordinary dependency resolution",
            source: POSITIVE,
            expected: succeeds,
        },
        Fixture {
            label: "raw constructor privacy",
            source: RAW_CONSTRUCTION,
            expected: rejects_raw_construction,
        },
        Fixture {
            label: "test-support feature confinement",
            source: TEST_HELPER,
            expected: rejects_test_helper,
        },
    ];

    for fixture in fixtures {
        fs::write(&source_path, fixture.source).map_err(|error| error.to_string())?;
        let (success, diagnostic) =
            cargo_check(&manifest_path, &temporary.path().join("target"))
                .map_err(|error| format!("{label}: {error}", label = fixture.label))?;
        let observed = if fixture.label == "ordinary dependency resolution" {
            success
        } else {
            !success && (fixture.expected)(&diagnostic)
        };
        if !observed {
            return Err(format!(
                "{label} did not produce its compiler contract:\n{diagnostic}",
                label = fixture.label,
                diagnostic = diagnostic_tail(&diagnostic),
            ));
        }
    }
    Ok(())
}

/// Runs the isolated downstream compiler contracts as part of the normal host feedback.
pub fn run(result: &mut CommandResult) {
    match check() {
        Ok(()) => result.push(StepResult::ok(STEP)),
        Err(detail) => result.push(StepResult::fail(STEP).detail(detail)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_manifest_disables_default_features() {
        let rendered = manifest(Path::new("/repo/common"));
        assert!(rendered.contains("default-features = false"));
        assert!(rendered.contains("path = \"/repo/common\""));
        assert!(rendered.contains("tinyvec = \"=1.11.0\""));
    }

    #[test]
    fn standalone_fixture_anchors_versions_to_the_workspace_lockfile() {
        let root = tempfile::tempdir().expect("root");
        fs::create_dir(root.path().join("common")).expect("common directory");
        fs::write(root.path().join("Cargo.lock"), "reviewed dependency graph")
            .expect("root lockfile");
        let fixture = tempfile::tempdir().expect("fixture");

        let (manifest_path, source_path) =
            prepare_fixture(root.path(), fixture.path()).expect("prepare fixture");

        assert_eq!(
            fs::read_to_string(fixture.path().join("Cargo.lock")).expect("fixture lockfile"),
            "reviewed dependency graph"
        );
        assert!(source_path.parent().expect("source parent").is_dir());
        assert!(
            fs::read_to_string(manifest_path)
                .expect("fixture manifest")
                .contains(root.path().join("common").to_string_lossy().as_ref())
        );
    }

    #[test]
    fn raw_constructor_contract_requires_privacy_diagnostic() {
        assert!(rejects_raw_construction(
            "error[E0423]: cannot initialize a tuple struct which contains private fields: RenderedHtml"
        ));
        assert!(!rejects_raw_construction("error: unrelated failure"));
    }

    #[test]
    fn helper_contract_requires_missing_test_support_diagnostic() {
        assert!(rejects_test_helper(
            "error[E0433]: could not find `test_support` in `common`"
        ));
        assert!(!rejects_test_helper("error: unrelated failure"));
    }
}
