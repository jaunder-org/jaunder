//! Fixed-action compatibility is a process boundary, not a second mutation owner.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use std::os::unix::fs::PermissionsExt;

const REDIRECTS: [&str; 6] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
];

fn fixture_command(binary: &Path, root: &Path) -> Command {
    let mut command = Command::new(binary);
    command.current_dir(root);
    for name in REDIRECTS {
        command.env_remove(name);
    }
    command
}

fn write(root: &Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn fixed_delegates_preserve_argv_diagnostics_and_child_exit() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let nix = root.join("bin/nix");
    write(root, "flake.nix", b"{}\n");
    write(root, "xtask/Cargo.toml", b"[package]\n");
    checked(
        fixture_command(Path::new("git"), root)
            .args(["init", "-q"])
            .output()
            .unwrap(),
    );
    std::fs::create_dir(root.join("nested")).unwrap();
    write(root, "bin/nix", br#"#!/usr/bin/env bash
set -eu
[[ "$*" == "run --accept-flake-config --no-update-lock-file --no-write-lock-file .#adr-promoter -- $EXPECTED_ACTION" ]] || exit 91
[[ -z ${GIT_INDEX_FILE+x} && -z ${GIT_DIR+x} ]] || exit 92
[[ "$PWD" == "$EXPECTED_ROOT" ]] || exit 93
printf 'child stdout\n'
printf 'child stderr\n' >&2
exit "$CHILD_EXIT"
"#);
    std::fs::set_permissions(&nix, std::fs::Permissions::from_mode(0o755)).unwrap();
    checked(
        fixture_command(Path::new("git"), root)
            .args(["add", "."])
            .output()
            .unwrap(),
    );
    for action in ["promote", "sync-readme", "promoter"] {
        for code in [0, 37] {
            let output =
                fixture_command(Path::new(env!("CARGO_BIN_EXE_xtask")), &root.join("nested"))
                    .args(["adr", action])
                    .env(
                        "PATH",
                        format!(
                            "{}:{}",
                            root.join("bin").display(),
                            std::env::var("PATH").unwrap()
                        ),
                    )
                    .env(
                        "EXPECTED_ACTION",
                        if action == "promoter" {
                            "controller"
                        } else {
                            action
                        },
                    )
                    .env(
                        "GITHUB_EVENT_PATH",
                        root.join("deliberately-absent-event.json"),
                    )
                    .env("GITHUB_REPOSITORY", "fixture/repo")
                    .env_remove("GH_TOKEN")
                    .env_remove("GITHUB_TOKEN")
                    .env_remove("GH_ENTERPRISE_TOKEN")
                    .env_remove("GITHUB_ENTERPRISE_TOKEN")
                    .env("EXPECTED_ROOT", root)
                    .env("CHILD_EXIT", code.to_string())
                    .env("GIT_INDEX_FILE", "/not-this-fixture")
                    .env("GIT_DIR", "/not-this-fixture")
                    .output()
                    .unwrap();
            assert_eq!(output.status.code(), Some(code), "{output:?}");
            assert_eq!(output.stdout, b"child stdout\n");
            assert_eq!(output.stderr, b"child stderr\n");
        }
    }
}

// Parse the real workflow, not a grep of its final command: privilege and trusted
// code selection must survive the entry-point migration unchanged.
#[test]
fn workflow_keeps_trusted_authority_and_selects_just() {
    let actual: serde_yaml::Value =
        serde_yaml::from_str(include_str!("../../.github/workflows/adr-promoter.yml")).unwrap();
    let expected: serde_yaml::Value = serde_yaml::from_str(
        r#"
name: ADR promoter
on:
  push:
    branches: [main]
  workflow_dispatch:
  pull_request:
    branches: [main]
    types: [dequeued]
permissions:
  contents: read
concurrency:
  group: adr-promoter-${{ github.event.pull_request.number || 'generate' }}
  cancel-in-progress: false
jobs:
  promote:
    name: Promote pending ADR drafts
    runs-on: ubuntu-24.04
    steps:
      - name: Mint ADR promoter token
        id: app-token
        uses: actions/create-github-app-token@v3
        with:
          client-id: ${{ vars.ADR_PROMOTER_CLIENT_ID }}
          private-key: ${{ secrets.ADR_PROMOTER_APP_PRIVATE_KEY }}
          permission-actions: read
          permission-contents: write
          permission-pull-requests: write
          permission-checks: read
          permission-statuses: read
          permission-metadata: read
      - name: Checkout trusted main
        uses: actions/checkout@v7.0.1
        with:
          ref: refs/heads/main
          fetch-depth: 0
          token: ${{ steps.app-token.outputs.token }}
      - uses: ./.github/actions/setup-ci
        with:
          github-token: ${{ github.token }}
          cachix-auth-token: ${{ secrets.CACHIX_AUTH_TOKEN }}
      - name: Run ADR promoter
        env:
          GH_TOKEN: ${{ steps.app-token.outputs.token }}
        run: nix develop .#ci --accept-flake-config -c just adr-promoter
"#,
    )
    .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn offline_workflow_just_and_legacy_routes_preserve_controller_outcomes() {
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    // Realize the actual locked output before entering any fake environment.
    // The production outer wrapper pins real gh/Git, so this intentionally
    // executes its identical inner Python console with fake process children.
    let selected = fixture_command(Path::new("nix"), checkout)
        .args([
            "build",
            "--offline",
            "--print-out-paths",
            "--no-link",
            "--accept-flake-config",
            "--no-update-lock-file",
            "--no-write-lock-file",
            ".#adr-promoter.pythonPackage",
        ])
        .output()
        .unwrap();
    assert!(selected.status.success(), "{selected:?}");
    let selected = String::from_utf8(selected.stdout).unwrap();
    let console = Path::new(selected.trim()).join("bin/adr-promoter");
    assert!(console.is_file());
    let real_git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("git"))
        .find(|path| path.is_file())
        .unwrap();
    let workflow: serde_yaml::Value =
        serde_yaml::from_str(include_str!("../../.github/workflows/adr-promoter.yml")).unwrap();
    let run = workflow["jobs"]["promote"]["steps"][3]["run"]
        .as_str()
        .unwrap();
    for route in ["workflow", "just", "legacy"] {
        for scenario in ["ignored", "not-rearmed", "error"] {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path();
            write(root, "flake.nix", b"{}\n");
            write(root, "xtask/Cargo.toml", b"[package]\n");
            write(root, "Justfile", include_bytes!("../../Justfile"));
            write(root, ".gitignore", b".trace/\n");
            std::fs::create_dir(root.join(".trace")).unwrap();
            let action = if scenario == "ignored" {
                "opened"
            } else {
                "dequeued"
            };
            write(root, "event.json", format!(r#"{{"action":"{action}","pull_request":{{"number":7,"head":{{"ref":"automation/adr-promoter","sha":"head"}},"base":{{"ref":"main","sha":"base"}}}}}}"#).as_bytes());
            write(root, "bin/nix", br#"#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> .trace/nix
if [[ "$*" == 'develop .#ci --accept-flake-config -c just adr-promoter' ]]; then
  exec just adr-promoter
fi
[[ "$*" == 'run --accept-flake-config --no-update-lock-file --no-write-lock-file .#adr-promoter -- controller' ]] || exit 91
exec "$SELECTED_CLI" controller
"#);
            write(
                root,
                "bin/git",
                br#"#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> .trace/git
args=("$@")
if [[ ${args[0]:-} == -C ]]; then args=("${args[@]:2}"); fi
case "${args[0]:-}" in
  rev-parse|ls-files) ;;
  worktree) [[ "${args[*]}" == 'worktree list --porcelain' ]] || exit 92 ;;
  *) echo 'fixture blocked unexpected Git operation' >&2; exit 93 ;;
esac
exec "$REAL_GIT" "$@"
"#,
            );
            write(
                root,
                "bin/gh",
                br#"#!/usr/bin/env bash
set -eu
printf '%s\n' "$*" >> .trace/gh
[[ "$SCENARIO" != ignored ]] || exit 94
[[ "$*" == 'api /repos/fixture/repo/pulls/7' ]] || exit 95
if [[ "$SCENARIO" == not-rearmed ]]; then
  printf '{"status":"404"}\n'; exit 1
fi
printf '{"errors":[{"message":"sandbox query failure"}]}\n'
"#,
            );
            for binary in ["nix", "git", "gh"] {
                std::fs::set_permissions(
                    root.join("bin").join(binary),
                    std::fs::Permissions::from_mode(0o755),
                )
                .unwrap();
            }
            checked(
                fixture_command(&real_git, root)
                    .args(["init", "-q"])
                    .output()
                    .unwrap(),
            );
            checked(
                fixture_command(&real_git, root)
                    .args(["add", "."])
                    .output()
                    .unwrap(),
            );
            let mut command = match route {
                "workflow" => {
                    let mut words = run.split_whitespace();
                    let mut command = fixture_command(Path::new(words.next().unwrap()), root);
                    command.args(words);
                    command
                }
                "just" => {
                    let mut command = fixture_command(Path::new("just"), root);
                    command.arg("adr-promoter");
                    command
                }
                _ => {
                    let mut command = fixture_command(Path::new(env!("CARGO_BIN_EXE_xtask")), root);
                    command.args(["adr", "promoter"]);
                    command
                }
            };
            let output = command
                .env(
                    "PATH",
                    format!(
                        "{}:{}",
                        root.join("bin").display(),
                        std::env::var("PATH").unwrap()
                    ),
                )
                .env("SELECTED_CLI", &console)
                .env("REAL_GIT", &real_git)
                .env("SCENARIO", scenario)
                .env("GITHUB_REPOSITORY", "fixture/repo")
                .env("GITHUB_EVENT_PATH", root.join("event.json"))
                .env_remove("GH_TOKEN")
                .env_remove("GITHUB_TOKEN")
                .env_remove("GH_ENTERPRISE_TOKEN")
                .env_remove("GITHUB_ENTERPRISE_TOKEN")
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(if scenario == "error" { 1 } else { 0 }),
                "{route}/{scenario}: {output:?}"
            );
            let expected = match scenario {
                "ignored" => "event does not target the ADR promoter\n",
                "not-rearmed" => "promoter PR not re-armed: pull request is absent\n",
                _ => "",
            };
            assert_eq!(output.stdout, expected.as_bytes(), "{route}/{scenario}");
            let stderr = String::from_utf8(output.stderr).unwrap();
            let diagnostic = "adr-promoter: graphql: [{\"message\": \"sandbox query failure\"}]\n";
            if scenario == "error" {
                assert!(stderr.contains(diagnostic), "{stderr}");
            }
            if route == "legacy" {
                assert_eq!(stderr, if scenario == "error" { diagnostic } else { "" });
            }
            let calls = std::fs::read_to_string(root.join(".trace/nix")).unwrap();
            let selection = "run --accept-flake-config --no-update-lock-file --no-write-lock-file .#adr-promoter -- controller\n";
            assert_eq!(
                calls,
                if route == "workflow" {
                    format!("develop .#ci --accept-flake-config -c just adr-promoter\n{selection}")
                } else {
                    selection.to_string()
                }
            );
            if scenario == "ignored" {
                assert!(!root.join(".trace/gh").exists());
            } else {
                assert_eq!(
                    std::fs::read_to_string(root.join(".trace/gh")).unwrap(),
                    "api /repos/fixture/repo/pulls/7\n"
                );
            }
            // No fixture mutation was authorized; fake Git blocks every writer.
            assert!(
                fixture_command(&real_git, root)
                    .args(["diff", "--exit-code"])
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
    }
}

fn checked(output: Output) {
    assert!(output.status.success(), "{output:?}");
}

fn admission_fixture(root: &Path) {
    let checkout = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let predicate = checkout.join("nix/adr-promoter-source.nix");
    let flake = format!(
        r#"{{ outputs = {{ self }}: {{ packages = builtins.listToAttrs (map (name: {{
      inherit name;
      value.adr-promoter.sourceAdmits = import (builtins.toPath {predicate:?});
    }}) [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ]); }}; }}"#
    );
    write(root, "flake.nix", flake.as_bytes());
    write(root, "xtask/Cargo.toml", b"[package]\n");
    write(
        root,
        "docs/adr/drafts/one.md",
        b"# ADR-DRAFT: One\n\n- Status: proposed\n",
    );
    write(
        root,
        "docs/README.md",
        b"<!-- adr-table:begin -->\n<!-- adr-table:end -->\n",
    );
    write(
        root,
        ".gitignore",
        b"ignored/\ntools/adr-promoter/tests/ignored.py\n",
    );
    write(
        root,
        "tools/adr-promoter/tests/ignored.py",
        b"ignored despite predicate membership\n",
    );
    checked(
        fixture_command(Path::new("git"), root)
            .args(["init", "-q"])
            .output()
            .unwrap(),
    );
    checked(
        fixture_command(Path::new("git"), root)
            .args(["add", "."])
            .output()
            .unwrap(),
    );
}

