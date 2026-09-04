use xshell::Shell;

use crate::{
    cli::Command,
    git,
    result::{CommandResult, Mode, StepResult},
    steps,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionPolicy {
    Exhaustive,
    FailFast,
}

pub(crate) fn execution_policy(command: &Command) -> ExecutionPolicy {
    match command {
        Command::Precommit | Command::Prepush => ExecutionPolicy::FailFast,
        Command::Check { .. } | Command::Validate { .. } => ExecutionPolicy::Exhaustive,
        _ => ExecutionPolicy::Exhaustive,
    }
}

pub(crate) fn run_with_policy<T>(
    policy: ExecutionPolicy,
    result: &mut CommandResult,
    runners: impl IntoIterator<Item = T>,
    mut run: impl FnMut(T, &mut CommandResult),
) {
    for runner in runners {
        let before = result.steps.len();
        run(runner, result);
        if matches!(policy, ExecutionPolicy::FailFast)
            && result.steps[before..]
                .iter()
                .any(StepResult::is_blocking_failure)
        {
            break;
        }
    }
}

pub(crate) enum HostGateStep {
    StaticChecks(steps::static_checks::Phase),
    ResultOnly {
        name: &'static str,
        run: fn(&mut CommandResult),
        markdown_eligible: bool,
    },
    HostTests {
        markdown_eligible: bool,
    },
}

impl HostGateStep {
    pub(crate) fn run(
        &self,
        sh: &Shell,
        mode: Mode,
        policy: ExecutionPolicy,
        result: &mut CommandResult,
    ) {
        match self {
            Self::StaticChecks(phase) => {
                run_static_phase_with(
                    sh,
                    mode,
                    *phase,
                    policy,
                    result,
                    steps::static_checks::run_spec,
                );
            }
            Self::ResultOnly { name, run, .. } => {
                debug_assert!(!name.is_empty());
                let before = result.steps.len();
                let start = std::time::Instant::now();
                run(result);
                let duration = start.elapsed().as_millis();
                for step in &mut result.steps[before..] {
                    if step.duration_ms == 0 {
                        step.duration_ms = duration;
                    }
                }
            }
            Self::HostTests { .. } => steps::host_tests::run(sh, result),
        }
    }

    fn run_markdown(
        &self,
        sh: &xshell::Shell,
        mode: Mode,
        policy: ExecutionPolicy,
        result: &mut CommandResult,
    ) {
        match self {
            Self::StaticChecks(phase) => run_markdown_static_phase_with(
                sh,
                mode,
                *phase,
                policy,
                result,
                steps::static_checks::run_spec,
            ),
            Self::ResultOnly { run, .. } => {
                let before = result.steps.len();
                let start = std::time::Instant::now();
                run(result);
                let duration = start.elapsed().as_millis();
                for step in &mut result.steps[before..] {
                    if step.duration_ms == 0 {
                        step.duration_ms = duration;
                    }
                }
            }
            Self::HostTests { .. } => unreachable!("host tests are not Markdown-route eligible"),
        }
    }

    fn markdown_eligible(&self, _mode: Mode) -> bool {
        match self {
            // The phase is a catalog container; its specs retain their own
            // Markdown-eligibility metadata, avoiding a parallel list.
            Self::StaticChecks(_) => true,
            Self::ResultOnly {
                markdown_eligible, ..
            } => *markdown_eligible,
            Self::HostTests { markdown_eligible } => *markdown_eligible,
        }
    }

    #[cfg(test)]
    fn push_names(&self, mode: Mode, names: &mut Vec<&'static str>) {
        match self {
            Self::StaticChecks(phase) => names.extend(
                steps::static_checks::specs_for_phase(*phase, mode)
                    .into_iter()
                    .map(|spec| spec.name),
            ),
            Self::ResultOnly { name, .. } => names.push(name),
            Self::HostTests { .. } => names.push("host-tests"),
        }
    }
}

pub(crate) fn run_static_phase_with(
    sh: &Shell,
    mode: Mode,
    phase: steps::static_checks::Phase,
    policy: ExecutionPolicy,
    result: &mut CommandResult,
    mut run: impl FnMut(&Shell, &steps::static_checks::StepSpec) -> StepResult,
) {
    run_with_policy(
        policy,
        result,
        steps::static_checks::specs_for_phase(phase, mode),
        |spec, result| {
            result.push(run(sh, &spec));
        },
    );
}

fn run_markdown_static_phase_with(
    sh: &Shell,
    mode: Mode,
    phase: steps::static_checks::Phase,
    policy: ExecutionPolicy,
    result: &mut CommandResult,
    mut run: impl FnMut(&Shell, &steps::static_checks::StepSpec) -> StepResult,
) {
    run_with_policy(
        policy,
        result,
        steps::static_checks::specs_for_phase(phase, mode)
            .into_iter()
            .filter(|spec| spec.markdown_eligible),
        |spec, result| result.push(run(sh, &spec)),
    );
}

fn run_flow_docs(result: &mut CommandResult) {
    result.push(steps::flow_docs::run());
}

pub(crate) const HOST_GATE_NON_TEST_STEPS: &[HostGateStep] = &[
    // Source consistency first: these are fixable or concrete file-shape errors,
    // and they must run before expensive compile/type work.
    HostGateStep::StaticChecks(steps::static_checks::Phase::SourceConsistency),
    HostGateStep::ResultOnly {
        name: "sequence-check",
        run: steps::sequence_check::run,
        markdown_eligible: true,
    },
    HostGateStep::ResultOnly {
        name: "adr-filenames",
        run: steps::adr_check::run,
        markdown_eligible: true,
    },
    HostGateStep::ResultOnly {
        name: "doc-links",
        run: steps::doc_links::run,
        markdown_eligible: true,
    },
    HostGateStep::ResultOnly {
        name: "flow-docs",
        run: run_flow_docs,
        markdown_eligible: true,
    },
    HostGateStep::ResultOnly {
        name: "error-swallowing-inventory",
        run: steps::error_swallowing_inventory_check::run,
        markdown_eligible: true,
    },
    HostGateStep::ResultOnly {
        name: "test-patterns",
        run: steps::test_pattern_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "server-fn-registrar",
        run: steps::server_fn_registrar_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "server-fn-tracing",
        run: steps::server_fn_tracing_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "server-fn-coverage",
        run: steps::server_fn_coverage_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "server-fn-wire-arg-error",
        run: steps::server_fn_wire_arg_error_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "traced-context",
        run: steps::traced_context_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "e2e-telemetry-boundary",
        run: steps::e2e_telemetry_boundary_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "proffered-secret",
        run: steps::proffered_secret_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "e2e-goto-wrapper",
        run: steps::e2e_goto_wrapper_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "e2e-server-fn-endpoints",
        run: steps::e2e_server_fn_endpoint_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "target-arch-placement",
        run: steps::target_arch_placement_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "write-transaction-contract",
        run: steps::write_transaction_contract_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "common-host-target-closure",
        run: steps::common_host_target_closure::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "lint-suppression",
        run: steps::lint_suppression_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "thin-components",
        run: steps::thin_components::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "sqlx-newtype-bind",
        run: steps::sqlx_newtype_bind_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "sqlx-newtype-decode",
        run: steps::sqlx_newtype_decode_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "doctest-fences",
        run: steps::doctest_fences::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "rendered-html-compiler-boundary",
        run: steps::rendered_html_compiler_boundary::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "raw-html-door",
        run: steps::raw_html_door_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "html-sink",
        run: steps::html_sink_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "e2e-scaffold",
        run: steps::e2e_scaffold_check::run,
        markdown_eligible: false,
    },
    HostGateStep::ResultOnly {
        name: "xlang-literal",
        run: steps::xlang_literal_check::run,
        markdown_eligible: false,
    },
    // Compile/type surfaces after cheap repository-shape checks. They are still
    // before host runtime tests, which cannot pass if compilation is broken.
    HostGateStep::StaticChecks(steps::static_checks::Phase::CompileAndType),
    HostGateStep::StaticChecks(steps::static_checks::Phase::HostRuntime),
];

pub(crate) const HOST_TESTS_STEP: HostGateStep = HostGateStep::HostTests {
    markdown_eligible: false,
};

pub(crate) fn run_host_steps_with<'a>(
    sh: &Shell,
    mode: Mode,
    policy: ExecutionPolicy,
    result: &mut CommandResult,
    steps: impl IntoIterator<Item = &'a HostGateStep>,
    mut run_step: impl FnMut(&HostGateStep, &Shell, Mode, ExecutionPolicy, &mut CommandResult),
) {
    run_with_policy(policy, result, steps, |step, result| {
        run_step(step, sh, mode, policy, result);
    });
}

