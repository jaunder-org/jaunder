//! The local-original command must not invoke server ingress or mutate originals.
use std::{fs, process::Command};

use serde_json::{Value, json};

fn extract(files: &[(&str, &str)]) -> (i32, Value) {
    let root = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_org-timezone-manifest"));
    command.current_dir(root.path());
    for (name, source) in files {
        let path = root.path().join(name);
        fs::write(&path, source).unwrap();
        command.arg(path);
    }
    let output = command.output().unwrap();
    for (name, source) in files {
        assert_eq!(fs::read_to_string(root.path().join(name)).unwrap(), *source);
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), files.len());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}: stdout={:?}, stderr={:?}",
            output.stdout, output.stderr
        )
    });
    (output.status.code().unwrap(), report)
}

// A manifest reader can retain exact decimal IDs and reuse common zone validation.
#[derive(serde::Deserialize)]
struct ConsumerMapping {
    post_id: String,
    timezone: common::time::PublicationTimezone,
}

#[test]
fn canonical_ids_and_aliases_remain_usable_by_a_typed_manifest_consumer() {
    let (exit, report) = extract(&[
        (
            "max.org",
            "#+PROPERTY: JAUNDER_ID 9223372036854775807\n#+PROPERTY: JAUNDER_DATE_TZ US/Eastern\n",
        ),
        (
            "zeroes.org",
            "#+PROPERTY: JAUNDER_ID 0002\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n",
        ),
    ]);
    assert_eq!(exit, 0);
    let mappings: Vec<ConsumerMapping> =
        serde_json::from_value(report["mappings"].clone()).unwrap();
    assert_eq!(mappings[0].post_id, "2");
    assert_eq!(mappings[1].post_id, "9223372036854775807");
    assert_eq!(mappings[1].timezone.as_ref(), "US/Eastern");
}

#[test]
fn agreeing_zones_and_invalid_source_claims_cannot_disambiguate_ids() {
    let (exit, report) = extract(&[
        (
            "a.org",
            "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n",
        ),
        (
            "b.org",
            "#+PROPERTY: JAUNDER_ID 007\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n",
        ),
        (
            "c.org",
            "#+PROPERTY: JAUNDER_ID 8\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n",
        ),
        (
            "d.org",
            "#+PROPERTY: JAUNDER_ID 8\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n",
        ),
        (
            "independent.org",
            "#+PROPERTY: JAUNDER_ID 9\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n",
        ),
    ]);
    assert_eq!(exit, 1);
    assert_eq!(
        report["mappings"],
        json!([{"post_id": "9", "timezone": "UTC"}])
    );
    for name in ["a.org", "b.org", "c.org"] {
        assert_eq!(codes(&report, name), ["duplicate_id", "unmapped_source"]);
    }
    assert_eq!(
        codes(&report, "d.org"),
        ["duplicate_id", "duplicate_zone_property", "unmapped_source"]
    );
}

#[test]
fn invocation_requires_explicit_files_and_supports_option_terminator() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        vec![],
        vec!["--"],
        vec!["--inventory", "anything"],
        vec!["--discover"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_org-timezone-manifest"))
            .current_dir(root.path())
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8(output.stderr).unwrap().contains("usage:"));
    }
    fs::write(
        root.path().join("-local.org"),
        "#+PROPERTY: JAUNDER_ID 1\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_org-timezone-manifest"))
        .current_dir(root.path())
        .args(["--", "-local.org"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[cfg(target_os = "linux")]
#[test]
fn output_write_failure_is_exit_two_not_success() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("a.org");
    fs::write(
        &path,
        "#+PROPERTY: JAUNDER_ID 1\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_org-timezone-manifest"))
        .arg(path)
        .stdout(
            fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")
                .unwrap(),
        )
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("org-timezone-manifest:")
    );
}

fn codes(report: &Value, filename: &str) -> Vec<String> {
    report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| {
            std::path::Path::new(d["source"].as_str().unwrap())
                .file_name()
                .unwrap()
                == filename
        })
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn rejects_ambiguous_ids_including_repeated_properties_and_invalid_zone_sources() {
    let (exit, report) = extract(&[
        (
            "a.org",
            "#+PROPERTY: JAUNDER_ID 7\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n",
        ),
        (
            "b.org",
            "#+PROPERTY: JAUNDER_ID 007\n#+PROPERTY: JAUNDER_DATE_TZ +02:00\n",
        ),
        (
            "c.org",
            "#+PROPERTY: JAUNDER_ID 9\n#+PROPERTY: JAUNDER_DATE_TZ US/Eastern\n",
        ),
        (
            "d.org",
            "#+PROPERTY: JAUNDER_ID 9\n#+PROPERTY: JAUNDER_ID 10\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n",
        ),
        (
            "e.org",
            "#+PROPERTY: JAUNDER_ID 10\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n",
        ),
    ]);
    assert_eq!(exit, 1);
    assert_eq!(report["mappings"], json!([]));
    assert_eq!(codes(&report, "a.org"), ["duplicate_id", "unmapped_source"]);
    assert_eq!(
        codes(&report, "b.org"),
        ["duplicate_id", "invalid_zone", "unmapped_source"]
    );
    assert_eq!(
        codes(&report, "d.org"),
        [
            "duplicate_id",
            "duplicate_id",
            "duplicate_id_property",
            "unmapped_source"
        ]
    );
    assert_eq!(codes(&report, "c.org"), ["duplicate_id", "unmapped_source"]);
    assert_eq!(codes(&report, "e.org"), ["duplicate_id", "unmapped_source"]);
    let duplicate_ids: Vec<_> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "duplicate_id")
        .map(|d| d["post_id"].as_str().unwrap())
        .collect();
    assert_eq!(duplicate_ids, ["7", "7", "9", "10", "9", "10"]);
}

