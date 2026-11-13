//! Exercise the Linux-only Nix hard gate against status fixtures, not producers.

#![cfg(target_os = "linux")]

use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(serde::Deserialize)]
struct Gate {
    script: String,
    producer: String,
}

fn gate() -> Gate {
    let output = Command::new("nix")
        .args([
            "eval", "--impure", "--json", ".#checks", "--apply",
            "all: let checks = all.${builtins.currentSystem}; in { script = checks.doctests-gate.buildCommand; producer = toString checks.doctests; }",
        ])
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).unwrap())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn nix_doctest_gate_accepts_only_one_valid_success_status() {
    let gate = gate();
    let successful = doctests::status::DoctestStatus::from_violations(Vec::new()).to_json();
    for (name, status, passes) in [
        ("producer-success", Some(successful.as_str()), true),
        ("optional-defaults", Some(r#"{"category":"ok"}"#), true),
        (
            "explicit-null-detail",
            Some(r#"{"category":"ok","violations":[],"infra_detail":null}"#),
            true,
        ),
        (
            "invalid-violations-type",
            Some(r#"{"category":"ok","violations":"invalid"}"#),
            false,
        ),
        (
            "contradictory-violations",
            Some(
                r#"{"category":"ok","violations":[{"file":"a.rs","line":1,"kind":"failed","detail":"failed"}]}"#,
            ),
            false,
        ),
        (
            "contradictory-infrastructure",
            Some(r#"{"category":"ok","violations":[],"infra_detail":"could not run"}"#),
            false,
        ),
        (
            "null-violations",
            Some(r#"{"category":"ok","violations":null}"#),
            false,
        ),
        (
            "unsupported-field",
            Some(r#"{"category":"ok","violations":[],"version":2}"#),
            false,
        ),
        (
            "unknown-category",
            Some(r#"{"category":"future","violations":[]}"#),
            false,
        ),
        (
            "non-ok-category",
            Some(r#"{"category":"infra","infra_detail":"failed"}"#),
            false,
        ),
        ("missing-category", Some(r#"{"violations":[]}"#), false),
        ("malformed-json", Some("{"), false),
        ("empty-status", Some(""), false),
        (
            "multiple-documents",
            Some("{\"category\":\"ok\"}\n{\"category\":\"ok\"}\n"),
            false,
        ),
        ("non-object", Some("[]"), false),
        ("missing-status", None, false),
    ] {
        let fixture = tempfile::tempdir().unwrap();
        if let Some(status) = status {
            fs::write(fixture.path().join("status.json"), status).unwrap();
        }
        let out = fixture.path().join("gate-passed");
        // Substitute only the producer fixture. Run the actual evaluated gate
        // with Nix's fail-fast shell semantics and the pinned jq on PATH.
        let script = gate
            .script
            .replace(&gate.producer, &fixture.path().to_string_lossy());
        let output = Command::new("bash")
            .args(["--noprofile", "--norc", "-e", "-c", &script])
            .env("out", &out)
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            passes,
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            out.exists(),
            passes,
            "{name}: only a valid status may create the gate output"
        );
    }
}
