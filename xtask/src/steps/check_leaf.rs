//! Callable boundary for Rust-only static checks invoked individually by Just.
//!
//! Each invocation dispatches one existing check. This module has no gate
//! population, ordering, mode selection, or workflow reporting. Consumer adapters
//! invoke existing owners after Just has requested their producer outputs.

use std::path::Path;

use anyhow::{Result, bail, ensure};

use crate::{result::CommandResult, steps};

fn flow_docs(result: &mut CommandResult) {
    result.push(steps::flow_docs::run());
}

/// Run exactly one Rust-owned check, preserving its existing algorithm and leaf
/// diagnostic. An unknown name is a caller error rather than a skipped success.
pub(crate) fn run(name: &str) -> Result<crate::result::StepResult> {
    run_at(name, Path::new("."))
}

fn run_at(name: &str, root: &Path) -> Result<crate::result::StepResult> {
    let mut result = CommandResult::new("check-leaf");
    match name {
        "clean-tree" => result.push(crate::lifecycle::clean_tree_precheck(false)),
        "prepush-docs" => {
            crate::lifecycle::run_shared_static_docs_request(
                root,
                true,
                steps::nix::shared_static_docs_check,
                &mut result,
            );
            return consumer_result(name, result);
        }
        "workspace-doctests" => {
            steps::doctest_fences::run_workspace(&xshell::Shell::new()?, &mut result);
        }
        "wasm-budget" => steps::wasm_budget::run(&mut result),
        "server-fn-flow" => {
            steps::server_fn_coverage_check::verify_after_validate(&mut result, true);
        }
        "elisp-coverage-consume" => {
            steps::nix::consume_elisp_coverage(&mut result);
            return consumer_result(name, result);
        }
        "sequence-check" => steps::sequence_check::run(&mut result),
        "adr-format" => result.push(steps::adr_check::format_step_at(root)),
        "adr-readme-parity" => result.push(steps::adr_check::readme_parity_step_at(root)),
        "adr-view-parity" => result.push(steps::adr_check::view_parity_step_at(root)),
        "doc-links" => steps::doc_links::run(&mut result),
        "flow-docs" => flow_docs(&mut result),
        "test-patterns" => steps::test_pattern_check::run(&mut result),
        "server-fn-registrar" => steps::server_fn_registrar_check::run(&mut result),
        "server-fn-tracing" => steps::server_fn_tracing_check::run(&mut result),
        "server-fn-coverage" => steps::server_fn_coverage_check::run(&mut result),
        "server-fn-wire-arg-error" => steps::server_fn_wire_arg_error_check::run(&mut result),
        "traced-context" => steps::traced_context_check::run(&mut result),
        "e2e-telemetry-boundary" => steps::e2e_telemetry_boundary_check::run(&mut result),
        "e2e-collector-config" => steps::e2e_collector_config_check::run(&mut result),
        "proffered-secret" => steps::proffered_secret_check::run(&mut result),
        "e2e-goto-wrapper" => steps::e2e_goto_wrapper_check::run(&mut result),
        "e2e-server-fn-endpoints" => steps::e2e_server_fn_endpoint_check::run(&mut result),
        "target-arch-placement" => steps::target_arch_placement_check::run(&mut result),
        "write-transaction-contract" => steps::write_transaction_contract_check::run(&mut result),
        "common-host-target-closure" => steps::common_host_target_closure::run(&mut result),
        "lint-suppression" => steps::lint_suppression_check::run(&mut result),
        "thin-components" => steps::thin_components::run(&mut result),
        "sqlx-newtype-bind" => steps::sqlx_newtype_bind_check::run(&mut result),
        "sqlx-newtype-decode" => steps::sqlx_newtype_decode_check::run(&mut result),
        "doctest-fences" => steps::doctest_fences::run(&mut result),
        "raw-html-door" => steps::raw_html_door_check::run(&mut result),
        "html-sink" => steps::html_sink_check::run(&mut result),
        "e2e-scaffold" => steps::e2e_scaffold_check::run(&mut result),
        "xlang-literal" => steps::xlang_literal_check::run(&mut result),
        _ => bail!("unknown Rust check leaf `{name}`"),
    }
    ensure!(
        result.steps.len() == 1,
        "Rust check leaf `{name}` did not produce exactly one result"
    );
    Ok(result.steps.remove(0))
}

// These two existing owners produce multiple diagnostics for one safety/consumer
// operation. Flatten only their text/status, never serialize a workflow envelope.
fn consumer_result(name: &str, result: CommandResult) -> Result<crate::result::StepResult> {
    ensure!(
        !result.steps.is_empty(),
        "leaf `{name}` produced no verdict"
    );
    let ok = result.steps.iter().all(|step| step.ok || step.skipped);
    let detail = result
        .steps
        .into_iter()
        .map(|step| format!("{}: {}", step.name, step.detail.unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(if ok {
        crate::result::StepResult::ok(name).detail(detail)
    } else {
        crate::result::StepResult::fail(name).detail(detail)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_repo() -> tempfile::TempDir {
        let root = tempfile::tempdir().expect("temporary repository");
        std::fs::create_dir_all(root.path().join("docs/adr")).unwrap();
        root
    }

    #[test]
    fn rust_only_check_leaf_runs_one_existing_check() {
        let step = run("flow-docs").expect("known leaf");
        assert_eq!(step.name, "flow-docs");
    }

    #[test]
    fn unknown_rust_check_leaf_is_a_caller_error() {
        assert!(run("not-a-check").is_err());
    }

    #[test]
    fn adr_leaves_keep_their_independent_populations_and_verdicts() {
        let root = scratch_repo();
        let adr_dir = root.path().join("docs/adr");
        std::fs::write(
            root.path().join("docs/README.md"),
            "<!-- adr-table:begin -->\n\n| # | Title | Status |\n| --- | --- | --- |\n\n<!-- adr-table:end -->\n",
        )
        .unwrap();
        std::fs::write(root.path().join("docs/ARCHITECTURE.md"), "# Architecture\n").unwrap();

        let format_ok = run_at("adr-format", root.path()).unwrap();
        let readme_ok = run_at("adr-readme-parity", root.path()).unwrap();
        let view_ok = run_at("adr-view-parity", root.path()).unwrap();
        assert_eq!(format_ok.name, "adr-format");
        assert!(format_ok.ok);
        assert_eq!(readme_ok.name, "adr-readme-parity");
        assert!(readme_ok.ok);
        assert_eq!(view_ok.name, "adr-view-parity");
        assert!(view_ok.ok);

        std::fs::write(
            adr_dir.join("0001-broken.md"),
            "# Not a canonical ADR heading\n\n- Status: accepted\n",
        )
        .unwrap();
        let format_failed = run_at("adr-format", root.path()).unwrap();
        assert_eq!(format_failed.name, "adr-format");
        assert!(!format_failed.ok);

        std::fs::write(
            root.path().join("docs/README.md"),
            "<!-- adr-table:begin -->\n\n| [0001](adr/wrong.md) | Broken | accepted |\n\n<!-- adr-table:end -->\n",
        )
        .unwrap();
        let readme_failed = run_at("adr-readme-parity", root.path()).unwrap();
        assert_eq!(readme_failed.name, "adr-readme-parity");
        assert!(!readme_failed.ok);

        std::fs::write(
            adr_dir.join("0001-broken.md"),
            "# ADR-0001: Broken\n\n- Status: accepted\n",
        )
        .unwrap();
        let view_failed = run_at("adr-view-parity", root.path()).unwrap();
        assert_eq!(view_failed.name, "adr-view-parity");
        assert!(!view_failed.ok);
    }
}
