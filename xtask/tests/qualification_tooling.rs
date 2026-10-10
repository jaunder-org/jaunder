//! Executable admission observes selected Nix outputs, not Git build revisions.

use std::fs;
use xtask::qualification_tooling::{verify_helper_override, verify_selected_tool};

#[test]
fn helper_override_must_resolve_to_the_selected_store_executable() {
    let selected = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("bash"))
        .find(|path| path.is_file())
        .unwrap();
    let selected = fs::canonicalize(selected).unwrap();
    verify_helper_override(&selected, None).unwrap();
    verify_helper_override(&selected, Some(&selected)).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let copied = temp.path().join("copied-helper");
    fs::copy(&selected, &copied).unwrap();
    let error = verify_helper_override(&selected, Some(&copied)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not match the selected Nix output")
    );
}

#[test]
fn missing_nix_provenance_is_refused_before_executable_access() {
    let error = verify_selected_tool(
        std::path::Path::new("/absent/selected"),
        std::path::Path::new("/absent/executing"),
        None,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("requires Nix-built tooling provenance")
    );
}

#[test]
fn selected_tool_is_reusable_without_a_git_build_revision() {
    let executable = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("bash"))
        .find(|path| path.is_file())
        .unwrap();
    let executable = fs::canonicalize(executable).unwrap();
    assert!(executable.starts_with("/nix/store"));
    let output = executable.parent().unwrap().parent().unwrap();
    // A consumer selects an output rather than demanding a Git revision stamp.
    assert!(verify_selected_tool(&executable, &executable, Some(output)).is_ok());
}

#[test]
fn copied_executable_cannot_claim_its_original_store_output() {
    let executable = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("bash"))
        .find(|path| path.is_file())
        .unwrap();
    let executable = fs::canonicalize(executable).unwrap();
    let output = executable.parent().unwrap().parent().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let copied = temp.path().join("copied-tool");
    fs::copy(&executable, &copied).unwrap();
    let error = verify_selected_tool(&executable, &copied, Some(output)).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not match the selected Nix output")
    );
}

#[test]
fn mixed_output_metadata_is_refused_even_when_executable_paths_match() {
    let executable = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("bash"))
        .find(|path| path.is_file())
        .unwrap();
    let executable = fs::canonicalize(executable).unwrap();
    let error = verify_selected_tool(
        &executable,
        &executable,
        Some(std::path::Path::new("/nix/store/wrong-output")),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("does not match the selected Nix output")
    );
}
