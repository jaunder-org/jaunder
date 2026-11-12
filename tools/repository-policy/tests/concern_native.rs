//! Actual native policy boundaries: independent verdicts and non-green read failures.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn write(root: &Path, path: &str, body: impl AsRef<[u8]>) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}
fn git(root: &Path, args: &[&str]) {
    let output = repository_policy::git::at(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn checkout() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    git(temp.path(), &["init", "-q"]);
    write(temp.path(), "flake.nix", "{}\n");
    write(temp.path(), "tools/Cargo.toml", "[workspace]\n");
    temp
}
fn run(root: &Path, source: bool, operation: &str) -> Output {
    Command::new(if source {
        env!("CARGO_BIN_EXE_source-contracts")
    } else {
        env!("CARGO_BIN_EXE_repository-docs")
    })
    .arg(operation)
    .current_dir(root)
    .output()
    .unwrap()
}
fn verdict(output: Output, code: i32, diagnostic: &str) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = if code == 0 {
        &output.stdout
    } else {
        &output.stderr
    };
    assert!(
        String::from_utf8_lossy(text).contains(diagnostic),
        "{}",
        String::from_utf8_lossy(text)
    );
}
#[test]
fn view_checks_only_accepted_adrs_and_names_unreadable_input() {
    let temp = checkout();
    let root = temp.path();
    write(
        root,
        "docs/adr/0001-choice.md",
        "# ADR-0001: Choice\n- Status: accepted\n",
    );
    write(root, "docs/ARCHITECTURE.md", "# Architecture\nADR-0001\n");
    verdict(run(root, false, "adr-view-parity"), 0, "adr-view-parity");
    write(root, "docs/ARCHITECTURE.md", "# Architecture\n");
    verdict(run(root, false, "adr-view-parity"), 1, "not cited");
    write(root, "docs/ARCHITECTURE.md", [0xff]);
    verdict(run(root, false, "adr-view-parity"), 1, "reading");
}
#[test]
fn links_use_tracked_markdown_and_do_not_hide_read_errors() {
    let temp = checkout();
    let root = temp.path();
    write(root, "guide.md", "[good](target.md)\n");
    write(root, "target.md", "# Target\n");
    write(root, "untracked.md", "[bad](absent.md)\n");
    git(root, &["add", "guide.md", "target.md"]);
    verdict(run(root, false, "doc-links"), 0, "doc-links");
    write(root, "guide.md", "[bad](absent.md)\n");
    verdict(run(root, false, "doc-links"), 1, "absent.md");
    write(root, "guide.md", [0xff]);
    verdict(run(root, false, "doc-links"), 1, "reading guide.md");
}
#[test]
fn sequence_preserves_independent_migration_parity_and_enumeration_failure() {
    let temp = checkout();
    let root = temp.path();
    fs::create_dir_all(root.join("docs/adr/drafts")).unwrap();
    write(root, "docs/adr/drafts/choice.md", "# ADR-DRAFT: Choice\n");
    write(root, "storage/migrations/sqlite/0001_x.sql", "");
    write(root, "storage/migrations/postgres/0001_x.sql", "");
    verdict(
        run(root, false, "sequence-check"),
        0,
        "identifier-collisions",
    );
    write(root, "storage/migrations/sqlite/0002_y.sql", "");
    verdict(
        run(root, false, "sequence-check"),
        1,
        "migration backend parity",
    );
    fs::remove_dir_all(root.join("storage/migrations/postgres")).unwrap();
    verdict(
        run(root, false, "sequence-check"),
        1,
        "cannot enumerate identifier population",
    );
}
#[test]
fn source_scan_applies_backend_policy_and_rejects_partial_input() {
    let temp = checkout();
    let root = temp.path();
    write(
        root,
        "server/tests/test.rs",
        "#[apply(backends)]\n#[tokio::test]\nasync fn works(#[case] backend: Backend) { backend.setup().await; }\n",
    );
    fs::create_dir_all(root.join("storage/src")).unwrap();
    verdict(run(root, true, "test-patterns"), 0, "test-backend-pattern");
    write(
        root,
        "server/tests/test.rs",
        "#[tokio::test]\nasync fn unclassified() {}\n",
    );
    verdict(run(root, true, "test-patterns"), 1, "backend");
    write(root, "server/tests/test.rs", [0xff]);
    verdict(run(root, true, "test-patterns"), 1, "cannot read");
}
#[test]
fn placement_uses_all_three_roots_and_fails_on_unreadable_root() {
    let temp = checkout();
    let root = temp.path();
    for path in ["web/src", "client/src", "csr/src"] {
        fs::create_dir_all(root.join(path)).unwrap();
    }
    write(
        root,
        "web/src/mod.rs",
        "#[cfg(target_arch = \"wasm32\")]\nmod component;\n",
    );
    verdict(
        run(root, true, "target-arch-placement"),
        0,
        "target-arch-placement",
    );
    write(
        root,
        "web/src/leaf.rs",
        "#[cfg(target_arch = \"wasm32\")]\nfn wrong() {}\n",
    );
    verdict(run(root, true, "target-arch-placement"), 1, "leaf.rs");
    fs::remove_dir(root.join("csr/src")).unwrap();
    verdict(
        run(root, true, "target-arch-placement"),
        1,
        "cannot scan csr/src",
    );
}
#[test]
fn oxc_endpoints_are_checked_against_real_rust_inventory() {
    let temp = checkout();
    let root = temp.path();
    write(
        root,
        "web/src/posts/api.rs",
        "#[macros::server]\npub async fn create() {}\n",
    );
    write(
        root,
        "end2end/tests/posts.spec.ts",
        "await page.request.post('/api/posts/create');\n",
    );
    verdict(
        run(root, true, "e2e-server-fn-endpoints"),
        0,
        "e2e-server-fn-endpoints",
    );
    write(
        root,
        "end2end/tests/posts.spec.ts",
        "await page.request.post('/api/posts/absent');\n",
    );
    verdict(
        run(root, true, "e2e-server-fn-endpoints"),
        1,
        "/api/posts/absent",
    );
    write(root, "end2end/tests/posts.spec.ts", [0xff]);
    verdict(run(root, true, "e2e-server-fn-endpoints"), 1, "reading");
}
#[test]
fn static_inventory_does_not_require_runtime_capture_and_rejects_missing_snapshot() {
    let temp = checkout();
    let root = temp.path();
    write(
        root,
        "web/src/posts/api.rs",
        "#[macros::server]\npub async fn create() {}\n",
    );
    write(
        root,
        "docs/coverage/server-fns.json",
        r#"{"covered":["posts::create"],"orphans":{}}"#,
    );
    verdict(run(root, true, "server-fn-coverage"), 0, "1 server fn");
    write(
        root,
        "docs/coverage/server-fns.json",
        r#"{"covered":[],"orphans":{}}"#,
    );
    verdict(run(root, true, "server-fn-coverage"), 1, "posts::create");
    fs::remove_file(root.join("docs/coverage/server-fns.json")).unwrap();
    verdict(run(root, true, "server-fn-coverage"), 1, "reading");
}
#[test]
fn telemetry_ownership_keeps_tracked_census_and_read_failure_distinct() {
    let temp = checkout();
    let root = temp.path();
    write(
        root,
        "end2end/tests/capture-trace.ts",
        "page.on(\x27console\x27, listener); page.on(\x27pageerror\x27, listener);\n",
    );
    let json = ["e2e.console_", "json"].concat();
    let dropped = ["e2e.console_", "dropped"].concat();
    write(
        root,
        "end2end/tests/performance.ts",
        format!("otlpAttribute('{json}', value);\notlpAttribute('{dropped}', value);\n"),
    );
    // The detector and maintained fixtures remain in the tracked census, without
    // a self-exclusion that could silently hide a newly introduced listener.
    write(
        root,
        "tools/repository-policy/src/source_contracts/e2e_telemetry_boundary_check.rs",
        include_str!("../src/source_contracts/e2e_telemetry_boundary_check.rs"),
    );
    write(
        root,
        "tools/repository-policy/tests/concern_native.rs",
        include_str!("concern_native.rs"),
    );
    git(root, &["add", "end2end", "tools/repository-policy"]);
    verdict(
        run(root, true, "e2e-telemetry-boundary"),
        0,
        "e2e-telemetry-boundary",
    );
    write(
        root,
        "end2end/tests/rogue.ts",
        "page.on(\x27console\x27, listener);\n",
    );
    git(root, &["add", "end2end/tests/rogue.ts"]);
    verdict(run(root, true, "e2e-telemetry-boundary"), 1, "rogue.ts");
    write(root, "end2end/tests/rogue.ts", [0xff]);
    verdict(run(root, true, "e2e-telemetry-boundary"), 1, "cannot read");
}
#[test]
fn flow_tokens_use_static_inventory_snapshot_and_matrix_headings() {
    let temp = checkout();
    let root = temp.path();
    write(
        root,
        "web/src/app/route_policy.rs",
        r#"macro_rules! app_routes { ($consumer:ident) => { $consumer! { (Login, Public, "/login", leptos_router::StaticSegment("login"), $crate::auth::LoginPage) } }; }"#,
    );
    write(
        root,
        "docs/coverage/server-fns.json",
        r#"{"covered":[],"orphans":{}}"#,
    );
    write(
        root,
        "docs/coverage/matrix.md",
        "# Matrix\n## Authentication\n",
    );
    write(root, "docs/flows/README.md", "# Index\n`route:<shell>`\n");
    write(
        root,
        "docs/flows/login.md",
        "# Login\n`route:/login`\n`matrix:docs/coverage/matrix.md#authentication`\n",
    );
    verdict(run(root, false, "flow-docs"), 0, "checked 2 flow docs");
    write(
        root,
        "docs/flows/login.md",
        "# Login\n`route:/ghost`\n`matrix:docs/coverage/matrix.md#authentication`\n",
    );
    verdict(run(root, false, "flow-docs"), 1, "ghost");
    fs::remove_file(root.join("docs/coverage/server-fns.json")).unwrap();
    verdict(run(root, false, "flow-docs"), 1, "reading");
}
#[test]
fn unknown_operations_are_refused_before_checkout_bootstrap() {
    let temp = tempfile::tempdir().unwrap();
    for source in [false, true] {
        verdict(run(temp.path(), source, "--all"), 2, "usage:");
        assert!(!temp.path().join(".git").exists());
    }
}

