//! Exercise the actual pinned Just parser with fake tools, never the real index.
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

struct Workflow {
    root: tempfile::TempDir,
}

impl Workflow {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../Justfile"),
            root.path().join("Justfile"),
        )
        .unwrap();
        fs::create_dir(root.path().join("recipes")).unwrap();
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../recipes/prettier.just"),
            root.path().join("recipes/prettier.just"),
        )
        .unwrap();
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../recipes/static.just"),
            root.path().join("recipes/static.just"),
        )
        .unwrap();
        fs::create_dir(root.path().join("scripts")).unwrap();
        // Gate composition observes only the aggregate boundary. The actual helper's
        // settlement, artifact and failure behavior is exercised by e2e_workflow.
        fs::write(
            root.path().join("scripts/e2e-workflow"),
            r#"set -eu
[ "$#" -eq 2 ] && [ "$1" = all ] && [ -z "$2" ]
printf 'e2e-workflow\tall\n' >> "$FAKE_LOG"
if [ "${FAIL_COMMAND:-}" = 'e2e-workflow all' ]; then exit 17; fi
"#,
        )
        .unwrap();
        let bin = root.path().join("bin");
        fs::create_dir(&bin).unwrap();
        for tool in [
            "cargo",
            "nix",
            "devtool",
            "prettier",
            "git",
            "scoped-leptosfmt",
            "emacs",
            "ast-grep",
            "tsc",
        ] {
            let path = bin.join(tool);
            fs::write(
                &path,
                r#"#!/bin/sh
set -eu
name=${0##*/}
if [ "$name $*" = 'git worktree list --porcelain' ]; then
    printf 'worktree %s\n\nworktree %s/.\n\nworktree %s/cache-link\n' "$PWD" "$PWD" "$PWD"
    exit 0
fi
if [ "$name" = cargo ]; then
    printf '%s\n' "$RUSTC_WRAPPER" "$CARGO_INCREMENTAL" "$SCCACHE_BASEDIRS" > cache-env
fi
printf '%s\t' "$name" "$@" >> "$FAKE_LOG"
printf '\n' >> "$FAKE_LOG"
if [ "$name" = git ]; then
    if [ -n "${GIT_DIR+x}${GIT_WORK_TREE+x}${GIT_INDEX_FILE+x}${GIT_OBJECT_DIRECTORY+x}${GIT_COMMON_DIR+x}${GIT_NAMESPACE+x}" ]; then
        printf 'Git inherited a repository override\n' >&2
        exit 23
    fi
    if [ -f dirty-status ]; then
        printf ' M tracked.rs\n'
    fi
    case "$*" in
      'rev-parse --verify HEAD^{commit}'|'rev-parse --verify --end-of-options HEAD^{commit}') printf 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n' ;;
      'rev-parse --verify --end-of-options source^{commit}') printf 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n' ;;
      'rev-parse --verify --end-of-options target^{commit}') printf 'cccccccccccccccccccccccccccccccccccccccc\n' ;;
      'remote get-url origin')
        if [ -f wrong-origin ]; then printf 'https://example.invalid/foreign\n'; else printf 'https://github.com/jaunder-org/jaunder.git\n'; fi ;;
    esac
fi
if [ "$name $*" = "cargo xtask test-local" ]; then
    touch producer-resource
    trap 'rm producer-resource; printf "cleaned\n" >> producer-cleanup' EXIT