pub(crate) fn run_host_gate_without_tests(
    sh: &Shell,
    mode: Mode,
    policy: ExecutionPolicy,
    result: &mut CommandResult,
) {
    run_host_steps_with(
        sh,
        mode,
        policy,
        result,
        HOST_GATE_NON_TEST_STEPS,
        HostGateStep::run,
    );
}

fn run_markdown_host_gate(
    sh: &Shell,
    mode: Mode,
    policy: ExecutionPolicy,
    result: &mut CommandResult,
) {
    run_host_steps_with(
        sh,
        mode,
        policy,
        result,
        HOST_GATE_NON_TEST_STEPS
            .iter()
            .filter(|step| step.markdown_eligible(mode)),
        HostGateStep::run_markdown,
    );
}

pub(crate) fn run_host_gate(
    sh: &Shell,
    mode: Mode,
    policy: ExecutionPolicy,
    result: &mut CommandResult,
) {
    run_host_steps_with(
        sh,
        mode,
        policy,
        result,
        HOST_GATE_NON_TEST_STEPS
            .iter()
            .chain(std::iter::once(&HOST_TESTS_STEP)),
        HostGateStep::run,
    );
}

pub(crate) fn dispatch_precommit_host_gate_with(
    class: git::PrecommitChangeClass,
    result: &mut CommandResult,
    run_markdown: impl FnOnce(&mut CommandResult),
    run_broad: impl FnOnce(&mut CommandResult),
) {
    match class {
        git::PrecommitChangeClass::StagedMarkdownOnly => run_markdown(result),
        git::PrecommitChangeClass::Broad(_) => run_broad(result),
    }
}