#[test]
fn public_delegates_refuse_omitted_python_inputs_before_child_mutation() {
    let real_nix = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("nix"))
        .find(|path| path.is_file())
        .unwrap();
    for action in ["promote", "sync-readme"] {
        for input in [
            "tools/adr-promoter/adr_promoter/new.py",
            "tools/adr-promoter/tests/new.py",
            "tools/adr-promoter/pyproject.toml",
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path();
            admission_fixture(root);
            write(
                root,
                "bin/nix",
                br#"#!/usr/bin/env bash
set -eu
if [[ "$1" == eval ]]; then exec "$REAL_NIX" "$@"; fi
printf 'child stdout\n'
printf 'child stderr\n' >&2
printf mutated > child-ran
exit 37
"#,
            );
            std::fs::set_permissions(root.join("bin/nix"), std::fs::Permissions::from_mode(0o755))
                .unwrap();
            checked(
                fixture_command(Path::new("git"), root)
                    .args(["add", "bin/nix"])
                    .output()
                    .unwrap(),
            );
            write(root, input, b"fixture source\n");
            let invoke = || {
                fixture_command(Path::new(env!("CARGO_BIN_EXE_xtask")), root)
                    .args(["adr", action])
                    .env(
                        "PATH",
                        format!(
                            "{}:{}",
                            root.join("bin").display(),
                            std::env::var("PATH").unwrap()
                        ),
                    )
                    .env("REAL_NIX", &real_nix)
                    .output()
                    .unwrap()
            };
            let before = fixture_command(Path::new("git"), root)
                .args(["write-tree"])
                .output()
                .unwrap()
                .stdout;
            let refused = invoke();
            assert!(!refused.status.success(), "{refused:?}");
            assert!(
                String::from_utf8_lossy(&refused.stderr).contains(input),
                "{refused:?}"
            );
            assert!(
                String::from_utf8_lossy(&refused.stderr).contains("git add -N"),
                "{refused:?}"
            );
            assert!(!root.join("child-ran").exists());
            assert_eq!(
                before,
                fixture_command(Path::new("git"), root)
                    .args(["write-tree"])
                    .output()
                    .unwrap()
                    .stdout
            );
            checked(
                fixture_command(Path::new("git"), root)
                    .args(["add", "-N", input])
                    .output()
                    .unwrap(),
            );
            write(root, "notes.md", b"unrelated\n");
            write(root, "tools/adr-promoter/data.txt", b"not admitted\n");
            write(root, "ignored/secret.py", b"ignored\n");
            let admitted = invoke();
            assert_eq!(admitted.status.code(), Some(37), "{admitted:?}");
            assert_eq!(admitted.stdout, b"child stdout\n");
            assert_eq!(admitted.stderr, b"child stderr\n");
            assert!(root.join("child-ran").exists());
        }
    }
}

