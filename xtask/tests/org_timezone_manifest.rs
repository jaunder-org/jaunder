//! The local-original command must not invoke server ingress or mutate originals.
use std::{fs, process::Command};

use serde_json::{Value, json};

fn extract(files: &[(&str, &str)]) -> (i32, Value) {
    let root = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_xtask"));
    command
        .current_dir(root.path())
        .arg("org-timezone-manifest");
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
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .current_dir(root.path())
            .arg("org-timezone-manifest")
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