pub(crate) fn dispatch_precommit_host_gate(
    class: git::PrecommitChangeClass,
    sh: &xshell::Shell,
    policy: ExecutionPolicy,
    result: &mut CommandResult,
) {
    dispatch_precommit_host_gate_with(
        class,
        result,
        |result| run_markdown_host_gate(sh, Mode::Fix, policy, result),
        |result| run_host_gate(sh, Mode::Fix, policy, result),
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrepushPhase {
    HostGate,
    LocalTests,
    WorkspaceDoctests,
}

impl PrepushPhase {
    fn run(self, sh: &Shell, policy: ExecutionPolicy, result: &mut CommandResult) {
        match self {
            Self::HostGate => run_host_gate(sh, Mode::Check, policy, result),
            Self::LocalTests => steps::test_local::run(sh, result, &[]),
            Self::WorkspaceDoctests => steps::doctest_fences::run_workspace(sh, result),
        }
    }

    #[cfg(test)]
    const fn name(self) -> &'static str {
        match self {
            Self::HostGate => "host-gate",
            Self::LocalTests => "test-local",
            Self::WorkspaceDoctests => "workspace-doctests",
        }
    }
}

const PREPUSH_PHASES: &[PrepushPhase] = &[
    PrepushPhase::HostGate,
    PrepushPhase::LocalTests,
    PrepushPhase::WorkspaceDoctests,
];

pub(crate) fn run_local_push_gate(sh: &Shell, policy: ExecutionPolicy, result: &mut CommandResult) {
    run_with_policy(policy, result, PREPUSH_PHASES, |phase, result| {
        phase.run(sh, policy, result);
    });
}

pub(crate) fn run_prepush_with(
    sh: &Shell,
    policy: ExecutionPolicy,
    result: &mut CommandResult,
    precheck: impl FnOnce() -> StepResult,
    run_gate: impl FnOnce(&Shell, ExecutionPolicy, &mut CommandResult),
) {
    let precheck_start = std::time::Instant::now();
    let precheck = precheck().with_duration(precheck_start.elapsed());
    let blocked = precheck.is_blocking_failure();
    result.push(precheck);
    if !blocked {
        run_gate(sh, policy, result);
    }
}

#[cfg(test)]
fn host_gate_step_names_for_test(mode: Mode) -> Vec<&'static str> {
    let mut names = host_gate_without_tests_step_names_for_test(mode);
    HOST_TESTS_STEP.push_names(mode, &mut names);
    names
}

