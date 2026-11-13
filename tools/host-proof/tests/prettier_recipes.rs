//! Execute the shared host/hermetic leaves with the real Just parser.
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};

#[test]
fn real_prettier_verify_refuses_without_mutating_then_fix_repairs() {
    let recipe = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../recipes/prettier.just");
    for (name, file, unformatted) in [
        ("prettier-markdown", "guide.md", "#   Guide\n"),
        ("prettier-end2end", "end2end/example.ts", "const x={a:1};\n"),
        ("prettier-css", "nested/style.css", "a{color:red}\n"),
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, unformatted).unwrap();
        let run = |mode: &str| {
            Command::new("just")
                .arg("--justfile")
                .arg(&recipe)
                .arg("--working-directory")
                .arg(root.path())
                .args([name, mode])
                .output()
                .unwrap()
        };
        assert!(!run("verify").status.success());
        assert_eq!(fs::read_to_string(&path).unwrap(), unformatted);
        assert!(run("fix").status.success());
        assert_ne!(fs::read_to_string(&path).unwrap(), unformatted);
        assert!(run("verify").status.success());
    }
}

#[test]
fn shared_prettier_preserves_populations_modes_working_directory_and_failure() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let prettier = bin.join("prettier");
    fs::write(
        &prettier,
        "#!/bin/sh\nprintf '%s\\n' \"$PWD\" \"$@\" > commands\nexit \"$FAIL_CODE\"\n",
    )
    .unwrap();
    fs::set_permissions(&prettier, fs::Permissions::from_mode(0o755)).unwrap();
    // Matching files detect accidental shell glob expansion before Prettier.
    fs::create_dir(root.path().join("nested")).unwrap();
    fs::write(root.path().join("nested/guide.md"), "# Guide\n").unwrap();
    fs::write(root.path().join("nested/style.css"), "a {}\n").unwrap();
    let recipe = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../recipes/prettier.just");
    for (name, population) in [
        ("prettier-markdown", "**/*.md"),
        ("prettier-end2end", "end2end"),
        ("prettier-css", "**/*.css"),
    ] {
        for mode in [None, Some("verify"), Some("fix")] {
            for code in [0, 17] {
                let mut command = Command::new("just");
                command
                    .arg("--justfile")
                    .arg(&recipe)
                    .arg("--working-directory")
                    .arg(root.path())
                    .arg(name);
                if let Some(mode) = mode {
                    command.arg(mode);
                }
                let output = command
                    .env(
                        "PATH",
                        format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
                    )
                    .env("FAIL_CODE", code.to_string())
                    .output()
                    .unwrap();
                assert_eq!(output.status.success(), code == 0, "{name}: {output:?}");
                let log = fs::read_to_string(root.path().join("commands")).unwrap();
                assert_eq!(
                    log.lines().collect::<Vec<_>>(),
                    [
                        root.path().to_str().unwrap(),
                        if mode == Some("fix") { "-w" } else { "--check" },
                        population,
                    ]
                );
                if code != 0 {
                    assert!(String::from_utf8_lossy(&output.stderr).contains("exit code 17"));
                }
            }
        }
    }
}
