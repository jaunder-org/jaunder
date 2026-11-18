//! Public native commands operate on the caller's checkout, not their build root.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

/// Keeps every Git a test runs, directly or through a command under test, to
/// its fixture: a hook's exported repository overrides and the caller's
/// configuration stay out.
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

fn git(root: &Path, args: &[&str]) -> Output {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).args(args);
    let output = isolated(&mut command).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn checkout() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-q"]);
    fs::create_dir_all(temp.path().join("tools")).unwrap();
    fs::create_dir_all(temp.path().join("docs/adr")).unwrap();
    fs::write(temp.path().join("flake.nix"), "{}\n").unwrap();
    fs::write(temp.path().join("tools/Cargo.toml"), "[workspace]\n").unwrap();
    temp
}
fn docs(root: &Path) -> Output {
    isolated(
        Command::new(env!("CARGO_BIN_EXE_repository-docs"))
            .arg("adr-format")
            .current_dir(root),
    )
    .output()
    .unwrap()
}
#[test]
fn documentation_verdict_emits_plain_diagnostics_without_result_artifacts() {
    let temp = checkout();
    let summary = temp.path().join("summary.md");
    let output = isolated(
        Command::new(env!("CARGO_BIN_EXE_repository-docs"))
            .arg("doc-links")
            .current_dir(temp.path())
            .env("GITHUB_STEP_SUMMARY", &summary),
    )
    .output()
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"[ ok ] doc-links\n");
    assert!(!summary.exists());
    assert!(!temp.path().join(".xtask/last-result.json").exists());
}