#[cfg(test)]
fn host_gate_without_tests_step_names_for_test(mode: Mode) -> Vec<&'static str> {
    let mut names = Vec::new();
    for step in HOST_GATE_NON_TEST_STEPS {
        step.push_names(mode, &mut names);
    }
    names
}

#[cfg(test)]
fn precommit_host_step_names_for_test() -> Vec<&'static str> {
    host_gate_step_names_for_test(Mode::Fix)
}

#[cfg(test)]
fn markdown_precommit_step_names_for_test() -> Vec<&'static str> {
    let mut names = Vec::new();
    for step in HOST_GATE_NON_TEST_STEPS {
        match step {
            HostGateStep::StaticChecks(phase) => names.extend(
                steps::static_checks::specs_for_phase(*phase, Mode::Fix)
                    .into_iter()
                    .filter(|spec| spec.markdown_eligible)
                    .map(|spec| spec.name),
            ),
            HostGateStep::ResultOnly {
                name,
                markdown_eligible: true,
                ..
            } => names.push(name),
            HostGateStep::ResultOnly { .. } | HostGateStep::HostTests { .. } => {}
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(names: &[&str], name: &str) -> usize {
        names
            .iter()
            .position(|candidate| *candidate == name)
            .unwrap_or_else(|| panic!("{name} is present"))
    }

    #[test]
    fn prepush_phase_plan_is_ordered_unique_and_nix_free() {
        let phases: Vec<_> = PREPUSH_PHASES.iter().map(|phase| phase.name()).collect();

        assert_eq!(
            phases,
            ["host-gate", "test-local", "workspace-doctests"],
            "the production-used prepush plan is the complete local gate"
        );
        assert_eq!(
            phases.len(),
            phases
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            "each prepush phase runs exactly once"
        );
        assert!(!phases.iter().any(|name| name.contains("nix")));

        let host_steps = host_gate_step_names_for_test(Mode::Check);
        assert_eq!(
            host_steps
                .iter()
                .filter(|name| **name == "host-tests")
                .count(),
            1
        );
        assert!(
            !host_steps.iter().any(|name| name.contains("nix")),
            "the host phase must remain Nix-free"
        );
    }

    #[test]
    fn precommit_host_surface_is_check_no_test_surface() {
        let check = host_gate_step_names_for_test(Mode::Fix);
        let precommit = precommit_host_step_names_for_test();
        assert_eq!(precommit, check);
        assert!(!precommit.contains(&"nix-wasm-tests"));
        assert!(!precommit.contains(&"nix-coverage"));
        assert!(!precommit.contains(&"nix-doctests"));
        assert!(!precommit.contains(&"proffered-filename"));
    }

    #[test]
    fn staged_markdown_route_filters_the_production_host_catalog_in_order() {
        assert_eq!(
            markdown_precommit_step_names_for_test(),
            [
                "prettier-markdown",
                "sequence-check",
                "adr-filenames",
                "doc-links",
                "flow-docs",
                "error-swallowing-inventory",
            ]
        );
    }

    #[test]
    fn host_gate_order_prioritizes_cheap_actionable_feedback() {
        let names = host_gate_step_names_for_test(Mode::Check);
        let write_transaction_contract = position(&names, "write-transaction-contract");
        let target_closure = position(&names, "common-host-target-closure");

        let fmt = position(&names, "fmt");
        let flow_docs = position(&names, "flow-docs");
        let clippy = position(&names, "clippy");
        let host_tests = position(&names, "host-tests");

        assert!(
            fmt < flow_docs,
            "source consistency runs before repo-shape checks"
        );
        assert!(
            target_closure < clippy,
            "target-resolved repository-shape checks run before compile checks"
        );
        assert!(
            write_transaction_contract < clippy,
            "write-transaction contract runs before compile checks"
        );
        assert!(
            flow_docs < clippy,
            "repo-shape checks run before expensive compile checks"
        );
        assert!(
            clippy < host_tests,
            "compile checks run before host runtime tests"
        );
    }

    #[test]
    fn prepush_clean_tree_short_circuits_the_local_gate() {
        let sh = Shell::new().expect("create shell");
        let mut result = CommandResult::new("prepush");
        let invoked = std::cell::Cell::new(false);

        run_prepush_with(
            &sh,
            execution_policy(&Command::Prepush),
            &mut result,
            || StepResult::fail("clean-tree").detail("dirty"),
            |_, _, _| invoked.set(true),
        );

        assert!(
            !invoked.get(),
            "the local gate must not run after a failed precheck"
        );
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.name.as_str())
                .collect::<Vec<_>>(),
            ["clean-tree"]
        );
    }

    #[test]
    fn fail_fast_detects_a_failure_when_one_phase_appends_multiple_results() {
        let mut result = CommandResult::new("prepush");
        let mut invoked = Vec::new();

        run_with_policy(
            ExecutionPolicy::FailFast,
            &mut result,
            PREPUSH_PHASES.iter().copied(),
            |phase, result| {
                invoked.push(phase);
                if matches!(phase, PrepushPhase::HostGate) {
                    result.push(StepResult::ok("host-gate-started"));
                    result.push(StepResult::fail("host-gate-failed").detail("preserved failure"));
                } else {
                    result.push(StepResult::ok(phase.name()));
                }
            },
        );

        assert_eq!(invoked, [PrepushPhase::HostGate]);
        assert_eq!(
            result
                .steps
                .iter()
                .map(|step| step.name.as_str())
                .collect::<Vec<_>>(),
            ["host-gate-started", "host-gate-failed"]
        );
        assert_eq!(result.steps[1].detail.as_deref(), Some("preserved failure"));
    }

    #[test]
    fn prepush_stops_before_product_and_doctest_phases_after_host_failure() {
        let mut result = CommandResult::new("prepush");
        let mut invoked = Vec::new();

        run_with_policy(
            execution_policy(&Command::Prepush),
            &mut result,
            PREPUSH_PHASES.iter().copied(),
            |phase, result| {
                invoked.push(phase);
                match phase {
                    PrepushPhase::HostGate => {
                        result.push(StepResult::fail("synthetic-host").detail("host failed"))
                    }
                    PrepushPhase::LocalTests => result.push(StepResult::ok("synthetic-product")),
                    PrepushPhase::WorkspaceDoctests => {
                        result.push(StepResult::ok("synthetic-doctests"))
                    }
                }
            },
        );

        assert_eq!(invoked, [PrepushPhase::HostGate]);
        assert_eq!(result.steps[0].detail.as_deref(), Some("host failed"));
    }

    #[test]
    fn prepush_stops_before_workspace_doctests_after_product_failure() {
        let mut result = CommandResult::new("prepush");
        let mut invoked = Vec::new();

        run_with_policy(
            execution_policy(&Command::Prepush),
            &mut result,
            PREPUSH_PHASES.iter().copied(),
            |phase, result| {
                invoked.push(phase);
                if matches!(phase, PrepushPhase::LocalTests) {
                    result.push(StepResult::fail("synthetic-product").detail("product failed"));
                } else {
                    result.push(StepResult::ok("synthetic-phase"));
                }
            },
        );

        assert_eq!(invoked, [PrepushPhase::HostGate, PrepushPhase::LocalTests]);
        assert_eq!(result.steps[1].detail.as_deref(), Some("product failed"));
    }

    #[test]
    fn command_execution_policies_keep_explicit_gates_exhaustive_and_hooks_fail_fast() {
        assert_eq!(
            execution_policy(&Command::Check { no_test: true }),
            ExecutionPolicy::Exhaustive
        );
        assert_eq!(
            execution_policy(&Command::Validate {
                no_e2e: true,
                allow_dirty: false,
            }),
            ExecutionPolicy::Exhaustive
        );
        assert_eq!(
            execution_policy(&Command::Precommit),
            ExecutionPolicy::FailFast
        );
        assert_eq!(
            execution_policy(&Command::Prepush),
            ExecutionPolicy::FailFast
        );
    }

    #[test]
    fn explicit_host_gates_keep_running_production_steps_after_a_static_failure() {
        fn assert_exhaustive(command: &str, policy: ExecutionPolicy, include_host_tests: bool) {
            let sh = xshell::Shell::new().expect("create shell");
            let mut result = CommandResult::new(command);
            let expected = if include_host_tests {
                host_gate_step_names_for_test(Mode::Check)
            } else {
                host_gate_without_tests_step_names_for_test(Mode::Check)
            };
            let mut invoked = Vec::new();
            let mut failed = false;
            let mut run_step =
                |step: &HostGateStep,
                 sh: &xshell::Shell,
                 mode: Mode,
                 policy: ExecutionPolicy,
                 result: &mut CommandResult| match step {
                    HostGateStep::StaticChecks(phase) => {
                        run_static_phase_with(sh, mode, *phase, policy, result, |_, spec| {
                            invoked.push(spec.name);
                            if std::mem::replace(&mut failed, true) {
                                StepResult::ok(spec.name)
                            } else {
                                StepResult::fail(spec.name).detail("synthetic static failure")
                            }
                        });
                    }
                    HostGateStep::ResultOnly { name, .. } => {
                        invoked.push(name);
                        result.push(StepResult::ok(name));
                    }
                    HostGateStep::HostTests { .. } => {
                        invoked.push("host-tests");
                        result.push(StepResult::ok("host-tests"));
                    }
                };

            if include_host_tests {
                run_host_steps_with(
                    &sh,
                    Mode::Check,
                    policy,
                    &mut result,
                    HOST_GATE_NON_TEST_STEPS
                        .iter()
                        .chain(std::iter::once(&HOST_TESTS_STEP)),
                    &mut run_step,
                );
            } else {
                run_host_steps_with(
                    &sh,
                    Mode::Check,
                    policy,
                    &mut result,
                    HOST_GATE_NON_TEST_STEPS,
                    &mut run_step,
                );
            }

            assert_eq!(
                invoked, expected,
                "{command} must keep every later production host step after a static failure"
            );
            assert_eq!(result.steps.len(), expected.len());
            assert!(!result.ok);
        }

        assert_exhaustive(
            "check",
            execution_policy(&Command::Check { no_test: true }),
            true,
        );
        assert_exhaustive(
            "validate",
            execution_policy(&Command::Validate {
                no_e2e: true,
                allow_dirty: false,
            }),
            false,
        );
    }

    #[test]
    fn validate_host_surface_keeps_wasm_budget_before_host_tests() {
        let mut validate = host_gate_without_tests_step_names_for_test(Mode::Check);
        validate.push("wasm-budget");
        HOST_TESTS_STEP.push_names(Mode::Check, &mut validate);

        let wasm_budget = validate
            .iter()
            .position(|name| *name == "wasm-budget")
            .expect("validate includes wasm-budget");
        let host_tests = validate
            .iter()
            .position(|name| *name == "host-tests")
            .expect("validate includes host-tests");
        assert!(wasm_budget < host_tests);
    }

    #[test]
    fn check_uses_local_product_tests_before_nix_only_test_checks() {
        let mut names = host_gate_step_names_for_test(Mode::Fix);
        names.push("test-local");
        names.extend(steps::nix::check_supporting_test_check_names());

        let test_local = names
            .iter()
            .position(|name| *name == "test-local")
            .expect("check includes the host-native product test lane");
        let wasm_tests = names
            .iter()
            .position(|name| *name == "wasm-tests")
            .expect("check keeps the Nix-only wasm browser tests");

        assert!(test_local < wasm_tests);
        assert!(!names.contains(&"coverage"));
        assert!(names.contains(&"doctests"));
    }

    #[test]
    fn ci_shape_keeps_the_elisp_verdict_in_the_static_lane() {
        let workflow = include_str!("../../.github/workflows/ci.yml");

        assert!(workflow.contains("validate-no-e2e:"));
        assert!(workflow.contains("cargo xtask validate\n          --no-e2e"));
        assert!(workflow.contains(".xtask/gcroots/elisp-coverage-producer/elisp-coverage/"));
        assert!(workflow.contains("needs: [e2e]"));
        assert!(!workflow.contains("elisp-integration"));
    }

    #[test]
    fn check_and_validate_include_flow_docs() {
        let with_tests = host_gate_step_names_for_test(Mode::Fix);
        let without_tests = host_gate_without_tests_step_names_for_test(Mode::Check);

        assert_eq!(
            with_tests
                .iter()
                .filter(|name| **name == "flow-docs")
                .count(),
            1,
            "check host gate must include flow-docs exactly once"
        );
        assert_eq!(
            without_tests
                .iter()
                .filter(|name| **name == "flow-docs")
                .count(),
            1,
            "validate host gate must include flow-docs exactly once before host-tests"
        );

        let doc_links = without_tests
            .iter()
            .position(|name| *name == "doc-links")
            .expect("doc-links is in host gate");
        let flow_docs = without_tests
            .iter()
            .position(|name| *name == "flow-docs")
            .expect("flow-docs is in host gate");
        assert_eq!(
            flow_docs,
            doc_links + 1,
            "flow-docs must run immediately after doc-links"
        );
        assert!(
            !without_tests.contains(&"host-tests"),
            "validate's early host gate excludes host tests"
        );
    }

    #[test]
    fn fail_fast_static_phase_stops_after_the_first_failed_spec() {
        let sh = Shell::new().expect("create shell");
        let mut result = CommandResult::new("precommit");
        let mut invoked = Vec::new();

        run_static_phase_with(
            &sh,
            Mode::Fix,
            steps::static_checks::Phase::SourceConsistency,
            ExecutionPolicy::FailFast,
            &mut result,
            |_, spec| {
                invoked.push(spec.name);
                if invoked.len() == 1 {
                    StepResult::fail(spec.name).detail("synthetic static failure")
                } else {
                    StepResult::ok(spec.name)
                }
            },
        );

        assert_eq!(invoked.len(), 1, "later static specs must not run");
        assert_eq!(result.steps.len(), 1);
        assert_eq!(
            result.steps[0].detail.as_deref(),
            Some("synthetic static failure")
        );
    }

    #[test]
    fn exhaustive_static_phase_continues_after_a_failed_spec() {
        let sh = Shell::new().expect("create shell");
        let expected = steps::static_checks::specs_for_phase(
            steps::static_checks::Phase::SourceConsistency,
            Mode::Check,
        )
        .len();
        let mut result = CommandResult::new("check");
        let mut invoked = 0;

        run_static_phase_with(
            &sh,
            Mode::Check,
            steps::static_checks::Phase::SourceConsistency,
            ExecutionPolicy::Exhaustive,
            &mut result,
            |_, spec| {
                invoked += 1;
                if invoked == 1 {
                    StepResult::fail(spec.name).detail("synthetic static failure")
                } else {
                    StepResult::ok(spec.name)
                }
            },
        );

        assert_eq!(
            invoked, expected,
            "exhaustive execution keeps later diagnostics"
        );
        assert_eq!(result.steps.len(), expected);
        assert!(!result.ok);
    }
}
