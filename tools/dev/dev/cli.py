"""The `dev` command line: one subcommand per workflow."""

import dataclasses
import re
from collections.abc import Callable
from pathlib import Path
from typing import Any

import click

from dev import (
    checkout,
    csr,
    e2e,
    e2e_local,
    forwarding,
    gates,
    issues,
    perf,
    pr_land,
    pr_watch,
    static,
    steps,
    summary,
    trace_analysis,
    upgrade,
    wasm_audit,
)
from dev.static import Mode


def _gate(body: Callable[[], None], sandbox: bool = False) -> None:
    """Runs a workflow from the checkout root, or in place inside a Nix sandbox."""
    context = click.get_current_context()
    context.exit(steps.gate(checkout.enter(sandbox), context.command_path, body))


@click.group()
def cli() -> None:
    """Jaunder developer, hook, CI and sandbox workflows."""


mode_argument = click.argument(
    "mode",
    type=click.Choice([mode.value for mode in Mode]),
    default=Mode.VERIFY.value,
    callback=lambda _context, _parameter, value: Mode(value),
)


def _workflow(name: str, body: Callable[[], None], summary: str) -> None:
    @cli.command(name, help=summary)
    def command() -> None:
        _gate(body)


_workflow(
    "check-no-test",
    gates.check_no_test,
    "The host check groups in Fix mode, then the auxiliary host tests.",
)
_workflow(
    "check",
    gates.check,
    "check-no-test, then test-local, Nix wasm-tests and the doctests "
    "producer and gate.",
)
_workflow(
    "prepush",
    gates.prepush,
    "Clean tree; the host check groups in Verify mode (on x86_64-linux with "
    "shared Nix verdicts for Rust format, Markdown, tsc and Elisp), the auxiliary host "
    "tests, test-local and the doctests producer and gate.",
)
_workflow(
    "validate-no-e2e",
    gates.validate_no_e2e,
    "Clean tree; Verify host checks, the platform contract, Nix static "
    "proof, wasm budget and tests, coverage, doctests and Elisp coverage.",
)
_workflow("validate", gates.validate, "validate-no-e2e, then the full E2E workflow.")
_workflow(
    "ci-validate-host",
    gates.ci_validate_host,
    "CI host lane: clean tree, Verify host checks, the platform contract "
    "and the auxiliary host tests.",
)
_workflow(
    "ci-validate-hermetic",
    gates.ci_validate_hermetic,
    "CI hermetic lane: clean tree, Nix static proof and the wasm budget.",
)
_workflow(
    "ci-validate-test-checks",
    gates.ci_validate_test_checks,
    "CI test-checks lane: clean tree, wasm tests, doctests and Elisp coverage.",
)
_workflow(
    "ci-validate-docs",
    gates.ci_validate_docs,
    "CI docs lane, for a Markdown-only change: clean tree, Verify Markdown "
    "Prettier and the documentation-shape checks.",
)


_workflow("coverage", gates.coverage, "Clean tree, then Rust coverage and CRAP policy.")
_workflow(
    "doctests", gates.doctests, "Clean tree, then the doctests producer and gate."
)
_workflow(
    "precommit",
    gates.precommit,
    "The pre-commit gate: snapshot the index, run the host checks the change "
    "selects, re-stage safe fixes and, for Markdown alone, verify shared docs.",
)
_workflow(
    "docs-shape",
    gates.docs_shape,
    "Documentation sequence, ADR format and parity, doc links and flow docs.",
)
_workflow(
    "source-contracts",
    gates.source_contracts,
    "ast-grep rules and every repository source-contract policy.",
)


@cli.command("nix-check")
@click.argument("check")
def nix_check(check: str) -> None:
    """Build the flake check CHECK as one traced step with its cache state."""
    _gate(lambda: steps.nix_check(check))


@cli.command("format")
@mode_argument
def format_command(mode: Mode) -> None:
    """Formatters, in Fix or Verify MODE."""
    _gate(lambda: gates.format_checks(mode))


@cli.command("compile-checks")
@mode_argument
def compile_checks(mode: Mode) -> None:
    """TypeScript, cargo-deny and the Clippy units."""
    _gate(lambda: gates.compile_checks(mode))


@cli.command("elisp-checks")
@mode_argument
def elisp_checks(mode: Mode) -> None:
    """Elisp byte-compilation and ERT."""
    _gate(lambda: gates.elisp_checks(mode))


