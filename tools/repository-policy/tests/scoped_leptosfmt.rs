//! The public adapter refuses incomplete scope before Fix and rejects false greens.
use std::{fs, path::Path, process::Command};

/// Keeps every Git the fixture, `dev`'s shim or `dev` runs to the fixture: a
/// hook's exported repository overrides and the caller's configuration stay out.
fn isolated(command: &mut Command) -> &mut Command {
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
}

fn adapter(root: &Path, mode: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_scoped-leptosfmt"));
    command.current_dir(root).arg(mode);
    isolated(&mut command);
    command
}

fn run(root: &Path, mode: &str) -> std::process::Output {
    adapter(root, mode).output().unwrap()
}

/// A Git checkout whose `tools/dev` is this repository's runner: the devShell's
/// `dev` shim runs `tools/dev` from the checkout root it finds, and the adapter
/// runs the pin through it.
fn checkout(root: &Path) {
    let init = isolated(Command::new("git").args(["init", "-q"]).arg(root))
        .status()
        .unwrap();
    assert!(init.success());
    fs::create_dir(root.join("tools")).unwrap();
    std::os::unix::fs::symlink(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../dev"),
        root.join("tools/dev"),
    )
    .unwrap();
}

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for scope in ["web/src", "csr/src"] {
        fs::create_dir_all(root.path().join(scope)).unwrap();
    }
    checkout(root.path());
    root
}

#[test]
fn empty_population_never_scans_unrelated_sources() {
    let root = fixture();
    fs::write(root.path().join("unrelated.rs"), "fn broken(").unwrap();
    // No executable is available: an empty population must not spawn even `dev`.
    assert!(
        adapter(root.path(), "fix")
            .env("PATH", "")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(root.path().join("unrelated.rs")).unwrap(),
        "fn broken("
    );
}

#[test]
fn missing_scope_and_invalid_utf8_fail_before_fix_mutates_valid_input() {
    let root = fixture();
    let valid = "use leptos::prelude::*; fn view() { let _ = view! {<p>  Hello </p>}; }\n";
    fs::write(root.path().join("web/src/valid.rs"), valid).unwrap();
    fs::write(root.path().join("csr/src/invalid.rs"), [0xff]).unwrap();
    assert!(!run(root.path(), "fix").status.success());
    assert_eq!(
        fs::read_to_string(root.path().join("web/src/valid.rs")).unwrap(),
        valid
    );
    fs::remove_file(root.path().join("csr/src/invalid.rs")).unwrap();
    fs::remove_dir(root.path().join("csr/src")).unwrap();
    assert!(!run(root.path(), "fix").status.success());
}

#[test]
fn literal_checkout_hidden_sources_and_unrelated_population_match_the_pin() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("checkout[1]");
    fs::create_dir(&root).unwrap();
    for directory in ["web/src/nested", "csr/src", "other", "target"] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    checkout(&root);
    for file in [
        "web/src/a[1].rs",
        "web/src/nested/b.rs",
        "csr/src/.hidden.rs",
    ] {
        fs::write(root.join(file), "fn valid() {}\n").unwrap();
    }
    fs::write(root.join("other/broken.rs"), "fn broken(").unwrap();
    fs::write(root.join("web/src/ignored.txt"), [0xff]).unwrap();
    let output = run(&root, "verify");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Checked 3 files"));
    assert!(run(&root, "fix").status.success());
    assert_eq!(
        fs::read_to_string(root.join("other/broken.rs")).unwrap(),
        "fn broken("
    );
}

#[test]
fn directory_as_source_is_not_a_silently_skipped_read_error() {
    let root = fixture();
    fs::create_dir(root.path().join("web/src/directory.rs")).unwrap();
    assert!(!run(root.path(), "fix").status.success());
}

#[cfg(unix)]
#[test]
fn inaccessible_nested_scope_fails_instead_of_selecting_a_partial_population() {
    use std::os::unix::fs::PermissionsExt;
    let root = fixture();
    let private = root.path().join("web/src/private");
    fs::create_dir(&private).unwrap();
    fs::write(private.join("source.rs"), "fn valid() {}\n").unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o000)).unwrap();
    let output = run(root.path(), "fix");
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        !output.status.success(),
        "permission fixture requires an unprivileged user"
    );
}

#[cfg(unix)]
#[test]
fn formatter_spawn_status_and_zero_status_markers_cannot_be_false_greens() {
    use std::os::unix::fs::PermissionsExt;
    let root = fixture();
    fs::write(root.path().join("web/src/valid.rs"), "fn valid() {}\n").unwrap();
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let formatter = bin.join("leptosfmt");
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    for script in [
        "exit 7",
        "printf '❌ web/src/valid.rs\\n'; printf 'formatter read error\\n' >&2",
    ] {
        fs::write(&formatter, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&formatter, fs::Permissions::from_mode(0o755)).unwrap();
        let output = adapter(root.path(), "verify")
            .env("PATH", std::env::join_paths(&paths).unwrap())
            .output()
            .unwrap();
        assert!(!output.status.success());
        if script.starts_with("printf") {
            assert!(String::from_utf8_lossy(&output.stdout).contains("❌ web/src/valid.rs"));
            assert!(String::from_utf8_lossy(&output.stderr).contains("formatter read error"));
        }
    }
    let missing = adapter(root.path(), "fix")
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("running leptosfmt"));
}

#[test]
fn malformed_scoped_rust_fails_in_both_modes_with_real_pinned_formatter() {
    let root = fixture();
    fs::write(root.path().join("web/src/broken.rs"), "fn broken(").unwrap();
    for mode in ["verify", "fix"] {
        let output = run(root.path(), mode);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("❌ "));
    }
}