#[test]
fn selected_package_refuses_omitted_inputs_then_mutates_after_intent_to_add() {
    let checkout = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    for action in ["promote", "sync-readme", "controller"] {
        for input in [
            "tools/adr-promoter/adr_promoter/new.py",
            "tools/adr-promoter/tests/new.py",
            "tools/adr-promoter/pyproject.toml",
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path();
            admission_fixture(root);
            write(root, input, b"fixture source\n");
            std::fs::create_dir(root.join("nested")).unwrap();
            let invoke = || {
                fixture_command(Path::new("nix"), &root.join("nested"))
                    .args([
                        "run",
                        "--offline",
                        "--accept-flake-config",
                        "--no-update-lock-file",
                        "--no-write-lock-file",
                    ])
                    .arg(format!("{}#adr-promoter", checkout.display()))
                    .args(["--", action])
                    // Never inherit a real workflow event/token: this controller
                    // probe must fail on a missing sandbox event before any API.
                    .env("GITHUB_EVENT_PATH", root.join("missing-fixture-event.json"))
                    .env("GITHUB_REPOSITORY", "fixture/repo")
                    .env_remove("GH_TOKEN")
                    .env_remove("GITHUB_TOKEN")
                    .env_remove("GH_ENTERPRISE_TOKEN")
                    .env_remove("GITHUB_ENTERPRISE_TOKEN")
                    .output()
                    .unwrap()
            };
            let before = fixture_command(Path::new("git"), root)
                .args(["write-tree"])
                .output()
                .unwrap()
                .stdout;
            let refused = invoke();
            assert!(!refused.status.success(), "{refused:?}");
            assert!(
                String::from_utf8_lossy(&refused.stderr).contains(input),
                "{refused:?}"
            );
            assert!(
                String::from_utf8_lossy(&refused.stderr).contains("git add -N"),
                "{refused:?}"
            );
            assert_eq!(
                before,
                fixture_command(Path::new("git"), root)
                    .args(["write-tree"])
                    .output()
                    .unwrap()
                    .stdout
            );
            assert!(root.join("docs/adr/drafts/one.md").exists());
            assert_eq!(
                std::fs::read(root.join("docs/README.md")).unwrap(),
                b"<!-- adr-table:begin -->\n<!-- adr-table:end -->\n"
            );
            checked(
                fixture_command(Path::new("git"), root)
                    .args(["add", "-N", input])
                    .output()
                    .unwrap(),
            );
            write(root, "notes.md", b"unrelated\n");
            write(root, "tools/adr-promoter/data.txt", b"not admitted\n");
            write(root, "ignored/secret.py", b"ignored\n");
            let admitted = invoke();
            if action == "controller" {
                assert_eq!(admitted.status.code(), Some(1), "{admitted:?}");
                assert!(
                    String::from_utf8_lossy(&admitted.stderr)
                        .contains("missing-fixture-event.json")
                );
                continue;
            }
            assert!(admitted.status.success(), "{admitted:?}");
            if action == "promote" {
                assert_eq!(
                    std::fs::read(root.join("docs/adr/0000-one.md")).unwrap(),
                    b"# ADR-0000: One\n\n- Status: accepted\n"
                );
            } else {
                assert!(String::from_utf8_lossy(&admitted.stdout).contains("rows"));
                assert!(root.join("docs/adr/drafts/one.md").exists());
            }
        }
    }
}

