use std::process::Command;

#[test]
fn command_capture_is_not_a_devtool_route() {
    for args in [vec!["run", "--", "true"], vec!["help", "run"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_devtool"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand 'run'"));
    }
    let output = Command::new(env!("CARGO_BIN_EXE_devtool"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(!help.lines().any(|line| line.starts_with("  run ")));
}

#[test]
fn locked_workspace_seed_caller_reaches_devtool_help() {
    let tools_manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("Cargo.toml");
    let output = Command::new("cargo")
        .args(["run", "--locked", "--manifest-path"])
        .arg(tools_manifest)
        .args([
            "-p", "devtool", "--bin", "devtool", "--", "seed-e2e", "--help",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("Usage: devtool seed-e2e"));
    for argument in ["--db", "--test-support-bin", "--jaunder-bin"] {
        assert!(help.contains(argument), "{help}");
    }
}

#[test]
fn sandbox_tooling_routes_remain_available() {
    for route in [
        "coverage",
        "wasm-coverage",
        "diagnostic-build",
        "doctests",
        "pg",
        "check",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_devtool"))
            .args([route, "--help"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{route}: {output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("Usage:"));
    }
}
