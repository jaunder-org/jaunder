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
    let store = temp.path().join("fake xtask output");
    let executable_dir = store.join("bin");
    fs::create_dir_all(&executable_dir).unwrap();
    let executable = executable_dir.join("xtask");
    fs::write(
        &executable,
        "#!/usr/bin/env bash\nprintf '%s\\0' \"$@\" > \"$JAUNDER_TEST_XTASK_ARGS\"\nprintf '%s\\n' \"$PWD\" > \"$JAUNDER_TEST_XTASK_CWD\"\nprintf 'child output\\n'\nexit 73\n",
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    let nix = bin.join("nix");
    fs::write(&nix, "#!/usr/bin/env bash\nprintf '%s\\0' \"$@\" > \"$JAUNDER_TEST_ARGS\"\nprintf '%s\\n' \"$JAUNDER_TEST_OUTPUT\"\n").unwrap();
    fs::set_permissions(&nix, fs::Permissions::from_mode(0o755)).unwrap();
    let args_file = temp.path().join("args");
    let xtask_args_file = temp.path().join("xtask-args");
    let xtask_cwd_file = temp.path().join("xtask-cwd");
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
        .env("JAUNDER_TEST_OUTPUT", &store)
        .env("JAUNDER_TEST_XTASK_ARGS", &xtask_args_file)
        .env("JAUNDER_TEST_XTASK_CWD", &xtask_cwd_file)
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
        vec!["build", ".#xtask", "--no-link", "--print-out-paths"]
    );
    let bytes = fs::read(xtask_args_file).unwrap();
    let args: Vec<_> = bytes
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .map(|arg| String::from_utf8(arg.to_vec()).unwrap())
        .collect();
    assert_eq!(
        args,
        vec!["--json", "issue", "candidates", "--milestone", "22"]
    );
    assert_eq!(
        fs::read_to_string(xtask_cwd_file).unwrap(),
        format!("{}\n", fs::canonicalize(root).unwrap().display())
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
    // Use only pinned shell tools as fixture inputs: exercise real Nix installable
    // parsing and package execution without product dependencies.
    let bash = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("bash"))
        .find(|path| path.is_file())
        .unwrap();
    let bash = fs::canonicalize(bash).unwrap();
    assert!(
        bash.starts_with("/nix/store"),
        "test requires the pinned devShell Bash"
    );
    let coreutils = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("mkdir"))
        .find(|path| path.is_file())
        .unwrap();
    let coreutils = fs::canonicalize(coreutils)
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    assert!(coreutils.starts_with("/nix/store"));
    let flake = r#"{
      inputs = {
        bashSrc = { url = "path:@BASH_ROOT@"; flake = false; };
        coreutilsSrc = { url = "path:@COREUTILS_ROOT@"; flake = false; };
      };
      outputs = { self, bashSrc, coreutilsSrc }:
        let systems = [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ];
            bash = "${bashSrc.outPath}/bin/bash";
            coreutils = "${coreutilsSrc.outPath}/bin";
        in { packages = builtins.listToAttrs (map (name: {
          inherit name;
          value.xtask = builtins.derivation {
            name = "fixture-xtask";
            system = name;
            builder = bash;
            args = [ "-c" "mkdir -p \"$out/bin\"; ln -s ${bash} \"$out/bin/xtask\"" ];
            PATH = coreutils;
          };
        }) systems); };
    }"#
    .replace(
        "@BASH_ROOT@",
        bash.parent().unwrap().parent().unwrap().to_str().unwrap(),
    )
    .replace("@COREUTILS_ROOT@", coreutils.to_str().unwrap());
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
        // This fixture's tiny store-backed shell builder only tests real Nix
        // installable selection; sandboxing is not part of its contract.
        .env("NIX_CONFIG", "sandbox = false")
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
