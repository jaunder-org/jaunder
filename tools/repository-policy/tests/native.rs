//! Public native commands operate on the caller's checkout, not their build root.
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn git(root: &Path, args: &[&str]) -> Output {
    let mut command = Command::new("git");
    command.arg("-C").arg(root).args(args);
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
    ] {
        command.env_remove(name);
    }
    let output = command.output().unwrap();
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
    Command::new(env!("CARGO_BIN_EXE_repository-docs"))
        .arg("adr-format")
        .current_dir(root)
        .output()
        .unwrap()
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
    for (operation, expected) in [("precommit", 1), ("prepush-docs", 2)] {
        let refused = safety(temp.path(), operation);
        assert_eq!(refused.status.code(), Some(expected));
        assert!(
            refused.stdout.is_empty(),
            "workload must not run after failed admission"
        );
    }
}
fn safety(root: &Path, operation: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_git-index-safety"))
        .arg(operation)
        .current_dir(root)
        .output()
        .unwrap()
}
fn staged_fixture(policy: &str) -> tempfile::TempDir {
    let temp = checkout();
    let root = temp.path();
    git(root, &["config", "user.name", "fixture"]);
    git(root, &["config", "user.email", "fixture@example.invalid"]);
    fs::write(root.join(".gitignore"), ".xtask/\n").unwrap();
    fs::write(root.join("guide.txt"), "original\n").unwrap();
    fs::write(root.join("docs/adr/0001-choice.md"), policy).unwrap();
    let docs = env!("CARGO_BIN_EXE_repository-docs");
    fs::write(root.join("Justfile"), format!("precommit-broad-host:\n    printf 'formatted\\n' > guide.txt\n    '{docs}' adr-format\n\nprecommit-markdown-host: precommit-broad-host\n")).unwrap();
    git(root, &["add", "."]);
    git(
        root,
        &["-c", "core.hooksPath=/dev/null", "commit", "-qm", "fixture"],
    );
    fs::write(root.join("guide.txt"), "staged\n").unwrap();
    git(root, &["add", "guide.txt"]);
    temp
}
#[test]
fn precommit_reconciles_staged_fixes_even_after_real_policy_failure() {
    let temp = staged_fixture("# ADR-0001: Choice\n- Status: proposed\n");
    let output = safety(temp.path(), "precommit");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("numbering is the acceptance event"),
        "{stderr}"
    );
    assert_eq!(stderr.matches("precommit-staging:").count(), 1, "{stderr}");
    assert_eq!(
        git(temp.path(), &["show", ":guide.txt"]).stdout,
        b"formatted\n"
    );
    assert_eq!(
        git(temp.path(), &["status", "--porcelain"]).stdout,
        b"M  guide.txt\n"
    );
    assert!(
        !stderr.contains("nix-static-docs"),
        "failed policy must skip Nix"
    );
}
#[test]
fn precommit_preserves_host_failure_when_after_snapshot_query_fails() {
    let temp = staged_fixture("# ADR-0001: Choice\n- Status: accepted\n");
    // Leave an invalid Git marker so failed queries cannot discover a parent repo.
    fs::write(temp.path().join("Justfile"), "precommit-broad-host:\n    printf 'formatted\\n' > guide.txt\n    mkdir -p .xtask\n    mv .git .xtask/parked-git\n    printf 'gitdir: missing-fixture-git\\n' > .git\n    exit 23\n").unwrap();
    git(temp.path(), &["add", "Justfile"]);
    let index_before = git(temp.path(), &["diff", "--cached", "--raw"]).stdout;

    let output = safety(temp.path(), "precommit");
    fs::remove_file(temp.path().join(".git")).unwrap();
    fs::rename(
        temp.path().join(".xtask/parked-git"),
        temp.path().join(".git"),
    )
    .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("precommit-host: `just precommit-broad-host` exited exit status: 23"),
        "{stderr}"
    );
    assert!(
        stderr.contains("after-snapshot failed; refusing staging"),
        "{stderr}"
    );
    assert!(
        stderr.contains("git status --porcelain=v1 -z failed"),
        "{stderr}"
    );
    assert_eq!(stderr.matches("precommit-staging:").count(), 1, "{stderr}");
    assert!(!stderr.contains("nix-static-docs"), "{stderr}");
    assert_eq!(
        git(temp.path(), &["diff", "--cached", "--raw"]).stdout,
        index_before
    );
    assert_eq!(
        git(temp.path(), &["show", ":guide.txt"]).stdout,
        b"staged\n"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("guide.txt")).unwrap(),
        "formatted\n"
    );
    assert_eq!(
        git(temp.path(), &["diff", "--name-only"]).stdout,
        b"guide.txt\n"
    );
}
#[test]
fn precommit_stages_only_safe_fixes_when_workload_creates_unowned_changes() {
    let temp = staged_fixture("# ADR-0001: Choice\n- Status: accepted\n");
    fs::write(temp.path().join("unowned.txt"), "author\n").unwrap();
    fs::write(temp.path().join("Justfile"),
        "precommit-broad-host:\n    printf 'formatted\\n' > guide.txt\n    printf 'unexpected\\n' > unowned.txt\n    printf 'new\\n' > new.txt\n").unwrap();
    git(temp.path(), &["add", "Justfile", "unowned.txt"]);
    git(
        temp.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "fixture host",
        ],
    );
    fs::write(temp.path().join("guide.txt"), "staged second\n").unwrap();
    git(temp.path(), &["add", "guide.txt"]);
    let output = safety(temp.path(), "precommit");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("will not add work the user did not stage"),
        "{stderr}"
    );
    assert!(
        stderr.contains("new untracked file created during precommit"),
        "{stderr}"
    );
    assert_eq!(stderr.matches("precommit-staging:").count(), 1);
    assert_eq!(
        git(temp.path(), &["show", ":guide.txt"]).stdout,
        b"formatted\n"
    );
    assert_eq!(
        git(temp.path(), &["show", ":unowned.txt"]).stdout,
        b"author\n"
    );
    assert_eq!(
        git(temp.path(), &["status", "--porcelain"]).stdout,
        b"M  guide.txt\n M unowned.txt\n?? new.txt\n"
    );
}
#[test]
fn precommit_staged_success_tolerates_best_effort_hook_config_failure() {
    let temp = staged_fixture("# ADR-0001: Choice\n- Status: accepted\n");
    fs::write(temp.path().join(".git/config.lock"), "held by fixture\n").unwrap();
    let output = safety(temp.path(), "precommit");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("warning: could not set core.hooksPath"));
    assert_eq!(stderr.matches("precommit-staging:").count(), 1);
    assert_eq!(
        git(temp.path(), &["show", ":guide.txt"]).stdout,
        b"formatted\n"
    );
}
#[test]
fn precommit_never_stages_mixed_work_or_preexisting_untracked_files() {
    let temp = staged_fixture("# ADR-0001: Choice\n- Status: accepted\n");
    fs::write(temp.path().join("guide.txt"), "unstaged author work\n").unwrap();
    fs::write(temp.path().join("notes.txt"), "untracked author work\n").unwrap();
    let output = safety(temp.path(), "precommit");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("pre-existing mixed staged/unstaged state"),
        "{stderr}"
    );
    assert_eq!(stderr.matches("precommit-staging:").count(), 1);
    assert_eq!(
        git(temp.path(), &["show", ":guide.txt"]).stdout,
        b"staged\n"
    );
    assert_eq!(
        git(temp.path(), &["status", "--porcelain"]).stdout,
        b"MM guide.txt\n?? notes.txt\n"
    );
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn staged_markdown_success_keeps_the_selected_docs_root_and_reconciles_once() {
    let temp = staged_fixture("# ADR-0001: Choice\n- Status: accepted\n");
    let shell = Command::new("bash")
        .args(["-c", "printf %s \"$BASH\""])
        .output()
        .unwrap();
    assert!(shell.status.success());
    let shell = String::from_utf8(shell.stdout).unwrap();
    let shell_root = Path::new(&shell).parent().unwrap().parent().unwrap();
    fs::write(
        temp.path().join("flake.nix"),
        format!(
            r#"{{ outputs = {{ self }}: {{ checks.x86_64-linux.static-docs = builtins.derivation {{
      name = "repository-policy-docs-fixture"; system = "x86_64-linux";
      builder = builtins.appendContext "{shell}" {{ "{shell_root}" = {{ path = true; }}; }};
      args = [ "-c" "printf passed > $out" ];
    }}; }}; }}"#,
            shell_root = shell_root.display()
        ),
    )
    .unwrap();
    let docs = env!("CARGO_BIN_EXE_repository-docs");
    fs::write(temp.path().join("Justfile"), format!("precommit-markdown-host:\n    printf '# Formatted\\n' > guide.md\n    '{docs}' adr-format\n")).unwrap();
    git(temp.path(), &["add", "."]);
    git(
        temp.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "narrow fixture",
        ],
    );
    fs::write(temp.path().join("guide.md"), "# Staged\n").unwrap();
    git(temp.path(), &["add", "guide.md"]);
    let output = safety(temp.path(), "precommit");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("class=staged-markdown-only reason=isolated-staged-markdown")
    );
    assert_eq!(stderr.matches("precommit-staging:").count(), 1, "{stderr}");
    assert!(
        stderr.contains("shared-static-docs-state: unchanged"),
        "{stderr}"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join(".xtask/gcroots/static-docs")).unwrap(),
        "passed"
    );
    assert_eq!(
        git(temp.path(), &["status", "--porcelain"]).stdout,
        b"A  guide.md\n"
    );
    assert!(!temp.path().join(".xtask/last-result.json").exists());
}
#[test]
fn prepush_docs_retains_real_nix_failure_logs_and_runs_postcheck() {
    let temp = staged_fixture("# ADR-0001: Choice\n- Status: accepted\n");
    git(
        temp.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "staged fixture",
        ],
    );
    let output = safety(temp.path(), "prepush-docs");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("nix-static-docs exited"), "{stderr}");
    assert!(
        stderr.contains("shared-static-docs-state: unchanged"),
        "{stderr}"
    );
    let diagnostics = temp.path().join(".xtask/diagnostics/static-docs");
    let log = fs::read_to_string(diagnostics.join("build.log")).unwrap();
    let excerpt = fs::read_to_string(diagnostics.join("failure-excerpt.log")).unwrap();
    assert!(log.contains("error:"), "{log}");
    assert!(excerpt.starts_with("error:"), "{excerpt}");
    assert!(stderr.contains("scoped excerpt (read first)"));
    assert!(
        git(temp.path(), &["status", "--porcelain"])
            .stdout
            .is_empty()
    );
}
#[cfg(unix)]
#[test]
fn prepush_docs_preserves_nix_failure_when_postrequest_query_fails() {
    use std::os::unix::fs::PermissionsExt;
    let temp = staged_fixture("# ADR-0001: Choice\n- Status: accepted\n");
    git(
        temp.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "clean fixture",
        ],
    );
    let index_before = git(temp.path(), &["diff", "--cached", "--raw"]).stdout;
    let programs = tempfile::tempdir().unwrap();
    let nix = programs.path().join("nix");
    // Fail the external request and its subsequent real Git query independently.
    fs::write(&nix, "#!/bin/sh\nprintf 'request mutation\\n' > guide.txt\nmv .git .xtask/parked-git\nprintf 'gitdir: missing-fixture-git\\n' > .git\nprintf 'error: original request failure\\n' >&2\nexit 19\n").unwrap();
    fs::set_permissions(&nix, fs::Permissions::from_mode(0o755)).unwrap();
    let mut path = vec![programs.path().to_path_buf()];
    path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));

    let output = Command::new(env!("CARGO_BIN_EXE_git-index-safety"))
        .arg("prepush-docs")
        .current_dir(temp.path())
        .env("PATH", std::env::join_paths(path).unwrap())
        .output()
        .unwrap();
    fs::remove_file(temp.path().join(".git")).unwrap();
    fs::rename(
        temp.path().join(".xtask/parked-git"),
        temp.path().join(".git"),
    )
    .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("original request failure"), "{stderr}");
    assert!(
        stderr.contains("nix-static-docs exited exit status: 19"),
        "{stderr}"
    );
    assert!(
        stderr.contains("shared-static-docs-state: git status --porcelain=v1 -z failed"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("shared-static-docs-state: unchanged"),
        "{stderr}"
    );
    assert_eq!(stderr.matches("precommit-staging:").count(), 0, "{stderr}");
    assert_eq!(
        git(temp.path(), &["diff", "--cached", "--raw"]).stdout,
        index_before
    );
    assert_eq!(
        git(temp.path(), &["show", ":guide.txt"]).stdout,
        b"staged\n"
    );
    assert_eq!(
        git(temp.path(), &["status", "--porcelain"]).stdout,
        b" M guide.txt\n"
    );
}
#[cfg(unix)]
#[test]
fn failed_docs_request_cannot_hide_postrequest_mutation_or_restage_it() {
    use std::os::unix::fs::PermissionsExt;
    for (mutation, expected) in [
        ("printf changed > late.md", "?? late.md\n"),
        (
            "printf changed > docs/adr/0001-choice.md",
            " M docs/adr/0001-choice.md\n",
        ),
        ("printf changed > .prettierrc.json", " M .prettierrc.json\n"),
        ("printf changed > flake.lock", " M flake.lock\n"),
        (
            "printf changed > late.md; git add -- late.md",
            "A  late.md\n",
        ),
    ] {
        let temp = staged_fixture("# ADR-0001: Choice\n- Status: accepted\n");
        fs::write(temp.path().join(".prettierrc.json"), "{}\n").unwrap();
        fs::write(temp.path().join("flake.lock"), "{}\n").unwrap();
        git(temp.path(), &["add", "."]);
        git(
            temp.path(),
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "commit",
                "-qm",
                "staged fixture",
            ],
        );
        let programs = tempfile::tempdir().unwrap();
        let nix = programs.path().join("nix");
        // Fault only the external request; Git still observes the real fixture.
        fs::write(&nix, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > .xtask/request-args\n{mutation}\nprintf 'error: fixture request failed\\n' >&2\nexit 19\n")).unwrap();
        fs::set_permissions(&nix, fs::Permissions::from_mode(0o755)).unwrap();
        let mut path = vec![programs.path().to_path_buf()];
        path.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        let output = Command::new(env!("CARGO_BIN_EXE_git-index-safety"))
            .arg("prepush-docs")
            .current_dir(temp.path())
            .env("PATH", std::env::join_paths(path).unwrap())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("exit status: 19"), "{mutation}: {stderr}");
        assert!(
            stderr.contains("Git state changed during static-docs request"),
            "{mutation}: {stderr}"
        );
        assert_eq!(stderr.matches("precommit-staging:").count(), 0);
        assert_eq!(
            git(temp.path(), &["status", "--porcelain"]).stdout,
            expected.as_bytes()
        );
        assert_eq!(
            fs::read_to_string(temp.path().join(".xtask/request-args")).unwrap(),
            "build\n-L\n--keep-failed\n--log-lines\n50\n--accept-flake-config\n--no-update-lock-file\n--no-write-lock-file\n--out-link\n.xtask/gcroots/static-docs\n.#checks.x86_64-linux.static-docs\n"
        );
    }
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
        native = tool.sourceAdmits "tools/repository-policy/src/safety.rs" "regular";
        dependency = tool.sourceAdmits "tools/repository-policy/Cargo.toml" "regular";
        test = tool.sourceAdmits "tools/repository-policy/tests/native.rs" "regular";
        evaluation = tool.sourceAdmits "nix/repository-policy.nix" "regular";
        evaluationInCargo = tool.cargoSourceAdmits "nix/repository-policy.nix" "regular";
        productInNative = tool.sourceAdmits "server/src/main.rs" "regular";
        devtoolInNative = tool.sourceAdmits "tools/devtool/src/main.rs" "regular";
        csrInNative = tool.sourceAdmits "csr/src/lib.rs" "regular";
        nativeInCsrAdmission = p.csrBundle.sourceAdmits "tools/repository-policy/src/safety.rs" "regular";
        nativeFile = builtins.pathExists (tool.src + "/tools/repository-policy/src/safety.rs");
        nativeInDevtool = builtins.pathExists (p.devtool.src + "/tools/repository-policy/src/safety.rs");
        nativeInProduct = builtins.pathExists (p.jaunder.src + "/tools/repository-policy/src/safety.rs");
        nativeInCsr = builtins.pathExists (p.csrWasm.src + "/tools/repository-policy/src/safety.rs");
        nativeInWasm = builtins.pathExists (f.checks.${builtins.currentSystem}.wasm-tests.src + "/tools/repository-policy/src/safety.rs");
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
        serde_json::json!({"native":true,"dependency":true,"test":true,"evaluation":true,"evaluationInCargo":false,"productInNative":false,"devtoolInNative":false,"csrInNative":false,"nativeInCsrAdmission":false,"nativeFile":true,"nativeInDevtool":false,"nativeInProduct":false,"nativeInCsr":false,"nativeInWasm":false})
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
