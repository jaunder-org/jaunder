//! Prove Cargo's external-command seam preserves checkout selection and arguments.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn cargo_launcher_selects_the_callers_checkout_and_returns_child_status() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("current checkout");
    let nested = root.join("nested");
    fs::create_dir_all(&nested).unwrap();
    let init = xtask::git::at(&root)
        .args(["init", "--quiet"])
        .output()
        .unwrap();
    assert!(init.status.success());
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let nix = bin.join("nix");
    fs::write(&nix, "#!/usr/bin/env bash\nprintf '%s\\0' \"$@\" > \"$JAUNDER_TEST_ARGS\"\nprintf 'child output\\n'\nexit 73\n").unwrap();
    fs::set_permissions(&nix, fs::Permissions::from_mode(0o755)).unwrap();
    let args_file = temp.path().join("args");
    let scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts");
    let mut paths = vec![bin, scripts];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let output = Command::new("cargo")
        .args([
            "xtask",
            "--json",
            "issue",
            "candidates",
            "--milestone",
            "22",
        ])
        .current_dir(&nested)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("JAUNDER_TEST_ARGS", &args_file)
        // A hook's redirecting Git environment must not select another checkout.
        .env("GIT_DIR", "/not-the-callers-checkout")
        .env("GIT_WORK_TREE", "/not-the-callers-checkout")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(73),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"child output\n");
    let bytes = fs::read(args_file).unwrap();
    let args: Vec<_> = bytes
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .map(|arg| String::from_utf8(arg.to_vec()).unwrap())
        .collect();
    assert_eq!(
        args,
        vec![
            "run",
            ".#xtask",
            "--",
            "--json",
            "issue",
            "candidates",
            "--milestone",
            "22"
        ]
    );
}

#[test]
fn real_nix_selects_a_checkout_with_installable_delimiters_in_its_name() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("current #?% checkout");
    let nested = root.join("nested");
    fs::create_dir_all(&nested).unwrap();
    assert!(
        xtask::git::at(&root)
            .args(["init", "--quiet"])
            .status()
            .unwrap()
            .success()
    );
    // No inputs, builders or product dependencies: exercise the real installable
    // parser and execution boundary, not a mock that accepts malformed references.
    let bash = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("bash"))
        .find(|path| path.is_file())
        .unwrap();
    let bash = fs::canonicalize(bash).unwrap();
    assert!(
        bash.starts_with("/nix/store"),
        "test requires the pinned devShell Bash"
    );
    let flake = r#"{
      outputs = { self }:
        let systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
        in { apps = builtins.listToAttrs (map (name: {
          inherit name;
          value.xtask = { type = "app"; program = @BASH@; };
        }) systems); };
    }"#
    .replace("@BASH@", &serde_json::to_string(&bash).unwrap());
    fs::write(root.join("flake.nix"), flake).unwrap();
    assert!(
        xtask::git::at(&root)
            .args(["add", "flake.nix"])
            .status()
            .unwrap()
            .success()
    );
    let scripts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../scripts");
    let mut paths = vec![scripts];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let output = Command::new("cargo")
        .args(["xtask", "-c", "printf '%s\\n' \"$PWD\"; exit 73"])
        .current_dir(nested)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(73),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{}\n", fs::canonicalize(root).unwrap().display())
    );
}
