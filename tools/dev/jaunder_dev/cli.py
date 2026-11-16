"""The `dev` command line: one subcommand per workflow."""

import functools
import os
import subprocess
from collections.abc import Callable
from pathlib import Path
from typing import Any

import click

from jaunder_dev import gates, issues, static, steps, summary, workflows


def _git(*args: str) -> subprocess.CompletedProcess[str]:
    environment = {
        key: value
        for key, value in os.environ.items()
        if key not in gates.GIT_OVERRIDES
    }
    return subprocess.run(
        ["git", *args], env=environment, capture_output=True, text=True, check=False
    )


def checkout_root() -> Path:
    found = _git("rev-parse", "--show-toplevel")
    if found.returncode != 0:
        raise click.ClickException(
            f"cannot find the checkout root: {found.stderr.strip()}"
        )
    return Path(found.stdout.strip())


def compile_cache_environment() -> dict[str, str]:
    """Host Cargo compilation shares rustc artifacts across every worktree.

    A failed worktree listing keeps the current root alone.
    """
    listed = _git("worktree", "list", "--porcelain").stdout.splitlines()
    worktrees = [
        Path(line.removeprefix("worktree "))
        for line in listed
        if line.startswith("worktree ")
    ]
    roots = {
        str(path.resolve())
        for path in [*worktrees, Path.cwd()]
        if path.is_absolute() and path.is_dir()
    }
    return {
        "RUSTC_WRAPPER": "sccache",
        "CARGO_INCREMENTAL": "0",
        "SCCACHE_BASEDIRS": ":".join(sorted(roots)),
    }


def _gate(body: Callable[[], None], sandbox: bool = False) -> None:
    """Runs a workflow from the checkout root, or in place inside a Nix sandbox."""
    context = click.get_current_context()
    root = Path.cwd() if sandbox else checkout_root()
    os.chdir(root)
    if not sandbox:
        os.environ.update(compile_cache_environment())
    context.exit(steps.gate(root, context.command_path, body))


@click.group()
def cli() -> None:
    """Jaunder developer, hook, CI and sandbox workflows."""


mode_argument = click.argument(
    "mode", type=click.Choice(["verify", "fix"]), default="verify"
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
    "Clean tree; the host check groups in Verify mode, the auxiliary host "
    "tests, test-local and the workspace doctests.",
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
    "ci-validate-coverage",
    gates.coverage,
    "CI coverage lane: clean tree, then Rust coverage and CRAP policy.",
)
_workflow("coverage", gates.coverage, "Clean tree, then Rust coverage and CRAP policy.")
_workflow(
    "doctests", gates.doctests, "Clean tree, then the doctests producer and gate."
)
_workflow(
    "precommit",
    gates.precommit,
    "The pre-commit gate: Git/index safety selects and runs the host checks.",
)
_workflow(
    "precommit-markdown-host",
    gates.precommit_markdown_host,
    "Fix Markdown and end2end formatting, then docs-shape. Called only by "
    "the Git/index safety owner.",
)
_workflow(
    "precommit-broad-host",
    gates.check_no_test,
    "check-no-test, as the Git/index safety owner calls it.",
)
_workflow(
    "docs-shape",
    gates.docs_shape,
    "Documentation sequence, ADR format and parity, doc links and flow docs.",
)
_workflow(
    "source-contracts",
    gates.source_contracts,
    "ast-grep rules, every repository source-contract policy and the "
    "doctest fence check.",
)


@cli.command("format")
@mode_argument
@click.argument("docs", type=click.Choice(["host", "prepush"]), default="host")
def format_command(mode: str, docs: str) -> None:
    """Formatters, in Fix or Verify MODE.

    DOCS `prepush` takes the shared clean-index Markdown verdict on
    x86_64-linux; it only verifies.
    """
    if docs == "prepush" and mode != "verify":
        raise click.UsageError("prepush docs require verify")
    _gate(lambda: gates.format_checks(mode == "fix", docs))


@cli.command("compile-checks")
@mode_argument
def compile_checks(mode: str) -> None:
    """TypeScript, cargo-deny and the Clippy units."""
    _gate(lambda: gates.compile_checks(mode == "fix"))


@cli.command("elisp-checks")
@mode_argument
def elisp_checks(mode: str) -> None:
    """Elisp byte-compilation and ERT."""
    _gate(lambda: gates.elisp_checks(mode == "fix"))


@cli.command("source-contract")
@click.argument("name")
def source_contract(name: str) -> None:
    """One repository source-contract policy by NAME, e.g. html-sink."""
    _gate(lambda: gates.source_contract(name))


@cli.command()
@mode_argument
def leptosfmt(mode: str) -> None:
    """Format Leptos view! macros with the adapter built from this checkout."""
    _gate(lambda: gates.leptosfmt(mode == "fix"))


@cli.command()
@mode_argument
def tsc(mode: str) -> None:
    """Type-check the end-to-end suite after linking its pinned node_modules.

    TypeScript has no autofix, so MODE changes nothing.
    """
    _gate(lambda: gates.tsc(mode == "fix"))