@cli.command("source-contract")
@click.argument("name")
def source_contract(name: str) -> None:
    """One repository source-contract policy by NAME, e.g. html-sink."""
    _gate(lambda: gates.source_contract(name))


@cli.command("static-all")
@mode_argument
def static_all(mode: Mode) -> None:
    """Every host static leaf in order, each prepared from this checkout."""
    _gate(lambda: static.run_all(mode))


@cli.command("adr-format")
def adr_format() -> None:
    """Check the ADR format."""
    _gate(lambda: gates.repository_docs("adr-format"))


class _Forwarding(click.Command):
    """An entry point that passes every argument to its tool verbatim.

    Options and `--help` belong to the tool, and nothing is parsed here. A
    leading `--` is optional and dropped, so
    `dev X -- ARGS` and `dev X ARGS` forward the same argv; any later `--`
    is forwarded with the rest.
    """

    def parse_args(self, ctx: click.Context, args: list[str]) -> list[str]:
        forwarded = args[1:] if args[:1] == ["--"] else args
        ctx.params["args"] = tuple(forwarded)
        return []


def _forwarding(
    name: str,
    body: Callable[[tuple[str, ...]], None],
    summary: str,
) -> None:
    def command(args: tuple[str, ...]) -> None:
        _gate(lambda: body(args))

    cli.add_command(_Forwarding(name, callback=command, help=summary))


_forwarding(
    "test-local",
    gates.test_local,
    "Focused Rust proof with isolated PostgreSQL; ARGS are nextest selectors "
    "(none: the workspace).",
)
_forwarding(
    "sandbox", forwarding.sandbox, "Run the UX sandbox with ARGS, kept as argv."
)
_forwarding(
    "pg-run",
    forwarding.pg_run,
    "Run a command against a throwaway PostgreSQL cluster "
    "(JAUNDER_PG_TEST_URL exported).",
)
_forwarding(
    "coverage-check",
    forwarding.coverage_check,
    "Manual coverage evidence tools: probe-source | wasm-probe | wasm-measure.",
)


@cli.group("wasm-audit")
def wasm_audit_group() -> None:
    """Frontend bundle size diagnostics; `validate` runs the raw wasm budget."""


@wasm_audit_group.command("report")
@click.option("--site-path", help="Measure this built CSR bundle root.")
@click.option(
    "--breakdown",
    is_flag=True,
    help="Attribute the unstripped `.#csrWasm` to sections and crates; "
    "its total is not the shipped size.",
)
@click.option("--wasm", help="With --breakdown: this wasm instead of `.#csrWasm`.")
def wasm_audit_report(site_path: str | None, breakdown: bool, wasm: str | None) -> None:
    """Measure the shipped bundle, building `.#csrBundle` unless given a root.

    Run after a change expected to move it, or periodically to watch the trend.
    """
    if wasm is not None and not breakdown:
        raise click.UsageError("--wasm requires --breakdown")
    if site_path is not None and breakdown:
        raise click.UsageError("--site-path is the shipped bundle; --breakdown is not")
    if breakdown:
        _gate(lambda: wasm_audit.breakdown(wasm or wasm_audit.csr_wasm()))
    else:
        _gate(lambda: wasm_audit.report(site_path or gates.csr_bundle()))


@cli.command("upgrade-test")
@click.option("--from", "revision", help="Earlier revision to upgrade from.")
@click.option("--tag", help="Upgrade from the `v*` tag before this one.")
@click.argument("backend", type=click.Choice(upgrade.BACKENDS))
def upgrade_test(revision: str | None, tag: str | None, backend: str) -> None:
    """Upgrade a site an earlier build wrote to this checkout, on BACKEND, in a VM.

    Without --from or --tag the earlier build is the newest green CI run on main.
    """
    if revision is not None and tag is not None:
        raise click.UsageError("--from and --tag are exclusive")
    _gate(lambda: upgrade.run(revision, tag, backend))


@cli.command("upgrade-dispatch")
@click.option(
    "--from", "revision", help="Earlier revision; default: the last green run."
)
@click.option(
    "--ref", default="main", show_default=True, help="Branch the workflow runs on."
)
def upgrade_dispatch(revision: str | None, ref: str) -> None:
    """Start the CI upgrade workflow."""
    _gate(lambda: upgrade.dispatch(revision, ref))


@cli.command("build-csr")
@click.argument("profile", type=click.Choice(["debug", "release"]), default="debug")
def build_csr(profile: str) -> None:
    """Build the checkout's CSR bundle into target/site."""
    _gate(lambda: csr.build(profile))


