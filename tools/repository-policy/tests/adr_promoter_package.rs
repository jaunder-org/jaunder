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
fn selected_package_syncs_readme_after_admission_and_passes_read_only_gates() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let drifted: &[u8] =
        b"<!-- adr-table:begin -->\n| [0001](adr/wrong.md) | Curated | rejected |\n<!-- adr-table:end -->\n";
    let files: [(&str, &[u8]); 6] = [
        ("flake.nix", b"{}\n"),
        ("docs/adr/0001-old.md", b"# ADR-0001: Old\n\n- Status: accepted\n"),
        ("docs/README.md", drifted),
        ("docs/ARCHITECTURE.md", b"adr/0001-old.md\n"),
        ("docs/adr/drafts/README.md", b"Authoring guide\n"),
        ("event.json", br#"{"action":"opened","pull_request":{"number":7,"head":{"ref":"automation/adr-promoter","sha":"head"},"base":{"ref":"main","sha":"base"}}}"#),
    ];
    for (path, bytes) in files {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap();
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
    let refused = request("sync-readme");
    assert!(!refused.status.success() && refused.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&refused.stderr)
            .contains("Git flakes omit untracked declared inputs")
    );
    assert_eq!(std::fs::read(root.join("docs/README.md")).unwrap(), drifted);
    checked(command("git", root).args(["add", input]).output().unwrap());
    assert_eq!(run("sync-readme"), b"1 rows (0 added, 0 removed)\n");
    assert_eq!(std::fs::read(root.join("docs/README.md")).unwrap(),
               b"<!-- adr-table:begin -->\n\n| #   | Title | Status |\n| --- | ----- | ------ |\n| [0001](adr/0001-old.md) | Curated | accepted |\n\n<!-- adr-table:end -->\n");
    assert_eq!(run("sync-readme"), b"1 rows, already in sync\n");
    assert!(repository_policy::adr::format_problems(root).is_empty());
    for gate in ["adr-readme-parity", "adr-view-parity", "doc-links"] {
        repository_policy::repository_docs::Operation::parse(gate)
            .expect("fixed documentation operation")
            .check(root)
            .unwrap_or_else(|error| panic!("{gate}: {error}"));
    }
}