@cli.command("static-all")
@mode_argument
def static_all(mode: str) -> None:
    """Every static check, with devtool and the adapter built from this checkout."""
    _gate(lambda: gates.static_all(mode == "fix"))


@cli.command("adr-format")
def adr_format() -> None:
    """Check the ADR format."""
    _gate(lambda: gates.repository_docs("adr-format"))


class _Forwarding(click.Command):
    """An entry point that passes every argument to its tool verbatim.

    As Just's variadic recipes did, options and `--help` belong to the tool,
    and nothing is parsed here. A leading `--` is optional and dropped, so
    `dev X -- ARGS` and `dev X ARGS` forward the same argv; any later `--`
    is forwarded with the rest.
    """

    def __init__(self, name: str, required: bool, **settings: Any) -> None:
        super().__init__(name, **settings)
        self.required = required

    def parse_args(self, ctx: click.Context, args: list[str]) -> list[str]:
        forwarded = args[1:] if args[:1] == ["--"] else args
        if self.required and not forwarded:
            raise click.UsageError("missing arguments", ctx)
        ctx.params["args"] = tuple(forwarded)
        return []

    def collect_usage_pieces(self, ctx: click.Context) -> list[str]:
        return ["[--] ARGS..." if self.required else "[--] [ARGS]..."]


def _forwarding(
    name: str,
    body: Callable[[tuple[str, ...]], None],
    summary: str,
    required: bool = False,
) -> None:
    def command(args: tuple[str, ...]) -> None:
        _gate(lambda: body(args))

    cli.add_command(_Forwarding(name, required, callback=command, help=summary))


_forwarding(
    "test-local",
    gates.test_local,
    "Focused Rust proof with isolated PostgreSQL; ARGS are nextest selectors "
    "(none: the workspace).",
)
_forwarding("sandbox", workflows.sandbox, "Run the UX sandbox with ARGS, kept as argv.")
_forwarding(
    "census",
    workflows.census,
    "Informational source census: a fixed domain command, not a validation.",
)
_forwarding(
    "pg-run",
    workflows.pg_run,
    "Run a command against a throwaway PostgreSQL cluster "
    "(JAUNDER_PG_TEST_URL exported).",
)
_forwarding(
    "coverage-check",
    workflows.coverage_check,
    "Manual coverage evidence tools: probe-source | wasm-probe | wasm-measure.",
)
for _binary, _summary in {
    "wasm-audit": "Bundle size diagnostics: report [--site-path P] "
    "[--breakdown [--wasm F]] | budget.",
    "source-diagnostics": "Eval-only Nix guard: cache-safety-probe.",
    "trace-analysis": "Manual OTel trace diagnostics: analyze <files> | "
    "run [--single-worker] | boot-phases <files>.",
    "e2e-local": "Focused browser proof: [--browser B] [spec-or-file:line] or "
    "--update-visual-snapshots.",
}.items():
    _forwarding(
        _binary, functools.partial(workflows.host_proof_report, _binary), _summary
    )
_forwarding(
    "pr-watch",
    workflows.pr_watch,
    "Read-only caller-actionable PR observation; only this tool polls GitHub.",
)
_forwarding(
    "org-timezone-manifest",
    workflows.org_timezone_manifest,
    "Timezone manifest of explicit local Org originals; no checkout admission.",
    required=True,
)


@cli.command("production-baseline-discover")
@click.argument("revision")
def production_baseline_discover(revision: str) -> None:
    """Qualify REVISION against this clean, pushed HEAD as the harness."""
    _gate(lambda: workflows.production_baseline(revision, ""))


@cli.command("production-baseline-accept")
@click.argument("source")
@click.argument("target")
def production_baseline_accept(source: str, target: str) -> None:
    """Accept the change from SOURCE to TARGET with this clean, pushed HEAD."""
    # Only discovery may leave the target empty.
    if not target:
        raise click.ClickException(
            "production-baseline accept requires a nonempty target"
        )
    _gate(lambda: workflows.production_baseline(source, target))


@cli.command("production-baseline-qualify-styling")
def production_baseline_qualify_styling() -> None:
    """Styling qualification with this clean HEAD as the harness."""
    _gate(workflows.production_baseline_qualify_styling)


@cli.command("production-baseline-smoke")
def production_baseline_smoke() -> None:
    """Mutable local lifecycle diagnostic; deliberately no qualification."""
    _gate(workflows.production_baseline_smoke)


@cli.command("build-csr")
@click.argument("profile", type=click.Choice(["debug", "release"]), default="debug")
def build_csr(profile: str) -> None:
    """Build the checkout's CSR bundle into target/site."""
    _gate(lambda: workflows.build_csr(profile))


@cli.command()
@click.argument("profile", default="medium")
@click.argument("backends", default="sqlite,postgres")
@click.argument("browsers", default="")
@click.argument("workload", default="all")
@click.argument("posts", default="")
@click.argument("authors", default="")
@click.argument("revisions", default="")
def perf(
    profile: str,
    backends: str,
    browsers: str,
    workload: str,
    posts: str,
    authors: str,
    revisions: str,
) -> None:
    """Measure the selected performance producers, keeping each one's evidence."""
    _gate(
        lambda: workflows.perf(
            profile, backends, browsers, workload, posts, authors, revisions
        )
    )