@cli.command("perf")
@click.argument("profile", default="medium")
@click.argument("backends", default="sqlite,postgres")
@click.argument("browsers", default="")
@click.argument("workload", default="all")
@click.argument("posts", default="")
@click.argument("authors", default="")
@click.argument("revisions", default="")
def perf_command(**request: str) -> None:
    """Measure the selected performance producers, keeping each one's evidence."""
    _gate(lambda: perf.measure(perf.Request(**request)))


@cli.command("perf-compare")
@click.argument("result")
def perf_compare(result: str) -> None:
    """Compare RESULT with the committed baseline; advisory only."""
    _gate(lambda: perf.compare(result))


@cli.command("perf-import-baseline")
@click.argument("run_id", type=click.IntRange(min=1))
def perf_import_baseline(run_id: int) -> None:
    """Replace the baseline from an authenticated CI run RUN_ID."""
    _gate(lambda: perf.import_baseline(run_id))


@cli.command("e2e")
@click.argument("jobs", default="")
def e2e_command(jobs: str) -> None:
    """Every E2E producer, at most JOBS at once, then every reconciliation."""
    _gate(lambda: e2e.run_all(jobs))


@cli.command("e2e-local")
@click.option(
    "--browser",
    type=click.Choice(e2e_local.BROWSERS),
    help="Run this browser's one Playwright project (default chromium).",
)
@click.argument("test", required=False)
@click.option(
    "--update-visual-snapshots",
    is_flag=True,
    help="Update every Chromium and Firefox visual baseline using release CSR.",
)
def e2e_local_command(
    browser: str | None, test: str | None, update_visual_snapshots: bool
) -> None:
    """Focused browser proof: use a spec or file:line TEST while iterating.

    Rerun the same focused command after a fix. Use representative hermetic
    `dev e2e-producer <identity>` proof when backend/browser parity,
    release-build behavior or the hermetic environment is load-bearing; local
    proof does not replace CI/full validation.

    Each browser gets its own lifecycle: build the CSR bundle and server, start
    one `jaunder serve` per Playwright worker on an ephemeral port with the VM's
    capture environment and a fresh database, seed each with `test-support
    seed-e2e`, run Playwright against them and stop everything on every exit
    path. Visual update mode builds release CSR and updates Chromium and then
    Firefox baselines. The step summary is on stdout; child output goes to
    stderr.
    """
    if update_visual_snapshots and browser is not None:
        raise click.UsageError("--update-visual-snapshots conflicts with --browser")
    if update_visual_snapshots and test is not None:
        raise click.UsageError("--update-visual-snapshots conflicts with TEST")
    _gate(lambda: e2e_local.run(browser, test, update_visual_snapshots))


@cli.command("e2e-producer")
@click.argument("lane")
def e2e_producer(lane: str) -> None:
    """One isolated E2E producer LANE, as CI runs it."""
    _gate(lambda: e2e.producer(lane))


@cli.command("e2e-regenerate-coverage")
def e2e_regenerate_coverage() -> None:
    """Regenerate the server-function coverage snapshot from fresh SQLite/Chromium lanes."""
    _gate(e2e.regenerate_coverage)


@cli.command("e2e-reconcile")
@click.argument("backend")
@click.argument("browser")
@click.argument("artifacts", default=".xtask/diagnostics")
def e2e_reconcile(backend: str, browser: str, artifacts: str) -> None:
    """Reconcile BACKEND/BROWSER producer ARTIFACTS without building any producer."""
    _gate(lambda: e2e.reconcile(backend, browser, artifacts))


@cli.command("issue-discover")
@click.argument("milestone")
def issue_discover(milestone: str) -> None:
    """Exhaustive read-only candidates in MILESTONE; never claims work."""
    _gate(lambda: issues.discover(milestone))


def _issue_decisions(command: Callable[..., None]) -> Callable[..., None]:
    """Both issue front doors take every decision explicitly, then any blockers."""
    command = click.argument("blockers", nargs=-1)(command)
    for field in reversed(dataclasses.fields(issues.Decisions)):
        command = click.argument(field.name)(command)
    return command


@cli.command("issue-create")
@_issue_decisions
def issue_create(blockers: tuple[str, ...], **decisions: str) -> None:
    """Create an issue from explicit decisions; gh mutates, readback verifies."""
    _gate(lambda: issues.create(issues.Decisions(**decisions), blockers))