#[test]
fn unreadable_checkout_admission_precedes_hooks_and_policy() {
    let temp = checkout();
    fs::remove_file(temp.path().join("tools/Cargo.toml")).unwrap();
    let output = docs(temp.path());
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("reading Jaunder checkout marker tools/Cargo.toml")
    );
    assert!(
        git(temp.path(), &["config", "--local", "--list"])
            .stdout
            .windows(b"hookspath".len())
            .all(|s| s != b"hookspath")
    );
    assert!(
        output.stdout.is_empty(),
        "policy must not run after failed admission"
    );
}
#[test]
fn selected_nix_binary_refuses_omitted_source_before_hooks_or_policy() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let selected = Command::new("nix")
        .args([
            "build",
            "--no-update-lock-file",
            "--no-write-lock-file",
            "--no-link",
            "--print-out-paths",
            ".#repository-policy",
        ])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        selected.status.success(),
        "{}",
        String::from_utf8_lossy(&selected.stderr)
    );
    let selected = String::from_utf8(selected.stdout).unwrap();
    let binary = Path::new(selected.trim()).join("bin/repository-docs");
    let temp = checkout();
    // A dependency-free Git flake supplies the selected output-admission interface;
    // literal real-package source membership is checked separately below.
    fs::write(temp.path().join("flake.nix"), r#"{ outputs = { self }: { packages = builtins.listToAttrs (map (name: { inherit name; value.repository-policy.sourceAdmits = path: kind: path == "new.rs"; }) [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ]); }; }"#).unwrap();
    git(temp.path(), &["add", "flake.nix", "tools/Cargo.toml"]);
    fs::write(temp.path().join("new.rs"), "omitted source\n").unwrap();
    let output = Command::new(&binary)
        .arg("adr-format")
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("Git flakes omit untracked declared inputs: new.rs"),
        "{stderr}"
    );
    assert!(!stderr.contains("set core.hooksPath"), "{stderr}");
    assert!(output.stdout.is_empty());
    assert!(
        git(temp.path(), &["config", "--local", "--list"])
            .stdout
            .windows(b"hookspath".len())
            .all(|s| s != b"hookspath")
    );
    assert_eq!(
        git(temp.path(), &["status", "--porcelain"]).stdout,
        b"A  flake.nix\nA  tools/Cargo.toml\n?? new.rs\n"
    );
    git(temp.path(), &["add", "-N", "new.rs"]);
    let admitted = Command::new(&binary)
        .arg("adr-format")
        .current_dir(temp.path().join("docs"))
        .output()
        .unwrap();
    assert_eq!(
        admitted.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&admitted.stderr)
    );
    assert_eq!(admitted.stdout, b"[ ok ] adr-format\n");
}
// The WASM verdict whose source this case inspects is Linux-only.
#[cfg(target_os = "linux")]
#[test]
fn selected_native_source_excludes_product_and_other_tool_implementations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let expression = r#"let f = builtins.getFlake ("git+file://" + toString ./.); p = f.packages.${builtins.currentSystem}; tool = p.repository-policy;
      in {
        task2Inputs = builtins.all (relative:
          tool.cargoSourceAdmits relative "regular"
          && builtins.pathExists (tool.src + "/${relative}")
          && !(builtins.pathExists (p.system-artifacts.src + "/${relative}"))
          && !(builtins.pathExists (p.jaunder.src + "/${relative}"))
          && !(builtins.pathExists (p.csrWasm.src + "/${relative}"))
          && !(builtins.pathExists (f.checks.${builtins.currentSystem}.wasm-tests.src + "/${relative}"))
        ) [ "tools/repository-policy/src/scoped_leptosfmt.rs" "tools/repository-policy/src/bin/scoped-leptosfmt.rs"
            "tools/repository-policy/src/org_timezone_manifest.rs" "tools/repository-policy/src/bin/org-timezone-manifest.rs" ];
        common = tool.cargoSourceAdmits "common/src/org.rs" "regular" && builtins.pathExists (tool.src + "/common/src/org.rs");
        macros = tool.cargoSourceAdmits "macros/src/lib.rs" "regular" && builtins.pathExists (tool.src + "/macros/src/lib.rs");
        native = tool.sourceAdmits "tools/repository-policy/src/checkout.rs" "regular";
        dependency = tool.sourceAdmits "tools/repository-policy/Cargo.toml" "regular";
        test = tool.sourceAdmits "tools/repository-policy/tests/native.rs" "regular";
        evaluation = tool.sourceAdmits "nix/repository-policy.nix" "regular";
        evaluationInCargo = tool.cargoSourceAdmits "nix/repository-policy.nix" "regular";
        productInNative = tool.sourceAdmits "server/src/main.rs" "regular";
        toolsInNative = tool.sourceAdmits "tools/system-artifacts/src/main.rs" "regular";
        csrInNative = tool.sourceAdmits "csr/src/lib.rs" "regular";
        nativeInCsrAdmission = p.csrBundle.sourceAdmits "tools/repository-policy/src/checkout.rs" "regular";
        nativeFile = builtins.pathExists (tool.src + "/tools/repository-policy/src/checkout.rs");
        nativeInTools = builtins.pathExists (p.system-artifacts.src + "/tools/repository-policy/src/checkout.rs");
        nativeInProduct = builtins.pathExists (p.jaunder.src + "/tools/repository-policy/src/checkout.rs");
        nativeInCsr = builtins.pathExists (p.csrWasm.src + "/tools/repository-policy/src/checkout.rs");
        nativeInWasm = builtins.pathExists (f.checks.${builtins.currentSystem}.wasm-tests.src + "/tools/repository-policy/src/checkout.rs");
      }"#;
    let output = Command::new("nix")
        .args([
            "eval",
            "--impure",
            "--json",
            "--no-update-lock-file",
            "--no-write-lock-file",
            "--expr",
            expression,
        ])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let actual: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        actual,
        serde_json::json!({"task2Inputs":true,"common":true,"macros":true,"native":true,"dependency":true,"test":true,"evaluation":true,"evaluationInCargo":false,"productInNative":false,"toolsInNative":false,"csrInNative":false,"nativeInCsrAdmission":false,"nativeFile":true,"nativeInTools":false,"nativeInProduct":false,"nativeInCsr":false,"nativeInWasm":false})
    );
}
#[test]
fn adr_format_has_independent_success_policy_and_read_error_verdicts() {
    let temp = checkout();
    let adr = temp.path().join("docs/adr/0001-choice.md");
    fs::write(&adr, "# ADR-0001: Choice\n\n- Status: accepted\n").unwrap();
    let good = docs(temp.path());
    assert_eq!(good.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(good.stdout).unwrap(),
        "[ ok ] adr-format\n"
    );
    fs::write(&adr, "# ADR-0001: Choice\n\n- Status: proposed\n").unwrap();
    let bad = docs(temp.path());
    assert_eq!(bad.status.code(), Some(1));
    assert!(
        String::from_utf8(bad.stderr)
            .unwrap()
            .contains("numbering is the acceptance event")
    );
    fs::write(&adr, [0xff]).unwrap();
    let unreadable = docs(temp.path());
    assert_eq!(unreadable.status.code(), Some(1));
    assert!(
        String::from_utf8(unreadable.stderr)
            .unwrap()
            .contains("cannot read")
    );
}