@cli.command("perf-compare")
@click.argument("result")
def perf_compare(result: str) -> None:
    """Compare RESULT with the committed baseline; advisory only."""
    _gate(lambda: workflows.perf_compare(result))


@cli.command("perf-import-baseline")
@click.argument("run_id")
def perf_import_baseline(run_id: str) -> None:
    """Replace the baseline from an authenticated CI run RUN_ID."""
    _gate(lambda: workflows.perf_import_baseline(run_id))


@cli.command()
@click.argument("jobs", default="")
def e2e(jobs: str) -> None:
    """Every E2E producer, at most JOBS at once, then both reconciliations."""
    _gate(lambda: gates.e2e(jobs))


@cli.command("e2e-producer")
@click.argument("lane")
def e2e_producer(lane: str) -> None:
    """One isolated E2E producer LANE, as CI runs it."""
    _gate(lambda: workflows.e2e_producer(lane))


@cli.command("e2e-regenerate-coverage")
def e2e_regenerate_coverage() -> None:
    """Regenerate the server-function coverage snapshot from its producer."""
    _gate(workflows.e2e_regenerate_coverage)


@cli.command("e2e-reconcile")
@click.argument("backend")
@click.argument("artifacts", default=".xtask/diagnostics")
def e2e_reconcile(backend: str, artifacts: str) -> None:
    """Reconcile BACKEND's producer ARTIFACTS without building any producer."""
    _gate(lambda: workflows.e2e_reconcile(backend, artifacts))


@cli.command("issue-discover")
@click.argument("milestone")
def issue_discover(milestone: str) -> None:
    """Exhaustive read-only candidates in MILESTONE; never claims work."""
    _gate(lambda: workflows.issue_discover(milestone))


ISSUE_DECISIONS = [
    "title",
    "issue_type",
    "milestone",
    "priority",
    "labels",
    "body_file",
]


def _issue_decisions(command: Callable[..., None]) -> Callable[..., None]:
    """Both issue front doors take every decision explicitly, then any blockers."""
    for name in reversed([*ISSUE_DECISIONS, "blockers"]):
        command = click.argument(name, nargs=-1 if name == "blockers" else 1)(command)
    return command


@cli.command("issue-create")
@_issue_decisions
def issue_create(blockers: tuple[str, ...], **decisions: str) -> None:
    """Create an issue from explicit decisions; gh mutates, readback verifies."""
    _gate(
        lambda: issues.create([decisions[name] for name in ISSUE_DECISIONS], blockers)
    )


@cli.command("issue-edit")
@click.argument("number")
@_issue_decisions
def issue_edit(number: str, blockers: tuple[str, ...], **decisions: str) -> None:
    """Replace issue NUMBER's metadata with explicit decisions; readback verifies."""
    _gate(
        lambda: issues.edit(
            number, [decisions[name] for name in ISSUE_DECISIONS], blockers
        )
    )


@cli.command("pr-land", context_settings={"ignore_unknown_options": True})
@click.argument("number")
@click.argument("expected_head")
@click.argument("mode", default="ordinary")
def pr_land(number: str, expected_head: str, mode: str) -> None:
    """Land an approved PR head; MODE --reenqueue needs one-shot authorization.

    Prior recorded exact PR/head human approval is mandatory, not inferred here.
    """
    _gate(lambda: workflows.pr_land(number, expected_head, mode))


@cli.command("pr-cleanup")
@click.argument("number")
@click.argument("expected_head")
@click.argument("ownership")
def pr_cleanup(number: str, expected_head: str, ownership: str) -> None:
    """Clean up after a merged PR; the caller must verify unmanaged OWNERSHIP."""
    _gate(lambda: workflows.pr_cleanup(number, expected_head, ownership))


@cli.group("static")
def static_group() -> None:
    """Pinned static checks: one named check, or a fixed group of them.

    Checks without an autofix ignore --fix.
    """


def _static_command(name: str, summary: str) -> None:
    @static_group.command(name, help=summary)
    @click.option("--fix", is_flag=True, help="Fix instead of verifying.")
    @click.option(
        "--sandbox",
        is_flag=True,
        help="Run in place, offline against the provided Cargo homes, as a Nix "
        "check does.",
    )
    def command(fix: bool, sandbox: bool) -> None:
        if sandbox and static.missing_home(name):
            raise click.ClickException("sandbox Cargo home is missing")
        _gate(lambda: static.run(name, fix, sandbox), sandbox)


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
        path = summary.latest(checkout_root() / ".xtask" / "run")
    # Any record content loads; a run that cannot be read is reported, never
    # a traceback.
    try:
        table = summary.rows(path)
    except OSError as error:
        raise click.ClickException(f"cannot summarise {path}: {error}") from error
    click.echo(summary.render(table, markdown), nl=False)