@cli.command("issue-edit")
@click.argument("number")
@_issue_decisions
def issue_edit(number: str, blockers: tuple[str, ...], **decisions: str) -> None:
    """Replace issue NUMBER's metadata with explicit decisions; readback verifies."""
    _gate(lambda: issues.edit(number, issues.Decisions(**decisions), blockers))


_SHA = re.compile("[0-9a-f]{40}")
_RUN_URL = re.compile(r"https://github\.com/[^/]+/[^/]+/actions/runs/[0-9]+")


def _matching(
    pattern: re.Pattern[str], expected: str
) -> Callable[[click.Context, click.Parameter, str | None], str | None]:
    def validate(
        _context: click.Context, _parameter: click.Parameter, value: str | None
    ) -> str | None:
        if value is not None and not pattern.fullmatch(value):
            raise click.BadParameter(f"expected {expected}")
        return value

    return validate


_full_sha = _matching(_SHA, "a full lowercase 40-hex head SHA")


@cli.command("pr-watch")
@click.argument("number", required=False, type=click.IntRange(min=0))
@click.option(
    "--interval",
    type=click.IntRange(min=5),
    default=pr_watch.Watch.interval,
    show_default=True,
    metavar="SECONDS",
    help="Seconds between polls.",
)
@click.option(
    "--timeout",
    type=click.IntRange(min=1),
    default=pr_watch.Watch.timeout,
    show_default=True,
    metavar="MINUTES",
    help="Minutes to observe before reporting timed-out.",
)
@click.option("--once", is_flag=True, help="Observe one snapshot instead of polling.")
@click.option(
    "--until",
    type=click.Choice(["merged"]),
    help="Continue across ready-to-land while another actor may arm the merge.",
)
@click.option(
    "--expected-head",
    callback=_full_sha,
    help="Bind every verdict, even merged, to this approved head.",
)
@click.option(
    "--acknowledged-ejection",
    callback=_matching(_RUN_URL, "a GitHub Actions run URL"),
    help="A historical failed merge-group run URL, disregarded as an ejection "
    "only; needs --expected-head and authorizes nothing.",
)
@click.option(
    "--observed-queued-head",
    callback=_full_sha,
    help="Positive same-head queue readback from a separate action; must "
    "equal --expected-head.",
)
def pr_watch_command(**options: Any) -> None:
    """Read-only caller-actionable PR observation. Never merges or mutates.

    Omit NUMBER to observe the open PR for the current branch.
    """
    watch = pr_watch.Watch(**options)
    if watch.once and watch.until:
        raise click.UsageError("--once conflicts with --until")
    if watch.expected_head is None and (
        watch.acknowledged_ejection or watch.observed_queued_head
    ):
        raise click.UsageError(
            "--acknowledged-ejection and --observed-queued-head require --expected-head"
        )
    # Queue history is evidence about one head only.
    if watch.observed_queued_head not in (None, watch.expected_head):
        raise click.UsageError("observed queued head must equal expected head")
    _gate(lambda: pr_watch.run(watch))


@cli.command("pr-land")
@click.argument("number", type=click.IntRange(min=1))
@click.argument("expected_head", callback=_full_sha)
@click.option(
    "--reenqueue",
    is_flag=True,
    help="Re-enqueue an observed ejection of any cause, once; needs separate "
    "recovery authorization.",
)
def pr_land_command(number: int, expected_head: str, reenqueue: bool) -> None:
    """Land an approved PR head, re-enqueueing one infrastructure ejection.

    Prior recorded exact PR/head human approval is mandatory, not inferred here.
    """
    _gate(lambda: pr_land.run(pr_land.Land(number, expected_head, reenqueue)))


@cli.command("pr-cleanup")
@click.argument("number")
@click.argument("expected_head")
@click.argument("ownership")
def pr_cleanup(number: str, expected_head: str, ownership: str) -> None:
    """Clean up after a merged PR; the caller must verify unmanaged OWNERSHIP."""
    _gate(lambda: forwarding.pr_cleanup(number, expected_head, ownership))


@cli.group("static")
def static_group() -> None:
    """Pinned static checks: one named check, or a fixed group of them.

    On the host a check first prepares what it needs from this checkout.
    Checks without an autofix ignore --fix.
    """