// These policies share checkout admission, not their input populations or verdicts.
fn documentation_checkout() -> tempfile::TempDir {
    let temp = checkout();
    let root = temp.path();
    fs::create_dir_all(root.join("docs/adr")).unwrap();
    write(
        root,
        "docs/README.md",
        "<!-- adr-table:begin -->\n\n| # | Title | Status |\n| --- | --- | --- |\n\n<!-- adr-table:end -->\n",
    );
    write(root, "docs/ARCHITECTURE.md", "# Architecture\n");
    temp
}

#[test]
fn readme_success_does_not_require_architecture_input() {
    let temp = documentation_checkout();
    fs::remove_file(temp.path().join("docs/ARCHITECTURE.md")).unwrap();
    let output = run(temp.path(), false, "adr-readme-parity");
    verdict(output, 0, "adr-readme-parity");
}
#[test]
fn readme_policy_failure_is_independent_of_format_and_view() {
    let temp = documentation_checkout();
    fs::write(
        temp.path().join("docs/adr/0001-choice.md"),
        "# ADR-0001: Choice\n- Status: accepted\n",
    )
    .unwrap();
    fs::write(
        temp.path().join("docs/ARCHITECTURE.md"),
        "# Architecture\nADR-0001\n",
    )
    .unwrap();
    assert!(run(temp.path(), false, "adr-format").status.success());
    let output = run(temp.path(), false, "adr-readme-parity");
    verdict(output, 1, "ADR 0001 has no README table row");
}
#[test]
fn unreadable_readme_is_not_an_empty_passing_population() {
    let temp = documentation_checkout();
    fs::remove_file(temp.path().join("docs/README.md")).unwrap();
    let output = run(temp.path(), false, "adr-readme-parity");
    assert!(String::from_utf8_lossy(&output.stderr).contains("docs/README.md"));
    verdict(output, 1, "reading");
}

