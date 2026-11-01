#![cfg(unix)]

#[test]
fn launcher_preserves_controlling_terminal_input_and_restores_foreground() {
    let fixture = tempfile::tempdir().unwrap();
    let output = std::process::Command::new("python3")
        .args(["-c", include_str!("fixtures/controlling_terminal.py")])
        .arg(env!("CARGO_BIN_EXE_traced-run"))
        .current_dir(fixture.path())
        .env_remove("JAUNDER_TRACED_RUN_CONTEXT")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