fi
if [ "$name $*" = "${FAIL_COMMAND:-}" ]; then exit 17; fi
"#,
            )
            .unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self { root }
    }

    fn run(&self, args: &[&str], failure: &str) -> (bool, Vec<Vec<String>>, String) {
        let log = self.root.path().join("commands");
        fs::write(&log, "").unwrap();
        let output = Command::new("just")
            .args(args)
            .current_dir(self.root.path())
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.root.path().join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("FAKE_LOG", &log)
            .env("CARGO_TARGET_DIR", self.root.path().join("target"))
            // Exercise hook-style inherited overrides without touching real Git.
            .env("GIT_DIR", "foreign-git-dir")
            .env("GIT_WORK_TREE", "foreign-work-tree")
            .env("GIT_INDEX_FILE", "foreign-index")
            .env("GIT_OBJECT_DIRECTORY", "foreign-objects")
            .env("GIT_COMMON_DIR", "foreign-common-dir")
            .env("GIT_NAMESPACE", "foreign-namespace")
            .env("FAIL_COMMAND", failure)
            .output()
            .unwrap();
        let commands = fs::read_to_string(log)
            .unwrap()
            .lines()
            .map(|line| {
                line.trim_end_matches('\t')
                    .split('\t')
                    .map(str::to_owned)
                    .collect()
            })
            .collect();
        (
            output.status.success(),
            commands,
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

#[test]
fn qualification_just_preflight_fetches_complete_distinct_set_once() {
    let workflow = Workflow::new();
    let (ok, actual, stderr) =
        workflow.run(&["production-baseline-accept", "source", "target"], "");
    assert!(ok, "{stderr}");
    assert_eq!(
        actual,
        commands(&[
            "git status --porcelain --untracked-files=all",
            "git rev-parse --verify HEAD^{commit}",
            "git rev-parse --verify --end-of-options source^{commit}",
            "git rev-parse --verify --end-of-options target^{commit}",
            "git remote get-url origin",
            "git fetch --quiet --no-write-fetch-head --refetch origin aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb cccccccccccccccccccccccccccccccccccccccc",
            "cargo xtask qualification-leaf --expected-harness aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa --source bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb --target cccccccccccccccccccccccccccccccccccccccc",
        ])
    );
}

#[test]
fn acceptance_refuses_empty_target_before_fetch_or_host_invocation() {
    let workflow = Workflow::new();
    let (ok, actual, stderr) = workflow.run(&["production-baseline-accept", "HEAD", ""], "");
    assert!(!ok, "empty acceptance target must refuse: {stderr}");
    assert_eq!(
        stderr.lines().next(),
        Some(
            "error: call to function `error` failed: production-baseline accept requires a nonempty target"
        ),
        "{stderr}"
    );
    assert!(!actual.iter().any(|command| command[0] == "cargo"));
    assert!(
        !actual
            .iter()
            .any(|command| command.get(1).is_some_and(|arg| arg == "fetch"))
    );
}

#[test]
fn qualification_preflight_refuses_before_host_invocation() {
    for marker in ["dirty-status", "wrong-origin"] {
        let workflow = Workflow::new();
        fs::write(workflow.root.path().join(marker), "refuse").unwrap();
        let (ok, actual, stderr) = workflow.run(&["production-baseline-discover", "source"], "");
        assert!(!ok, "{marker}: {stderr}");
        assert!(!actual.iter().any(|command| command[0] == "cargo"));
        assert!(
            !actual
                .iter()
                .any(|command| command.get(1).is_some_and(|arg| arg == "fetch"))
        );
    }
    let workflow = Workflow::new();
    let (ok, actual, stderr) =
        workflow.run(&["production-baseline-accept", "source", "source"], "");
    assert!(!ok, "{stderr}");
    assert!(stderr.contains("distinct resolved"), "{stderr}");
    assert!(!actual.iter().any(|command| command[0] == "cargo"));
    let (ok, actual, stderr) = workflow.run(&["production-baseline-discover", "source"], "git fetch --quiet --no-write-fetch-head --refetch origin aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    assert!(!ok, "{stderr}");
    assert!(!actual.iter().any(|command| command[0] == "cargo"));
}

#[test]
fn discovery_deduplicates_harness_and_styling_and_smoke_stay_local() {
    let workflow = Workflow::new();
    let (ok, actual, stderr) = workflow.run(&["production-baseline-discover", "HEAD"], "");
    assert!(ok, "{stderr}");
    let fetches: Vec<_> = actual
        .iter()
        .filter(|command| command.get(1).is_some_and(|arg| arg == "fetch"))
        .collect();
    assert_eq!(fetches, commands(&["git fetch --quiet --no-write-fetch-head --refetch origin aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]).iter().collect::<Vec<_>>());
    let (ok, actual, stderr) = workflow.run(&["production-baseline-qualify-styling"], "");
    assert!(ok, "{stderr}");
    assert_eq!(
        actual,
        commands(&[
            "git status --porcelain --untracked-files=all",
            "git rev-parse --verify HEAD^{commit}",
            "cargo xtask qualification-leaf --expected-harness aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ])
    );
    let (ok, actual, stderr) = workflow.run(&["production-baseline-smoke"], "");
    assert!(ok, "{stderr}");
    assert_eq!(actual, commands(&["cargo xtask qualification-leaf"]));
}

#[test]
fn real_git_refuses_unpushed_harness_and_resolves_product_aliases() {
    let root = tempfile::tempdir().unwrap();
    let tools = tempfile::tempdir().unwrap();
    let upstream = tools.path().join("upstream.git");
    let git = |args: &[&str]| {
        let output = xtask::git::at(root.path()).args(args).output().unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init", "--quiet"]);
    git(&["config", "user.name", "Fixture"]);
    git(&["config", "user.email", "fixture@example.invalid"]);
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../Justfile"),
        root.path().join("Justfile"),
    )
    .unwrap();
    git(&["add", "Justfile"]);
    git(&["commit", "--quiet", "-m", "source"]);
    let source = git(&["rev-parse", "HEAD"]);
    git(&["tag", "source-alias"]);
    git(&["commit", "--quiet", "--allow-empty", "-m", "target"]);
    let target = git(&["rev-parse", "HEAD"]);
    git(&["tag", "target-alias"]);
    git(&["init", "--quiet", "--bare", upstream.to_str().unwrap()]);
    git(&["push", upstream.to_str().unwrap(), "HEAD:refs/heads/main"]);
    git(&[
        "remote",
        "add",
        "origin",
        "https://github.com/jaunder-org/jaunder.git",
    ]);
    git(&["commit", "--quiet", "--allow-empty", "-m", "harness"]);
    let harness = git(&["rev-parse", "HEAD"]);
    let real_git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("git"))
        .find(|path| path.is_file())
        .unwrap();
    for (name, text) in [
        (
            "git",
            "#!/usr/bin/env bash\nargs=(\"$@\")\nif [[ ${args[0]} == fetch ]]; then\n  for i in \"${!args[@]}\"; do\n    [[ ${args[$i]} != origin ]] || args[$i]=$UPSTREAM\n  done\nfi\nexec \"$REAL_GIT\" \"${args[@]}\"\n",
        ),
        ("cargo", "#!/bin/sh\nprintf '%s\\n' \"$@\"\n"),
    ] {
        let path = tools.path().join(name);
        fs::write(&path, text).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let run = |args: &[&str]| {
        let mut command = Command::new("just");
        command
            .args(args)
            .current_dir(root.path())
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    tools.path().display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("REAL_GIT", &real_git)
            .env("UPSTREAM", &upstream);
        for variable in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_COMMON_DIR",
            "GIT_NAMESPACE",
        ] {
            command.env_remove(variable);
        }
        command.output().unwrap()
    };
    let args = ["production-baseline-accept", "source-alias", "target-alias"];
    for protocol in ["0", "2"] {
        git(&["config", "protocol.version", protocol]);
        let refused = run(&args);
        assert!(
            !refused.status.success(),
            "protocol={protocol}: {refused:?}"
        );
        assert!(!String::from_utf8_lossy(&refused.stdout).contains("qualification-leaf"));
    }
    git(&["push", upstream.to_str().unwrap(), "HEAD:refs/heads/main"]);
    fs::write(root.path().join(".git/FETCH_HEAD"), "preserve fetch head\n").unwrap();
    let admitted = run(&args);
    assert!(admitted.status.success(), "{admitted:?}");
    assert_eq!(
        String::from_utf8(admitted.stdout).unwrap(),
        format!(
            "xtask\nqualification-leaf\n--expected-harness\n{harness}\n--source\n{source}\n--target\n{target}\n"
        )
    );
    assert_eq!(
        fs::read_to_string(root.path().join(".git/FETCH_HEAD")).unwrap(),
        "preserve fetch head\n"
    );
    assert!(
        !run(&["production-baseline-accept", "source-alias", &source])
            .status
            .success()
    );
    assert!(
        !run(&["production-baseline-discover", "not-a-revision"])
            .status
            .success()
    );
    git(&[
        "commit",
        "--quiet",
        "--allow-empty",
        "-m",
        "local-only product",
    ]);
    let local_product = git(&["rev-parse", "HEAD"]);
    git(&["update-ref", "HEAD", &harness]);
    for protocol in ["0", "2"] {
        git(&["config", "protocol.version", protocol]);
        for products in [
            [local_product.as_str(), target.as_str()],
            [source.as_str(), local_product.as_str()],
        ] {
            let refused = run(&["production-baseline-accept", products[0], products[1]]);
            assert!(
                !refused.status.success(),
                "protocol={protocol}: {refused:?}"
            );
            assert!(!String::from_utf8_lossy(&refused.stdout).contains("qualification-leaf"));
        }
    }
    git(&["remote", "remove", "origin"]);
    assert!(
        run(&["production-baseline-qualify-styling"])
            .status
            .success()
    );
}

fn broad(mode: &str, shared_docs: bool) -> Vec<Vec<String>> {
    let mut expected = Vec::new();
    for name in [
        "fmt",
        "leptosfmt",
        "prettier-markdown",
        "prettier-end2end",
        "prettier-css",
        "elisp-fmt",
        "tools-fmt",
        "ast-grep-tests",
        "no-full-reload",
    ] {
        if name == "prettier-markdown" && shared_docs {
            expected.extend(commands(&["cargo run --locked --quiet --manifest-path tools/Cargo.toml -p repository-policy --bin git-index-safety -- prepush-docs"]));
        } else {
            if name == "leptosfmt" {
                expected.extend(commands(&["cargo build --locked --quiet --manifest-path tools/Cargo.toml -p repository-policy --bin scoped-leptosfmt --target-dir tools/target"]));
            }
            expected.push(devtool(name, mode));
        }
    }
    expected.extend(commands(&[if mode == "fix" {
        "cargo fmt --manifest-path xtask/Cargo.toml"
    } else {
        "cargo fmt --manifest-path xtask/Cargo.toml --check"
    }]));
    // Literal required population from the existing gate, not Justfile introspection.
    for name in [
        "sequence-check",
        "adr-format",
        "adr-readme-parity",
        "adr-view-parity",
        "doc-links",
        "flow-docs",
        "test-patterns",
        "server-fn-registrar",
        "server-fn-tracing",
        "server-fn-coverage",
        "server-fn-wire-arg-error",
        "traced-context",
        "e2e-telemetry-boundary",
        "e2e-collector-config",
        "proffered-secret",
        "e2e-goto-wrapper",
        "e2e-server-fn-endpoints",
        "target-arch-placement",
        "write-transaction-contract",
        "common-host-target-closure",
        "lint-suppression",
        "thin-components",
        "sqlx-newtype-bind",
        "sqlx-newtype-decode",
        "doctest-fences",
        "raw-html-door",
        "html-sink",
        "e2e-scaffold",
        "xlang-literal",
    ] {
        if [
            "sequence-check",
            "adr-format",
            "adr-readme-parity",
            "adr-view-parity",
            "doc-links",
            "flow-docs",
        ]
        .contains(&name)
        {
            expected.extend(commands(&[&format!("cargo run --locked --quiet --manifest-path tools/Cargo.toml -p repository-policy --bin repository-docs -- {name}")]));
        } else if name != "doctest-fences" {
            expected.extend(commands(&[&format!("cargo run --locked --quiet --manifest-path tools/Cargo.toml -p repository-policy --bin source-contracts -- {name}")]));
        } else {
            expected.push(vec![
                "cargo".into(),
                "xtask".into(),
                "check-leaf".into(),
                name.into(),
            ]);
        }
    }
    for name in [
        "byte-compile",
        "tsc",
        "cargo-deny",
        "product-core-clippy",
        "web-server-clippy",
        "web-no-server-clippy",
        "client-host-clippy",
        "wasm-clippy",
        "tools-clippy",
    ] {
        expected.push(devtool(name, mode));
    }
    expected.extend(commands(&[
        "cargo clippy --manifest-path xtask/Cargo.toml --all-targets -- -D warnings",
    ]));
    expected.push(devtool("ert", mode));
    expected
}

fn devtool(name: &str, mode: &str) -> Vec<String> {
    if let Some(population) = name.strip_prefix("prettier-") {
        return vec![
            "prettier".into(),
            if mode == "fix" { "-w" } else { "--check" }.into(),
            match population {
                "markdown" => "**/*.md",
                "end2end" => "end2end",
                "css" => "**/*.css",
                _ => panic!("unexpected Prettier population"),
            }
            .into(),
        ];
    }
    let line = match name {
        "fmt" => "cargo fmt",
        "leptosfmt" => return vec!["scoped-leptosfmt".into(), mode.into()],
        "tools-fmt" => "cargo fmt --manifest-path tools/Cargo.toml --all",
        "elisp-fmt" => {
            return commands(&[if mode == "fix" {
                "emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-fix"
            } else {
                "emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-check"
            }])
            .pop()
            .unwrap();
        }
        "ast-grep-tests" => "ast-grep test --config sgconfig.yml",
        "no-full-reload" => {
            "ast-grep scan --config sgconfig.yml --filter ^no-full-reload$ web/src client/src"
        }
        "byte-compile" => "emacs --batch -Q -l elisp/scripts/byte-compile.el",
        "cargo-deny" => "cargo deny check",
        "product-core-clippy" => {
            "cargo clippy --workspace --exclude web --exclude client --exclude csr --all-targets -- -D warnings"
        }
        "web-server-clippy" => "cargo clippy -p web --features server --all-targets -- -D warnings",
        "web-no-server-clippy" => {
            "cargo clippy -p web --no-default-features --all-targets -- -D warnings"
        }
        "client-host-clippy" => {
            "cargo clippy -p client --features csr --all-targets -- -D warnings"
        }
        "wasm-clippy" => {
            "cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings"
        }
        "tools-clippy" => {
            "cargo clippy --manifest-path tools/Cargo.toml --workspace --all-targets -- -D warnings"
        }
        "tsc" => "tsc --noEmit -p end2end/tsconfig.json",
        "ert" => {
            return {
                let mut argv = commands(&["cargo run --locked --quiet --manifest-path tools/Cargo.toml -p devtool --bin devtool -- check"]).pop().unwrap();
                argv.push(name.into());
                if mode == "fix" {
                    argv.push("--fix".into());
                }
                argv
            };
        }
        _ => panic!("unexpected static leaf {name}"),
    };
    let mut argv = commands(&[line]).pop().unwrap();
    if mode != "fix" && matches!(name, "fmt" | "tools-fmt") {
        argv.push("--check".into());
    }
    argv
}

fn host_tests() -> Vec<Vec<String>> {
    commands(&[
        "cargo test --manifest-path xtask/Cargo.toml --lib --bins --tests",
        "cargo test --manifest-path tools/Cargo.toml --lib --bins --tests",
    ])
}

fn nix(check: &str) -> Vec<String> {
    vec![
        "nix".into(),
        "build".into(),
        "-L".into(),
        "--accept-flake-config".into(),
        "--out-link".into(),
        format!(".xtask/gcroots/{check}"),
        format!(".#checks.x86_64-linux.{check}"),
    ]
}

fn platform() -> Vec<String> {
    vec![
        "nix".into(),
        "eval".into(),
        "--impure".into(),
        "--expr".into(),
        "import ./nix/platform-contract.nix { flake = builtins.getFlake (toString ./.); }".into(),
    ]
}

fn static_checks() -> Vec<Vec<String>> {
    [
        "static-docs",
        "static-typescript",
        "rust-format",
        "static-code",
        "static-elisp",
        "static-e2e-census",
        "static-ert",
        "xtask-format",
        "xtask-clippy",
    ]
    .map(nix)
    .to_vec()
}

fn public_clean() -> Vec<Vec<String>> {
    commands(&["git status --porcelain --untracked-files=all"])
}

fn producer_and_gate(check: &str) -> Vec<Vec<String>> {
    commands(&[
        &format!(
            "nix build --no-update-lock-file --no-write-lock-file -L .#checks.x86_64-linux.{check} --out-link .xtask/gcroots/{check}"
        ),
        &format!(
            "nix build --no-update-lock-file --no-write-lock-file -L .#checks.x86_64-linux.{check}-gate --no-link"
        ),
    ])
}

fn coverage() -> Vec<Vec<String>> {
    let mut argv = producer_and_gate("coverage");
    argv.extend(commands(&[
        "cargo xtask coverage-policy --out .xtask/gcroots/coverage",
    ]));
    argv
}

fn doctests() -> Vec<Vec<String>> {
    producer_and_gate("doctests")
}
fn elisp() -> Vec<Vec<String>> {
    let mut argv = vec![nix("elisp-coverage-producer")];
    argv.extend(commands(&["cargo xtask check-leaf elisp-coverage-consume"]));
    argv
}

fn commands(lines: &[&str]) -> Vec<Vec<String>> {
    lines
        .iter()
        .map(|line| line.split(' ').map(str::to_owned).collect())
        .collect()
}

#[test]
fn dirty_fix_check_keeps_doctest_population_while_public_doctests_refuses_dirty_state() {
    let workflow = Workflow::new();
    fs::write(
        workflow.root.path().join("dirty-status"),
        "fixture dirty status",
    )
    .unwrap();

    let (ok, actual, stderr) = workflow.run(&["check"], "");
    assert!(ok, "dirty Fix check must reach its doctest gate: {stderr}");
    assert!(actual.ends_with(&doctests()));
    assert!(
        !actual
            .iter()
            .any(|argv| argv.first().is_some_and(|name| name == "git"))
    );
    assert!(actual.contains(&devtool("fmt", "fix")));

    let (ok, actual, stderr) = workflow.run(&["doctests"], "");
    assert!(!ok, "public doctests must refuse dirty state");
    assert_eq!(actual, public_clean(), "no producer work after refusal");
    assert!(stderr.contains("require a clean checkout"), "{stderr}");
}

#[test]
fn direct_clean_admission_refuses_dirty_or_failed_git_before_work() {
    for recipe in [
        "prepush",
        "validate-no-e2e",
        "ci-validate-host",
        "ci-validate-hermetic",
        "ci-validate-test-checks",
        "ci-validate-coverage",
        "coverage",
        "doctests",
    ] {
        for failure in ["", "git status --porcelain --untracked-files=all"] {
            let workflow = Workflow::new();
            if failure.is_empty() {
                fs::write(workflow.root.path().join("dirty-status"), "dirty fixture").unwrap();
            }
            let (ok, actual, stderr) = workflow.run(&[recipe], failure);
            assert!(!ok, "{recipe} admitted dirty or unavailable Git state");
            assert_eq!(
                actual,
                public_clean(),
                "{recipe} started checks before admission"
            );
            assert!(
                stderr.contains(if failure.is_empty() {
                    "require a clean checkout"
                } else {
                    "17"
                }),
                "{recipe}: {stderr}"
            );
        }
    }
}

#[test]
fn broad_fix_and_verify_preserve_the_actual_complete_success_order() {
    let workflow = Workflow::new();
    let mut check = broad("fix", false);
    check.extend(host_tests());
    for recipe in ["check-no-test", "precommit-broad-host"] {
        let (ok, actual, stderr) = workflow.run(&[recipe], "");
        assert!(ok, "{recipe}: {stderr}");
        assert_eq!(actual, check, "{recipe}");
    }
    check.extend(commands(&["cargo xtask test-local"]));
    check.push(nix("wasm-tests"));
    check.extend(doctests());
    let (ok, actual, stderr) = workflow.run(&["check"], "");
    assert!(ok, "{stderr}");
    assert_eq!(actual, check);

    let mut prepush = public_clean();
    prepush.extend(broad(
        "verify",
        cfg!(all(target_os = "linux", target_arch = "x86_64")),
    ));
    prepush.extend(host_tests());
    prepush.extend(commands(&[
        "cargo xtask test-local",
        "cargo xtask check-leaf workspace-doctests",
    ]));
    let (ok, actual, stderr) = workflow.run(&["prepush"], "");
    assert!(ok, "{stderr}");
    assert_eq!(actual, prepush);
}

#[test]
fn validation_and_ci_lanes_preserve_required_populations_and_argument_boundaries() {
    let workflow = Workflow::new();
    let clean = public_clean();
    let mut host = broad("verify", false);
    host.push(platform());
    let mut hermetic = static_checks();
    hermetic.extend(commands(&["cargo xtask check-leaf wasm-budget"]));
    let mut test_checks = vec![nix("wasm-tests")];
    test_checks.extend(doctests());
    test_checks.extend(elisp());
    for (recipe, mut expected) in [
        ("ci-validate-host", {
            let mut argv = host.clone();
            argv.extend(host_tests());
            argv
        }),
        ("ci-validate-hermetic", hermetic.clone()),
        ("ci-validate-test-checks", test_checks),
        ("ci-validate-coverage", coverage()),
    ] {
        expected.splice(0..0, clean.clone());
        let (ok, actual, stderr) = workflow.run(&[recipe], "");
        assert!(ok, "{recipe}: {stderr}");
        assert_eq!(actual, expected, "{recipe}");
        assert!(
            !workflow
                .root
                .path()
                .join(".xtask/last-result.json")
                .exists()
        );
    }
    let mut validate = clean;
    validate.extend(host);
    validate.extend(hermetic);
    validate.extend(host_tests());
    validate.push(nix("wasm-tests"));
    validate.extend(coverage());
    validate.extend(doctests());
    validate.extend(elisp());
    let (ok, actual, stderr) = workflow.run(&["validate-no-e2e"], "");
    assert!(ok, "{stderr}");
    assert_eq!(actual, validate);
    validate.extend(commands(&["e2e-workflow all"]));
    let (ok, actual, stderr) = workflow.run(&["validate"], "");
    assert!(ok, "{stderr}");
    assert_eq!(actual, validate);
}

#[test]
fn every_required_leaf_failure_is_nonzero_and_stops_subsequent_work() {
    let workflow = Workflow::new();
    let mut tested = std::collections::BTreeSet::new();
    for recipe in [
        "precommit-markdown-host",
        "check",
        "prepush",
        "validate",
        "ci-validate-host",
        "ci-validate-hermetic",
        "ci-validate-test-checks",
        "ci-validate-coverage",
    ] {
        let (ok, success, stderr) = workflow.run(&[recipe], "");
        assert!(ok, "{recipe}: {stderr}");
        for (index, command) in success.iter().enumerate() {
            // Shared leaves need one fault injection, plus each public recipe's
            // first and final boundaries. Avoid repeating the whole host surface.
            if !tested.insert(command.clone()) && index != 0 && index + 1 != success.len() {
                continue;
            }
            let (ok, failed, stderr) = workflow.run(&[recipe], &command.join(" "));
            assert!(!ok, "{recipe} ignored {command:?}: {stderr}");
            assert_eq!(failed, success[..=index], "{recipe}: {command:?}");
            assert!(stderr.contains("17"), "child failure lost: {stderr}");
        }
    }
}

#[test]
fn synchronous_producer_cleanup_finishes_before_just_returns_success_or_failure() {
    for failure in ["", "cargo xtask test-local"] {
        let workflow = Workflow::new();
        let (ok, _, stderr) = workflow.run(&["check"], failure);
        assert_eq!(ok, failure.is_empty(), "{stderr}");
        assert!(!workflow.root.path().join("producer-resource").exists());
        assert_eq!(
            fs::read_to_string(workflow.root.path().join("producer-cleanup")).unwrap(),
            "cleaned\n"
        );
    }
}

#[test]
fn precommit_calls_only_the_focused_single_process_safety_owner() {
    let workflow = Workflow::new();
    for failure in [
        "",
        "cargo run --locked --quiet --manifest-path tools/Cargo.toml -p repository-policy --bin git-index-safety -- precommit",
    ] {
        let (ok, actual, stderr) = workflow.run(&["precommit"], failure);
        assert_eq!(ok, failure.is_empty(), "{stderr}");
        assert_eq!(
            actual,
            commands(&[
                "cargo run --locked --quiet --manifest-path tools/Cargo.toml -p repository-policy --bin git-index-safety -- precommit"
            ])
        );
    }
}

#[test]
fn actual_just_parser_rejects_old_option_spellings_instead_of_silently_forwarding() {
    let workflow = Workflow::new();
    for argv in [
        vec!["check", "--no-test"],
        vec!["validate", "--no-e2e"],
        vec!["ci-validate", "host"],
    ] {
        let (ok, actual, _) = workflow.run(&argv, "");
        assert!(!ok, "unexpectedly accepted {argv:?}");
        assert!(actual.is_empty(), "invalid syntax started work: {argv:?}");
    }
}

#[test]
fn hooks_invoke_only_their_just_workflow_and_preserve_its_status() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    for (hook, recipe) in [("pre-commit", "precommit"), ("pre-push", "prepush")] {
        for status in [0, 17] {
            let fixture = tempfile::tempdir().unwrap();
            let just = fixture.path().join("just");
            fs::write(
                &just,
                "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HOOK_ARGS\"\nexit \"$HOOK_STATUS\"\n",
            )
            .unwrap();
            fs::set_permissions(&just, fs::Permissions::from_mode(0o755)).unwrap();
            let args = fixture.path().join("args");
            let output = Command::new("bash")
                .arg(root.join(".githooks").join(hook))
                .current_dir(fixture.path())
                .env(
                    "PATH",
                    format!(
                        "{}:{}",
                        fixture.path().display(),
                        std::env::var("PATH").unwrap()
                    ),
                )
                .env("HOOK_ARGS", &args)
                .env("HOOK_STATUS", status.to_string())
                .env("SKIP_PRE_COMMIT", "0")
                .env("SKIP_PRE_PUSH", "0")
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(status), "{hook}: {output:?}");
            assert_eq!(fs::read_to_string(args).unwrap(), format!("{recipe}\n"));
        }
    }
}

#[test]
fn ci_keeps_independent_job_verdicts_and_producer_artifacts_not_gate_sidecars() {
    let source = include_str!("../../.github/workflows/ci.yml");
    let workflow: serde_yaml::Value = serde_yaml::from_str(source).unwrap();
    let jobs = &workflow["jobs"];
    for lane in ["host", "hermetic", "test-checks", "coverage"] {
        let id = format!("validate-{lane}");
        let job = &jobs[&id];
        let steps = job["steps"].as_sequence().unwrap();
        let commands = steps
            .iter()
            .filter_map(|step| step["run"].as_str())
            .map(|run| run.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|run| run.contains("just ci-validate-"))
            .collect::<Vec<_>>();
        assert_eq!(
            commands,
            [format!(
                "nix develop .#ci --accept-flake-config -c just ci-validate-{lane}"
            )]
        );
        let encoded = serde_yaml::to_string(job).unwrap();
        assert!(!encoded.contains("last-result.json"));
        assert!(!encoded.contains(&format!("{lane}-result.json")));
    }
    let aggregate = &jobs["validate-no-e2e"];
    assert_eq!(aggregate["name"], "Validate (no e2e)");
    assert_eq!(aggregate["if"], "always()");
    assert_eq!(
        aggregate["needs"],
        serde_yaml::to_value([
            "validate-host",
            "validate-hermetic",
            "validate-test-checks",
            "validate-coverage"
        ])
        .unwrap()
    );
    let verdict = aggregate["steps"][0]["run"].as_str().unwrap();
    for (lane, variable) in [
        ("host", "host"),
        ("hermetic", "hermetic"),
        ("test-checks", "test_checks"),
        ("coverage", "coverage"),
    ] {
        assert!(verdict.contains(&format!("needs.validate-{lane}.result")));
        assert!(verdict.contains(&format!("test \"${variable}\" = \"success\"")));
    }
    assert!(source.contains(".xtask/gcroots/coverage/status.json"));
    assert!(source.contains(".xtask/gcroots/elisp-coverage-producer/elisp-coverage/"));
    assert_eq!(
        jobs["e2e-gate"]["needs"],
        serde_yaml::to_value([
            "e2e-catalog",
            "e2e-chromium",
            "e2e-firefox-lane",
            "e2e-firefox-reconcile",
        ])
        .unwrap()
    );
    assert_eq!(jobs["e2e-gate"]["if"], "always()");
    for producer in ["e2e-chromium", "e2e-firefox-lane"] {
        assert_eq!(
            jobs[producer]["needs"],
            serde_yaml::to_value(["e2e-catalog"]).unwrap()
        );
    }
    let reconcile = &jobs["e2e-firefox-reconcile"];
    assert_eq!(
        reconcile["needs"],
        serde_yaml::to_value(["e2e-catalog", "e2e-firefox-lane"]).unwrap()
    );
    assert_eq!(reconcile["if"], "always()");
    let lines = jobs["e2e-gate"]["steps"]
        .as_sequence()
        .unwrap()
        .iter()
        .filter_map(|step| step["run"].as_str())
        .flat_map(str::lines)
        .map(str::trim)
        .collect::<std::collections::BTreeSet<_>>();
    for binding in [
        "chromium=\"${{ needs.e2e-chromium.result }}\"",
        "firefox_lanes=\"${{ needs.e2e-firefox-lane.result }}\"",
        "firefox_reconcile=\"${{ needs.e2e-firefox-reconcile.result }}\"",
    ] {
        assert!(lines.contains(binding), "aggregate missing {binding}");
    }
    assert_eq!(
        lines
            .iter()
            .copied()
            .filter(|line| line.starts_with("test "))
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            "test \"${{ needs.e2e-catalog.result }}\" = \"success\"",
            "test \"$chromium\" = \"success\"",
            "test \"$firefox_lanes\" = \"success\"",
            "test \"$firefox_reconcile\" = \"success\"",
        ])
    );
}

#[test]
fn cargo_recipes_preserve_cache_environment_and_normalize_worktree_roots() {
    let workflow = Workflow::new();
    std::os::unix::fs::symlink(
        workflow.root.path(),
        workflow.root.path().join("cache-link"),
    )
    .unwrap();
    for recipe in ["_host-tests", "check-no-test"] {
        let (ok, _, stderr) = workflow.run(&[recipe], "");
        assert!(ok, "{stderr}");
        assert_eq!(
            fs::read_to_string(workflow.root.path().join("cache-env")).unwrap(),
            format!(
                "sccache\n0\n{}\n",
                workflow.root.path().canonicalize().unwrap().display()
            )
        );
    }
}

#[test]
fn host_devtool_launch_reflects_source_edits_without_shell_reentry() {
    let workflow = Workflow::new();
    // Exercise the maintained ERT launch, not the retired TSC devtool consumer.
    // Only composition is reduced: the actual variable and request line stay intact.
    let source = fs::read_to_string(workflow.root.path().join("Justfile")).unwrap();
    let owner = source
        .lines()
        .find(|line| line.starts_with("devtool :="))
        .unwrap();
    let request = source
        .lines()
        .find(|line| line.contains("{{devtool}} check ert"))
        .unwrap();
    fs::write(
        workflow.root.path().join("Justfile"),
        format!("{owner}\nert mode=\"verify\":\n{request}\n"),
    )
    .unwrap();
    fs::remove_file(workflow.root.path().join("bin/cargo")).unwrap();
    let tools = workflow.root.path().join("tools");
    fs::create_dir_all(tools.join("src")).unwrap();
    fs::write(
        tools.join("Cargo.toml"),
        "[package]\nname = \"devtool\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n",
    )
    .unwrap();
    // A dependency-free lockfile permits the exact --locked host launch.
    fs::write(
        tools.join("Cargo.lock"),
        "version = 4\n[[package]]\nname = \"devtool\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    for version in ["first", "edited"] {
        fs::write(
            tools.join("src/main.rs"),
            format!(
                "fn main() {{ std::fs::write(\"source-version\", \"{version}\").unwrap(); std::process::exit(17); }}"
            ),
        )
        .unwrap();
        let (ok, _, stderr) = workflow.run(&["ert", "verify"], "");
        assert!(!ok, "fixture must return the retained ERT workload failure");
        assert!(stderr.contains("17"), "{stderr}");
        assert_eq!(
            fs::read_to_string(workflow.root.path().join("source-version")).unwrap(),
            version
        );
    }
}

#[test]
fn markdown_host_runs_the_complete_narrow_fix_surface_in_order() {
    let workflow = Workflow::new();
    let (ok, actual, stderr) = workflow.run(&["precommit-markdown-host"], "");
    assert!(ok, "{stderr}");
    assert_eq!(
        actual,
        commands(&[
            "prettier -w **/*.md",
            "prettier -w end2end",
            "cargo run --locked --quiet --manifest-path tools/Cargo.toml -p repository-policy --bin repository-docs -- sequence-check",
            "cargo run --locked --quiet --manifest-path tools/Cargo.toml -p repository-policy --bin repository-docs -- adr-format",
            "cargo run --locked --quiet --manifest-path tools/Cargo.toml -p repository-policy --bin repository-docs -- adr-readme-parity",
            "cargo run --locked --quiet --manifest-path tools/Cargo.toml -p repository-policy --bin repository-docs -- adr-view-parity",
            "cargo run --locked --quiet --manifest-path tools/Cargo.toml -p repository-policy --bin repository-docs -- doc-links",
            "cargo run --locked --quiet --manifest-path tools/Cargo.toml -p repository-policy --bin repository-docs -- flow-docs",
        ])
    );
}
