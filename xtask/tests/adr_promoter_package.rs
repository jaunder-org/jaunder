//! One selected-package integration smoke, with no GitHub authority or network data.

use std::path::Path;
use std::process::{Command, Output};

fn command(binary: &str, root: &Path) -> Command {
    let mut command = Command::new(binary);
    command.current_dir(root);
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "GH_ENTERPRISE_TOKEN",
        "GITHUB_ENTERPRISE_TOKEN",
    ] {
        command.env_remove(name);
    }
    command
}

fn checked(output: Output) -> Vec<u8> {
    assert!(output.status.success(), "{output:?}");
    output.stdout
}

#[test]
fn selected_package_promotes_complete_documents_and_passes_read_only_gates() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let files: [(&str, &[u8]); 10] = [
        ("flake.nix", b"{}\n"),
        ("xtask/Cargo.toml", b"[package]\n"),
        ("docs/adr/0001-old.md", b"# ADR-0001: Old\n\n- Status: accepted\n"),
        ("docs/adr/drafts/alpha.md", b"# ADR-DRAFT: Alpha\r\n\r\n- Status: proposed\r\n\r\n[beta](../drafts/beta.md)\r\n"),
        ("docs/adr/drafts/beta.md", b"# ADR-DRAFT: Beta\n\n- Status: proposed\n\n[alpha](../drafts/alpha.md)\n"),
        ("docs/README.md", b"<!-- adr-table:begin -->\n| [0001](adr/0001-old.md) | Curated | accepted |\n<!-- adr-table:end -->\n"),
        ("docs/ARCHITECTURE.md", b"adr/0001-old.md adr/drafts/alpha.md adr/drafts/beta.md\n"),
        ("citation.txt", b"docs/adr/drafts/alpha.md drafts/beta\n"),
        ("docs/adr/drafts/README.md", b"Authoring guide\n"),
        ("event.json", br#"{"action":"opened","pull_request":{"number":7,"head":{"ref":"automation/adr-promoter","sha":"head"},"base":{"ref":"main","sha":"base"}}}"#),
    ];
    for (path, bytes) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let predicate = checkout.join("nix/adr-promoter-source.nix");
    std::fs::write(
        root.join("flake.nix"),
        format!(
            r#"{{ outputs = {{ self }}: {{ packages = builtins.listToAttrs (map (name: {{
          inherit name;
          value.adr-promoter.sourceAdmits = import (builtins.toPath {predicate:?});
        }}) [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ]); }}; }}"#
        ),
    )
    .unwrap();
    checked(command("git", root).args(["init", "-q"]).output().unwrap());
    checked(command("git", root).args(["add", "."]).output().unwrap());
    let selected = format!("{}#adr-promoter", checkout.display());
    let request = |action: &str| {
        command("nix", root)
            .args([
                "run",
                "--accept-flake-config",
                "--no-update-lock-file",
                "--no-write-lock-file",
                &selected,
                "--",
                action,
            ])
            .env("GITHUB_EVENT_PATH", root.join("event.json"))
            .output()
            .unwrap()
    };
    let run = |action: &str| checked(request(action));
    assert_eq!(
        run("controller"),
        b"event does not target the ADR promoter\n"
    );
    let input = "tools/adr-promoter/adr_promoter/new.py";
    std::fs::create_dir_all(root.join(input).parent().unwrap()).unwrap();
    std::fs::write(root.join(input), b"# admitted fixture input\n").unwrap();
    let before = checked(command("git", root).arg("write-tree").output().unwrap());
    let refused = request("promote");
    assert!(!refused.status.success() && refused.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&refused.stderr)
            .contains("Git flakes omit untracked declared inputs")
    );
    assert_eq!(
        checked(command("git", root).arg("write-tree").output().unwrap()),
        before
    );
    assert!(root.join("docs/adr/drafts/alpha.md").exists());
    checked(command("git", root).args(["add", input]).output().unwrap());
    assert!(
        String::from_utf8(run("promote"))
            .unwrap()
            .ends_with("; staged\n")
    );
    assert_eq!(
        std::fs::read(root.join("docs/adr/0002-alpha.md")).unwrap(),
        b"# ADR-0002: Alpha\r\n\r\n- Status: accepted\r\n\r\n[beta](0003-beta.md)\r\n"
    );
    assert_eq!(
        std::fs::read(root.join("docs/adr/0003-beta.md")).unwrap(),
        b"# ADR-0003: Beta\n\n- Status: accepted\n\n[alpha](0002-alpha.md)\n"
    );
    assert_eq!(
        std::fs::read(root.join("citation.txt")).unwrap(),
        b"docs/adr/0002-alpha.md 0003-beta\n"
    );
    assert_eq!(
        std::fs::read(root.join("docs/ARCHITECTURE.md")).unwrap(),
        b"adr/0001-old.md adr/0002-alpha.md adr/0003-beta.md\n"
    );
    assert_eq!(std::fs::read(root.join("docs/README.md")).unwrap(),
               b"<!-- adr-table:begin -->\n\n| #   | Title | Status |\n| --- | ----- | ------ |\n| [0001](adr/0001-old.md) | Curated | accepted |\n| [0002](adr/0002-alpha.md) | Alpha | accepted |\n| [0003](adr/0003-beta.md) | Beta | accepted |\n\n<!-- adr-table:end -->\n");
    assert_eq!(run("sync-readme"), b"3 rows, already in sync\n");
    assert_eq!(
        checked(
            command("git", root)
                .args(["diff", "--name-only"])
                .output()
                .unwrap()
        ),
        b""
    );
    let staged = checked(
        command("git", root)
            .args(["diff", "--cached", "--name-only"])
            .output()
            .unwrap(),
    );
    let staged = String::from_utf8(staged).unwrap();
    assert!(
        staged.contains("docs/adr/0002-alpha.md") && !staged.contains("docs/adr/drafts/alpha.md")
    );
    assert!(repository_policy::adr::format_problems(root).is_empty());
    for gate in ["adr-readme-parity", "adr-view-parity", "doc-links"] {
        let output = checked(
            command(env!("CARGO_BIN_EXE_xtask"), root)
                .args(["check-leaf", gate])
                .output()
                .unwrap(),
        );
        assert!(String::from_utf8_lossy(&output).contains(&format!("[ ok ] {gate}")));
    }
}
