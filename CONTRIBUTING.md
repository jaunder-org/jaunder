# Contributing to jaunder

This is the definitive working guide for both human contributors and coding
agents. Agent-specific entry points such as `AGENTS.md` should point here
instead of duplicating project policy.

## Project guides

- `docs/DESIGN.md`: project goals
- `docs/ARCHITECTURE.md`: project architecture
- `docs/ROADMAP.md`: project roadmap

## Repository layout

- `flake.nix`: development environment, comprehensive test environment, and
  PostgreSQL testing
- `common/`: code shared between other packages
- `end2end/`: Playwright end-to-end tests
- `elisp/`: Emacs blogging client (`jaunder.el`), ERT-tested
- `hydrate/`: frontend driver
- `storage/`: storage traits, records, migrations, and backend-specific storage
  support
- `server/`: backend, CLI, server runner, and integration tests
- `web/`: Leptos server functions in `web/src/*.rs` and page components in
  `web/src/pages/`

## Development setup

### Prerequisites

This project uses [Nix](https://nixos.org/) to manage the development
environment. All required tools are provided by the Nix flake.

Enter the development shell:

```
nix develop
```

The default local backend remains SQLite. PostgreSQL development is also
supported:

```bash
nix run .#postgres-testing-vm
export JAUNDER_DB=postgres://jaunder@127.0.0.1:55432/jaunder
```

With that environment set, `jaunder init`, `jaunder serve`, and targeted storage
tests will use the PostgreSQL test VM instead of SQLite.

If a PostgreSQL URL omits the password, `jaunder` also supports:

```bash
export JAUNDER_DB_PASSWORD=secret
# or
export JAUNDER_DB_PASSWORD_FILE=/run/secrets/jaunder-db-password
```

Those inputs are for steady-state connections. PostgreSQL database and role
provisioning is handled separately by `jaunder create-pg-db`, which is intended
for one-time administrative bootstrap before `jaunder init` runs migrations.
Because `create-pg-db` is meant to be run manually by an experienced
administrator, it takes its inputs explicitly via command-line flags rather than
via `JAUNDER_*` environment variables. The command is intentionally simple:
bootstrap URL, application database URL, and a separate application-role
password. It fails if the requested role or database already exists. Use
`--bootstrap-db` for the elevated connection and `--app-db` for the long-term
application connection details.

### Git hooks

The repository includes git hooks in `.githooks/` that enforce code quality
standards. Configure git to use them after cloning:

```
git config core.hooksPath .githooks
```

**`pre-commit`** runs the full **`cargo xtask check`** (Fix mode) on every
commit — formatting + clippy + the Nix `coverage` check (the SQLite +
ephemeral-PostgreSQL suites under instrumentation) — so history stays green
commit-by-commit. `check` auto-fixes formatting and auto-heals the coverage
baseline / CRAP manifest; those heals are idempotent on a pure line-shift (the
baseline compares line-independently, the CRAP manifest ignores line
attribution), so it only changes the tree on a _real_ fix. When it does, the
hook **fails and asks you to `git add` and re-commit** — so you consciously
include the change rather than the hook silently folding it in. Bypass with
`SKIP_PRE_COMMIT=1 git commit` for WIP.

**`pre-push`** runs `cargo xtask validate --no-e2e` (verify-only): the static
checks plus the Nix `coverage` check, gating test failures and coverage
regressions, and it refuses a dirty tree. The e2e VM checks are not run here —
they run in CI, or locally via `cargo xtask validate`. Bypass with
`SKIP_PRE_PUSH=1 git push` for WIP.

## Development workflow

- Track development work with `beads` (`bd`) rather than ad-hoc markdown TODO
  lists. Use beads for durable memory items when possible.
- Prefer focused, atomic changes. The system should remain in a working state at
  each commit.
- Write and commit preparatory refactors before the behavior changes that use
  them.
- Every module should have comprehensive internal documentation.
- Avoid `cat >>` or other append-only shell edits when modifying files; use
  structured editing tools.
- Address all `clippy` lints **by fixing the code**, not by silencing them.
  Adding a lint suppression — `#[allow(...)]` or `#[expect(...)]`, whether a
  `clippy::` lint or a rustc lint — requires **explicit user approval** before
  it lands; do not introduce one to make the gate pass on your own initiative
  (this is the actionable form of the "never suppress … linting without explicit
  approval" rule under Testing).
- Unless explicitly instructed otherwise, request review before committing.

### Adding an ADR

Start from [`docs/adr/template.md`](docs/adr/template.md): copy it to
`docs/adr/drafts/<slug>.md` — a numberless **draft**, never a hand-picked
number. The `docs/adr/drafts/` directory is gitignored (except its `README.md`),
so a draft lives out of git until it ships and cannot be committed with a
premature number; keep the draft heading `# ADR-DRAFT: <Title>` and cite the
draft by its `docs/adr/drafts/<slug>.md` path. See
[`docs/adr/drafts/README.md`](docs/adr/drafts/README.md) for the full flow.

At ship, after the final rebase onto `main`, run `cargo xtask adr promote` (via
`devtool run -- …`): for each draft it assigns the next free number, moves it to
`docs/adr/NNNN-<slug>.md`, rewrites its path-form references, syncs the README
table, and stages the result — so the number is assigned as late as possible and
the ADR's first appearance in git history is already collision-free. If a
collision still surfaces between that commit and the merge, re-rebase, re-run,
and **amend the commit that introduced the ADR** — never a fixup commit.

ADRs are `docs/adr/NNNN-slug.md` with a canonical `# ADR-NNNN: <title>` heading
and a single-token `- Status: <token>` line (one of
proposed/accepted/superseded/deprecated/rejected). They are indexed in the table
in `docs/README.md`, whose number, link, and status cells are **generated** from
`docs/adr/`: the `adr-format` and `adr-readme-parity` steps of
`cargo xtask check`/`validate` fail on a non-canonical heading/status or a table
that has drifted from the directory (recovery: `cargo xtask adr sync-readme`,
which is also folded into `adr promote`/`renumber`). Table titles are
hand-curated — a new row is seeded from the ADR heading, then owned by you
(ADR-0036 §#196 addendum). A numberless draft under `docs/adr/drafts/` is
invisible to all three ADR gates by construction: their shared `is_file` → `.md`
→ leading-number enumeration is non-recursive over `docs/adr/`.

The `identifier-collisions` step fails if two _committed_ ADRs — or two
migrations, per backend — share a number, or if the sqlite/postgres migration
sets diverge. Because two differently-named files (`0099-foo.md` /
`0099-bar.md`) merge with no git conflict, this check is what makes a
concurrent-branch collision **loud** instead of silent (ADR-0036). An
already-committed ADR that collides after a rebase is bumped with
`cargo xtask adr renumber` (it `git mv`s it to the next free number, rewrites
references, and re-syncs the table — then amend the introducing commit; the ADR
already on `origin/main` keeps its number). Migrations use the same sequential
convention and detection check but are renumbered by hand on the rare occasion
it is needed.

## Testing

There are several testing layers in this repository. Use the smallest one that
matches the change first, then move up to the broader checks before pushing.
Never remove functionality to pass tests, and never bypass or suppress testing,
coverage, or linting without explicit approval.

Every HTTP endpoint must have both an integration test and an end-to-end test.
Unit tests belong in the same file as the code being tested. End-to-end tests
belong in `end2end/` and use Playwright.

For tests requiring a database, use `sqlite::memory:` and run migrations with
`sqlx::migrate!("./migrations").run(&pool).await?` before creating the
`AppState`.

**Tests that spawn `git` must scrub the repo-redirecting `GIT_*` env.** When git
runs a hook it exports `GIT_DIR`/`GIT_INDEX_FILE` (and
`GIT_WORK_TREE`/`GIT_OBJECT_DIRECTORY`/`GIT_COMMON_DIR`/`GIT_NAMESPACE`), and
those **override `-C <dir>`**. So a test that builds a throwaway repo with
`git -C <tmpdir> …` will, when run inside the pre-commit/pre-push hooks (which
invoke `cargo xtask check`/`validate`, whose host tests then run that code), be
redirected at the **real** repository — corrupting HEAD, the index, and the
shared worktree config. Build every git command for such a test (and any
production helper it calls) through a constructor that `env_remove`s those vars
— see `git::at()` in `xtask/src/git.rs`. Read-only production queries
(`rev-parse`/`log`/`diff`/`ls-files`) are safe unscrubbed since they don't
mutate.

### Local checks: `cargo xtask`

The driver for all checks is `cargo xtask`. The host runs only the static
checks + clippy; **all tests, coverage, and e2e run in the Nix checks that match
CI**. When `cargo xtask validate` is green, you may push.

| Command                         | Runs                                                                                                                                            | Formatting    |
| ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | ------------- |
| `cargo xtask check --no-test`   | host static checks + clippy                                                                                                                     | auto-fixes    |
| `cargo xtask check`             | + the Nix `coverage` check (full instrumented test suite — SQLite + PostgreSQL together under an ephemeral PostgreSQL — plus the coverage gate) | auto-fixes    |
| `cargo xtask validate --no-e2e` | static (verify-only) + coverage — the pre-push gate (the `.githooks/pre-push` hook runs this)                                                   | never mutates |
| `cargo xtask validate`          | + e2e (all four `{sqlite,postgres}×{chromium,firefox}` combos) — the full local gate                                                            | never mutates |

`check` is the inner-loop fixer: it auto-fixes formatting and (in Fix mode)
auto-heals the coverage baseline when a change only removes or covers gaps.
`validate` is the strict, never-mutating gate. Both commands write a
machine-readable result to `.xtask/last-result.json` and a `xtask-done:`
completion line to stderr.

CI does **not** run `cargo xtask validate` as a single job. It runs
`cargo xtask validate --no-e2e` (static + clippy + coverage) in one job, plus a
`{backend}×{browser}` e2e matrix where each job runs
`cargo xtask e2e <backend> <browser>` for one combo, aggregated by an `e2e-gate`
job. Running every combo in parallel across runners cuts e2e wall-clock;
`cargo xtask validate` remains the full local equivalent. See
[ADR-0034](docs/adr/0034-ci-e2e-matrix-distribution.md).

- `cargo fmt --check` checks Rust formatting.
- `leptosfmt -x .direnv -x .git -x target --check '**/*.rs'` checks files that
  contain Leptos `view!` macros.
- `prettier --check end2end '**/*.md'` checks Playwright/frontend test assets
  and all tracked Markdown (`proseWrap: always`; scoped by `.prettierignore`,
  which excludes `docs/archive/`).
- `elisp-fmt` and `ert` run the elisp subproject's formatter and ERT suite under
  `emacs --batch` (see the Elisp subproject section below).
- `cargo clippy --all-targets -- -D warnings` checks the whole workspace,
  including test, bench, and example targets, for lint errors.
- `cargo nextest run` runs the default Rust unit and integration test suite.
- For e2e perf diagnostics, set `JAUNDER_E2E_WARMUP=1` before `playwright test`
  to warm each test page context before test instrumentation; tune with
  `JAUNDER_E2E_WARMUP_URL` and `JAUNDER_E2E_WARMUP_TIMEOUT_MS`.
- In hydration-heavy e2e tests, use
  `hydrationHeavyTimeoutMs(testInfo, chromiumBudgetMs)` for whole-test budgets
  and `hydrationHeavyFirstNavigationTimeoutMs(testInfo, chromiumBudgetMs)` for
  first navigation waits (see
  [ADR-0012](docs/adr/0012-environment-aware-timeouts.md)).

### Elisp subproject (`elisp/`)

The Emacs client lives in `elisp/` (see [`elisp/README.md`](elisp/README.md)).
Its ERT suite and formatter run in the verify ladder: `ert` and `elisp-fmt` are
`cargo xtask check`/`validate` steps, mirrored by the `ert-check` /
`elisp-fmt-check` Nix checks. prettier cannot format Emacs Lisp, so `elisp-fmt`
uses built-in `emacs-lisp-mode` indentation (auto-fix under `check`, verify
under `validate`). elisp is interim-exempt from the Rust coverage gate
(cargo-llvm-cov is Rust-only; follow-on #82) — instead, write an ERT test for
every pure mapping/transform function. Rationale:
[ADR-0031](docs/adr/0031-elisp-separately-tested-subproject.md).

### Observability and Performance Analysis

Jaunder uses OpenTelemetry for deep performance analysis (see
[ADR-0011](docs/adr/0011-unified-observability.md)).

- **No PII in telemetry**: span fields and the structured error boundary
  (`error.source`/`error.context`) must never carry user PII or secrets (emails,
  tokens, passwords, post bodies); use stable identifiers like `user_id`,
  `db.system`, and `error.kind` instead. See
  [ADR-0011](docs/adr/0011-unified-observability.md).

- **Trace Analysis**: Use `cargo xtask traces analyze` to process trace
  artifacts (JSONL) from VM runs or local tests.

  ```bash
  cargo xtask traces analyze \
    /path/to/otel-traces-sqlite.jsonl/otel-traces.jsonl \
    /path/to/otel-traces-postgres.jsonl/otel-traces.jsonl
  ```

  The analyzer reports on:
  - Slowest spans overall and per `e2e.test`.
  - Top navigation and action hotspots.
  - `commit -> hydration` splits by `cacheWarmth`.
  - Hydration component hotspots (`wasm_init`, `leptos_hydrate`, etc.).

  **Optional Filters**:
  - `--top N`: Limit the number of rows in each section.
  - `--trace TRACE_ID`: Analyze a specific trace.
  - `--project NAME`: Filter by browser (e.g., `firefox`, `webkit`).

- **WASM Audit**: Use `cargo xtask audit-wasm` to measure the size of the
  frontend WASM and JS bundles from the deterministic Nix build.

  ```bash
  cargo xtask audit-wasm
  ```

  Useful options: `cargo xtask --json audit-wasm` for machine-readable output,
  or `--site-path` to reuse a build. Run `cargo xtask audit-wasm --help` for
  details.

- **Run & Analyze**: Use `cargo xtask traces run` to build the full VM e2e suite
  and immediately analyze the results.
  - Use `--cold` to run against cold caches instead of the default warmup
    checks.
  - Use `--browser chromium|firefox` to restrict the run to one browser
    (default: both).

### Targeted Rust tests

When a change is confined to one area, run the relevant target directly.

- CLI and command behavior: `cargo test -p jaunder --test commands`
- Storage behavior: `cargo test -p jaunder --test storage`
- Web/server-function behavior:
  - `cargo test -p jaunder --test web_auth`
  - `cargo test -p jaunder --test web_account`
  - `cargo test -p jaunder --test web_email`
  - `cargo test -p jaunder --test web_password_reset`
- Library-only tests: `cargo test -p jaunder --lib`

`cargo nextest list -p jaunder --tests` shows the currently registered Rust test
targets if you need to confirm the target split.

### PostgreSQL-backed Rust tests

The integration suite is backend-parametric via
[`rstest`](https://docs.rs/rstest): a storage behavior is written **once** and
annotated `#[apply(backends)]`, which expands it into two cases —
`::case_1_sqlite` and `::case_2_postgres`. A `Backend` enum selects the backend
through `Backend::setup()` (defined in `server/tests/helpers/mod.rs`); genuinely
single-backend tests use `#[apply(sqlite_only)]` or `#[apply(postgres_only)]`.
The HTTP-layer integration tests are backend-parametric too: they use the same
`Backend::setup()` fixture (`#[apply(backends)]`, or a
`#[values(Backend::Sqlite, Backend::Postgres)]` + `#[case]` matrix for clustered
rejection/authorization tests), so the **whole** integration suite — storage and
HTTP — runs on both backends per run; the old env-selected `test_state` harness
is gone. Both cases run in the same single nextest pass. The consequence is that
a bare `cargo nextest run` **requires a reachable PostgreSQL**: the postgres
cases connect to `JAUNDER_PG_TEST_URL` (defaulting to
`postgres://jaunder@127.0.0.1:55432/jaunder`) and fail if nothing is listening.
Each test creates its own database — a clone of a once-migrated template (see
`server/tests/helpers/mod.rs`) — so the cases run **in parallel**; no
`--test-threads=1` is needed. (The `#[template]`/`#[apply]` macros come from the
`rstest_reuse` dev-dependency, which requires the bare `use rstest_reuse;`
import at the top of any test file that uses them.)

The simplest way to run them against a throwaway PostgreSQL is `devtool pg run`,
which starts an ephemeral cluster, exports the connection env, runs the command,
and tears everything down:

```bash
cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run -- cargo nextest run -p jaunder
```

To use a persistent instance instead (e.g. the dev VM), set the env yourself:

```bash
nix run .#postgres-testing-vm
export JAUNDER_PG_TEST_URL=postgres://jaunder@127.0.0.1:55432/jaunder
export JAUNDER_PG_BOOTSTRAP_TEST_URL=postgres://postgres@127.0.0.1:55432/postgres
cargo nextest run -p jaunder
```

Per-test databases are not dropped after each run, so a persistent instance
accumulates `jaunder_test_*` databases over time; the ephemeral wrapper avoids
this by discarding the whole cluster.

### Coverage and dependency policy

- Coverage is gated **host-side by xtask**, which reads the Nix `coverage`
  check's report. It is part of `cargo xtask check` and `cargo xtask validate`
  (and therefore the pre-push gate and CI), so coverage regressions are caught
  before push rather than at merge.
- `cargo deny check` verifies dependency policy, advisories, and licensing (run
  as one of the static checks).

The gate does **line-identity** classification against the committed
`coverage-baseline.json`, which records the accepted-uncovered gaps. A
previously-covered line going uncovered, or a new uncovered line that is not in
the baseline, **fails** the gate — a strict ratchet. The CRAP baseline is
`crap-manifest.json`. Both are committed, ordinary (non-dotted) files.

`cargo xtask check` runs in Fix mode: when a change only removes gaps or covers
previously-uncovered lines, it auto-heals the baseline. `cargo xtask validate`
is Check-only and never mutates the baseline. A line that is genuinely
uncoverable can be excluded from the ratchet with a `// cov:ignore` comment.

The heal is keyed on **uncovered-text identity**, not line number: a
line-shifting change that moves an accepted-uncovered gap — the diff removes it
at the old line and it reappears at a new line with identical source text — is
recognised as a safe **re-anchor** (`cargo xtask check` re-anchors the baseline
and passes; `cargo xtask validate` passes without mutating). Only a _new_
uncovered text with no removed-gap counterpart (a genuine lowering) fails the
gate. This is what lets benign line-shifts — including those introduced by
concurrently-merged branches — self-heal instead of forcing manual regeneration
(ADR-0030). Residual ambiguity: two identical-text lines in one file, where one
is removed as an accepted gap while an unrelated identical-text line regresses
in the same change, can be conflated as a safe move — bounded, and the
line-identity classifier remains the primary signal.

When a coverage gate fails on a genuine lowering, run
`cargo xtask coverage reanchor` (it consumes the report the gate just built
under `.xtask/gcroots/coverage`). It re-anchors a safe line-shift in place, or —
if the lowering is real — refuses, writes the would-be baseline to
`.xtask/coverage-baseline.candidate.json`, and prints the offending
`file:line: text`. Accepting a genuinely-approved lowering is then a deliberate,
reviewable step: inspect with
`git diff --no-index coverage-baseline.json .xtask/coverage-baseline.candidate.json`
and, if approved per the coverage-baseline policy, `cp` the candidate over the
committed baseline and commit it. There is no flag that lowers the baseline
automatically.

`crap-manifest.json` retains a per-function `line` field as a
**non-authoritative jump-to hint**: it can lag the true line until the next
change that actually moves a CRAP score (which refreshes every entry's line
wholesale). The CRAP regression check and the manifest's rewrite trigger both
ignore `line`, so a pure line-shift neither hides a regression nor churns the
committed manifest.

When a coverage gate fails on a **CRAP regression** (a function whose
complexity-risk score worsened), run `cargo xtask coverage refresh-crap` (it
consumes the same report under `.xtask/gcroots/coverage`). With no regression it
refreshes `crap-manifest.json` in place — a no-op when nothing CRAP-relevant
changed. With a regression it refuses, writes the would-be manifest to
`.xtask/crap-manifest.candidate.json`, and prints each offending
`file::fn old → new`. Accepting genuinely-stale drift is then a deliberate,
reviewable step: inspect with
`git diff --no-index crap-manifest.json .xtask/crap-manifest.candidate.json`
and, if approved, `cp` the candidate over the committed manifest and commit it.
As with the baseline, there is no flag that accepts a regression automatically —
the symmetric recovery to `cargo xtask coverage reanchor` (#131).

Both committed artifacts use a **keep-ours git merge driver** (`.gitattributes`)
so overlapping branches do not produce conflict markers; the Fix-mode heal
restores authoritative content on the next `cargo xtask check`. The driver
auto-registers on any `cargo xtask` run (self-healing, like `core.hooksPath`),
so a fresh clone wires itself up on first gate run.

**Working-tree contract.** The gate reflects your _working tree_, not just
committed state: the Nix coverage build instruments dirty tracked content
**and** untracked, non-gitignored files. So you do **not** need to commit or
stage first — line-shifting edits to tracked files are mapped from the baseline
anchor to the working tree, and a new untracked `.rs` file is measured, with its
uncovered lines reported as new uncovered code rather than as a regression. (The
source filter that pulls untracked files into the build is a known purity rough
edge, tracked in issue #37.)

Coverage counts source lines with at least one execution hit across all test
binaries, deduplicating generic-function instantiations across compile units
(unlike the inflated `cargo llvm-cov --json` summary percentages for code
exercised by multiple test binaries). It runs the entire test suite in a single
nextest pass with an ephemeral PostgreSQL available for the whole run, so the
SQLite-backed and PostgreSQL-backed integration tests execute together (no
`#[ignore]`, no `--run-ignored`) and `storage/src/postgres/*` gets real
instrumented coverage. Reported line coverage is the union of both backends.
This — and thus backend parity — runs inside the Nix `coverage` check.

Never lower the baseline without user approval; approved baseline changes must
be committed in the same commit as the file whose coverage changed. Coverage
improvements are always allowed.

Some areas have inherent host-side coverage gaps and should not be force-fitted
with artificial tests:

- **WASM entry point** (`hydrate/src/lib.rs`, 0%): runs only in the browser WASM
  context.
- **Leptos page components** (`web/src/pages/*.rs`, varied): `#[component]`
  functions render view trees; correctness is validated by e2e tests in the Nix
  VM.
- **A few PostgreSQL storage error branches**
  (`storage/src/postgres/feed_events.rs` ~91%,
  `backup.rs`/`bootstrap.rs`/`mod.rs` ~95–99%): the host-PostgreSQL coverage
  pass now drives the common paths, so most of `storage/src/postgres/*`
  (`media.rs`, `sessions.rs`, `posts.rs`) is ~100%; only some error branches
  remain uncovered and should not be force-fitted with artificial tests.
- **Asset serving** (`server/src/assets.rs`, 0%): compile-time embedded assets
  are not practical to unit test.

### Nix VM checks

`nix flake check` runs the full Nix-backed validation matrix, including:

- `checks.x86_64-linux.nextest` — Rust nextest suite
- `checks.x86_64-linux.clippy` — clippy
- `checks.x86_64-linux.rustfmt` — rustfmt
- `checks.x86_64-linux.leptosfmt-check` — leptosfmt
- `checks.x86_64-linux.prettier-check` — prettier for `end2end/`
- `checks.x86_64-linux.ert-check` — ERT suite for `elisp/`
- `checks.x86_64-linux.elisp-fmt-check` — emacs-lisp indentation check for
  `elisp/`
- `checks.x86_64-linux.deny` — cargo-deny
- `checks.x86_64-linux.e2e-sqlite-chromium` — Playwright end-to-end flow against
  SQLite on Chromium with `JAUNDER_E2E_WARMUP=1` (default)
- `checks.x86_64-linux.e2e-sqlite-firefox` — Playwright end-to-end flow against
  SQLite on Firefox with `JAUNDER_E2E_WARMUP=1` (default)
- `checks.x86_64-linux.e2e-postgres-chromium` — Playwright end-to-end flow
  against PostgreSQL on Chromium with `JAUNDER_E2E_WARMUP=1` (default)
- `checks.x86_64-linux.e2e-postgres-firefox` — Playwright end-to-end flow
  against PostgreSQL on Firefox with `JAUNDER_E2E_WARMUP=1` (default)
- `checks.x86_64-linux.postgres-integration` — every `server/tests/*.rs`
  integration binary against PostgreSQL (including the ignored PostgreSQL-only
  cases), all in one VM

Additional Nix-backed checks available as packages (not run by default):

- `packages.x86_64-linux.e2e-sqlite-chromium-cold` — Playwright end-to-end flow
  against SQLite on Chromium without warmup
- `packages.x86_64-linux.e2e-sqlite-firefox-cold` — Playwright end-to-end flow
  against SQLite on Firefox without warmup
- `packages.x86_64-linux.e2e-postgres-chromium-cold` — Playwright end-to-end
  flow against PostgreSQL on Chromium without warmup
- `packages.x86_64-linux.e2e-postgres-firefox-cold` — Playwright end-to-end flow
  against PostgreSQL on Firefox without warmup

All PostgreSQL integration binaries run in a **single** VM
(`postgres-integration`): per-test databases isolate the tests, so they run with
libtest's normal in-process parallelism rather than one VM per binary. This is
much faster and far lighter on memory than the former per-binary matrix.

If you only need one of the VM-backed checks, you can run it directly:

```bash
nix build .#checks.x86_64-linux.e2e-sqlite-chromium
nix build .#checks.x86_64-linux.e2e-postgres-firefox
nix build .#packages.x86_64-linux.e2e-sqlite-firefox-cold
nix build .#packages.x86_64-linux.e2e-postgres-firefox-cold
nix build .#checks.x86_64-linux.postgres-integration
```

## Code conventions

- Use Rust, except end-to-end tests, which use Playwright and TypeScript.
- All Rust code is formatted with `cargo fmt`.
- Files containing Leptos `view!` macros are additionally formatted with
  `leptosfmt` (run it first, then `cargo fmt`).
- Follow Conventional Commits: `<type>: <imperative summary, ≤72 chars>`, where
  `<type>` is one of `feat`, `fix`, `refactor`, `perf`, `docs`, `test`, `build`,
  `ci`, `chore` (optional scope, e.g. `fix(storage): …`). Reference the beads
  issue the commit addresses with a `Refs: <bead-id>` trailer (the bead carries
  milestone/epic context, so the subject needs no `M`/`§` prefix), and keep the
  `Co-Authored-By` trailer required of agent commits.
- Every commit that changes behavior must include appropriate tests, unless the
  user explicitly waives this requirement. (Docs/build/chore commits with no
  behavior change are out of scope, not waivers.)
- Never use `.unwrap()` or `.expect()` in production code; both are permitted in
  tests. This is enforced by clippy (`unwrap_used`/`expect_used` denied outside
  tests).
- Use Rust's type system to make invalid states impossible with infallible
  types.
- At boundaries (`#[server]` functions, DB calls), parse data into infallible
  types, reject invalid data, and handle `Result`/`Option` conversion
  explicitly.
- Keep data transformations pure where possible so they are easy to test and
  reason about.
- Comment for intent, not mechanics. A comment should state **what the code
  intends to achieve** (so a reviewer can judge whether it is fit for purpose)
  and, where the code takes a path that is not, at first glance, the obvious
  one, **why it is done that way**. A why-comment's job is to return
  surprising-looking code to a state of being obviously correct — the reader
  stops, understands why the non-obvious path was necessary, and sees that it is
  right. Do not narrate what the code mechanically does — that is readable from
  the code itself. For example:

  ```rust
  // ❌ mechanical — restates the code
  // Loop over the sessions and remove the expired ones.

  // ✅ intent (+ why the non-obvious choice)
  // Reap expired sessions on read rather than via a background sweep:
  // logins are rare enough that a periodic job isn't worth the moving part,
  // and reaping here keeps the auth check the single source of truth.
  ```

## Storage and web conventions

- Use `sqlx` for database access.
- Support SQLite and PostgreSQL, dynamically selected at runtime.
- Store SQL migrations in `storage/migrations` using the `000x_description.sql`
  numbering convention.
- Define storage traits such as `UserStorage` and `SessionStorage`, plus their
  record types, in `storage/src/*.rs`. This lets `web` and `server` use them
  without circular dependencies.
- Keep concrete SQLite/PostgreSQL implementations in the `server` crate, for
  example `server/src/storage/sqlite.rs`, and re-export them from
  `server/src/storage/mod.rs` for the CLI and server runner.
- Use specialized storage error enums in `common::storage`, such as
  `UserAuthError` and `CreateUserError`, with `thiserror`.
- Use `sqlx` unique violation checks (`is_unique_violation()`) to handle
  "already exists" errors gracefully.
- Use the `AppState` struct from `common::storage` to bundle storage handles. In
  web server functions, retrieve it with `expect_context::<Arc<AppState>>()`.
- **Dependency injection / composition-root invariant (see
  [ADR-0016](docs/adr/0016-dependency-injection-and-appstate.md)):** No type may
  be both (a) a heterogeneous dependency holder and (b) passed beyond the
  composition root. Declare a component's dependencies as constructor parameters
  on the component that uses them — do not add a field to a shared bundle to
  make a dependency reachable. A storage `Backend` factory may mint storage
  handles, but only the composition root may hold it; it is never injected into
  a subsystem (that would be a service locator). Services (mailer, WebSub
  client, background workers) are constructed at the root and injected
  per-consumer; there is no "services bundle."
- The web framework is Leptos with SSR via `cargo-leptos`.
- Leptos components should only render data; business logic belongs in server
  functions or pure transformation functions.
- API methods are automatically prefixed with `/api`.
- Define `#[server]` functions in the relevant `web/src/*.rs` module, such as
  `web/src/auth.rs`. Use `#[cfg(feature = "ssr")]` for server-only imports and
  logic in those files.
- Convert storage errors to `leptos::prelude::ServerFnError` using
  `.map_err(|e| ServerFnError::new(e.to_string()))`.
- Use `require_auth().await?` at the start of any server function that requires
  a logged-in user. It returns an `AuthUser` containing `user_id`, `username`,
  and `token_hash`.
- Use `set_session_cookie(raw_token)` and `clear_session_cookie()` from
  `web/src/auth.rs` to manage the `session` cookie in server functions.
- Enforce lowercase usernames at the boundary before passing them to storage
  methods.
- Production deployment is expected to run behind a reverse proxy providing
  HTTPS.

## Backend parity rules

- Any change that adds persisted state must include both a SQLite migration and
  a PostgreSQL migration in the same change.
- Any change that alters a storage trait or persisted behavior must implement
  the change on both backends before merge.
- New storage-backed tests must either cover both backends or state explicitly
  why one backend is intentionally deferred.
- Backend-specific optimizations are allowed, but user-visible behavior
  differences must be documented explicitly up front.

### The `test-backend-pattern` guard (enforced across `server/tests`)

The `cargo xtask check`/`validate` static pass runs a `test-backend-pattern`
guard that scans every file under `server/tests/` and fails if a
`#[tokio::test]` (including parameterized `#[tokio::test(flavor = …)]` forms) is
not declared backend-explicit. Every DB-touching integration test must carry
exactly one of:

- `#[apply(backends)]` — dual-backend, single-axis (the test takes
  `#[case] backend: Backend`).
- `#[apply(backends_matrix)]` — dual-backend for a test that ALSO has its own
  local `#[case]` matrix (the `#[values]`-based template; the test takes a plain
  `backend: Backend`). Use this when `#[apply(backends)]` would collide with
  local `#[case]` rows.
- `#[apply(sqlite_only)]` / `#[apply(postgres_only)]` — a deliberately
  single-backend test. It MUST carry a `// reason:` comment stating the
  backend-specific behavior the other backend can't exhibit (e.g. a SQLite
  lock-flake reproduction, a Postgres-only `pg_database` teardown). "It
  currently hardcodes SQLite" is NOT a valid reason — convert such a test
  instead.

A genuinely **non-DB** integration test (exercises real router/middleware wiring
but touches no database) is exempt via a `// guard:no-backend — <reason>`
comment immediately above its `#[tokio::test]`. A test that is really a **unit
test** (a pure function/extractor, no router or DB) belongs in a
`#[cfg(test)] mod tests` in the owning crate, not in `server/tests`. Pure
synchronous `#[test]` unit tests are never flagged.

## NixOS integration

- The shared NixOS module is `nixosModules.jaunder`.
- Production imports should enable the service with
  `services.jaunder.enable = true;` and set `services.jaunder.bind` as needed.
- Set `services.jaunder.db` to choose the backend for a NixOS deployment. The
  default remains `sqlite:./data/jaunder.db`.
- Do not set `JAUNDER_MAIL_CAPTURE_FILE` in production. That is test-only and
  should stay in the interactive VM or e2e test node config.
- The `jaunder` CLI is installed for the `jaunder` user via
  `users.users.jaunder.packages`.

## Interactive testing VM

- Start it with `nix run .#interactive-testing-vm`.
- It auto-logs in as `jaunder` on the console.
- The VM user password is `jaunder`.
- `sudo` is passwordless for `wheel` in the VM only.
- The VM does not use SSH; it is intended for local console interaction and app
  testing.

## PostgreSQL test VM

- Start it with `nix run .#postgres-testing-vm`.
- The forwarded connection string is
  `postgres://jaunder@127.0.0.1:55432/jaunder`.
- Point the app at it with
  `export JAUNDER_DB=postgres://jaunder@127.0.0.1:55432/jaunder`.
- The PostgreSQL-backed tests each create their own database (template clones),
  so they run in parallel without interference; see "PostgreSQL-backed Rust
  tests" above. For a self-contained run, prefer `devtool pg run`.
