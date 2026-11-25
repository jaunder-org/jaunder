//! admit-inputs refuses untracked inputs the selected flake output declares.
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

/// Keeps git, run directly or by admit-inputs, to the fixture: no inherited
/// `GIT_*` override (as set inside a Git hook) and none of the caller's
/// configuration.
fn isolated(command: &mut Command) -> &mut Command {
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
}

/// Runs admit-inputs in `root` with `bin` first on PATH.
fn admit(root: &Path, bin: &Path) -> std::process::Output {
    isolated(&mut Command::new(env!("CARGO_BIN_EXE_admit-inputs")))
        .arg(".#csrBundle.sourceAdmits")
        .current_dir(root)
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .output()
        .unwrap()
}

#[test]
fn only_a_declared_untracked_input_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("checkout");
    let bin = temp.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    assert!(
        isolated(Command::new("git").args(["init", "--quiet"]).arg(&root))
            .status()
            .unwrap()
            .success()
    );
    // Stands in for the flake's source predicate: it admits only `new.rs`.
    let nix = bin.join("nix");
    fs::write(
        &nix,
        "#!/bin/sh\ncase \"$JAUNDER_XTASK_UNTRACKED_INPUTS\" in\n  *new.rs*) printf '[\"new.rs\"]' ;;\n  *) printf '[]' ;;\nesac\n",
    )
    .unwrap();
    fs::set_permissions(&nix, fs::Permissions::from_mode(0o755)).unwrap();

    let clean = admit(&root, &bin);
    assert!(clean.status.success(), "{clean:?}");

    fs::write(root.join("notes.md"), "harmless").unwrap();
    let harmless = admit(&root, &bin);
    assert!(harmless.status.success(), "{harmless:?}");

    fs::write(root.join("new.rs"), "omitted source").unwrap();
    let refused = admit(&root, &bin);
    assert_eq!(refused.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.starts_with("admit-inputs: "), "{stderr}");
    assert!(stderr.contains("new.rs"), "{stderr}");
}
