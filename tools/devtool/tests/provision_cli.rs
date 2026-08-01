//! CLI-surface tests for `devtool provision-node-modules` — the flag parsing, env
//! fallback and cwd handling that `devtool check tsc`'s in-process call bypasses
//! (#229). Without these, nothing automated would exercise the clap layer at all.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const EXE: &str = env!("CARGO_BIN_EXE_devtool");

fn tmp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("devtool-provision-cli-")
        .tempdir()
        .unwrap()
}

fn fake_types_dir(base: &Path, marker: &str) -> PathBuf {
    let dir = base.join(marker);
    fs::create_dir_all(dir.join("typescript")).unwrap();
    dir
}

fn fake_playwright(base: &Path) -> PathBuf {
    let dir = base.join("playwright-test");
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn help_lists_the_flags_and_their_env_fallback() {
    let out = Command::new(EXE)
        .args(["provision-node-modules", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let help = String::from_utf8(out.stdout).unwrap();
    for needle in [
        "--types-node-modules",
        "--playwright-test",
        "--root",
        "E2E_TYPES_NODE_MODULES",
        "E2E_PLAYWRIGHT_TEST",
    ] {
        assert!(
            help.contains(needle),
            "help should mention {needle}:\n{help}"
        );
    }
}

#[test]
fn defaults_from_the_environment_and_the_current_directory() {
    let t = tmp();
    let types = fake_types_dir(t.path(), "store");
    let pw = fake_playwright(t.path());
    let work = t.path().join("work");
    fs::create_dir_all(&work).unwrap();

    let st = Command::new(EXE)
        .arg("provision-node-modules")
        .current_dir(&work)
        .env("E2E_TYPES_NODE_MODULES", &types)
        .env("E2E_PLAYWRIGHT_TEST", &pw)
        .status()
        .unwrap();

    assert!(st.success());
    assert_eq!(
        fs::read_link(work.join("end2end/node_modules/typescript")).unwrap(),
        types.join("typescript")
    );
}

#[test]
fn root_flag_targets_another_tree_and_leaves_cwd_alone() {
    let t = tmp();
    let types = fake_types_dir(t.path(), "store");
    let pw = fake_playwright(t.path());
    let work = t.path().join("work");
    let target = t.path().join("target");
    fs::create_dir_all(&work).unwrap();
    fs::create_dir_all(&target).unwrap();

    let st = Command::new(EXE)
        .args(["provision-node-modules", "--root"])
        .arg(&target)
        .current_dir(&work)
        .env("E2E_TYPES_NODE_MODULES", &types)
        .env("E2E_PLAYWRIGHT_TEST", &pw)
        .status()
        .unwrap();

    assert!(st.success());
    assert!(target.join("end2end/node_modules/typescript").exists());
    assert!(
        !work.join("end2end").exists(),
        "--root must not provision the current directory"
    );
}

#[test]
fn explicit_flags_beat_the_environment() {
    let t = tmp();
    let decoy = fake_types_dir(t.path(), "decoy");
    let real = fake_types_dir(t.path(), "real");
    let pw = fake_playwright(t.path());
    let work = t.path().join("work");
    fs::create_dir_all(&work).unwrap();

    let st = Command::new(EXE)
        .args(["provision-node-modules", "--types-node-modules"])
        .arg(&real)
        .arg("--playwright-test")
        .arg(&pw)
        .current_dir(&work)
        .env("E2E_TYPES_NODE_MODULES", &decoy)
        .env("E2E_PLAYWRIGHT_TEST", t.path().join("decoy-playwright"))
        .status()
        .unwrap();

    assert!(st.success());
    assert_eq!(
        fs::read_link(work.join("end2end/node_modules/typescript")).unwrap(),
        real.join("typescript")
    );
    assert_eq!(
        fs::read_link(work.join("end2end/node_modules/@playwright/test")).unwrap(),
        pw
    );
}

#[test]
fn unset_environment_names_the_variable_and_the_devshell() {
    let t = tmp();
    let out = Command::new(EXE)
        .arg("provision-node-modules")
        .current_dir(t.path())
        .env_remove("E2E_TYPES_NODE_MODULES")
        .env_remove("E2E_PLAYWRIGHT_TEST")
        .output()
        .unwrap();

    assert!(!out.status.success());
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("E2E_TYPES_NODE_MODULES"), "got: {err}");
    assert!(err.contains("nix develop"), "got: {err}");
}

#[test]
fn unset_playwright_alone_names_its_own_variable() {
    // The types var resolving first must not mask the playwright one.
    let t = tmp();
    let types = fake_types_dir(t.path(), "store");
    let out = Command::new(EXE)
        .arg("provision-node-modules")
        .current_dir(t.path())
        .env("E2E_TYPES_NODE_MODULES", &types)
        .env_remove("E2E_PLAYWRIGHT_TEST")
        .output()
        .unwrap();

    assert!(!out.status.success());
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains("E2E_PLAYWRIGHT_TEST"), "got: {err}");
    assert!(err.contains("nix develop"), "got: {err}");
}