#[test]
fn reports_missing_malformed_and_unmapped_sources_without_guessing_from_body() {
    let mut files = vec![
        (
            "missing.org",
            "#+TITLE: 42\nBody\n#+PROPERTY: JAUNDER_ID 42\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n",
        ),
        (
            "empty.org",
            "#+PROPERTY: JAUNDER_ID\n#+PROPERTY: JAUNDER_DATE_TZ\n",
        ),
        (
            "zone.org",
            "#+PROPERTY: JAUNDER_ID 44\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n",
        ),
        (
            "unknown.org",
            "#+PROPERTY: JAUNDER_ID 45\n#+PROPERTY: JAUNDER_DATE_TZ Etc/Unknown\n",
        ),
        (
            "offset.org",
            "#+PROPERTY: JAUNDER_ID 46\n#+PROPERTY: JAUNDER_DATE_TZ -05:00\n",
        ),
        (
            "unsupported.org",
            "#+PROPERTY: JAUNDER_ID 47\n#+PROPERTY: JAUNDER_DATE_TZ Mars/Olympus\n",
        ),
        (
            "block.org",
            "#+BEGIN_SRC org\n#+PROPERTY: JAUNDER_ID 48\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n#+END_SRC\n",
        ),
    ];
    let invalid = [
        "-1",
        "+1",
        "0",
        "1.0",
        "1e2",
        "9223372036854775808",
        "https://example.test/49",
        "４９",
    ];
    let sources: Vec<_> = invalid
        .iter()
        .map(|id| format!("#+PROPERTY: JAUNDER_ID {id}\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n"))
        .collect();
    let names: Vec<_> = (0..invalid.len()).map(|n| format!("id{n}.org")).collect();
    for (name, source) in names.iter().zip(&sources) {
        files.push((name, source));
    }
    let (exit, report) = extract(&files);
    assert_eq!(exit, 1);
    assert_eq!(report["mappings"], json!([]));
    for name in ["missing.org", "block.org"] {
        assert_eq!(
            codes(&report, name),
            ["missing_id", "missing_zone", "unmapped_source"]
        );
    }
    assert_eq!(
        codes(&report, "empty.org"),
        ["invalid_id", "invalid_zone", "unmapped_source"]
    );
    assert_eq!(
        codes(&report, "zone.org"),
        ["duplicate_zone_property", "unmapped_source"]
    );
    for name in ["unknown.org", "offset.org", "unsupported.org"] {
        assert_eq!(codes(&report, name), ["invalid_zone", "unmapped_source"]);
    }
    for name in names {
        assert_eq!(codes(&report, &name), ["invalid_id", "unmapped_source"]);
    }
}

#[test]
fn reports_unreadable_sources_and_emits_stable_output_in_argument_independent_order() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("a.org"),
        "#+PROPERTY: JAUNDER_ID 1\n#+PROPERTY: JAUNDER_DATE_TZ UTC\n",
    )
    .unwrap();
    fs::write(root.path().join("bad.org"), [0xff]).unwrap();
    let run = |paths: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_org-timezone-manifest"))
            .current_dir(root.path())
            .args(paths)
            .output()
            .unwrap()
    };
    let first = run(&["missing.org", "bad.org", "a.org"]);
    let second = run(&["a.org", "bad.org", "missing.org"]);
    assert_eq!(first.status.code(), Some(1));
    assert_eq!(second.status.code(), Some(1));
    assert_eq!(first.stdout, second.stdout);
    let report: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(
        report["mappings"],
        json!([{"post_id": "1", "timezone": "UTC"}])
    );
    assert_eq!(
        codes(&report, "missing.org"),
        ["unmapped_source", "unreadable_source"]
    );
    assert_eq!(
        codes(&report, "bad.org"),
        ["unmapped_source", "unreadable_source"]
    );
    assert_eq!(fs::read(root.path().join("bad.org")).unwrap(), [0xff]);
    assert!(!root.path().join("missing.org").exists());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
    assert_eq!(run(&[]).status.code(), Some(2));
}

#[test]
fn extracts_exact_post_ids_and_alias_without_lifecycle_or_input_writes() {
    let (exit, report) = extract(&[
        (
            "b.org",
            "#+PROPERTY: JAUNDER_ID 9007199254740993\n#+PROPERTY: JAUNDER_DATE_TZ US/Eastern\n",
        ),
        (
            "a.org",
            "#+title: same title\n#+property: jaunder_id 2\n#+property: jaunder_date_tz Europe/London\n\nBody\n",
        ),
    ]);
    assert_eq!(exit, 0);
    assert_eq!(
        report["mappings"],
        json!([
            {"post_id": "2", "timezone": "Europe/London"},
            {"post_id": "9007199254740993", "timezone": "US/Eastern"}
        ])
    );
    assert_eq!(report["diagnostics"], json!([]));
    assert_eq!(
        report["server_coverage"],
        "unknown: no authorized server Post-ID inventory supplied; server IDs absent from originals cannot be determined"
    );
}