def _static_command(name: str, summary: str) -> None:
    @static_group.command(name, help=summary)
    @click.option("--fix", is_flag=True, help="Fix instead of verifying.")
    @click.option(
        "--sandbox",
        is_flag=True,
        help="Run in place, as a Nix check does: no checkout root, compile cache "
        "or host preparation, and Cargo checks offline against the provided "
        "Cargo homes.",
    )
    def command(fix: bool, sandbox: bool) -> None:
        if sandbox and static.missing_home(name):
            raise click.ClickException("sandbox Cargo home is missing")
        mode = Mode.FIX if fix else Mode.VERIFY
        _gate(lambda: static.run(name, mode, sandbox), sandbox)


for _name, _leaf in static.LEAVES.items():
    _static_command(_name, _leaf.summary)
for _name, (_summary, _) in static.GROUPS.items():
    _static_command(_name, _summary)


@cli.group("trace")
def trace_group() -> None:
    """Read traced runs."""


@trace_group.command("summary")
@click.argument(
    "path",
    required=False,
    type=click.Path(exists=True, file_okay=False, path_type=Path),
)
@click.option("--markdown", is_flag=True, help="Print a Markdown table.")
def trace_summary(path: Path | None, markdown: bool) -> None:
    """Show a run's slowest spans; PATH is a run directory, default the latest."""
    if path is None:
        path = summary.latest(checkout.root() / ".xtask" / "run")
    # Any record content loads; a run that cannot be read is reported, never
    # a traceback.
    try:
        summarized = summary.summarize(path)
    except OSError as error:
        raise click.ClickException(f"cannot summarise {path}: {error}") from error
    click.echo(summary.render(summarized, markdown), nl=False)


@cli.group("trace-analysis")
def trace_analysis_group() -> None:
    """Manual OTel trace diagnostics over the e2e lanes' exported traces."""


top_option = click.option(
    "--top",
    type=click.IntRange(min=1),
    default=25,
    show_default=True,
    help="Rows per ranked table. The boot-coverage, per-project and "
    "long-task-by-project tables always print every row.",
)
trace_option = click.option("--trace", help="Restrict analysis to one trace id.")
files_argument = click.argument(
    "files", nargs=-1, required=True, type=click.Path(path_type=Path)
)


@trace_analysis_group.command("analyze")
@top_option
@trace_option
@click.option(
    "--project",
    help="Restrict analysis to one e2e project (filters only `e2e.`-named spans).",
)
@click.option(
    "--playwright-report",
    "playwright_reports",
    multiple=True,
    type=click.Path(path_type=Path),
    help="A Playwright JSON report, repeatable: the per-test span-coverage "
    "denominator. Without one that section is skipped with a note.",
)
@files_argument
def trace_analysis_analyze(
    top: int,
    trace: str | None,
    project: str | None,
    playwright_reports: tuple[Path, ...],
    files: tuple[Path, ...],
) -> None:
    """Tables over FILES, `otel-traces.jsonl` exports: span coverage, boot
    coverage, slowest spans, per-test and per-project hotspots and trace totals.

    \b
    dev trace-analysis analyze sqlite-otel-traces.jsonl postgres-otel-traces.jsonl
    dev trace-analysis analyze --top 40 --project firefox a.jsonl b.jsonl
    dev trace-analysis analyze --playwright-report report.json traces.jsonl
    """
    _gate(
        lambda: trace_analysis.analyze_command(
            top, trace, project, playwright_reports, files
        )
    )


@trace_analysis_group.command("run")
@top_option
@trace_option
@click.option(
    "--single-worker",
    is_flag=True,
    help="Build the single-worker packages instead of the lane checks, for "
    "per-navigation cost free of worker contention.",
)
@click.option(
    "--browser",
    type=click.Choice(trace_analysis.BROWSERS),
    help="Restrict to one browser (default both); both backends are always built.",
)
def trace_analysis_run(
    top: int, trace: str | None, single_worker: bool, browser: str | None
) -> None:
    """Build every enabled e2e lane check (or the single-worker packages) for
    both backends, then analyze their captures."""
    _gate(lambda: trace_analysis.run_command(top, trace, single_worker, browser))


@trace_analysis_group.command("boot-phases")
@files_argument
def trace_analysis_boot_phases(files: tuple[Path, ...]) -> None:
    """Median boot-phase decomposition of FILES per (trace file, project,
    cacheWarmth, experiment arm) (#818).

    Every segment is document-relative and the segments close on
    `mount_done.startTime`; the Node-side `commitToMountMs` and the skew between
    the two frames are reported, never decomposed.
    """
    _gate(lambda: trace_analysis.boot_phases_command(files))