// Census retains its controlled analyzer population and domain-specific assertions.
fn census_fixture() -> tempfile::TempDir {
    let temp = checkout();
    let root = temp.path();
    git(root, &["config", "user.name", "census fixture"]);
    git(root, &["config", "user.email", "fixture@example.invalid"]);
    fs::create_dir_all(root.join("common/src")).unwrap();
    fs::write(root.join("common/src/a.rs"), "use serde::Serialize;\n").unwrap();
    fs::write(root.join("common/src/b.rs"), "use serde::Deserialize;\n").unwrap();
    for backend in ["sqlite", "postgres"] {
        let directory = root.join(format!("storage/src/{backend}"));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("pair.rs"), "fn adapter() {}\n").unwrap();
    }
    git(root, &["add", "common", "storage"]);
    git(
        root,
        &["-c", "core.hooksPath=/dev/null", "commit", "-qm", "fixture"],
    );
    // A controlled capability population uses real Git but no optional analyzers.
    // This makes unavailable independent of the developer machine's tool inventory.
    let git_path: PathBuf = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|directory| directory.join("git"))
        .find(|path| path.is_file())
        .unwrap();
    fs::create_dir(root.join("bin")).unwrap();
    std::os::unix::fs::symlink(git_path.canonicalize().unwrap(), root.join("bin/git")).unwrap();
    temp
}
fn census(root: &Path, json: bool) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_source-contracts"));
    command
        .arg("census")
        .current_dir(root)
        .env("PATH", root.join("bin"));
    if json {
        command.arg("--json");
    }
    command.output().unwrap()
}
#[test]
fn candidates_and_unavailable_cells_are_not_a_gate_and_both_outputs_agree() {
    let temp = census_fixture();
    let output = census(temp.path(), true);
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let sections = report["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 7);
    assert_eq!(
        sections
            .iter()
            .map(|section| section["cells"].as_array().unwrap().len())
            .sum::<usize>(),
        17
    );
    let json = String::from_utf8(output.stdout).unwrap();
    assert!(json.contains("candidates"));
    assert!(json.contains("unavailable"));
    assert!(json.contains("rust-dependency:serde"));
    let human = census(temp.path(), false);
    assert_eq!(human.status.code(), Some(0));
    let text = String::from_utf8(human.stdout).unwrap();
    assert!(text.contains("rust-dependency:serde"), "{text}");
    assert!(text.contains("unavailable"), "{text}");
    let repeated = census(temp.path(), true);
    assert_eq!(json.as_bytes(), repeated.stdout);
}
#[test]
fn malformed_source_fails_collection_but_retains_completed_cells() {
    let temp = census_fixture();
    fs::write(temp.path().join("common/src/a.rs"), "fn broken( {").unwrap();
    let output = census(temp.path(), true);
    assert_eq!(output.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let json = report.to_string();
    assert!(json.contains("failed"));
    assert!(json.contains("unavailable"));
    assert_eq!(report["sections"].as_array().unwrap().len(), 7);
    let completed = report["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|section| section["cells"].as_array().unwrap())
        .find(|cell| cell["collector"]["identity"] == "census-storage-adapter-paths")
        .expect("independent adapter collector retained");
    assert_eq!(completed["state"], "candidates");
    assert_eq!(completed["total_candidates"], 1);
    assert_eq!(
        completed["candidates"][0]["identity"],
        "adapter-pair:pair.rs"
    );
    assert_eq!(
        completed["candidates"][0]["paths"],
        serde_json::json!(["storage/src/postgres/pair.rs", "storage/src/sqlite/pair.rs"])
    );
}
#[test]
fn unreadable_tracked_input_is_not_reported_as_an_empty_clean_census() {
    let temp = census_fixture();
    fs::write(temp.path().join("common/src/a.rs"), [0xff]).unwrap();
    let output = census(temp.path(), true);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("a.rs"));
}