#[test]
fn python_promotion_output_passes_existing_rust_gates() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let files: [(&str, &[u8]); 7] = [
        ("docs/adr/0001-old.md", b"# ADR-0001: Old\n\n- Status: accepted\n"),
        ("docs/adr/drafts/alpha.md", b"# ADR-DRAFT: Alpha\n\n- Status: proposed\n\n[beta](../drafts/beta.md)\n"),
        ("docs/adr/drafts/beta.md", b"# ADR-DRAFT: Beta\n\n- Status: proposed\n\n[alpha](../drafts/alpha.md)\n"),
        ("docs/README.md", b"<!-- adr-table:begin -->\n| [0001](adr/0001-old.md) | Curated | accepted |\n<!-- adr-table:end -->\n"),
        ("docs/ARCHITECTURE.md", b"adr/0001-old.md adr/drafts/alpha.md adr/drafts/beta.md\n"),
        ("citation.txt", b"docs/adr/drafts/alpha.md drafts/beta\n"),
        ("docs/adr/drafts/README.md", b"Authoring guide\n"),
    ];
    for (path, bytes) in files {
        write(root, path, bytes);
    }
    write(root, "flake.nix", b"{}\n");
    write(root, "xtask/Cargo.toml", b"[package]\n");
    checked(
        fixture_command(Path::new("git"), root)
            .args(["init", "-q"])
            .output()
            .unwrap(),
    );
    checked(
        fixture_command(Path::new("git"), root)
            .args(["add", "."])
            .output()
            .unwrap(),
    );
    // Select the real current checkout package while executing only in this fixture.
    let checkout = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let output = fixture_command(Path::new("nix"), root)
        .args([
            "run",
            "--offline",
            "--accept-flake-config",
            "--no-update-lock-file",
            "--no-write-lock-file",
        ])
        .arg(format!("{}#adr-promoter", checkout.display()))
        .args(["--", "promote"])
        .output()
        .unwrap();
    checked(output);
    assert_eq!(
        std::fs::read(root.join("docs/adr/0002-alpha.md")).unwrap(),
        b"# ADR-0002: Alpha\n\n- Status: accepted\n\n[beta](0003-beta.md)\n"
    );
    assert_eq!(
        std::fs::read(root.join("docs/adr/0003-beta.md")).unwrap(),
        b"# ADR-0003: Beta\n\n- Status: accepted\n\n[alpha](0002-alpha.md)\n"
    );
    assert_eq!(
        std::fs::read(root.join("citation.txt")).unwrap(),
        b"docs/adr/0002-alpha.md 0003-beta\n"
    );
    for gate in [
        "adr-format",
        "adr-readme-parity",
        "adr-view-parity",
        "doc-links",
    ] {
        let output = fixture_command(Path::new(env!("CARGO_BIN_EXE_xtask")), root)
            .args(["check-leaf", gate])
            .output()
            .unwrap();
        assert!(output.status.success(), "{gate}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(&format!("[ ok ] {gate}")),
            "{output:?}"
        );
    }
}
