# Contributing to jaunder

This is the definitive working guide for both human contributors and coding
agents. Agent-specific entry points such as `AGENTS.md` should point here
instead of duplicating project policy.

## Project guides

- `docs/DESIGN.md`: project goals
- `docs/ARCHITECTURE.md`: the authoritative statement of the current
  architecture — the materialized view of the ADR log, covering the workspace
  layout and every subsystem
- `docs/codebase-audits.md`: the durable method for finding and remediating
  accumulated design and maintenance cruft
- `docs/ROADMAP.md`: project roadmap

## Repository layout

See [Workspace](docs/ARCHITECTURE.md#workspace) in the architecture view for the
crate map, each crate's responsibility, and the sibling trees outside the root
workspace (`tools/`, `elisp/`, `end2end/`). The remaining top-level entries are:

- `flake.nix`: assembly of the public Nix flake outputs
- `nix/`: concern-owned packages, checks, development shells, NixOS modules, and
  testing VMs
- `docs/`: maintained guides, ADRs (`docs/adr/`), and in-flight planning docs
- `.githooks/`: the `pre-commit`/`pre-push` hooks (see Git hooks below)
- `.github/`: CI workflows
- `public/`: static assets (the favicon), staged into the embedded asset set by
  `server/build.rs`
- `scripts/`: small helper scripts; `pg-run.py` is packaged by Nix onto the
  devShell PATH and into the coverage derivation

### `mod.rs` assembles the module surface

A `mod.rs` states what a module contains and exports, and holds nothing else. It
may contain only:

- `mod` / `pub mod` declarations,
- `use` / `pub use` / `pub(crate) use` re-exports,
- `//!` module documentation,
- attributes — inner ones, and outer attributes on those `mod` and `use` items.

Anything else — `fn`, `struct`, `enum`, `trait`, `impl`, `const`, `static`,
`type`, `macro_rules!`, or an inline `#[cfg(test)] mod tests { … }` body — lives
in a sibling file that `mod.rs` declares and re-exports. Prefer an explicit
re-export list (`pub use thing::{A, B};`) over a glob: a glob states nothing,
and stating the surface is the point.

The rule is workspace-wide — production crates, test trees, and `tools/` alike.
It is **enforced at review, not by a gate**: whether a given item earns its own
file is a judgement a syntactic check would get wrong in both directions. See
[`docs/adr/0128-mod-rs-assembles-module-surface.md`](docs/adr/0128-mod-rs-assembles-module-surface.md).

## Development setup

### Prerequisites

This project uses [Nix](https://nixos.org/) to manage the development
environment. All required tools are provided by the Nix flake.

Enter the development shell:

```
nix develop
```

The flake has five shells. `default` is the complete interactive shell. `ci` is
`default` without the interactive-only tools; the CI host validation lane and
the performance job enter it. `mutants` is `ci` plus `cargo-mutants`, for the
scheduled mutation job. `ci-nix` is Linux-only and holds only `nix`, `git`,
`dev`, `traced-run`, `jq`, basic shell utilities and the OpenSSL/dav1d libraries
for `LD_LIBRARY_PATH`, with no Rust toolchain and no C compiler. The CI jobs
that only run Nix builds and Nix-built tools enter `ci-nix`: the hermetic,
test-checks and coverage validation lanes and the e2e catalog, lane and
reconcile jobs. `theme-thumbnail` is described in [themes](docs/themes.md).

Enter the default, CI, mutants or `ci-nix` shell from the intended Git checkout.
At entry, their shared startup hook creates and verifies the absolute
`<checkout>/.xtask/tmp` directory and exports it as `TMPDIR`, deliberately
replacing any inherited value before provisioning or user commands run. Failure
to resolve the checkout or create/use that directory visibly stops shell
startup; there is no `/tmp` fallback. The value stays bound to that entry
checkout after `cd`; enter a new shell to bind to another checkout. A later
command-local `TMPDIR=/absolute/override command` still wins for that command
and its inheriting children.

This routes host-tool temporary files, not their ownership or cleanup. Ignored
`.xtask/` storage preserves existing unique leaves and cleanup owners; it does
not collect leftovers. Tools with explicit temporary/output paths (including
browser profiles and Playwright outputs), Nix daemon/build processes, and
sandbox storage are not universally redirected.

On Linux, entering any project shell (`ci-nix` included) also sets
`LD_LIBRARY_PATH` to exactly the Nix OpenSSL and dav1d library directories,
replacing any inherited value. Host library directories built against a
different glibc otherwise break the pinned Playwright browsers; re-export after
entry if you deliberately need extra entries.

The focused regression
`cargo test --locked --manifest-path tools/Cargo.toml -p host-proof --test dev_shell_tmpdir`
executes the evaluated hooks for all four shells in child Bash processes; it
does not realize their complete tool environments.

The default `.#default` and `.#ci` shells target `x86_64-linux`,
`aarch64-linux`, and `aarch64-darwin` (Apple Silicon macOS). The pinned nixpkgs
no longer supports Intel macOS (`x86_64-darwin`), so the flake does not
advertise that platform. Declared flake outputs are not production-target
commitments: production targets `x86_64-linux`; ARM Linux is not a delivery
requirement. Apple Silicon macOS targets the development environment only,
checked periodically from committed changes rather than for each
work-in-progress feature or as a production-package/merge prerequisite.

On Linux, Playwright uses the browsers pinned in Nix. On macOS, Nix supplies the
Rust/WASM/Node/database host tooling while Playwright browser binaries live in a
writable user cache whose path includes the pinned `@playwright/test` version;
shell entry never downloads browsers. Provision them explicitly with:

```bash
nix develop -c playwright install chromium firefox webkit
```

Use the repository's Linux VM lanes for canonical e2e verdicts and screenshot
baselines. macOS WebKit is useful for investigation, but it is not actual
Safari; verify Safari-specific behavior in Safari itself and keep the remaining
device-specific requirement in the issue evidence. Theme thumbnails, NixOS
services/VM tests, diagnostic coverage producers, and canonical screenshot
baseline updates remain Linux-only. Do not update canonical snapshots on macOS.
For host PostgreSQL testing on macOS, use `dev test-local` or
`dev pg-run <command>` rather than the NixOS testing VM below.

Check platform output boundaries and native shell/package evaluation with:

```bash
nix eval --impure --expr 'import ./nix/platform-contract.nix { flake = builtins.getFlake ("git+file://" + toString ./.); }'
```

Run this on the production `x86_64-linux` host and during periodic Apple Silicon
macOS development-environment checks from committed changes. ARM Linux has no
delivery-evaluation requirement. Cargo source preparation uses
import-from-derivation, so this check forces derivation paths only for its
native system; declared foreign outputs do not establish foreign-native
execution.

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

**`pre-commit`** runs **`dev precommit`**. Before gate work, it classifies the
complete pre-run dirty tree. `staged-markdown-only` is the sole narrow class: a
nonempty tree containing only staged-only, regular, case-sensitive `.md`
additions or modifications. Any unstaged, untracked, delete/rename,
type-changing, non-Markdown, malformed, unparseable, or otherwise unsupported
evidence uses the broad route. `dev precommit` classifies before selecting the
host checks as either
`class=staged-markdown-only reason=isolated-staged-markdown` or
`class=broad reason=<stable-kebab-case-reason>`; broad reason precedence is
`uncertain-status`, `empty-state`, `untracked-path`, `unstaged-path`,
`delete-or-rename`, `unsupported-change`, `unsupported-index-mode`, then
`non-markdown-path`.

For `staged-markdown-only`, the fixed ordered Markdown-sensitive surface is
Prettier, sequence/identifier collision checks, the ADR bundle, documentation
links, and flow-document parity. It is a filter over the normal production `dev`
workflows, not a per-path policy: Prettier formats global `end2end` plus
`**/*.md`, ADRs project into `docs/README.md` and `docs/ARCHITECTURE.md`, and
document links and flow documents use repository-wide relationships. Every other
classification runs the existing broad host surface. `tools`-only routing is
rejected because that workspace defines gate behavior and repository scanners
that require their full product, documentation, CI, and e2e input populations.

Precommit retains Fix-mode formatting and keeps single-process custody of the
Git index in `dev` itself (`tools/dev/dev/index.py`): it re-stages only
formatter/check mutations on already-staged tracked paths without pre-existing
unstaged changes. It always takes its after-snapshot and reconciles once,
including after a narrow-surface failure; mixed tracked paths, newly-created
untracked files, and changed delete/rename states fail closed, while
pre-existing delete/rename state and untracked files stay unstaged and
tolerated. On `x86_64-linux`, successful narrow staged-Markdown checks and
reconciliation are followed by admission (no remaining unstaged or nonignored
untracked changes, and raw worktree/index blob identity for every tracked
Markdown, `.prettierrc.json`, `.prettierignore`, and `tools/dev/` source input
consumed by `static-docs`, excluding `.claude`) and then the ordinary Git-backed
`static-docs` Nix-check step, built with `dev`'s single definition of Nix flags
and traced with its cache state. A post-request status/index check fails on
tracked, staged, or nonignored untracked mutation (including a rewritten
`flake.lock`) without restaging. Admission or host/staging failure skips the
request; request failure remains the hook verdict and is preserved if the
post-check also fails. Other platforms and the broad route retain host-only
behavior. Each phase (`snapshot-before`, the host checks, `snapshot-after`,
`staging`, `static-docs-admission`, `static-docs`, `static-docs-post-check`) is
its own span directly under `dev precommit`. Every index query is the `git` CLI
run with the hook's environment, so the evidence describes the index the commit
will write, including the temporary `GIT_INDEX_FILE` of `git commit <path>`; no
step inherits the hook's `GIT_*` overrides. Bypass with
`SKIP_PRE_COMMIT=1 git commit` for WIP.

**`pre-push`** runs `dev prepush`: the fast verify-only host/static surface,
existing auxiliary `tools` non-doc tests, host-native product Rust tests, and
then the Nix `doctests` producer and `doctests-gate` as `dev` Nix-check steps.
The producer runs `cargo test --workspace --doc` with the bidirectional
root-workspace fence reconciliation (ADR-0095); there is no host doctest run.
The hook has the same clean-tree refusal as `validate`. On `x86_64-linux`, only
the host `prettier-markdown` Verify at its existing position is replaced with
the same `static-docs` Nix-check step, with the same index custody. Before the
request, the clean consumer also checks raw/index identity for the complete
tracked Markdown and formatter-config input population; afterward it verifies
Git state stayed unchanged. Other checks retain their order, and other platforms
keep host Markdown Verify. Bypass with `SKIP_PRE_PUSH=1 git push` for WIP.

Run `dev static tsc` for the single-command TypeScript workflow, including after
`cd` to another worktree in an existing devShell. On the host it first runs
Nix's shared setup executable, which links the pinned dependencies into the
current checkout; the Nix check has done that setup before it runs
`dev static tsc --sandbox`. Shell entry uses the same setup for IDEs. Setup
refreshes current visible packages and the browser-matched Playwright override,
preserves unrelated directory entries, and unlinks a stale destination symlink
without touching its referent.

Host and hermetic callers use the named checks of `dev static` for formatting,
Prettier, ast-grep, byte-compilation, cargo-deny, ERT and the six Clippy units.
`dev static <name>` runs one check in Verify mode; `--fix` repairs where the
check has an autofix. On the host a check first prepares what it needs from the
current checkout: `tsc` links its pinned dependencies and `leptosfmt` builds its
narrow adapter. `--sandbox` runs a check in place, as a Nix check does: no
checkout root, compile cache or host preparation, and Cargo checks offline
against the Cargo homes the derivation provides, admitting every home before any
work. `rust-format` is a fixed group of checks. `dev static-all [fix|verify]` is
one command that runs every host static leaf in order, ERT last, each prepared
as above. Every `dev` call sets the host Cargo compile-cache policy
(`RUSTC_WRAPPER=kache`, `CARGO_INCREMENTAL=0`, `KACHE_CACHE_EXECUTABLES=0`) and
runs every step, Git or not, without the inherited `GIT_*` repository overrides
a hook exports; only precommit/prepush index custody honours them. This
environment reaches the checkout's tool builds and their compiling children as
well as direct Cargo steps; it adds no resource limits. CI uses the same
configuration.

Beyond that environment kache runs on its defaults: one store in the user cache
directory serves every worktree, its daemon starts on demand, and the store is
capped at 5 % of its volume (5 GiB floor, 100 GiB ceiling) with automatic GC.
Nothing runs `kache init` or writes kache, Cargo or shell configuration into
your files.

- **Disk.** kache restores artifacts into each worktree's `target/` as reflinks
  on a reflink filesystem (btrfs, or XFS with reflink), so worktrees share the
  store's blocks. On ext4 every restore is a private copy, and the store is
  extra disk on top of each worktree's full `target/`. Measured on 2026-11-20
  after `dev check-no-test`, with the line-table debug info that dev and test
  builds emit: a worktree's `target/` and `tools/target/` together took 13–15
  GB, and the store about 8 GB. Building the product tests (`dev check`,
  `dev test-local`) adds to both.
- **Location.** `KACHE_CACHE_DIR` moves the store. Keep it on the same
  filesystem as your worktrees, or restores fall back to copies.
- **Off switch.** If you suspect a stale cached artifact, run the command with
  `KACHE_DISABLED=1`: the wrapper stays in place but hands every compile
  straight to `rustc` without starting the daemon. `dev` passes it through.

All `dev` gate workflows fail fast: the first failed required step stops later
ordinary checks, and `dev` exits with that step's status. Prepush's clean-tree
refusal remains its first boundary. Precommit still performs its after-snapshot
and conservative staging reconciliation after the gate stops; safe
formatter/check changes can be restaged, while mixed or unsafe Git/index state
still fails closed. Work not reached is not reported as successful. Apart from
precommit's selected routing surface, the workflows remain broad, run the host
check groups in their documented order (see
[Local checks: `dev`](#local-checks-dev)), and preserve clean-tree/staged-subset
authority. Independent CI lanes and required aggregates remain authoritative;
local fail-fast behavior does not weaken their verdicts.

## Development workflow

- Track development work in GitHub issues (`jaunder-org/jaunder`) rather than
  ad-hoc markdown TODO lists; issues are the durable work memory.
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
  approval" rule under Testing). `dev check` and `dev validate` reject every
  `#[allow(...)]` and every `#[expect(...)]` without an immediately preceding,
  non-empty `// lint-suppression:allow <reason>` marker. After approval, add
  that reviewed marker in the same change; never use it to make an unapproved
  suppression pass.
- **Review gates the merge, not the commit.** Agents commit on their own
  recognizance — history is meant to be built up in focused, atomic steps, and
  the pre-commit hook already gates each one on the fast `dev precommit`. What
  requires explicit user approval is **landing** the work: merging a PR
  (`dev pr-land <PR> <approved-head-SHA>` requires previously obtained and
  recorded human approval for that exact PR/head). Ask for review before you
  merge, not before every commit.

### Read-only Org timezone evidence

Run `traced-run -- dev org-timezone-manifest /path/a.org /path/b.org` with
explicit local UTF-8 Org files (shell globs are also accepted). There is no
recursive discovery, inventory option, database connection, server access, or
import. The extractor only reads originals; normal command acquisition/build
artifacts remain outside them.

Stdout is a JSON manifest, independent of `--json`: `mappings` contains unique
`post_id` (decimal string, avoiding JSON integer precision loss) and `timezone`
pairs, ordered by numeric ID. Only positive ASCII decimal IDs within signed
64-bit range are accepted; leading zeros denote the same ID. Named zones use
`PublicationTimezone` validation and retain exact alias spelling. Surrounding
Org property whitespace is syntax, not part of the value. Only the leading
keyword block is inspected, using the same boundary as server Org metadata;
body/headline properties are not candidates. DATE, STATUS and body content are
not prerequisites, and unrelated metadata is not validated.

`diagnostics` is ordered by source path/code/ID. Codes distinguish missing or
invalid ID/zone, repeated ID/zone properties, duplicate IDs across sources, and
unreadable (including non-UTF-8) sources. Every rejected source also receives
`unmapped_source`; duplicate diagnostics name the ID. All sources naming an
ambiguous ID are excluded, even if zones agree or another source is invalid.
Other valid mappings remain in a partial report. Exit **0** means no local
exceptions, **1** means a partial manifest with diagnostics, and **2** means
invalid invocation or output serialization failure. A successful extraction is
not server-coverage proof: `server_coverage` always explicitly reports that
server IDs absent from originals cannot be determined without a separately
authorized read-only Post-ID inventory. This command does not accept or fetch
that inventory, infer matches from titles/slugs/dates/offsets, or change any
Post, revision, timestamp, or original file.

### Artifact lifecycle

Commit what defines, implements, explains, or continuously checks the system.
Keep code, regression tests and their minimal maintained fixtures, actively
compared baselines, ADRs, and current user/developer documentation in the tree.
An old delivery ledger is not a behavioral invariant, even if a gate checks its
format or completeness.

Commit the approved spec and any necessary outline before implementation.
Maintain them during the work and conformance review, then project enduring
contracts and rationale into current docs/ADRs and file unresolved work as
issues. Delete the completed spec and outline in a final cleanup commit rather
than moving them into an archive. Jaunder's non-squash merges retain their full
history. For a later review, recover a deleted document from an earlier commit
into session storage; historical citations use commit-pinned URLs.

Logs, traces, measurements, audit inventories, review packets and temporary
probes belong in ignored `.xtask/` or session storage, not tracked evidence
folders. Report the command, tested revision, outcome and relevant artifact
reference in the PR. Local evidence may expire after review; explicitly required
release/compliance evidence goes to its designated external artifact store with
an owner and retention period. Sanitizing a report does not make it source.

At specification and commit boundaries, distinguish proof to execute from
artifacts to commit. For each new document or data file, identify its ongoing
consumer and what future change requires updating it. Promote only enduring
conclusions, current procedures, or minimal fixtures used by real regression
tests. Do not weaken proof requirements merely to avoid retaining their outputs.

### Adding an ADR

Start from [`docs/adr/template.md`](docs/adr/template.md): copy it to
`docs/adr/drafts/<slug>.md` as a tracked, numberless **draft**, never a
hand-picked number. Keep the heading `# ADR-DRAFT: <Title>` and cite the draft
by its sole pre-promotion identity, `docs/adr/drafts/<slug>.md`. Commit the
draft and its architecture projection with the feature so both are reviewable.
See [`docs/adr/drafts/README.md`](docs/adr/drafts/README.md) for the link forms
that resolve both while the draft is tracked and after its one-directory
promotion.

Feature authors and shipping flows must not invoke promotion and must not edit
`docs/README.md`. The draft remains `proposed` when the feature reaches `main`.
A serialized workflow then derives from current `main`, runs the deterministic
promotion mutation, and opens one stable promoter PR. Promotion assigns each
draft the next free number, stages the tracked source deletion and
`docs/adr/NNNN-<slug>.md` destination, rewrites path citations and `proposed` to
`accepted`, and regenerates the ADR index. The dedicated least-privilege GitHub
App arms auto-merge; ordinary pull-request and merge-group checks run, and only
the merge queue writes the result to `main`.

An open promoter's head SHA and generated diff never change. Drafts merged later
wait for its merge and the next promotion pass. The ordinary promoter CI/queue
interval is a healthy **proposed-decision lag**, not a violation of the
acceptance model. A draft that remains proposed because the promoter failed to
create, check, or merge is failed-automation lag and requires diagnosis; it is
not a reason to promote from the feature branch.

When promoter automation fails, diagnose the visible PR, ref, check, or workflow
state and rerun the authorized controller with
`nix run --accept-flake-config --no-update-lock-file --no-write-lock-file .#adr-promoter -- controller`
in the trusted workflow context (not from a feature-author checkout). This
branch selects the Python package directly; trusted-main delivery and
observation remain separately gated. The controller cleans up incomplete
controller-owned state only with an exact-SHA lease and regenerates it from
fresh `main`; failed required checks remain visible. Never manually close,
delete, rebase, force-update, promote, or otherwise alter a promoter attempt.

ADRs are `docs/adr/NNNN-slug.md` with a canonical `# ADR-NNNN: <title>` heading
and a single-token `- Status: <token>` line. A **numbered** ADR carries one of
accepted/superseded/deprecated/rejected — never `proposed`, because promotion
and numbering are the acceptance event (ADR-0088). The table in `docs/README.md`
is a **generated** projection whose number, link, and status cells are owned by
promotion and the package's `sync-readme` action; only row titles are
hand-curated. The `adr-format` and `adr-readme-parity` gates reject a
non-canonical ADR or drifted table. Recovery for table-only drift is:

```text
nix run --accept-flake-config --no-update-lock-file --no-write-lock-file .#adr-promoter -- sync-readme
```

The `adr-view-parity` gate requires every `accepted` ADR to be cited in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). Update the view in the feature
PR and cite the numberless draft path; promotion rewrites that citation to the
numbered path as it accepts the decision. `CONTEXT.md` is a mandatory
consideration when the decision changes Jaunder's ubiquitous language.

Numberless files under `docs/adr/drafts/` remain outside the non-recursive
numbered-ADR enumeration used by `identifier-collisions`, `adr-format`, and
`adr-readme-parity`; `doc-links` does check them because they are tracked. The
collision gate still makes duplicate numbered ADR or migration prefixes loud,
but serialized post-merge allocation removes the feature-branch ADR collision
class. Migrations keep their sequential convention and are renumbered by hand on
the rare occasion it is needed.

## Testing

There are several testing layers in this repository. Use the smallest one that
matches the change first, then move up only when the boundary needs a broader
signal: `prepush` for local push, CI for hermetic PR proof, and full `validate`
for local full-gate confidence. Never remove functionality to pass tests, and
never bypass or suppress testing, coverage, or linting without explicit
approval.

Every HTTP endpoint exposed by the running Jaunder application must have both an
integration test and an end-to-end test. Unit tests belong in the same file as
the code being tested. End-to-end tests belong in `end2end/` and use Playwright.
A CLI-owned, loopback-only, command-lifetime HTTP transport with no supported
external address is not an application endpoint: cover its routes with host
integration tests and its public command through the highest real consumer
workflow instead of adding a test-only address for Playwright.

### Wasm-only browser unit tests

Keep pure logic host-testable: move target-independent behavior into `common` or
another host-compiled module and test it there. Use `wasm-bindgen-test` only
when the contract is an irreducible browser primitive such as `window`, DOM, or
`localStorage`. Put that test beside its implementation, configure the module
with `wasm_bindgen_test_configure!(run_in_browser)`, and do not rely on the
runner's Node default.

Run these tests hermetically with
`nix build -L --accept-flake-config .#checks.x86_64-linux.wasm-tests`, or
through the normal test-enabled `dev check`/`dev validate` workflow. The
dedicated check uses Nix-store Chromium and chromedriver paths; it executes
browser behavior but does not contribute wasm lines to the host coverage
denominator.

For tests requiring a database, use the shared harness: `#[apply(backends)]`
with `backend.setup().await`, which returns a `TestEnv` owning a private
`StorageFactory` plus the backend-specific `TestBase`. Request only the exact
storage handles or `WriteScope` the test needs from `TestEnv`; use `base` only
for backend lifecycle or deliberately backend-level assertions (ADR-0033). Do
**not** hand-roll a pool.

**Never `sqlite::memory:`.** It fails twice over. Each _connection_ to
`sqlite::memory:` gets its own separate database, so a multi-connection
`SqlitePool` over it is not a coherent shared store; and WAL — which both
production and the harness set — is unavailable for it. The harness uses a
temp-file SQLite database instead. ADR-0033 exists precisely to retire hardcoded
`sqlite::memory:` pools, which ran SQLite-only and left Postgres unexercised.

**Tests that spawn `git` must scrub the repo-redirecting `GIT_*` env.** When git
runs a hook it exports `GIT_DIR`/`GIT_INDEX_FILE` (and
`GIT_WORK_TREE`/`GIT_OBJECT_DIRECTORY`/`GIT_COMMON_DIR`/`GIT_NAMESPACE`), and
those **override `-C <dir>`**. So a test that builds a throwaway repo with
`git -C <tmpdir> …` will, when run inside the pre-commit/pre-push hooks (which
invoke `dev precommit`/`dev prepush`, whose host tests then run that code), be
redirected at the **real** repository — corrupting HEAD, the index, and the
shared worktree config. `dev` runs every step (cargo test, nextest and pytest
included) without those vars, and `tools/dev/tests/conftest.py` drops every
`GIT_*` variable for its tests; keep both. Build every Rust test's git command
(and any production helper it calls) through a constructor that `env_remove`s
those vars — see `git::at()` in `tools/repository-policy/src/git.rs`. Read-only
production queries (`rev-parse`/`log`/`diff`/`ls-files`) are safe unscrubbed
since they don't mutate.

### Evaluating the web UX interactively

Use `dev sandbox` for a host-native browser session without the Playwright
runner or telemetry collector:

```bash
# Disposable empty instance; removed when the command exits.
dev sandbox

# Persistent named instance with ordinary and operator accounts.
dev sandbox writing-flow --profile standard

# Persistent instance with four accounts, local images, and rich Markdown/Org Posts.
dev sandbox timeline --profile demo
```

The `standard` profile creates `user` and `operator`; `demo` also creates
`alice` and `bob`. Every seeded account uses the development-only password
`jaunder-dev`. Named workspaces live under `.xtask/sandboxes/<name>/` and resume
without a profile:

```bash
dev sandbox writing-flow
dev sandbox writing-flow --reset
dev sandbox writing-flow --reset --profile demo
```

`--profile` applies only when creating or resetting a named workspace. `--reset`
requires an existing name; without an explicit profile it reuses the recorded
profile. After readiness, the supervisor prints the URL; for nonempty profiles
it also prints the seeded `user` and `operator` development credentials. Stop
the foreground supervisor with Ctrl-C or SIGTERM. The first signal starts
graceful drain with automatic force after exactly 10 seconds; a further Ctrl-C
may force earlier, while repeated SIGTERM does not escalate. The initiating
signal returns 130/143. After force and direct-child reap, if ProcessKit reports
remaining group members the helper makes one bounded follow-up ProcessKit
stop/report with a five-second grace and escalation disabled, consuming signals
without restarting that operation. Cleanup requires direct-child reap and an
explicit empty-group report; an unconfirmed stop retains the group and
application guards with diagnostics rather than polling/retrying. This is not a
universal OS guarantee and does not change `jaunder shut-down`'s independent
900-second wait and refusal to escalate.

Run one admitted operational Jaunder command against a named workspace after
`--`:

```bash
dev sandbox writing-flow -- user-create --username reviewer
dev sandbox writing-flow -- site-config set site.title "Writing review"
dev sandbox writing-flow -- backup
```

Command mode pins the same workspace generation for the complete child lifetime
and returns the child's exact exit status. It accepts only `site-config`,
`user-create`, `app-password-create`, `user-invite`, `smtp-test`, `backup`, and
`websub`; storage selectors and lifecycle commands remain owned by the sandbox
supervisor.

`dev sandbox` forwards literal argv from the active checkout to
`nix run --no-update-lock-file --no-write-lock-file .#ux-sandbox -- <args>`. The
separately filtered `packages.ux-sandbox` builds only the helper and its
required Cargo path-dependency closure at the dev profile. It retains required
workspace manifests/configuration and the root lockfile, using placeholders for
excluded workspace targets rather than their implementation or application
CSR/public inputs. The root Cargo path patch also requires the retained SQLx
SQLite driver source and parent license notices for metadata resolution, even
though the helper does not compile SQLx. Those patch inputs, shared manifests,
lockfile, pinned toolchain or required helper dependencies can invalidate it.
The previous Cargo bootstrap already supported warm artifact reuse; this route
promises neither faster warm runs nor cache hits.

Helper bootstrap uses Git-flake capture: tracked-dirty and intent-to-add inputs
participate, but new untracked helper inputs are omitted without automatic
refusal by the bootstrap itself. Use `git add` or `git add -N` to include them.
This is distinct from subsequent application-artifact admission below; bootstrap
neither stages files nor adds a pre-bootstrap admission check.

The existing `validate-test-checks` CI job requests an ordinary helper build
through the unchanged Cachix setup. The helper name is eligible under the
existing filter, but this wiring establishes neither CI completion, upload nor
later substitution; those claims require direct retained evidence.

For application preparation, the helper still checks relevant untracked checkout
inputs against the selected Nix source admission and refuses omitted inputs
rather than silently selecting an incomplete source snapshot. It then selects
`.#sandboxArtifacts`, one immutable set containing debug-profile `jaunder` and
`test-support`; the Nix-built Jaunder executable embeds the selected CSR bundle,
as every Jaunder build does. The helper keeps its unique
`.xtask/sandbox-artifacts-*` Nix output link alive through preparation and the
server or command consumer lifetime. Nix/store owns this build concurrency; the
sandbox no longer publishes `target` application artifacts or takes
`.xtask/host-artifacts.lock` for them. Its named workspace, server-lease and
application runtime locks remain unchanged. Seeding remains the
`test-support seed-sandbox-profile` subprocess; its stdout is captured and
withheld from operational output while stderr and exact child status are
retained. No production seed API is added. The old xtask sandbox command was
retired.

Each named workspace stores its database, configuration, Media, and versioned
profile metadata together. Stable control locks and current-server metadata live
under `.xtask/sandboxes/.locks/`; they let interrupted reset operations recover
without publishing a partial profile. Resume runs the current startup
migrations. Command mode refuses to open storage while an unmanaged server holds
the runtime lock or while the managed server's executable digest differs from
the newly built command; stop and restart the sandbox to move both onto the same
generation.

This command optimizes the local UX-review loop; it is not deployment-fidelity
proof. Use the Nix VM E2E lanes below when backend, browser, release-build, or
hermetic environment parity matters.

### Running e2e tests locally

The host and the CI VM load the **same** `end2end/playwright.config.ts`, keeping
browser-flow definitions shared. A focused host pass does not establish the
hermetic backend/browser matrix verdict; CI remains authoritative.

- **Fast local loop** — `dev e2e-local` (Python inside `dev`,
  `tools/dev/dev/e2e_local.py`) owns the whole loop: it resolves the CSR bundle
  and builds the host server, starts an OTLP collector and one `jaunder serve`
  per Playwright worker (`JAUNDER_E2E_WORKERS`, default 1), each on its own
  ephemeral port with a fresh SQLite temp DB, seeds each instance, and runs the
  local Chromium lifecycle, tearing every process down after. The collector
  captures correlated browser and server spans, then the command retains the
  complete capture under `.xtask/e2e-local/<run-id>/<browser>/capture/` (each
  server's streams in its `instance-<i>/` subdirectory) and prints the exact
  `otel-traces.jsonl` path. Playwright's output is drained while it runs. No
  pre-existing server or collector is needed. Use this while iterating on the
  web UI; it uses the HTML reporter for interactive debugging. For
  browser-specific investigation on macOS, select WebKit after provisioning
  browsers, for example `dev e2e-local post-image-fit.spec.ts --browser webkit`;
  this proves Playwright WebKit only, not actual Safari.

  Ordinary runs select `.#sandboxArtifacts`: one source-consistent immutable set
  containing debug-profile host `jaunder` and `test-support`, with the debug CSR
  bundle embedded in Jaunder. `--update-visual-snapshots` selects
  `.#sandboxArtifactsReleaseCsr`, embedding the existing release CSR while
  retaining debug host and seed executables; there is no separate release-CSR
  flag. The helper's interruptible Nix invocation and each selected executable
  path belong to one invocation. E2E uses the existing `host_tooling` source
  admission: tracked dirty and intent-to-add inputs participate; relevant
  nonignored untracked inputs are refused with instructions to use `git add -N`
  or `git add`. It never silently falls back to older outputs or `target/site`.

  The selected output is rooted under `.xtask/gcroots/e2e-local-*` through all
  browser lifecycles and server/collector teardown. Nix owns artifact identity,
  store concurrency and reuse; `e2e-local` no longer uses
  `.xtask/host-artifacts.lock` to protect mutable checkout application builds.
  Cargo's dev profile and pinned toolchain still determine the embedded browser
  assertions and package inputs; a first Nix miss may be slower and retains an
  additional debug artifact.

  `e2e-local` refuses a set `JAUNDER_CSR_BUNDLE_DIR` before artifact production,
  server startup or seeding, with guidance to unset it; it does not silently
  ignore the input or fall back to Cargo/site output. This refusal is specific
  to e2e's immutable artifact contract. The variable remains a build-time input
  for Nix package builds and an explicit validated input for `test-local` and
  manual Cargo/Vite workflows. A first SIGINT/SIGTERM/SIGHUP stops the running
  seed or Playwright consumer, with a ten-second grace, and still runs the whole
  server/collector teardown and the panic gates. A second signal escalates the
  stop under way to an immediate kill. The run exits 128+n and releases its
  unique Nix root afterward. The standalone manual `dev build-csr debug|release`
  workflow and its local `target/site` output remain unchanged.

- **Focused browser-flow proof** — `dev e2e-local <spec-or-file:line>` scopes
  the local loop to one Playwright positional filter when that spec or line
  covers the changed behavior. The filter applies to the browser's one
  `<browser>` project, visual tests included, with no prerequisite ordering:

  ```bash
  dev e2e-local auth.spec.ts
  dev e2e-local auth.spec.ts:17
  ```

  This is the default proof for a changed browser flow before broader e2e gates.
  Rerun the same focused command after a targeted fix. The command does not
  forward arbitrary Playwright flags; use file or `file:line` targeting instead
  of title `--grep` examples.

  Mount completion is not data ownership. Before interacting with a permalink
  Post, wait for the fetched client card `#j-post-${post_id}` from the creation
  response and scope its controls to that card. Projector-seeded markup can show
  the same tags before the fetch resolves and be replaced during a pointer
  gesture, losing the click. Keep destination-content assertions; do not repair
  this readiness gap with sleeps, larger timeouts or retries.

- **The authoritative workflow** — `dev e2e` attempts all sixteen enabled
  hermetic Nix VM lanes, then reconciles every backend/browser pair even after
  producer failures. Any producer, reconciliation or coverage failure fails the
  workflow. Full `dev validate` calls this same synchronous workflow after its
  non-E2E surface. CI derives its matrices at runtime from
  `end2end/e2e-lanes.json`, using `scripts/e2e-matrix.jq`, and invokes
  `dev e2e-producer <identity>` per lane and
  `dev e2e-reconcile <backend> <browser> <artifact-root>` after that pair's
  producers settle. The artifact root contains `e2e-<identity>` directories. No
  Rust E2E scheduler or embedded lane catalog remains. Run representative
  hermetic proof when backend/browser parity, release behavior or the branch
  boundary matters; this is not required before every push. The push hook
  remains `prepush`.
- **Reconciliation and controls** — Per backend, Chromium runs three lanes,
  `--shard=1/3` to `--shard=3/3`, and Firefox five, `--shard=1/5` to
  `--shard=5/5`, each selecting the browser's one `<browser>` project. The
  project has no dependency, because `--shard` would rerun a dependency in every
  shard, so a pair's lanes run each test exactly once. The catalog's per-browser
  `shardWeights` (exported as Playwright's `PWTEST_SHARD_WEIGHTS`) size the
  shards by measured duration, and each string's part count is that browser's
  shard count. Re-derive the weights from lane reports' per-test durations when
  the shards drift apart; change a browser's count by changing its weights and
  lanes together. Every lane VM runs one server instance and database per
  Playwright worker. Producers lift available lane-qualified diagnostics before
  propagating their original Nix failure; missing diagnostics cannot turn a
  failure green. Reconciliation fail-closed checks the independent census,
  report union, retries, duration manifests, phase sidecars and trace captures.
  The retired Rust producer/control aliases are not parallel workflow
  entrypoints. Existing unsplit Firefox workers=2/workers=4 measurement
  installables remain available through ordinary `nix build`; workers=4 remains
  rejected for production.
- **Duration-pressure gate** — After an otherwise-successful VM lane has
  captured diagnostics, it reconciles `playwright-report-<lane>.json` and
  `duration-budget-manifest-<lane>.json` in `.xtask/diagnostics/e2e-<lane>/`.
  Missing, malformed, incomplete, or mismatched inputs fail closed. Every
  reported attempt is checked, including retries: an attempt using **80% or
  more** of its effective whole-test timeout fails the lane even when a later
  retry passes. This detects insufficient headroom; it does not right-size
  timeout budgets. Keep deriving an exceptional whole-test budget from its
  polling deadline rather than from observed duration.

- **Boot-decomposition coverage gate** — After the same successful VM lane lifts
  diagnostics, it reconciles the executed browser-project set in
  `playwright-report-<lane>.json` with `capture-<lane>.tar.gz`'s
  `capture/otel-traces.jsonl`. Each reported project needs non-dropped, mounted,
  current-schema navigation evidence with complete document-frame boot phases
  that close within 1 ms; `e2e.test` and secondary `e2e.page` spans both count.
  Missing, empty, malformed, duplicate, or mismatched evidence fails closed.
  This certifies trace evidence, not a boot time budget, source coverage, or
  `#[server]` request-flow coverage.

#### Bounding full local E2E fan-out

`dev e2e <N>` bounds producer admission with ordinary `xargs -P`. Omission runs
at most three producers, and so three NixOS VMs, at once: the local peak before
#1890, when each backend's three Firefox lanes ran together. Raise it only from
actual admitted CPU/RAM capacity; larger bounds cannot start more work than
there are enabled lanes. Invalid bounds fail before producers start. Full
`dev validate` uses the ordinary default; `dev validate-no-e2e` omits E2E.

A producer failure does not discard pending lanes. All producers settle before
every backend/browser reconciliation runs. The helper normalizes producer
failures for `xargs` so even exit 255 cannot cancel pending work. Lane artifacts
and GC roots remain under `.xtask/diagnostics/e2e-<identity>` and
`.xtask/gcroots/e2e-<identity>`. Nix's selected output JSON is retained as
`build.json`; successful-output verifiers read that immutable output, never a
success-only root left by an older invocation. Phase sidecars remain available
as artifacts; no Rust concurrency or aggregate phase presentation is
synthesized.

This is a host dispatch bound, not Nix's `max-jobs`/core scheduler or a limit on
all host processes, guest workers, dependency builds, substitutions or builders.
Nix settings and other invocations can further affect actual overlap. Current
production lanes request two vCPUs, with 4 GiB for Firefox guests and 3 GiB for
Chromium guests; these are configured guest sizes, not measured host use or
proof that any particular cap is safe. Each guest also runs one Jaunder instance
and database per worker, so a guest's size must cover its browsers and every
instance together; resize from measured guest memory, not from these figures.
Cross-checkout heavy-path admission is separate work in #1753, not a
prerequisite. No global queue, automatic host threshold or resource-pressure
cancellation is introduced.

#### Visual snapshot workflow

Visual coverage is deliberately five existing behavioral tests tagged `@visual`:
the public timeline, login form, authenticated `/app` cockpit, empty
`/posts/new` composer, and compact `/drafts` row actions. Keep the assertions
that prove each state ready; the screenshot comparison supplements them.
Expected PNGs live beside their owning specs under `*.spec.ts-snapshots/`, named
`<name>-<browser>-visual-linux.png` by the browser project's
`snapshotPathTemplate`. There is one Chromium and one Firefox baseline per
state, shared by the SQLite and PostgreSQL gates. Do not add backend, WebKit,
mobile, or alternate-theme variants without changing this policy.

Comparisons are exact: zero differing pixels, pinned DejaVu/fontconfig, and
screenshot-only animation/caret/font CSS. The public timeline masks only
`.j-post-time`; other dynamic state must be made deterministic rather than
masked or tolerated.

Failed visual assertions attach bounded, time-limited `visual-failure-*` JSON
through the existing Playwright artifacts. Geometry, computed font family and
interaction state are observed **after failure**, not at the raster instant;
selected rendered faces and initial pointer coordinates remain unmeasured. No
additional pass-path browser observations or replacement screenshots are taken.
An unavailable diagnostic must not replace the original screenshot failure.

Hermetic E2E diagnostics retain the selected output's
`producer-input-<lane>.json` when available, plus the invocation's `build.json`
output selection. These are attribution, not verdict evidence or an attestation
of hidden capture completeness. The focused host-local workflow retains its
existing `producer-checkout-host-local.json` behavior. Dirty source is
committed-base-only. Reused outputs do not establish their original producing
runtime or current execution. The clean revision intentionally identifies the
final VM check, not product dependency derivations; distinct clean commits do
not share a promised final test-verdict identity.

The only supported baseline updater is:

```bash
dev e2e-local --update-visual-snapshots
```

It release-builds CSR and gives Chromium and Firefox separate fresh
server/database lifecycles. **Never** hand-run Playwright's snapshot-update mode
against an ambient server or database. Review every changed PNG, then run
`dev e2e-local` without update mode before committing intentional baselines
together with the rendering or test change.

Visual tests follow the [E2E state rule](#e2e-tests-set-their-own-global-state)
and need no ordering. Each seeds its own fixed-name user, because usernames
render in the pixels, and calls `setSeededSiteAppearance`
(`end2end/tests/visual.ts`) to set the seeded title, tagline, Registration
Policy, Site Default Audience, Media Upload Capability and Site Theme it
renders. A visual test therefore passes wherever it runs on an instance other
tests have used. Visual tests take the run's retry policy like any other test. A
retry that lands on the instance where its earlier attempt seeded the fixed-name
user signs in to it with `signInAsVisualUser` instead of seeding it, or its
Posts, again; `--repeat-each` on a visual test is unsupported.

`dev e2e-local <spec-or-file:line>` remains the targeted loop. It selects the
browser's one project with that positional filter; an unrelated spec does not
schedule all visual tests. Update mode selects `@visual` tests and intentionally
rejects a positional filter.

#### Accessibility workflow

Accessibility coverage runs axe against the same four stable states as visual
coverage: the public timeline, login form, authenticated `/app` cockpit, and
empty `/posts/new` composer. Each test retains its behavioral readiness
assertions, carries the `@accessibility` tag, and checks the complete mounted
document against the machine-checkable WCAG 2.2 Level A and AA rules available
in the pinned axe version.

Every scanned state must report zero violations. Do not add exclusions, disable
rules, suppress findings, or weaken the shared policy; fix each violation at the
product markup or style that owns it.

To extend coverage, choose a stable existing behavioral test and keep its
readiness assertions. Add the `@accessibility` tag and call
`expectAccessible(page)` only after the complete intended state is mounted.
Automated axe checks cover only machine-checkable findings; they do not replace
keyboard navigation, screen-reader testing, manual assessment, or inclusive-user
assessment.

### E2E tests set their own global state

Every Playwright worker has its own server instance and database, but the tests
a worker runs share that instance in an order you do not control. Site settings
such as the title, tagline, `base_url`, Registration Policy, Site Default
Audience, Media Upload Capability and Site Theme are global to the instance. So:

- A test **sets the global state it depends on**, usually with
  `seedConfigViaTool` or the owning UI flow, before it relies on it.
- A test **asserts nothing about global state it did not set**.
- **Nobody cleans up.** Do not restore settings in `finally`, `afterEach` or
  `afterAll`, and do not add reset, restore or snapshot infrastructure.
- Fix an order-dependent failure with one of two moves: the test sets its own
  precondition, or the incidental assertion goes.
- Per-user state needs no rule: use the per-test identity fixtures (`user`,
  `mailbox`, `verifiedUser`) or `seedUserViaTool`.

The decision is recorded in
[per-worker E2E server instances](docs/adr/drafts/per-worker-e2e-server-instances.md).

### One boot per page

An e2e page performs **exactly one document load — its entry, at the URL under
test**; everything after it moves within the app. This is the test-side
counterpart of ADR-0076, and it is enforced two ways: a runtime per-`Page`
budget fails an undeclared second load, and the `e2e-goto-wrapper` static check
fails a `page.goto` written outside the navigation wrapper. The reasoning is in
[ADR-0111](docs/adr/0111-e2e-one-boot-per-page.md).

The API is in `end2end/tests/helpers.ts` (its docblock is the usage rules):
`goto` to enter, `navigateInApp` to move inside the app, and
`allowSecondBoot(page, reason)` to declare a further load where the
destination's cold render is the subject. A declared allowance that no load
consumes fails the test.

### Local checks: `dev`

`dev`, the Python package in `tools/dev/`
([Python `dev` runner](docs/adr/0251-dev-runner-replaces-just.md)), owns
developer, hook, CI and Nix-sandbox workflows. Each workflow is a `dev`
subcommand (`dev check`, `dev test-local -- -p storage foo`,
`dev ci-validate-host`, `dev static tsc --sandbox`). It invokes the pinned tools
and existing producer commands directly, passing arguments as argv lists, never
through a shell; Rust algorithms remain callable leaves, not a workflow catalog,
and Git/index custody is `dev`'s own. Gates return process status and logs, not
a gate-wide JSON report. The old cargo gate aliases and the `xtask` workspace
are retired.

On the host, the `default`, `ci`, `mutants` and `ci-nix` devShells put a `dev`
shim on `PATH`. The shim finds the checkout with `git rev-parse --show-toplevel`
and runs that checkout's `tools/dev/` with `python -P` under the pinned
`python3.withPackages` interpreter, so hooks and gates always run the working
tree's code with no virtualenv. Bytecode caching stays on; `__pycache__/` is
ignored. Nix checks run a `dev` package built from a filtered `tools/dev/`
source, so any edit there rebuilds them. The Rust host commands are
concern-owned executables in the `tools` workspace — `repository-policy`
(`repository-docs`, `source-contracts`), `host-proof` (`e2e-evidence`,
`trace-analysis`, `wasm-audit`, `source-diagnostics`, `admit-inputs`, which
`test-local` and `e2e-local` use, and `test-local`'s `csr-prerequisite`) and
`coverage` (`coverage-check`, nondefault `host` feature). `dev` runs them with
locked Cargo from the active checkout; CI's E2E verifier uses its Nix output
(`packages.host-proof`). Host tools run only on the host, never inside
derivations. Runtime paths belong to the active checkout, not the tool's store
build directory. The executables that run inside derivations are likewise owned
by the crates whose logic they run: `coverage-sandbox` (`coverage`), `doctests`
(`doctests`), and the dedicated `perf-producer` and `system-artifacts` crates.
Each is its own Nix package, and a derivation takes only the one it runs.

Each host tool prints exactly one JSON report on stdout (`host_tool::report`:
`command`, `ok`, `duration_ms`,
`steps[{name, ok, skipped, duration_ms, detail}]`, optional `exit_override`,
optional rendered `report` text and optional structured `evidence`). The shared
argument parser points fd 1 at stderr and reserves the real stdout for that
report, so child output goes to stderr; the exit status is the verdict. `dev`
renders the human view from that report (`tools/dev/dev/report.py`); output that
is not a report fails the step. There is no `--json` flag,
`.xtask/last-result.json` sidecar or completion sentinel. Elisp artifact lifting
remains alongside its consumer algorithm (`coverage-check elisp-consume`);
doctests run only in Nix (`checks.doctests`, `checks.tools-doctests`). The E2E
workflow now owns authoritative output selection and trace/snapshot
verification; the temporary `server-fn-flow` bridge is retired, while the
independent static `server-fn-coverage` leaf remains. Verification's ordinary
clean-tree admission is direct `dev`/Git; raw/index docs admission, post-request
checks and precommit reconciliation are `dev`'s distinct index-custody phases.

New, nonignored tooling inputs must be included in the Git-backed flake snapshot
(`git add` or `git add -N`) before normal invocation. The Nix source declaration
owns this admission; a missing input or failed realization is a refusal, not a
fallback to an older tool. Explicit
`cargo test --manifest-path tools/Cargo.toml` and native Cargo builds remain
source-development diagnostics.

The scheduled upgrade test upgrades a site an earlier build wrote to this
checkout and verifies its records
([upgrade test decision](docs/adr/drafts/upgrade-test.md)).
`dev upgrade-test --from <rev> <sqlite|postgres>` runs it in a local VM and
`dev upgrade-dispatch [--from <rev>] [--ref <branch>]` starts the CI workflow.
Changing `BaselineState` (`end2end/tests/production-baseline.ts`) or the demo
sandbox profile keeps a reader for every version written since the latest tag.

Host-side steps cover static checks, clippy, the standalone `tools` workspace,
and day-to-day root-workspace Rust tests. `prepush` adds the Nix doctest
producer and gate; it remains a fast local gate rather than an alias for
hermetic `dev validate-no-e2e`. Nix static proof, coverage/CRAP, wasm browser
tests, Elisp coverage, the wasm budget, and e2e retain their stated hermetic or
full-validate authorities.

#### Hermetic Rust coverage evidence

The Nix `coverage` producer is the sole execution authority for the unfiltered
**root-workspace** nextest population. It runs both SQLite and PostgreSQL
backends in one instrumented pass; backend-common behavior therefore retains
SQLite/PostgreSQL parity. Its machine-readable census supplies the expected
identities, and the terminal run records must reconcile exactly:
`expected = executed + ignored`. Every required producer subprocess has its exit
status checked; command output never turns a nonzero exit into success.

Run `dev coverage` or `dev doctests` for the direct single-command hermetic
workflows. Each first requires a clean checkout, then realizes its producer at
`.xtask/gcroots/{coverage,doctests}` and runs the hard Nix status gate. Coverage
then invokes the host-only `coverage-check policy --out .xtask/gcroots/coverage`
leaf to apply source-aware line and CRAP policy; that leaf does not orchestrate
producers or validate producer status. The Nix gate rejects missing, invalid, or
non-success status before policy runs. A missing, malformed, or empty text
report, or a report with zero executable lines, is rejected rather than reported
green. Validation and CI call these clean-gated public workflows. Fix-mode
`dev check` and `dev prepush` instead use the internal doctests workflow, the
same producer/gate population without the public `dev doctests` clean-tree
admission: `check` keeps its dirty-tree formatting behavior, and `prepush` has
already refused a dirty tree. Doctests and the auxiliary `tools` workspace
retain their separate test authorities outside this root-workspace nextest
population. The Rust gate aggregates are retired; the four `dev ci-validate-*`
commands compose the public workflows while preserving independent lane
verdicts.

Within the host/local gate, health checks are ordered for fast, actionable
feedback: clean/staged-tree preconditions first, source-format and generated-doc
consistency before compile/type work, cheap repository-shape invariants before
expensive Rust checks, compile/type checks before host runtime tests, and
hermetic Nix checks only where isolation is load-bearing. A codebase-health
check stays in a host concern tool (for example `repository-policy`) rather than
`cargo test` when it spans files, generated artifacts, Git state, docs, Nix
derivations, or multiple languages; use `cargo test` for one crate/module's
behavior contract.

For product Rust behavior changes, use the focused host-native lane before broad
gates when one Rust test, module, package, or subsystem can answer the current
question:

```bash
dev test-local -p storage site_config_primitives_round_trip
dev test-local -p jaunder storage::posts::post_create_and_get_by_id_works
dev test-local -p jaunder -E 'test(/^web::/)'
```

After a targeted fix, rerun the same focused command first; it is the fastest
proof that the regression moved from red to green. Escalate only at boundaries:
the enforced `pre-commit` hook runs `dev precommit` when you stage the final
intended tree and invoke `git commit`; inspect, re-stage, and retry after a hook
failure or mutation. Use `dev check-no-test` or `dev check` as explicit broad
diagnostic/integration commands, `dev prepush` through the push hook, and
CI/`dev validate-no-e2e` when hermetic confidence is the question. Focused
`test-local` is an accelerator, not a certification gate.

| Command               | Runs                                                                                                                                                                                                                        | Formatting    |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------- |
| `dev check-no-test`   | the host check groups (below) in Fix mode + auxiliary host tests                                                                                                                                                            | auto-fixes    |
| `dev check`           | adds `test-local`, Nix `wasm-tests` and `doctests`/`doctests-gate`                                                                                                                                                          | auto-fixes    |
| `dev precommit`       | Git/index custody selects Markdown or broad Fix checks, reconciles once, and conditionally verifies shared docs                                                                                                             | auto-fixes    |
| `dev prepush`         | clean tree; Verify host/static + auxiliary tests + `test-local` + Nix `doctests`/`doctests-gate`; x86_64-linux uses shared `static-docs` at the Markdown slot                                                               | never mutates |
| `dev validate-no-e2e` | clean tree; Verify host/static, the platform contract, Nix static proof (with `tools-doctests`), the `csrBundle` build and wasm budget, wasm tests, Rust coverage/CRAP, doctests, and authoritative Elisp producer/consumer | never mutates |
| `dev validate`        | adds six Chromium and six Firefox E2E lanes, reconciliation, and final authoritative SQLite/Chromium server-function flow verification                                                                                      | never mutates |

The host check surface that `check-no-test`, `prepush` and `validate-no-e2e`
share is five public groups, run in this order and stopping at the first
failure. `ci-validate-host` runs the same groups without the six Clippy units,
because CI takes its Clippy verdict from the hermetic lane's `static-code`
([CI Clippy verdict from Nix](docs/adr/0255-ci-clippy-verdict-from-nix.md)).
Each group can be run on its own to re-check one kind of failure; `mode` is
`verify` (the default) or `fix`:

| Group                       | Runs                                                                                   |
| --------------------------- | -------------------------------------------------------------------------------------- |
| `dev format [mode]`         | Rust/tools `cargo fmt`, `leptosfmt`, Prettier Markdown/`end2end`/CSS, Elisp formatting |
| `dev docs-shape`            | documentation sequence, ADR format and README/view parity, doc links, flow docs        |
| `dev source-contracts`      | ast-grep rule tests and `no-full-reload`, every repository source-contract policy      |
| `dev compile-checks [mode]` | `tsc`, `cargo-deny`, the six Clippy units                                              |
| `dev elisp-checks [mode]`   | Elisp byte-compilation and ERT                                                         |

`dev source-contract <name>` runs one source-contract policy, such as
`dev source-contract html-sink`. `dev nix-check <name>` builds one flake check,
such as `dev nix-check static-docs`, as a traced step with its cache state.

The CI host lane captures filesystem capacity/inodes, memory/swap and allocated
bytes for checkout-owned Cargo targets before validation. It does not traverse
the shared `/nix/store`: filesystem capacity comes from `df`, not a whole-store
`du`. Resource diagnostics do not replace or weaken the validation verdict.

Gate workflows have no gate-wide JSON report contract. Capture commands with
`traced-run -- dev <command>` and inspect its parked stdout/stderr separately;
read step timing from the run's trace (see
[Observation and traces](#observation-and-traces)). Keep producer-owned status,
reports, GC roots and diagnostics; a missing required producer artifact is not
success. For on-demand invalidation questions compare Nix derivation identities
directly; store reuse and elapsed time alone do not attribute compilation versus
substitution. Do not purge caches or run routine measurement campaigns.

#### Prepush parity by failure surface

The fast local lane and `dev validate-no-e2e` do not run the same tests in
different environments. They have the following explicit authority split
([ADR-0029](docs/adr/0029-git-enforced-verify-gate.md), #1117 supplement):

| `dev validate-no-e2e` surface                            | `prepush` coverage                                                                                            | Authority / rationale                                                                                                                                                    |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Verify-only host/static surface                          | Runs the same host surface except `x86_64-linux` replaces only `prettier-markdown` Verify with `static-docs`. | Prepush provides the Markdown leaf verdict; other Nix static proof remains authoritative for the sandboxed offline environment.                                          |
| Auxiliary `tools` non-doc tests                          | Runs once.                                                                                                    | The host tests are the applicable verdict; they are outside the application coverage/Nix test gates.                                                                     |
| Host-native product Rust tests                           | Runs `test-local`.                                                                                            | The host-native product verdict is cheap and cache-friendly.                                                                                                             |
| Root-workspace doctests and fence reconciliation         | Runs the Nix `doctests` producer and `doctests-gate`.                                                         | The same sandboxed verdict validation requests; there is no host doctest run.                                                                                            |
| Platform contract                                        | Does not run.                                                                                                 | Platform output boundaries and native shell/package evaluation are checked at validation, not per push.                                                                  |
| Nix static proof, including `tools-doctests`             | Only the shared `static-docs` leaf runs on `x86_64-linux`; other static derivations do not.                   | The leaf shares the docs formatting verdict; other static definitions still require the sandboxed offline environment.                                                   |
| Rust coverage/CRAP                                       | Does not run.                                                                                                 | Instrumentation and SQLite/PostgreSQL backend parity are part of the verdict.                                                                                            |
| Wasm browser tests                                       | Does not run.                                                                                                 | The verdict exercises wasm browser primitives unavailable to a cheap host lane.                                                                                          |
| Elisp coverage                                           | Does not run.                                                                                                 | The coverage VM is the authoritative execution environment.                                                                                                              |
| Wasm budget                                              | Does not run.                                                                                                 | The verdict is defined by the Nix-built artifact's size semantics: `dev` builds `.#csrBundle` as its own step and passes that output to `wasm-audit budget --site-path`. |
| Server-function flow verification (full `validate` only) | Does not run.                                                                                                 | This is an e2e/full-validate responsibility outside the `dev validate-no-e2e` comparator.                                                                                |

The Nix doctest producer/gate is the only execution of the root-workspace
doctests and reconciles their fence population in both directions; it may not
silently shrink it. `prepush`, `check` and validation all request that same
derivation.

Shared command definitions are not automatically shared verification verdicts.
The family decisions below retain useful sharing without replacing distinct
assurance. The table above and [Nix check inventory](#nix-vm-checks) describe
selected consumers, inputs and populations in detail:

- **Markdown/Prettier:** retain the x86_64-linux `static-docs` verdict shared by
  selected hook and hermetic consumers. Host Fix, complete raw/index admission,
  one staging reconciliation and post-request state checks remain host
  responsibilities; other platforms retain host Markdown verification.
- **Other formatting:** retain shared Prettier/`dev static` definitions and
  distinct host Fix/Verify versus hermetic Verify. Product/tools Rust formatting
  populations overlap within `rust-format`.
- **TypeScript/census:** retain separate compiler and E2E-population verdicts.
  The same named `tsc` definition serves host and hermetic callers, but neither
  its dedicated leaf nor shared dependency inputs replace the census contract.
- **Elisp static/ERT/coverage:** retain formatter/byte-compilation, standalone
  pure ERT and combined pure/live coverage. Repeated pure tests do not make
  these environments, populations or coverage responsibilities interchangeable.
- **Root/auxiliary Rust tests and Clippy:** retain fresh host product tests,
  explicit auxiliary suites and target-specific lint units. Auxiliary tests
  remain outside product coverage; lint compilation is not behavioral proof.
  Hooks and local gates run host Clippy; CI runs only the hermetic Clippy
  verdict, an attributable-cost decision under the deferral below (#2010). Host
  cargo-deny includes advisories; sandbox mode intentionally checks only offline
  bans, licenses and sources (ADR-0145).
- **Doctests:** execute the root fence population only in the hermetic producer,
  whose bidirectional reconciliation prevents population loss, plus the
  separate, unreconciled `tools-doctests` run of the tools workspace.
- **Rust coverage/CRAP:** retain the instrumented SQLite/PostgreSQL producer,
  hard status gate and host source-aware policy. A host test pass or a produced
  report alone cannot replace that verdict chain.
- **Wasm/browser/E2E:** retain distinct browser-primitive tests, artifact-size
  budget and diagnostic coverage. Full local E2E and CI share the enabled lane
  catalog, immutable producers and artifact reconciliation, while local dispatch
  and distributed scheduling differ. Focused host E2E is not the full matrix; a
  producer-output join is not the reconciled workflow verdict.

These decisions accept repeated host/hermetic checks rather than add universal
admission or orchestration machinery. Further sharing and dependency pruning are
deferred unless a concrete maintenance defect, consumer need or attributable
workflow cost justifies a separately scoped decision. Independent derivations
and narrow direct source filters alone establish neither equivalent verdicts nor
isolated transitive source/tool inputs.

For retained Markdown and full-local/CI E2E sharing, identical producer
selectors and declared-input Nix semantics establish structural identity, not
empirical whole-workflow equivalence. Actual successful hook/CI consumers,
representative changed-Markdown-input invalidation and executed admission,
topology, failure and output-handoff fixtures support the retained boundaries.
Literal runtime production-output reuse for either family and empirical E2E
relevant-input invalidation remain explicitly deferred, not demonstrated.
Revisit these observations only on a consumer/input-contract change, unexpected
reuse/invalidation behavior or concrete maintenance defect. No universal
workflow equivalence, speedup or cache attestation follows from this
disposition. Status-as-data still requires its verdict gate; missing, malformed,
incomplete or wrong-output evidence cannot pass. Output availability is not
proof of fresh execution.

Focused `dev test-local <nextest args>` and `dev e2e-local <spec-or-file:line>`
are the default feedback paths while editing. After a repair, rerun the same
targeted proof before broadening the question. Do not run a broad gate as a
routine per-helper or per-fix ritual. Normal pre-commit and pre-push hooks still
enforce their documented boundaries; focused proof does not replace them,
hermetic coverage/backend parity, or CI.

Choose `dev check-no-test` or `dev check` deliberately for a named broad
diagnostic/integration question: for example, whether a shared interface change
breaks host/static integration, or whether a repair affects other product Rust
tests and the wasm/doctest surfaces. Use explicit `dev validate-no-e2e` for
local non-e2e hermetic confidence and full `dev validate` when the complete
local matrix is needed. CI remains the authoritative integration proof.

`dev check` auto-fixes **formatting**, and nothing else; `dev validate` never
mutates the tree. The coverage gate is stateless (ADR-0050), so there is no
baseline to heal. `dev` returns nonzero on failure without a gate-wide result
envelope. `check`'s product Rust tests use the same host-native lane as
`test-local`, reusing the local target directory and the kache store shared
across worktrees.

CI does **not** run `dev validate` as a single job. It distributes the non-e2e
surface across `dev ci-validate-host`, `dev ci-validate-hermetic`,
`dev ci-validate-test-checks`, and `dev coverage`, and joins them under the
stable `Validate (no e2e)` result-only job. The coverage source-filter probe
(`dev coverage-check probe-source`) is an on-demand diagnostic, not an automatic
CI step.

E2E uses one catalog-derived matrix of sixteen isolated producers
(`dev e2e-producer <identity>`), four per backend/browser pair. An `always()`
reconciliation job per pair waits for the complete producer matrix, downloads
that pair's four artifacts and runs `dev e2e-reconcile` even if downloads fail;
the SQLite/Chromium job also verifies server-function coverage. Stable
`e2e gate` requires the catalog job, every producer, and all four
reconciliations. Running independent surfaces across runners cuts workflow
wall-clock; `dev validate` remains the full local equivalent and
`dev validate-no-e2e` remains its serial non-e2e form. See
[ADR-0034](docs/adr/0034-ci-e2e-matrix-distribution.md), the
[non-e2e validation lane decision](docs/adr/0192-split-ci-non-e2e-validation-lanes.md),
and the
[retained distributed topology](docs/adr/0203-distribute-ci-validation-and-firefox-e2e-lanes.md).

- `cargo fmt --check` checks Rust formatting.
- `dev static leptosfmt` invokes the pinned formatter on only `web/src` and
  `csr/src` (the same roots in Fix and Verify). It retains the default full
  macro paths `view` and `leptos::view`. Checked glob enumeration and UTF-8
  reads fail closed; the adapter also promotes the pin's per-file error marker
  even when its exit status is zero. Empty scoped populations are a no-op, not
  an implicit repository scan. A future frontend root requires an explicit
  scope, Nix-input and documentation update; this is not a syntax-placement
  policy. Rustfmt continues to cover Rust outside these roots.
- `prettier --check end2end`, `prettier --check '**/*.md'`, and
  `prettier --check '**/*.css'` check Playwright/frontend test assets, all
  tracked Markdown, and all tracked CSS respectively (`proseWrap: always`;
  scoped by `.prettierignore`).
- `doc-links` checks that every relative Markdown link in tracked `*.md`
  resolves to tracked repository content: file targets must be tracked, and
  directory targets must contain at least one tracked path. It excludes
  `docs/superpowers/` (in-flight specs and plans may link files they will
  create). Those planning documents still receive Markdown formatting.
- `thin-components` fails when a Leptos `#[component]` body carries more than
  **2** units of control flow on either of two surfaces: **setup** (the body
  outside any macro) and **view** (inside `view!`). A unit is `if`, `match`,
  `for`, `while`, `?`, `let … else`, or a guarded `match` arm — one per
  construct, so a 10-arm `match` is one and `else if` nests. The two surfaces
  are reported separately because the fixes differ: setup complexity belongs in
  a host-tested function, view complexity wants a subcomponent.

  **`<Show>`, `<For>`, and child components cost nothing**, deliberately — the
  cheapest way to satisfy the gate is the idiomatic Leptos form or a
  subcomponent. Note that a subcomponent is measured too, so it must itself be
  within budget; and extracting into a plain fn that returns `impl IntoView`
  does **not** count as a fix, because the gate measures only `#[component]`
  bodies and such a fn hides the branch rather than moving it somewhere checked.
  There is no suppression marker. See ADR-0083 for how to build a component that
  passes by construction.

- `elisp-fmt`, `ert`, and `byte-compile` run the elisp subproject's formatter,
  ERT suite, and warnings-as-errors byte-compilation under `emacs --batch` (see
  the Elisp subproject section below).
- Six discrete Clippy checks run through the named shared `dev static` checks:
  `product-core-clippy`, `web-server-clippy`, `web-no-server-clippy`,
  `client-host-clippy`, `wasm-clippy`, and `tools-clippy`. The local host ladder
  and hooks run them with host-local Cargo, the devshell toolchain and
  compile-cache environment. Hermetic Nix exposes one terminal derivation per
  unit and a narrow aggregate, which is CI's only Clippy verdict. Each unit
  derivation inherits a crane dependency build that runs the same `dev static`
  unit over a dummy source, so a Rust-only edit re-lints just the workspace
  crates; manifest, lockfile, `.cargo` config, vendored-source, toolchain or
  `tools/dev/` edits rebuild it (#2038). Product core excludes web, client, and
  csr as selected lint roots; their applicable server, featureless, host-CSR,
  and wasm targets are checked separately. `wasm-clippy` deliberately excludes
  `--all-targets`, because web's host-oriented test dependencies cannot compile
  for `wasm32-unknown-unknown`. Root and tools workspaces retain separate
  lock/test contexts (including `csr_bundle` and `performance` in tools). The
  former combined root web(server+CSR) configuration is intentionally not
  retained under approved request `2f0929a1-ebbd-4735-9a6c-b53e99a7a141`. The
  former `xtask-clippy` host step and Nix verdict are retired with the xtask
  workspace; host tools are linted by `tools-clippy`.
- `cargo nextest run` runs the default Rust unit and integration test suite.
- Whole-test budgets are **ambient** — an auto fixture gives every test a scaled
  `DEFAULT_TEST_BUDGET_MS`, which covers the entire suite (#270). Don't set one
  per test; a test that genuinely needs more is a signal to measure it first.
- For individual waits on slow browsers, use
  `slowBrowserTimeoutMs(testInfo, chromiumBudgetMs)` for an assertion and
  `slowBrowserFirstNavigationTimeoutMs(testInfo, chromiumBudgetMs)` for first
  navigation waits (see
  [ADR-0012](docs/adr/0012-environment-aware-timeouts.md)).

### Elisp subproject (`elisp/`)

The Emacs client lives in `elisp/` (see [`elisp/README.md`](elisp/README.md)).
Its ERT suite, formatter, and byte-compilation run in the verify ladder: `ert`,
`elisp-fmt`, and `byte-compile` (which byte-compiles the package modules with
warnings promoted to errors) are `dev check`/`dev validate` steps that run
through shared `dev static` checks for formatting, compilation and ERT — the
same implementations the `static-ert` and `static-elisp` Nix derivations run, so
`nix flake check` covers them too, with no duplicated sibling to drift (#188).
prettier cannot format Emacs Lisp, so `elisp-fmt` uses built-in
`emacs-lisp-mode` indentation (auto-fix under `check`, verify under `validate`).

`dev validate-no-e2e` builds `elisp-coverage-producer`, which runs the pure and
self-booting live ERT populations once in a hermetic VM. Its fixed
`$out/elisp-coverage/{lcov.info,summary.txt,status.json}` artifact set is
consumed on the host: the source-derived module/form census must reconcile with
LCOV, and each ordinary point must have exactly one LCOV record. The consumer
automatically counts as ignored/exempt, without a source marker, only a
zero-stop form with exactly one synthetic opening-line point that is `require`,
`provide`, `declare-function`, `defgroup`, or `cl-defstruct`, or that is
`defvar`, `defconst`, or `defcustom` with an absent, `nil`/`t`, number, string,
character, keyword, quote/function-quote, or literal vector initializer.
Computed calls, variable references, backquote/unquote, and all other evaluated
or unknown initializers remain measurable or need a trailing same-line
`;; cov:ignore: <reason>` with a non-empty trimmed reason. An ordinary point or
LCOV observation on a structural candidate is a guard violation. Controlled
producer outcomes (`ert-failure`, `instrumentation-failure`, and
`invalid-report`) and coverage findings fail the consumer; an uncontrolled Nix
or VM failure remains a failed build. Full `validate` inherits this verdict and
does not rerun live ERT.

### Observability and Performance Analysis

Jaunder uses OpenTelemetry for deep performance analysis (see
[ADR-0011](docs/adr/0011-unified-observability.md)).

#### Observation and traces

Every `dev` command records a trace of its run
([Python `dev` runner](docs/adr/0251-dev-runner-replaces-just.md)). The gate is
the root span and each step is a child span, nested as the gates compose. Child
processes join through the OpenTelemetry environment carrier
(`TRACEPARENT`/`TRACESTATE`) plus `JAUNDER_TRACE_DIR`, which names the run
directory; the root is the `dev` process that finds `JAUNDER_TRACE_DIR` unset.
`host-proof csr-prerequisite`, a step of `dev test-local`, writes its validation
phases into the same run; no other host-proof executable joins it. An
`e2e-producer` step adds its VM run's measured phases, read from the lifted
phase manifest, and the `coverage` step adds its producer's stages (below), both
as [producer-timed phases](docs/adr/0263-dev-traces-producer-timed-phases.md)
([observability](docs/observability.md)). A step span records:

- `jaunder.status`: `ok`, `failed`, `signalled`, or `incomplete` (set by the
  merge for a span with no end record or no parent, as when a child is killed);
- `process.exit.code` and `process.command_line`, capped at 1 KiB;
- `jaunder.cache`, for Nix build steps only: every Nix build `dev` runs as a
  step, including each `e2e-producer` step: `scripts/e2e-workflow` runs that
  build, and the step judges the one output its `build.json` selected;
- `jaunder.nix.output` and `jaunder.nix.fetched_bytes`, for the Nix build steps
  `dev` itself builds (not `e2e-producer`).

A Nix build step of a flake installable has two child steps: `eval`
(`nix path-info --derivation`) evaluates it, leaving the evaluation cached, and
`build` (`nix build`) builds it from that cache, so the flake is evaluated once
and the evaluation is timed. A derivation-path installable has only `build`.
`jaunder.nix.output` is the one output path the build prints, and
`jaunder.cache` describes only that path. `nix path-info` reports when the local
store registered it: `reused` if before the second the build started (after
`eval`), otherwise `built` if it is `ultimate` and `substituted` if not. Unless
it is `reused`, `jaunder.nix.fetched_bytes` sums the unpacked size of the paths
in the build's closure (`nix-store --query --requisites --include-outputs`) that
are not derivations, were registered from the start second on and are not
`ultimate`: what substituters supplied. A path the previous step registered in
that same second counts too; fixed-output downloads are `ultimate` and do not. A
path registered within the start second, a build printing several paths, or a
failed query records less, never more, and never changes the step. This assumes
the registration time is local, as for local builds and binary-cache
substitution: Nix keeps a source store's registration time when copying a path
in, so an output from a remote builder, an `ssh-ng://` or `local://` substituter
or `nix copy` may read as `reused`. It says nothing about the output's
dependencies, and no other step records a cache value.

Once the `coverage` step's `build` ends, `dev` reads `status.json` through the
`coverage` GC root. Every stage that started records `started_unix_ms` (wall
clock) beside `duration_ms`. `dev` writes each such stage that falls inside the
`build` step as a finished child of it, at those times, `ok` if the stage
succeeded and `failed` otherwise. So `build`'s untraced time is the Nix overhead
(fetch, unpack, install). A reused output's or an earlier run's stages fall
outside the step and are left out. A stage that never started, or a missing or
malformed `status.json`, yields no stage spans, and reading it never changes the
step. The census build also writes Cargo's `--timings` report, copied to the
output's `diagnostics/cargo-timing.html` when present; CI uploads it in
`validate-coverage-diagnostics`.

A Rust host tool comes from its flake package
([`dev` runs Nix-built host tools](docs/adr/0258-dev-runs-nix-built-host-tools.md)).
`dev` builds the package as its own Nix step, `<package>-build`
(`nix build .#<package>`), once per `dev` process, and then runs
`<out>/bin/<bin>` directly, so the tool's span times the tool and never the
build. A tool edit is picked up through the Git working tree, so a new tool
source file must be tracked (`git add`) before `dev` can build it. The tool's
own `cargo test` and `cargo clippy` remain its inner loop. `tools-tests-build`
and, when `dev` chooses the selection, `nextest-build` compile tests with their
run's own arguments and environment (`--no-run`) before `tools-tests` and
`nextest` run them.

The root span also records its runner where the machine exposes it:
`host.cpu.model.name` (`/proc/cpuinfo`), `jaunder.host.nproc` (the CPUs `dev`
may use, by Python's `os.process_cpu_count()`: the affinity mask `nproc` counts)
and `cloud.provider` (`aws` or `azure`, from the DMI system vendor; a local
Hyper-V machine reads as `azure`). In CI, setup-ci's first step exports
`JAUNDER_CI_JOB_START_NS`, which the root records as `jaunder.ci.job_start_ns`.
Neither elapsed time nor a missing `cache` attribute proves a fresh build,
substitution or newly executed Nix builder work. Nested or overlapping spans are
not additive CPU or human effort and must not be summed as such.

Every process appends span start and end records, one JSON object per line, to
its own `<pid>-<random>.jsonl` file in `.xtask/run/<run_id>.trace.d/`; the
format is locked by `tools/dev/tests/golden/span-records.jsonl` for both the
Python and the Rust writer. When the root ends, even after a failure, it merges
the records into `.xtask/run/<run_id>.trace.json` (Chrome trace JSON) with span
ids in `args` and overlapping siblings on separate tracks. Open that file at
<https://ui.perfetto.dev>. `dev trace summary [PATH] [--markdown]` reads a run
directory (the latest under `.xtask/run/` by default, or one downloaded from CI)
and prints the slowest spans, plus every span that did not end `ok`, with depth,
status, cache, fetched bytes and each parent's untraced time; overlapping
children are not summed. A runner line heads the table, and a `before dev` row
shows a CI job's time from its start to the root (setup-ci, earlier steps and
devShell entry). Only "Set up job" and checkout precede the mark, and the
trace-summary step, uploads and post-job cleanup follow the root. Spans go only
to these files; `dev` exports nothing over OTLP. Inside a Nix check the trace
lands in the build directory and is discarded; the host-side Nix step records
the cache result and, for coverage, the stages its producer timed.

Tracing never changes a step's exit status, stdout or stderr. If the trace
directory cannot be written, `dev` warns once on stderr and otherwise behaves
the same. A gate stops at the first failing step and exits with that child's
status; SIGTERM or SIGHUP sent to `dev` is forwarded to the running child, and a
child killed by signal n closes its spans as `signalled` with exit status 128+n.

Every CI validation and E2E job uploads `.xtask/run/` with `if: always()`,
hidden files included, for **14 days**, and appends
`dev trace summary --markdown` to its step summary. The summary is diagnostic
only and decides no job's result; when the job left no readable trace the step
adds a warning annotation instead. Missing files and upload failure warn visibly
without changing the workload verdict; required producer evidence and aggregate
checks remain authoritative, and optional diagnostics do not certify
completeness. Native CI logs keep their existing 14-day policy.

`traced-run -- PROGRAM` is a separate output-parking runner: it records no
spans, parks stdout/stderr in `.xtask/run/<id>.{out,err}` and prints a JSON
summary. Checkout-local parked output and traces are not pruned: they remain for
the worktree lifetime, with growing disk usage accepted. Monitor storage
explicitly. To keep a run beyond the worktree, copy its `<run_id>.trace.d/` and
`<run_id>.trace.json` to a destination you choose; copying grants no authority
to remove the source.

#### Repeatable performance harness

`dev perf [profile] [backends] [browsers] [workload] [posts] [authors] [revisions]`
produces fresh, versioned performance evidence. Defaults are `medium`,
`sqlite,postgres`, an empty browser list, `all`, and unset counts. Nix selects
producer descriptions from these native parameters and one fresh nonce; `dev`
builds and retains each producer serially, and the host-only `perf-data` command
validates and assembles the retained evidence. Rust does not schedule producers.
The default matrix for every profile is direct storage and browser work against
SQLite and PostgreSQL, with Chromium for browser work; it uses release-mode
application artifacts and runs the selected workloads in order. The canonical
comparison run is therefore:

```bash
dev perf medium
```

`small` (100 Posts, 10 authors, 500 revisions) is the fast smoke profile.
`medium` (5,000 Posts, 100 authors, 25,000 revisions) is the routine comparison
profile and the only canonical CI baseline matrix. `large` (50,000 Posts, 1,000
authors, 250,000 revisions) is an intentional stress run, not a routine
comparison:

```bash
dev perf small
dev perf medium
dev perf large
```

Count overrides support deterministic dataset-shape exploration without changing
the canonical profiles:

```bash
dev perf small sqlite,postgres "" all 1000 20 2000
```

`posts`, `authors`, and `revisions` each take a positive `u64`; an empty value
leaves that count unset. A supplied plan must have posts divisible by authors,
revisions at least posts, and more than 50 owner-history rows per author;
accepted custom datasets distribute their revisions deterministically. Any count
override marks the result noncanonical, even when its numeric value equals the
selected profile default, so it can be compared but the baseline importer
rejects it.

Select backends and browsers with comma-separated native parameters; duplicates
normalize to one selection. `workload` accepts `all`, `storage`, or `browser`.
An omitted browser list normalizes to Chromium only for browser work; an
explicit browser with storage-only work is rejected:

```bash
dev perf small sqlite "" storage
dev perf small postgres chromium browser
dev perf medium sqlite,postgres firefox browser
```

Nondefault normalized selections and all count overrides are visibly
noncanonical and cannot seed the baseline. Explicit default-equivalent selectors
remain canonical; baseline eligibility separately requires `medium`. Use
nondefault selectors and counts for focused investigation, not routine baseline
measurement.

Firefox remains supported for on-demand browser investigation, but it is not in
the routine baseline matrix. Storage workloads retain one cold sample then 30
warm samples for each workload/backend. Browser workloads retain 20 cold
action-to-semantic-ready samples, each in a fresh context with no pre-warm
navigation. Dataset provisioning is recorded separately and is not a measured
workload. The matrix covers initial and 80-percent-deep paginated read paths
plus revision detail for storage, and semantic-ready browser navigation and
history pagination. It is a read/rendering harness, not a write-throughput or
concurrency benchmark.

Each invocation creates a fresh nonce that is part of the producer derivation
identity; it never restores or publishes measurement derivations or result
outputs through a cache. Inspect the invocation-owned evidence under
`.xtask/performance/<nonce>/`: `performance-result-v1.json` is the authoritative
combined result. `selection.json` records Nix-selected identities and
`invocation.json` records retained-root metadata; each
`producers/<name>/performance/` tree retains `producer-status-v1.json`,
fragments, per-sample navigation/Playwright traces, and the correlated
server/storage `otel-traces.jsonl`. `dev` copies that complete tree before
status validation and before starting the next producer. A later build or status
failure stops later producers without deleting earlier evidence. The same
validated result is atomically copied to
`.xtask/performance/performance-result-v1.json` as the stable CI upload/import
entry point. CI uploads that complete tree as the `performance-result` artifact,
including whatever evidence exists after a producer failure.

When `tools/performance/baseline-v1.json` exists, compare a completed result:

```bash
dev perf-compare .xtask/performance/<nonce>/performance-result-v1.json
```

Comparison prints absolute median/p95 values and each sample's
minimum-to-maximum spread. A median or p95 increase of 20% or more is
highlighted as advisory evidence only: it does not fail the command or CI.
Missing samples, malformed fragments/results, incompatible identities, and
producer failures are real failures and must not be dismissed as timing noise.

There is intentionally no local baseline until a maintainer bootstraps one from
the first successful canonical `main` run. After a scheduled or manual
**Performance** workflow run succeeds, a maintainer imports its run ID:

```bash
dev perf-import-baseline <positive-run-id>
```

The importer fetches the `performance-result` artifact itself and accepts only a
successful `schedule` or `workflow_dispatch` run from `jaunder-org/jaunder`'s
`main`, with the exact `Performance` workflow and `performance` job identities
and a compatible complete result. Review and commit the resulting
`tools/performance/baseline-v1.json` diff normally; CI never writes the
baseline. A pull-request artifact, including a run started by the `performance`
label, may be downloaded and compared but can never be imported as a canonical
baseline.

- **No PII in telemetry**: span fields and the structured error boundary
  (`error.source`/`error.context`) must never carry user PII or secrets (emails,
  tokens, passwords, post bodies); use stable identifiers like `user_id`,
  `db.system`, and `error.kind` instead. See
  [ADR-0011](docs/adr/0011-unified-observability.md).

- **Span names identify operations; fields explain decisions**: do not encode
  branch outcomes into new span-name suffixes. Record bounded determinants on
  the narrowest span that owns the decision, declaring fields empty on the span
  and recording values when known. Keep meaningful called work as child spans
  when it is worth timing or has its own failure mode. See
  [docs/adr/0147-decision-path-observability.md](docs/adr/0147-decision-path-observability.md).

- **Trace Analysis**: Use `dev trace-analysis analyze` to process trace
  artifacts (JSONL) from VM runs or local tests.

  ```bash
  # Local e2e-local capture; use the exact path printed by the command.
  dev trace-analysis analyze \
    .xtask/e2e-local/<run-id>/chromium/capture/otel-traces.jsonl

  # VM captures are extracted from capture-<lane>.tar.gz; traces run does this.
  dev trace-analysis analyze \
    /path/to/sqlite-otel-traces.jsonl \
    /path/to/postgres-otel-traces.jsonl
  ```

  The analyzer reports on:
  - Slowest spans overall and per `e2e.test`.
  - Top action, navigation-phase, long-task, and resource-initiator hotspots.
  - Long-task totals and `e2e.test` duration by project.

  **Optional Filters**:
  - `--top N`: Limit the number of rows in each section.
  - `--trace TRACE_ID`: Analyze a specific trace.
  - `--project NAME`: Filter by browser (e.g., `firefox`, `webkit`).

- **Failure logs — look here first (#144)**: on a red E2E lane, read the scoped
  server-diagnostic log before the journal. It is a small JSONL file of only the
  server's **WARN+ events and panics** (no kernel/INFO noise), copied to
  `.xtask/diagnostics/e2e-<lane>/capture-<lane>.tar.gz` (which contains one
  `instance-<i>/diag.log` per server instance; in the VM:
  `/var/lib/jaunder/capture/instance-<i>/diag.log`). Panic records carry
  `"kind": "panic"`. The zero-panic gate
  ([ADR-0032](docs/adr/0032-e2e-zero-panic-gate.md)) sources `panicked at` from
  each instance's file unioned with that instance's journal;
  `jaunder-journal-instance-<i>-<lane>.log` stays captured as the last-resort
  fallback. See `docs/observability.md` for the JSONL shape.

- **Failed Nix checks**: direct `dev` requests report through command output;
  inspect the `traced-run` parked stderr/stdout or owning CI job log. Retained
  Rust producer APIs also write `.xtask/diagnostics/<check>/failure-excerpt.log`
  (#145): when present, read it before the accompanying `build.log`. It contains
  Nix's scoped `error:` block and de-interleaved failing-derivation tail. These
  producer-owned files remain uploaded by their owning jobs; a direct `dev`
  request does not promise a gate-generated excerpt.

- **WASM Audit**: Use `dev wasm-audit report` to measure the size of the
  frontend WASM and JS bundles from the deterministic Nix build. The budget gate
  never builds: `dev` realizes `.#csrBundle` as its own step, then runs
  `wasm-audit budget --site-path <that output>`, which requires the path.

  ```bash
  dev wasm-audit report
  ```

  Useful options: `--site-path P` to reuse a built bundle, or
  `--breakdown [--wasm F]` for per-section/per-crate attribution. The underlying
  host-proof `wasm-audit` prints one JSON report on stdout (tables in `report`,
  measurements in `evidence`); `dev` renders it.

- **Run & Analyze**: Use `dev trace-analysis run` to build the full VM e2e suite
  and immediately analyze the results.
  - Use `--single-worker` to build the workers=1 diagnostic packages instead of
    the gate checks, when the question is per-navigation cost rather than suite
    cost.
  - Use `--browser chromium|firefox` to restrict the run to one browser
    (default: both).

- **PR watching and approved land**: Use `dev pr-watch [N]` to follow required
  checks until the next caller-actionable outcome. A green, landable PR returns
  `ready-to-land`; an already armed or queued PR continues through queue
  processing. Omit `N` to resolve the open PR for the current branch. The sole
  observer is `dev pr-watch` itself (`tools/dev/dev/pr_watch.py`); it only reads
  through `gh` and never mutates. After obtaining and recording human approval
  for the exact PR/head/scope, use `dev pr-land <PR> <approved-head-SHA>`. Both
  identity arguments are required; the SHA must be full lowercase 40-hex. `dev`
  cannot attest to human approval. The thin native `gh`/`git` frontdoor requires
  origin `jaunder-org/jaunder`, live base `main`, matching PR/head, no divergent
  local PR branch, and a one-snapshot `ready-to-land` verdict from the same
  observer before one `gh pr merge --auto --merge --match-head-commit` action.

  The branch ruleset remains authoritative for direct required contexts and
  readiness. For an early failure, `pr-watch` uses workflow membership: a failed
  GitHub Actions check whose workflow file (read at the PR head) defines a job
  named after a ruleset context is immediately `checks-failed`, as if that
  context had failed. Any other failed check emits one explicitly optional
  warning with its log URL and remains non-terminal; an unusable workflow file
  also means optional, with its own warning naming the file and the reason (a
  transient read failure is retried on the next poll). That can only delay
  `checks-failed` until the required aggregate fails. Readiness still requires
  every ruleset context to succeed. Keep optional jobs out of workflows that
  define required contexts. The observer only reads this evidence: it does not
  cancel, rerun, rebase, enqueue, or otherwise mutate workflow jobs
  ([native observer decision](docs/adr/0264-pr-watch-native-python-observer.md)).

  ```bash
  dev pr-watch                             # wait for the next action
  dev pr-watch 731 --once                   # direct JSON, no --json flag
  dev pr-watch 731 --until merged           # passive external-approval watch
  dev pr-land 731 <approved-head-SHA>        # only after recorded human approval
  dev pr-land 731 <approved-head-SHA> --reenqueue # separately authorized recovery
  ```

  Outcomes are `ready-to-land`, `merged`, `checks-failed`, `ejected`,
  `dequeued`, `blocked`, `conflicted`, `closed-unmerged`, `stale`, `timed-out`,
  or `watcher-error`, plus `pending` when one-shot observation catches a PR
  mid-flight. They never collapse into one another: `dequeued` means an observed
  same-head queue entry vanished without a failed job in a current-head
  merge-group run, which `ejected` reports even while that run is still in
  progress; `timed-out` means GitHub never finished; `watcher-error` means the
  tooling could not establish a trustworthy verdict.

  `pr-watch` exits 0 for `ready-to-land` or `merged`; `pr-land` exits 0 only
  after complete exact-head `merged` evidence. Observer stdout is direct JSON:
  root `.outcome`, `.pr` (number), `.head_sha`, `.events`, and optional
  `.phase`, `.detail`, `.pointer`. There is no `--json` flag or result sidecar.
  Progress/diagnostics go to stderr. Adverse/pending observer outcomes exit 1;
  invalid arguments/setup failure exit 2 and may have no report. Land
  refusal/native failure is nonzero and may have no JSON; post-action
  observation retains its direct report/status. Honor exit status, not partial
  output. An arm or queue readback is not a merge verdict.

  `watch` only observes — it never merges, re-runs a job, rebases, or
  re-enqueues. `--until merged` explicitly keeps observing across
  `ready-to-land` while another actor may approve and arm the PR; it may consume
  the full timeout. **Land requires prior recorded exact PR/head human
  approval**; an invocation is not proof that approval exists. Land checks live
  head before action and verifies arm, queue or complete merged readback
  afterward. A silent no-op, failed action or uncertain readback stops without
  any retry/re-arm. Post-action observation binds all outcomes, even merged, to
  the supplied head. Positive same-head queue membership from native action
  readback is carried as read-only `--observed-queued-head <SHA>`, which must
  equal required `--expected-head`. Only positive live queued readback supplies
  this input; an arm alone or absent queue evidence does not. It seeds the
  observer's queue history, so a dequeue before the first poll is still reported
  as `dequeued`; no shell classifier or new poller is added, and default
  observation is unchanged. See
  [ADR-0087](docs/adr/0087-xtask-github-pr-observation.md) for the
  observer/armer split and
  [the actionable-handoff decision](docs/adr/0135-pr-watch-actionable-handoff.md)
  for the stopping policy.

  Default `land` refuses an observed ejection. Routine technical diagnosis is
  owned by the assigned Lead; recovery requires explicit authorization from the
  authority assigned by governing owner policy (the Manager where that policy
  reserves the decision, otherwise the Lead within approved scope). Diagnosis
  alone does not authorize recovery. Use
  `dev pr-land <PR> <approved-head-SHA> --reenqueue` only after that
  authorization and exact-head merge approval. It refuses queued/armed PRs,
  captures the historical run URL from live `ejected` preflight, and requires
  another one-snapshot `ready-to-land` verdict from the SAME observer with only
  that run acknowledged before its ONE native action. Incomplete, adverse or
  unavailable evidence refuses; there is no second classifier, poller or armer.
  Post-action observation uses
  `--expected-head <SHA> --acknowledged-ejection <URL>` on the same observer.
  These read-only inputs neither mutate nor authorize recovery. Acknowledgment
  requires an expected head and a GitHub Actions run URL; `dev pr-land` supplies
  the URL from preflight, not caller guesswork. The failed verdict stays failed;
  delayed fresh visibility uses the normal deadline, and a distinct fresh
  failure ends observation. Ordinary land refuses ejection; neither mode retries
  a no-op. Existing head, check, queue and fail-closed guards remain in force.
  See
  [explicit one-shot recovery](docs/adr/0220-explicit-one-shot-pr-reenqueue.md).

  **Post-merge ownership:** after `pr land` reports verified `merged`, choose
  the preservation or cleanup path by actual assignment ownership. Missing or
  conflicting ownership blocks affected cleanup.

  - **Manager-assigned work:** implementors and assigned Leads leave the
    semantic branch checked out and the checkout intact while the assignment is
    active. Skip `dev pr-cleanup` and equivalent detach/delete operations. Lead
    inspects and accepts merge, validation, and preservation evidence, then
    hands it to Manager: exact branch/HEAD and commits, dirty/staged/untracked
    inventory (including empty results), proof and artifact references,
    unresolved work, and the next owner/action. Delivered work can be complete
    while Manager-owned lifecycle cleanup remains pending; receipt alone is not
    acceptance or assignment release.

    Manager retains required branch selectors, reconciles exact current
    assignment/runtime identity, and owns authorized retirement through
    supported runtime routes. Moving the branch-deleting helper to Manager is
    not a safe handoff while the active assignment still needs that branch
    handle. A missing supported route is a Manager-owned capability blocker, not
    a dependency on a hypothetical runtime fix or a reason to withhold
    delivered-work evidence.

  - **Unmanaged work:** first positively establish that no unresolved required
    work, competing writer, or owned operation will be stranded. From that PR's
    checkout run `dev pr-cleanup <PR> <exact-head-SHA> unmanaged`. Both identity
    arguments are required; the ownership argument records the caller's positive
    assertion, not runtime discovery or proof of assignment ownership. Managed,
    unknown, or conflicting ownership means do not invoke it; those argument
    values are refused as an additional guard.

    Cleanup is explicit and local-only. It independently verifies complete
    merged PR evidence, target `main`, exact checked-out non-`main` branch/head
    matching the supplied PR/head, and a clean tree before fetching. It then
    proves the captured head is an ancestor of `origin/main`, detaches there,
    safely deletes only the captured branch with `git branch -d --`, and runs
    root `cargo clean`. Ordered stderr boundaries identify the first failure; no
    later step runs. Exit 0 and direct JSON `outcome: cleaned` establish
    completed cleanup and retain the captured head/merge commit/time. A refusal
    or failure is nonzero, never successful no-op. Preserve evidence, without
    force/reset/stash or bypass. Cleanup does not change the merge outcome,
    project Status, remote branches or local `main`, and does not authorize
    checkout deletion.

  Physical checkout removal is separate: it requires an explicit removal request
  and the runtime's interactive confirmation. Use the supported runtime route,
  never a direct CLI bypass, force, branch recreation, or external process kill.

  Set Status to **Done** manually only after verified eligible cleanup. Managed
  work hands remaining cleanup and tracker responsibility to Manager with an
  explicit next action; preservation handoff is not cleanup proof or permission
  to set Done early. If tracker readback is already Closed/Done, acknowledge it
  without undoing or reapplying it; that state is not cleanup proof or runtime
  retirement.

### Discovering milestone issue candidates

`dev issue-discover <number-or-exact-title>` gathers open issues, Jaunder
Backlog claim status, open native prerequisites, and local resume artifacts.
Discovery is read-only: it does not claim issues, acquire runtime ownership, or
dispatch work. An open milestone must match exactly and unambiguously by number
or title.

Every candidate and skipped row carries its Backlog `priority` (`P1`–`P4`, or
`null` when unset or the issue has no Backlog item), read in the same project
query as its Status. Candidates are listed in the order to work them: by
priority, `P1` first and unset last, then by issue number, so the first
candidate is the highest-priority unblocked, unclaimed issue. Skipped rows stay
in issue-number order. A priority outside `P1`–`P4`, or a reply missing the
Priority field, fails discovery like a malformed Status.

Success means the observed REST inventory was traversed through its last page,
including prerequisite pages before closed prerequisites are filtered out.
Candidate and skipped counts describe the gathered issues, excluding pull
requests. This is not an atomic snapshot: GitHub state may change during or
following discovery. Repeated issue identities across pages are refused rather
than silently deduplicated.

`gh api --paginate --slurp` owns REST traversal. The per-issue GraphQL project
read is bounded to 100 items and must explicitly report `hasNextPage=false`;
overflow or missing/malformed completeness evidence fails nonzero, not as false
absence. Any API, parsing, or later-page failure prevents a successful candidate
report; there is no partial-success mode. Do not interpret a failed read as an
empty or exhausted milestone. Stdout is the candidate/skipped JSON inventory,
not a host-tool step report.

### Creating and editing issues

Use one explicit frontdoor per operation, from the devShell:

```sh
dev issue-create 'fix(tooling): preserve metadata' Task 3 P2 tooling,dx /path/body.md
dev issue-edit 731 'fix(tooling): preserve metadata' Task 3 P2 tooling,dx /path/body.md 42
```

Arguments are title, GitHub type (`Task`, `Bug`, or `Feature`), exact open
milestone title/number, Priority (`P1`–`P4`), comma-separated topic labels, and
a UTF-8 body file. Edit takes the issue number first. All metadata is explicit;
edit replaces the supplied title/type/milestone/labels/body, rather than
inferring missing decisions. Files preserve body bytes, including trailing
newlines and angle placeholders. An empty body/label or readback mismatch fails.

Optional trailing numbers are explicitly chosen open `blocked-by` prerequisites
(42 in the edit example). The frontdoors use native dependency endpoints with
resolved integer database IDs and verify readback. Dependencies are additive;
editing does not erase pre-existing links. Do not supply closed prerequisites or
substitute prose for native links.

Both operations add/retain the issue in Jaunder Backlog (#1), set its explicit
Priority, and verify exact issue metadata/body and Backlog/Priority readback. An
issue already in the Backlog keeps its one existing item. A failed `item-add`
(as when project automation adds a new issue at the same moment) continues, with
a stderr note, only when the issue then has exactly one Backlog item. They do
not change claim Status. Stdout contains the verified issue JSON plus its
Backlog item; progress/failures go to stderr. Native `gh`/GitHub owns mutation
semantics. A nonzero result can follow partial application: inspect the printed
issue URL before retrying; no rollback or automatic duplicate issue creation is
attempted. Invalid/partial/API responses cannot produce success.

The three `dev` frontdoors replaced the former xtask issue dispatch/report
helpers. They compose native `gh`/`git` calls; they do not add a general command
runner or polling loop.

### Targeted Rust tests

The server integration tests compile as a **single** `integration` binary, so
run a subset by filtering on the module path rather than by picking a target.
Test paths follow the nested module path to the test name. A shallow file yields
`web::web_auth::<name>`; concern submodules add segments, as in
`web::posts::create::create_post_persists_rendered_published_post`,
`storage::posts::post_create_and_get_by_id_works`, and projector concern paths.
`misc::commands::…` is another nested example.

- One subsystem through the focused local lane:
  `dev test-local -p jaunder -E 'test(/^web::/)'` (or `atompub`, `feed`, `misc`,
  `projector`, `storage`).
- One file, concern module, or test (substring match):
  `dev test-local -p jaunder storage::posts::post_create_and_get_by_id_works`.
- One storage-crate test:
  `dev test-local -p storage site_config_primitives_round_trip`.
- Library/unit tests only, when PostgreSQL-backed integration setup is
  irrelevant: `cargo test -p jaunder --lib`.

`cargo nextest list -p jaunder` shows every registered Rust test with its full
`<subsystem>::…` path.

### Doctests and the fence vocabulary

`cargo nextest` structurally cannot run doctests. The Nix
`doctests`/`doctests-gate` producer/consumer runs the root workspace's doctests
(`dev prepush`, `dev check`, `dev doctests` and validation all request it). It
reads rustdoc fences with `syn` and **reconciles** that population against what
the runner reported, in both directions. A fence that exists but never ran is a
failure, and so is a reported doctest the scanner did not find. `tools/` is
deliberately outside the fence population: it has no doctests, and the separate
`checks.tools-doctests` runs its `cargo test --doc` without reconciliation.

Run them directly with `cargo test --workspace --doc`. Use `--workspace`, never
`-p`: package scoping silently drops the `#[cfg(feature = "sanitize")]` fences
in `common/src/render.rs`, because nothing in a narrower package set enables the
feature.

**Exactly three fence forms are accepted**, compared with whitespace removed:

| form                | means                                                  |
| ------------------- | ------------------------------------------------------ |
| ` ``` `             | a passing example — runs                               |
| ` ```compile_fail ` | a negative proof — must NOT compile                    |
| ` ```text `         | an illustration, not a test — not collected by rustdoc |

Everything else fails the gate, including `ignore`, `no_run`, `should_panic`,
and language tags. Two of those are actively dangerous: `ignore` **is**
collected and reported by libtest, so it silences a proof while still looking
like a test; and a wholly unrecognized word makes rustdoc skip the block **with
no warning at all**, so a typo deletes a proof and reports green forever. Fences
inside a multi-line `#[doc = "…"]` value are also rejected — libtest keys them
by the attribute's line plus a markdown offset, which the reconciliation key
cannot address.

**Every `compile_fail` needs a positive companion.** A `compile_fail` passes if
its snippet fails to compile _for any reason_ — a renamed path, an import that
stopped resolving — so it can rot into vacuous truth while still reporting
green. The rule:

- the negative carries its fixture as `#`-hidden lines (at least one non-empty),
  and
- every hidden line appears verbatim in a plain fence **in the same doc
  comment**.

`macros/src/lib.rs`'s `StrNewtype` docs are the worked example. Scoping to one
doc comment is deliberate: a file-wide rule would let one companion "cover"
negatives whose fixture it shares nothing with.

When writing a negative, ask what would happen if the property it forbids were
_added_. Assert on `.into()` rather than on a function argument — Rust never
coerces an argument through `From`, so `takes_foo(a_string)` fails for any two
distinct types and would keep failing even after the `From` impl the doc forbids
appears.

Test data that happens to be Rust must **not** be named `.rs`, or the gate will
police it; see `tools/doctests/testdata/`, which uses `.rs.txt`.

### PostgreSQL-backed Rust tests

The integration suite is backend-parametric via
[`rstest`](https://docs.rs/rstest): a storage behavior is written **once** and
annotated `#[apply(backends)]`, which expands it into two cases —
`::case_1_sqlite` and `::case_2_postgres`. A `Backend` enum selects the backend
through `Backend::setup()` (defined in `storage::test_support` — ADR-0033 — and
imported directly by the integration tests); genuinely single-backend tests use
`#[apply(sqlite_only)]` or `#[apply(postgres_only)]`. The HTTP-layer integration
tests are backend-parametric too: they use the same `Backend::setup()` fixture
(`#[apply(backends)]`, or a `#[values(Backend::Sqlite, Backend::Postgres)]` +
`#[case]` matrix for clustered rejection/authorization tests), so the **whole**
integration suite — storage and HTTP — runs on both backends per run; the old
env-selected `test_state` harness is gone. Both cases run in the same single
nextest pass. The consequence is that a bare `cargo nextest run` **requires a
reachable PostgreSQL**: the postgres cases connect to `JAUNDER_PG_TEST_URL`
(defaulting to `postgres://jaunder@127.0.0.1:55432/jaunder`) and fail if nothing
is listening. Each test creates its own database — a clone of a once-migrated
template (see `storage::test_support`) — so the cases run **in parallel**; no
`--test-threads=1` is needed. (The `#[template]`/`#[apply]` macros come from the
`rstest_reuse` dev-dependency, a companion crate to `rstest`: every module that
uses them needs its own `use rstest_reuse::*;` — a `use rstest::*;` glob alone
does not bring them in. See
[ADR-0124](docs/adr/0124-rstest-reuse-cross-module-templates.md).)

The simplest way to run them against a throwaway PostgreSQL is `dev test-local`,
which starts an ephemeral cluster through the devShell's `pg-run` script,
exports the connection env, runs `cargo nextest run`, tears everything down, and
applies the shared compile-cache environment. `pg-run` reaches the cluster over
a Unix socket in a short per-run directory under `XDG_RUNTIME_DIR` (or
`TMPDIR`); the exported URLs carry it as `?host=`
([pg-run as a Python script over a Unix socket](docs/adr/0249-pg-run-python-unix-socket.md)):

```bash
dev test-local
dev test-local -p jaunder
dev test-local -p storage site_config_primitives_round_trip
```

`test-local` is a `dev` step sequence, each step its own span in the run's trace
(`dev trace summary`):

1. `host-artifacts-lock` waits for the checkout-local
   `.xtask/host-artifacts.lock` (an exclusive `flock`, held by `dev` itself in a
   descriptor no child inherits) and keeps it until the run ends.
2. `csr-admission` (`host-proof admit-inputs .#csrBundle.sourceAdmits`) refuses
   relevant nonignored untracked source/producer/Nix inputs: use `git add -N` or
   `git add` to include them in the Git flake snapshot. Nothing is staged
   automatically; tracked dirty and intent-to-add inputs participate, unrelated
   notes do not.
3. `csr-bundle` realizes the native-platform `.#csrBundle` from the evaluated
   checkout as the GC root `.xtask/gcroots/csr-bundle` (not the incomplete
   `.#site`), recording its cache state. Nix and the configured Cachix
   substituter own artifact identity/reuse; no cached test verdict is used.
4. `csr-prerequisite` (`host-proof csr-prerequisite <root>`) validates the
   complete immutable output and prints its absolute root.
5. `image-worker` builds `target/debug/jaunder`, which storage and seeding tests
   start even when nextest selects only storage.
6. `nextest-build`, without args only, compiles the workspace's tests
   (`cargo nextest run --no-run --workspace`). Hand-typed args compile in
   `nextest`: nextest refuses some run options, such as `--no-fail-fast`, with
   `--no-run`.
7. `nextest` runs `pg-run cargo nextest run <args>` (`--workspace` without
   args).

Steps 5–7 share one environment, so build scripts that track it never rebuild
between them: `CFLAGS` set empty when absent (an inherited value is kept),
`JAUNDER_CSR_BUNDLE_DIR` set to the verified root and `JAUNDER_TEST_EXECUTABLE`
to the image worker.

An explicit `JAUNDER_CSR_BUNDLE_DIR` skips steps 2 and 3; step 4 validates it
instead. The first failing step stops the run, so admission, realization or
validation failure prevents compilation, without fallback to an older root or
`target/site`.

The lock guards the bundle's GC root and the image worker: without it a
concurrent `test-local` in the same checkout could repoint the root (leaving the
store path a running test uses collectable) or rebuild `target/debug/jaunder`
while tests start it. It stays, and it is per checkout: other worktrees have
their own roots and `target` directories. A second `test-local` in the same
checkout waits in `host-artifacts-lock` until the first finishes.

This prerequisite is release-built, so browser debug assertions are omitted and
a first cache miss can be expensive. Host tests and the actual image worker
remain debug-built and execute freshly. `check` and `prepush` share this lane.
Ordinary `e2e-local` uses the source-consistent immutable Nix artifact set
(described under [Running e2e tests locally](#running-e2e-tests-locally));
sandbox uses its own per-invocation set. Standalone `build-csr` keeps its own
CSR producer, and `test-local` its lock. Product-test work remains host-native
and invokes no Nix check derivations.

The same lane builds the locked host `jaunder` executable and supplies its
absolute path as `JAUNDER_TEST_EXECUTABLE` to storage and seeding tests that
launch the private image worker. HTTP integration tests use Cargo's explicit
`CARGO_BIN_EXE_jaunder` instead. Hermetic coverage selects the corresponding
instrumented executable; sandbox seeding receives its built server artifact.
This is a test/seeding input, not a production upload setting. Direct test or
seeder invocations must supply the matching real application executable; there
is no test-harness executable, PATH-search or no-op fallback.

An explicit `JAUNDER_CSR_BUNDLE_DIR` is intentional input: a missing or invalid
Vite manifest/output, rendered index file, or system-artifact inventory fails
before test compilation, with no fallback production. Supply it only from
matching resolved producer source inputs, toolchain and profile, or
independently verified equivalent inputs. The validators establish internal
integrity, **not source provenance**; bundle hashes, existence or matching HEAD
alone do not establish source equivalence. Arbitrary cross-source reuse is not
supported. The input's owner must consent to keeping it stable throughout
compilation; checkout-local advisory locking cannot protect a foreign root or an
uncooperative direct producer. Cooperating host artifact preparation uses the
same lock. Bare/dependency compilation may still produce an empty non-production
embed; that permission does not make a runnable test fixture deployable.

To use a persistent instance instead (e.g. the dev VM), set the env yourself:

```bash
nix run .#postgres-testing-vm
export JAUNDER_PG_TEST_URL=postgres://jaunder@127.0.0.1:55432/jaunder
export JAUNDER_PG_BOOTSTRAP_TEST_URL=postgres://postgres@127.0.0.1:55432/postgres
cargo nextest run -p jaunder
```

Each per-test database is dropped on teardown — an RAII guard issues
`DROP DATABASE … WITH (FORCE)` when the test's `TestEnv` / `PostgresDbGuard`
goes out of scope — so a persistent instance no longer accumulates
`jaunder_test_*` databases across a normal run. A test process killed before its
guards run (a hard crash or `SIGKILL`) can still leak one; the ephemeral wrapper
avoids the issue entirely by discarding the whole cluster.

### Coverage and dependency policy

- Coverage is gated **host-side by `coverage-check policy`** (`tools/coverage`,
  nondefault `host` feature), which reads the Nix `coverage` check's report. It
  is part of `dev validate`, `dev validate-no-e2e`, and CI, not `dev check` or
  `dev prepush`. The local push hook stays fast while CI remains the
  non-bypassable hermetic backstop.
- `cargo deny check` verifies dependency policy, advisories, and licensing (run
  as one of the static checks).

The gate is **stateless**: its verdict is a pure function of
`(coverage report, source tree)` — there is no committed baseline, anchor, or
manifest. Each run builds a fresh `cargo llvm-cov` report inside the Nix
`coverage` check and classifies every executable source line. An **uncovered
line fails the gate** unless one of two things exempts it:

> Leptos CSR UI needs no exemption. A `#[component]` lives in a wasm-only
> `component.rs` behind `#[cfg(target_arch = "wasm32")] mod component;`
> (ADR-0070), so it is **not compiled for the host at all** and its lines never
> enter the denominator. The `#[component]` / `#[client_only]` structural
> exemption that used to cover them was retired in #520 (ADR-0050, amended). To
> get component-adjacent logic _covered_ rather than merely absent, extract it
> into an ungated, host-tested file — the ADR-0070 §6 convention that
> `web::reactive::Invalidator`, `forms::Field`, and `tags::input_state` follow.

- **Structural exemption — `unreachable!("msg")`.** A literal `unreachable!`
  invocation carrying a **non-empty message** is dropped from the executable set
  with **no marker**. It is _self-enforcing_: reaching the line panics ⇒ the
  test fails ⇒ `cargo llvm-cov` exits non-zero ⇒ the `coverage` check produces
  no report — so, unlike a `cov:ignore` on a reachable line, you cannot silently
  cheat coverage on live code. It is _message-required_ (a bare `unreachable!()`
  stays **measured**, forcing an explicit justification, mirroring `crap:allow`)
  and **fail-closed** — `std::unreachable!`, aliases, and macro-generated forms
  are not recognized and stay measured. Use it for provably-dead lines (a match
  arm a domain invariant makes unreachable) in preference to a `cov:ignore`: the
  marker is a permanent, prose-only promise, whereas an `unreachable!` re-flags
  itself the moment the line ever goes live.
- **`// cov:ignore: <specific reason>` marker.** A line explicitly marked as an
  accepted gap.

`cov:ignore` is the **only** manual acceptance path — there is no baseline file
to edit, and every marker is visible and reviewable in the diff where it lives.
The presumption is to remove the gap with consumer-observable coverage or a
simplifying refactor. Retain a marker only when authoritative host coverage
cannot honestly exercise its exact span: non-host behavior, a genuinely
unreachable path, fault injection that is impractical to arrange, generated or
build-script code, or compiler bookkeeping. The source-local reason states those
facts; a category label by itself is not a reason. Point-in-time audit
dispositions are review evidence in the pull request and issue, not a committed
repository snapshot.

It has two forms:

- **Line form** — `// cov:ignore: <specific reason>` as the line's _real_
  trailing comment. The reason must be non-empty. The matcher is anchored to the
  actual `//` comment, so a marker appearing inside a string or doc comment does
  **not** suppress.
- **Block form** — `// cov:ignore-start: <specific reason>` …
  `// cov:ignore-stop` around a contiguous region, for lines that cannot carry a
  trailing comment (mid-expression lines, inside multi-line/raw string
  literals). The start reason must be non-empty; the stop takes no reason or
  trailing text. Nesting is not allowed, and a legacy bare/empty line or start,
  unmatched start, stray stop, or noncanonical stop is a **hard error** (the
  gate fails loudly rather than leaving an open-ended blind spot). There is no
  arbitrary block-size cap: semantic breadth is reviewed from the exact
  behavior, not formatted line count.

**CRAP** (Change Risk Anti-Patterns — cyclomatic complexity weighted by test
coverage) is gated by a **per-function threshold, T = 30**: any function whose
CRAP score exceeds T fails the gate unless it carries a
`// crap:allow: <reason>` marker somewhere within its line span. Like
`cov:ignore`, the override is a reviewable in-source marker, not hidden config,
and the reason is **required** (a `crap:allow` with an empty reason is not a
valid override). A fully-covered function's CRAP equals its cyclomatic
complexity, so legitimately complex, well-tested functions can sit near T and
use the override. There is no `crap-manifest.json`. Progressive tightening of T
is the remit of the _Code quality improvement_ milestone (#232+), not a
per-commit knob.

**`#[server]` flow coverage is a separate gate** (#681, ADR — _empirical
server-fn flow coverage_). Line coverage says a fn's body ran under _some_ test;
this says a real browser session actually drove the endpoint. It is derived from
the OTLP traces of the four reconciled SQLite/Chromium e2e lanes, extracted per
lane and unioned. Its sole committed generated artifact is
`docs/coverage/server-fns.json`, which records the covered `<vertical>::<ident>`
set plus orphan reasons and is compared byte-for-byte. Do not hand-edit it:
`regenerate` rewrites this snapshot, and `verify` recomputes and compares it
from an authoritative capture. The static lane reads the snapshot together with
the source-derived server-fn inventory.

**Per-test attribution is structural but intentionally not persisted (#757).**
The extractor still uses the propagated `e2e.test` span id to distinguish
requests driven inside a test from unattributed orphan traffic. It does not
commit test titles, because their sets are not reproducible: a test that ends
mid-navigation leaves its page booting, and the boot is truncated at a different
point each run. An uncompared title file could also go stale and therefore
cannot serve as durable flow proof. If request-to-test attribution is needed,
inspect a fresh trace capture; do not infer it from the committed snapshot.

**Adding a `#[server]` fn therefore requires an e2e flow that drives it**, or
the fast lane reddens immediately — `server-fn-coverage` runs in `dev check` and
`dev validate-no-e2e`, so you do not wait for the e2e matrix to find out:

`dev e2e` verifies coverage automatically as part of the SQLite/Chromium
reconciliation; it never regenerates the snapshot. For intentional regeneration,
use the single explicit command:

```bash
dev e2e-regenerate-coverage
```

It freshly runs the four SQLite/Chromium producers, then reconciles their lifted
artifacts before writing the snapshot. Producer, reconciliation or artifact
failures remain failures; snapshot verification is deliberately replaced by
regeneration only for this explicit action. Ordinary `dev e2e` and
`dev e2e-producer` remain verification-only.

The artifact-input CLI
(`e2e-evidence server-fn-coverage {verify,regenerate} --catalog <catalog> --diagnostics-root <root>`)
reads only already-lifted `e2e-<identity>` directories. It first reconciles the
SQLite/Chromium pair: the exact enabled lane set, an identical complete census
equal to the lanes' disjoint report union, and a successful phase sidecar per
lane. Only then are the captures read. A capture whose server `diag.log` records
an OpenTelemetry SDK span loss (`BatchSpanProcessor.*`), or that has no
`diag.log`, is refused as incomplete: a lost request span would otherwise
surface as an orphan (#2029). Each producer replaces its own lifted files before
it builds, and the local workflows that reach coverage rerun the producers
first. These checks do not claim detection of arbitrary hidden capture
omissions; no extra receipt or attestation schema is involved.

One further property is worth knowing before you fight the gate:

- **Endpoint drift is no longer writable.** `#[macros::server]` derives the
  endpoint from the fn's file path and identifier (#714, ADR-0082), so there is
  no bare `#[server]` to reject and no hand-written literal that can disagree
  with the fn. The gate computes the same value the macro does, and the
  committed seed capture is cross-checked against real captured traffic so the
  two derivations cannot silently diverge.

When a test needs a second browser context, use the `tracedContext` fixture,
never `browser.newContext()`. The `traced-context` check enforces this: a raw
context does not inherit the config-level `extraHTTPHeaders`, so its traffic
carries the run-wide traceparent, cannot be attributed to your test, and would
silently under-report coverage.

**Source contract.** The coverage build measures your **working tree**, bounded
to cargo sources — there is no stage-first ritual for the normal flow. An
untracked `.rs` file under an instrumented crate **is** measured (in the normal
dirty dev flow, and once staged/committed), so its uncovered lines are reported
like any other code. Untracked non-source junk (e.g. a stray `.txt`) does
**not** affect the build, because the coverage `src` is filtered to cargo
sources — with an explicit admission for `csr/index.html`, a compile-time
`include_str!` — and a `drvPath` probe guards that filter against silent drift
(#37).

Coverage counts source lines with at least one execution hit across all test
binaries, deduplicating generic-function instantiations across compile units
(unlike the inflated `cargo llvm-cov --json` summary percentages for code
exercised by multiple test binaries). It runs the entire test suite in a single
nextest pass with an ephemeral PostgreSQL available for the whole run, so the
SQLite-backed and PostgreSQL-backed integration tests execute together (no
`#[ignore]`, no `--run-ignored`) and `storage/src/postgres/*` gets real
instrumented coverage. Reported line coverage is the union of both backends.
This — and thus backend parity — runs inside the Nix `coverage` check.

`dev check` runs in Fix mode and auto-formats (fmt/leptosfmt/prettier), but the
pre-commit hook runs `dev precommit` so formatter changes to already-staged
clean tracked files are safely restaged. Fix mode no longer heals any coverage
state — there is none to heal. Accepting a gap (`cov:ignore`) or waiving a CRAP
score (`crap:allow`) is a manual source edit that lands as a reviewable diff;
`dev validate` is verify-only.

**Protection tradeoffs (stated honestly — not "stricter").** The stateless gate
dissolves a class of fragility — the verdict is identical at any checkout depth,
with no text-identity guessing after a rebase — but on two axes it is
**equivalent-or-weaker** than the old baseline ratchet, by deliberate design:

1. **CRAP is weaker below T.** A per-function threshold is blind to a function
   that worsens but stays under T (e.g. 5 → 29); the old manifest
   regression-check caught that. This argues for keeping T tight (the _Code
   quality improvement_ grind-down).
2. **`cov:ignore` is permanent.** Unlike the ratchet — which tracked
   covered-state and re-flagged a line that went covered → uncovered — a
   `cov:ignore`'d line that later becomes covered and then regresses is never
   re-flagged. The migrated markers bake in permanent (but in-source, greppable,
   reviewable) blind spots.

These are accepted: component UI is covered by e2e, all non-exempt code still
fails on any uncovered line, and the deleted machinery's fragility outweighed
the marginal ratchet protection on already-accepted lines. (A third tradeoff —
"component bodies are weaker", where a new uncovered line inside a
blanket-exempt component body passed silently — was **removed** in #520 rather
than mitigated: components no longer host-compile, so there is no blanket
exemption left to be weak.) See
[the stateless-coverage-gate ADR](docs/adr/0050-stateless-coverage-gate.md).

Some areas are inherently uncovered host-side and are accepted rather than
force-fitted with artificial tests:

- **Leptos view components** (`web/src/<vertical>/component.rs`): `#[component]`
  bodies render view trees, validated by the e2e matrix — **not host-compiled**
  (the file is wasm-only by its `mod` declaration), so no marker and no
  exemption are needed.
- **WASM entry point** (`csr/src/lib.rs`): runs only in the browser WASM context
  — `cov:ignore`'d.
- **Compiler-generated asset embed expansion** (`server/src/site.rs`, the
  `RustEmbed` derive only): `cov:ignore`'d. Handwritten asset handlers are
  measured; their pure response logic and real embedded lookups have tests.
- **A few PostgreSQL storage error branches** (`storage/src/postgres/*`):
  unreachable or impractical to exercise host-side — `cov:ignore`'d. A
  provably-dead branch here is a candidate for `unreachable!("msg")` instead
  (self-enforcing, no marker); migrating the existing markers is tracked in
  #245.

**Source-filter drift guard (`dev coverage-check probe-source`).** The Nix
`coverage` derivation's `src` is filtered to cargo sources (plus an explicit
`csr/index.html`), so untracked junk can't bust its hash (#37/#231). This
on-demand probe guards that filter against silent drift: in an ephemeral
worktree it asserts that staging a filter-**excluded** file leaves
`coverage.drvPath` unchanged (else the filter re-admits junk — the #37 impurity)
and that staging an **instrumented** `.rs` changes it (else the filter drops
source — a coverage hole the stateless gate can never see, because those lines
aren't in the report). It evaluates the **committed (HEAD)** filter, so it
guards what CI/PRs carry, not local uncommitted edits. Subtlety worth knowing: a
_new_ file must be `git add`-ed to be measured — nix ignores untracked files
even on a dirty tree. The probe runs only on request, not automatically in CI;
it is deliberately **not** part of `check`/`validate` (#241).

**Playwright/WASM coverage evidence (`dev coverage-check wasm-probe`).** This
manual host command realizes the independent Chromium and Firefox diagnostic
producers before deciding its outcome. It retains each unpacked producer root
under `.xtask/wasm-coverage/`, validates the `v1` status documents and complete
digest manifests fail-closed, and writes the aggregate verdict to
`.xtask/wasm-coverage/status.json` even when one browser fails. It only invokes
the compiler-matched `llvm-profdata`/`llvm-cov` from `.#wasm-coverage-csr` to
count-sum profiles when both browsers passed with matching module and toolchain
identities; otherwise no merged evidence is produced. The command exits zero
only for two source-mapped browser reports and a valid merged Rust-line report.

**WASM coverage overhead measurement
(`dev coverage-check wasm-measure --quiescent-window '<coordinated window acknowledgement>'`).**
This is a manual, coordinated-host command, never a gate. It refuses an empty
acknowledgement, warms the same-pinned-nightly baseline and instrumented
producer for each browser, then retains five alternating baseline/instrumented
pairs per browser. Every producer receives invocation-owned cache-buster entropy
which is part of its Nix derivation and retained evidence. The command writes
`.xtask/wasm-coverage/measurement/manifest-v1.json` only after reconciling all
20 fresh runs: producer/browser identity, alternating order, unique
cache-busters, nonempty realization evidence, focused-flow duration, and raw
uncompressed `pkg/jaunder.wasm` bytes. The baseline uses the same pinned
nightly, source closure, bundling, service and browser flow, but necessarily
omits instrumentation/minicov and diagnostic exports—the precisely measured
differences.

**Static-check derivation split.** The hermetic static surface has separate
semantic derivations, not a singular `static-checks`: `static-docs` owns
Markdown formatting and its configuration; `static-typescript` owns the named
`tsc` check; `rust-format` owns `fmt`, scoped `leptosfmt`, and `tools-fmt`;
`static-ert` owns standalone pure ERT; `static-elisp` owns Elisp formatting and
byte-compilation over its complete non-Markdown Elisp source filter;
`static-e2e-census` owns the existing provision-and-census command; and
`static-code` owns the remaining shared `dev static` definitions. TypeScript
formatting remains in Code. The census leaf retains the existing Playwright
listing configuration, test/helper/fixture files, including module-load assets,
and package manifests/lock inputs. Its direct source filter excludes unrelated
repository sources; its actual closure still includes the existing
whole-`end2end` `e2ePackage`, the shared Nix setup executable, and pinned
Node/Playwright packages. Changes to those inputs can invalidate it
transitively; no compiled tools executable or its Rust tooling closure is a
census input. This is documented coupling, not a claim of full source isolation.
The TypeScript split changes neither the independently landed Rust formatting
boundary nor distinct feature/target/tools Clippy commands, and makes no
equivalence, permanent deferral or closure claim. The census source filter's
inline assertions establish membership of the census script, Playwright
configuration, helper/fixture/font and package manifest/lock inputs, plus
exclusion of Markdown and unrelated server source. The CSR/site source closure
contains workspace manifests, the resolved `csr` target closure, its required
shell inputs, and deterministic workspace placeholders. The client wasm-test
closure analogously contains workspace manifests, the resolved `client`
wasm-test target closure, and its required inputs/placeholders. `common` and
`macros` remain wherever a resolved target reaches them; unrelated `server`,
`storage`, and host-only sources are absent from both closures.

`static-docs` runs the shared `dev static prettier-markdown` check directly,
without a compiled Rust docs tool. The documentation and site source filters
remain narrow; derivation identity includes transitive executable inputs as well
as admitted sources. Tooling preparation uses generated dummy sources and a
file-scoped lockfile, with repository-root target alignment and nonempty archive
admission for tool consumers. Release artifacts are not assumed to seed
dev-profile clippy. Stable identity is not proof of compiler freshness or
local/remote realization reuse.

Artifact compatibility includes compiler, target, profile, resolved features,
flags and native inputs; incompatible work must rebuild. Cache legitimate build
inputs aggressively. Every other Nix verdict, for example formatting, lint,
Clippy, type checks, ERT, the census, doctests and `dev-python`, is sandboxed
and input-addressed, so a stamp substituted for an identical derivation is
acceptable. Coverage and e2e verdicts are never substituted, and CI executes
them for every tested commit: the Cachix `pushFilter` keeps them out of the
cache in CI, where install-nix-action's `always-allow-substitutes = true`
overrides their `nonSubstitutable` wrapper
([substitutable static verdicts](docs/adr/0252-substitutable-static-verdicts.md)).
A push to `main` of a commit whose merge-queue run passed reuses that run's
verdict rather than executing them again
([merge-queue verdict draft](docs/adr/0260-merge-queue-verdict-covers-landed-commit.md)).

`dev` gate workflows return process status and parked logs, not the historical
JSON invalidation envelope. The Nix source filters assert representative
membership inline (the coverage filter also has
`dev coverage-check probe-source`); do not infer execution from reused outputs.
`dev validate` adds all sixteen retained E2E lanes, their reconciliation, and
server-function coverage verification to `dev validate-no-e2e`.

### Nix VM checks

`nix flake check` runs the full Nix-backed validation matrix, including:

- `checks.x86_64-linux.static-docs` — `dev static prettier-markdown --sandbox`,
  with the sandbox `dev` package, pinned Prettier and admitted Markdown and
  formatter configuration inputs; no Rust docs tool.
- `checks.x86_64-linux.static-typescript` — `dev static tsc --sandbox`, with the
  sandbox `dev` package and pinned compiler, its own validation verdict, and
  explicit dependency setup. The filtered `tools/dev/` source is admitted
  alongside the narrow direct TypeScript/config/manifest population, which still
  has the accepted whole-end2end transitive coupling through `e2ePackage`.
- `checks.x86_64-linux.rust-format` — `dev static rust-format --sandbox`:
  product `fmt`, scoped `leptosfmt`, then `tools-fmt --all`. Product/tools Cargo
  populations overlap, so these share one verdict. Flake checks select it, and
  hermetic validation requests it between `static-typescript` and residual
  `static-code`. Direct inputs include Rust sources, Cargo manifests/locks/build
  configuration, Rust toolchain and formatter configuration. Its real closure
  also includes compiled `repository-policy` for the narrow formatter adapter,
  both workspace offline Cargo homes and the pinned toolchain/formatter, with
  their transitive sources and vendor inputs. The adapter package also admits
  common/macros for the Org extractor; sharing a package is a real invalidation
  and dependency cost, not source isolation between its binaries. Valid
  local-store outputs may be reused; request duration is not evidence of current
  formatter execution, and no speedup or narrow-cache claim follows.
- `checks.x86_64-linux.static-code` — residual non-Clippy shared static
  definitions outside TypeScript, Rust formatting, standalone ERT, the census
  verdict and Elisp formatting/byte-compilation (including end-to-end
  formatting, cargo-deny, and repository scanners), executed with existing
  offline Cargo homes. It requests the narrow Clippy aggregate once; that
  aggregate depends on six independently buildable terminal verdicts and
  executes no lint itself. Each verdict reuses its own cached dependency build
  and re-lints only workspace crates. `dev static-all` runs the ordered,
  duplicate-free union of TypeScript, Rust formatting, Elisp, ast-grep,
  cargo-deny, the six Clippy units and ERT; Prettier is outside this inventory
  and called directly by `dev`/Nix.
- `checks.x86_64-linux.static-elisp` — shared `elisp-fmt` Verify followed by
  shared `byte-compile`, over the complete non-Markdown Elisp source tree in a
  writable copy. The sandbox `dev` package, `emacsForCi`, and `TZDIR` replace
  the former compiled build dependency. It needs no offline Cargo home,
  Node/Playwright, or Git. This selected change does not certify every family
  coupling or discharge the separately recorded invalidation deferrals.
- `checks.x86_64-linux.static-e2e-census` — Nix's `setup-e2e-node-modules`
  followed by `node end2end/e2eCensusContract.ts`, preserving the existing
  census population independently of `tsc`. Its direct sources are the
  non-Markdown `end2end` tree (Playwright config/projects, tests, fixtures,
  helpers, and module-load assets) plus the census script. The existing
  `e2ePackage` supplies the transitive pinned npm closure, which is broader than
  the direct census filter and may invalidate the leaf. It is requested once by
  hermetic static validation; the standalone host `dev static-all` population is
  unchanged.
- `checks.x86_64-linux.static-ert` — `dev static ert --sandbox`, the standalone
  pure runner over the complete non-Markdown Elisp tree, in a writable copy,
  with the sandbox `dev` package, existing `emacsForCi`, Git and timezone
  inputs. The separate combined pure/live coverage producer below is unchanged.
  Hermetic validation requests `static-docs`, `static-typescript`,
  `rust-format`, `static-code`, `static-elisp`, `static-e2e-census`,
  `static-ert`, `dev-python` and `tools-doctests`, in that order. ERT and TSC
  are traced like every other `dev` step; the old compiled timing events are
  retired. No new Nix child-timing collector is introduced. Retained logs are
  never timing inputs; Nix request duration and output reuse do not prove fresh
  builder execution.
- `checks.x86_64-linux.dev-python` — the `dev` package's own gate: ruff,
  `mypy --strict`, and pytest under coverage.py at `fail_under = 100` with
  branch coverage, subprocesses included. New `dev` code needs behaviour tests
  that keep it at 100%.
- `checks.x86_64-linux.wasm-tests` — `wasm-bindgen-test` cases executed in
  headless Nix-store Chromium through the matching Nix-store chromedriver. This
  is behavioral pass/fail execution for irreducible browser code, not wasm line
  coverage.
- `checks.x86_64-linux.coverage` and `.coverage-gate` — the instrumented test
  suite (SQLite and PostgreSQL together under an ephemeral PostgreSQL) and the
  verdict over its report. **This is where the Rust test suite runs**; there is
  no separate `nextest` check.
- `checks.x86_64-linux.elisp-coverage-producer` — one hermetic NixOS VM runs
  both ERT populations and realizes
  `$out/elisp-coverage/{lcov.info,summary.txt,status.json}` for controlled
  outcomes. `dev validate-no-e2e` lifts those artifacts and invokes the
  authoritative host consumer, which reconciles the production module/form
  census with LCOV and accepts only trailing same-line `;; cov:ignore: <reason>`
  markers with non-empty trimmed reasons.
- `checks.x86_64-linux.doctests` and `.doctests-gate` — the root workspace's
  doctests and the fence reconciliation over them
- `checks.x86_64-linux.tools-doctests` — a plain `craneLib.cargoDocTest` of the
  tools workspace, without fence reconciliation, over its own source
  (`toolsDocTestSrc` in `nix/packages.nix`) in which every tools member is real,
  reusing the shared tools dependency build
- `checks.x86_64-linux.e2e-{sqlite,postgres}-chromium-{1,2,3}-of-3` and
  `checks.x86_64-linux.e2e-{sqlite,postgres}-firefox-{1,2,3,4,5}-of-5` — three
  Chromium and five Firefox shards per backend, each over the browser's one
  project
- `checks.x86_64-linux.e2e` — the existing browser-only Nix aggregate. Direct
  realization does not perform the host artifact verdicts. `dev e2e` (also
  called by full `validate`) realizes the individual lane outputs above and then
  performs every reconciliation.

Additional Nix-backed checks available as packages (not run by default):

- `packages.x86_64-linux.e2e-{sqlite,postgres}-{chromium,firefox}-single-worker`
  — the historical unsplit browser/backend diagnostic flows pinned to
  `JAUNDER_E2E_WORKERS=1`, so per-navigation timings carry no worker contention.
  Built on demand by `dev trace-analysis run --single-worker`; not part of the
  gate.

The PostgreSQL-backed integration tests need no VM of their own: they run inside
the `coverage` derivation alongside the SQLite ones, against an ephemeral
cluster started for the whole run. Per-test databases isolate them, so they run
with libtest's normal in-process parallelism.

#### `e2eSalt` — the measurement cache-buster (#792)

A literal near the top of `nix/checks.nix`'s e2e section, for **performance
measurement runs**, not for normal development. It is committed empty, which is
a byte-exact no-op, and the `e2e-scaffold` static check fails the gate if it is
left set.

Nix caches the e2e check derivations, so re-running `dev trace-analysis run` on
an unchanged tree returns a **cached result and never runs the suite** —
silently handing back traces from whenever that check was last built, quite
possibly a CI runner under unknown load. Set the salt to a distinct value per
measurement run to force a genuinely fresh build:

```nix
e2eSalt = "run3";   # any distinct string; the value itself is never read
```

The salt rides the lane's extra-env string into the VM test script, so it
changes every E2E derivation hash — sixteen retained gate checks, four unsplit
Firefox workers=2/workers=4 control packages, and four historical single-worker
packages. It does **not** change `packages.x86_64-linux.jaunder`:
`nix/checks.nix` sits outside the crane source filter owned by
`nix/packages.nix`, so a salted run re-runs the VM suite without rebuilding the
Rust workspace.

**Revert it before committing.** Nothing fails if you don't, which is precisely
the problem: a committed salt costs every CI e2e job its cache, and the only
symptom is "CI got slow". Note this repo's worktrees may auto-stage edits, so
revert with `git checkout HEAD -- nix/checks.nix` **and**
`git reset HEAD -- nix/checks.nix`, then confirm with
`git diff HEAD -- nix/checks.nix`.

For matched arms, interleaving and interpretation, see
[Controlled experiments](docs/observability.md#controlled-experiments).

If you only need one of the VM-backed checks, you can run it directly:

```bash
nix build .#checks.x86_64-linux.e2e-sqlite-chromium-1-of-3
nix build .#checks.x86_64-linux.e2e-postgres-firefox-2-of-5
nix build .#packages.x86_64-linux.e2e-sqlite-firefox-single-worker
nix build .#packages.x86_64-linux.e2e-postgres-firefox-single-worker
nix build .#checks.x86_64-linux.elisp-coverage-producer
```

## Code conventions

- Use Rust for product code. End-to-end tests use Playwright and TypeScript.
  Choose the language for development and CI tools by how maintainable the code
  will be, not by startup, compile or port cost
  ([docs/adr/0261-tool-language-by-maintainability.md](docs/adr/0261-tool-language-by-maintainability.md)):
  - A tool is Python, unless it links product crates it genuinely needs or needs
    Rust-specific parsing (`syn`, wasm sections).
  - A tool that needs a product fact (an inventory, a manifest, a format
    version) gets it from a product command or file, not by linking product
    code. If there is no such interface, add one.
  - A format written by Rust and read in another language is validated against a
    JSON Schema generated from the writer's types, committed with a drift check.
  - Bash is allowed only for host scripts in `scripts/` that run commands and
    check their results, use traps only to remove temporary files or report the
    failed step, and get concurrency only from a standard tool (bounded
    `xargs -P`, a `tee` the script waits for). Signal forwarding, supervising
    long-lived children, or data shaping beyond `jq` means Python. Shell inside
    Nix builders and in GitHub workflow `run:` steps is outside this rule.
- All Rust code is formatted with `cargo fmt`.
- Files containing Leptos `view!` macros are additionally formatted with
  `leptosfmt` (run it first, then `cargo fmt`).
- Follow Conventional Commits: `<type>: <imperative summary, ≤72 chars>`, where
  `<type>` is one of `feat`, `fix`, `refactor`, `perf`, `docs`, `test`, `build`,
  `ci`, `chore` (optional scope, e.g. `fix(storage): …`). Reference the GitHub
  issue the commit addresses inline in the subject or body, e.g.
  `docs(adr): promote the role-tagged site URLs ADR to 0112 (#875)`; the issue
  carries milestone/epic context, so the subject needs no `M`/`§` prefix. **No
  commit trailers.**
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
- Never erase an **unexpected** failure into absence, a domain miss, a default,
  or a successful fallback. Propagate infrastructure/I/O/browser/subprocess/
  invariant/decode failures with their typed source. If preserving the primary
  result or an intentional degradation requires continuing, use the owning
  runtime's swallowed-error reporter and add a why-comment; the reporter emits
  the warning and metric atomically. Expected validation/domain rejection may
  still become ordinary control flow. This is a semantic rule, not a ban on
  `.ok()`, `unwrap_or`, `let _`, `Err(_)`, or `map_err`. In host tools and
  in-derivation tools, fail correctness-affecting operations and write a
  contextual stderr warning only for legitimate ancillary/cleanup failure. Never
  report a diagnostic transport/export failure through itself.
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

- **Comment against the present, not the past.** State intent and why as claims
  about the tree as it stands, so a reader can check them by reading it. Do not
  argue with a previous implementation, a deleted test, or a removed file ("used
  to", "no longer", "replaces `old_fn`"): the other half of that contrast lives
  only in `git log`, so the comment is unverifiable the day it is written and
  archaeology a year later. Keep the issue number; drop the narrative. Two
  carve-outs: a present-tense fact that happens to use past-shaped words ("a
  blank title is no longer expressible — the type forbids it") is fine, and
  backward _compatibility_ with live data ("markers written before #591 lack
  `is_operator`") describes the present world, not code history.
- **A comment that outgrows a few sentences is usually a decision record.** If
  it carries a rejected alternative, a measured trade, a workaround's root
  cause, or an accepted risk, promote it to an ADR — draft it in
  [`docs/adr/drafts/`](docs/adr/drafts/README.md) per "Adding an ADR" above —
  and leave a one-line pointer. Module-level `//!` docs are comprehensive by
  design and exempt from this pressure; an inline comment should be as short as
  a true statement of intent allows.

## Storage and web conventions

- Use `sqlx` for database access.
- Support SQLite and PostgreSQL, dynamically selected at runtime.
- Store SQL migrations in `storage/migrations/{sqlite,postgres}` using the
  `000x_description.sql` numbering convention, one per backend for the same
  change.
- Define storage traits such as `UserStorage` and `SessionStorage`, plus their
  record types, in `storage/src/*.rs`. This lets `web` and `server` use them
  without circular dependencies.
- Keep backend-specific SQL in the `storage` crate's per-trait dialect modules
  under `storage/src/{sqlite,postgres}/`, behind the shared generic store — see
  [Crate layout and the generic store pattern](docs/ARCHITECTURE.md#crate-layout-and-the-generic-store-pattern)
  in the architecture view
  ([ADR-0019](docs/adr/0019-generic-storage-backend-via-dialect.md)).
- Use specialized storage error enums from the `storage` crate, such as
  `UserAuthError` and `CreateUserError` (`storage/src/users.rs`), with
  `thiserror`.
- Use `sqlx` unique violation checks (`is_unique_violation()`) to handle
  "already exists" errors gracefully.
- In web server functions, retrieve the exact per-trait handle you need
  (`expect_context::<Arc<dyn UserStorage>>()`); no heterogeneous application
  state bundle crosses a runtime seam.
- **Dependency injection / composition-root invariant (see
  [ADR-0016](docs/adr/0016-dependency-injection-and-appstate.md)):** No type may
  be both (a) a heterogeneous dependency holder and (b) passed beyond the
  composition root. Declare a component's dependencies as constructor parameters
  on the component that uses them — do not introduce a shared bundle to make a
  dependency reachable. `StorageFactory` stays at composition roots and mints
  exact storage handles and `WriteScope` values; it is never injected into a
  subsystem. How the wiring is built is in
  [Dependency injection and composition roots](docs/ARCHITECTURE.md#dependency-injection-and-composition-roots).
- The web framework is Leptos in CSR (client-side-rendering) mode (ADR-0040);
  the wasm bundle is built by `dev build-csr [debug|release]` (default: debug)
  and served by `jaunder serve` (no cargo-leptos). `dev` runs it from the
  checkout root that `git rev-parse --show-toplevel` finds, including from a
  nested directory. After a successful locked wasm build, it recursively removes
  the fixed old `target/site` output, including regular files and dangling final
  symlinks, before running the checkout's `system-artifacts` tool and the CSR
  asset script. Final symlink referents are not removed; this is ordinary
  recursive removal, not a physical-containment guarantee. Invalid profiles fail
  before build or cleanup, and any failed stage stops the build.
- Leptos components should only render data; business logic belongs in server
  functions or pure transformation functions.
- API methods are automatically prefixed with `/api`.
- `web` is organized as co-located verticals; put each new file in the leaf its
  role dictates. The split is described in
  [Module layout — the per-vertical file split](docs/ARCHITECTURE.md#module-layout--the-per-vertical-file-split)
  (ADR-0070).
- Define `#[server]` functions in their vertical's `api.rs`
  (`web/src/<vertical>/api.rs`, e.g. `web/src/auth/api.rs`) and declare them
  `#[macros::server]` — spelled fully-qualified, never `use`d. It derives the
  wire endpoint, the ADR-0011 span name, and the error-boundary wrap, and
  hard-errors on any other file, which is what makes the `(vertical, ident)`
  pair a key **rustc** enforces (ADR-0082). Use a single grouped
  `#[cfg(feature = "server")]` block for server-only imports; the bodies
  themselves carry no cfgs. See
  [Server-fn surface, DI, and errors](docs/ARCHITECTURE.md#server-fn-surface-di-and-errors).
- Convert storage errors to `leptos::prelude::ServerFnError` using
  `.map_err(|e| ServerFnError::new(e.to_string()))`.
- Use `require_auth().await?` at the start of any server function that requires
  a logged-in user. It returns an `auth::User` containing `user_id`, `username`,
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

### The `test-backend-pattern` guard (enforced across `server/tests` and `storage/src`)

The `dev check`/`dev validate` static pass runs a `test-backend-pattern` guard
that scans every file under `server/tests/` and `storage/src/` and fails if a
`#[tokio::test]` (including parameterized `#[tokio::test(flavor = …)]` forms) is
not declared backend-explicit. This covers both `server`'s integration tests and
the `storage` crate's own in-file dialect/home-module tests (widened to
`storage/src` in #135). Every DB-touching test must carry exactly one of:

- `#[apply(backends)]` — dual-backend, single-axis (the test takes
  `#[case] backend: Backend`).
- `#[apply(backends_matrix)]` — dual-backend for a test that ALSO has its own
  local `#[case]` matrix (the `#[values]`-based template; the test takes a plain
  `backend: Backend`). Use this when `#[apply(backends)]` would collide with
  local `#[case]` rows.
- `#[apply(sqlite_only)]` / `#[apply(postgres_only)]` — a deliberately
  single-backend test. It MUST live in the matching `sqlite/` or `postgres/`
  dialect directory (ADR-0053 §1 "home by what it proves"; a dual
  `#[apply(backends…)]` test conversely must NOT live in a dialect dir), and it
  MUST carry a `// reason:` comment stating the backend-specific behavior the
  other backend can't exhibit (e.g. a SQLite lock-flake reproduction, a
  Postgres-only `pg_database` teardown). "It currently hardcodes SQLite" is NOT
  a valid reason — convert such a test instead.

A genuinely **non-DB** async test (touches no live pool) is exempt via a
`// guard:no-backend — <reason>` comment immediately above its `#[tokio::test]`
— e.g. a `server` integration test exercising real router/middleware wiring, or
a `storage` async unit test over a mock store, `DbConnectOptions` routing, or
password hashing. A `server` test that is really a **unit test** (a pure
function/extractor, no router or DB) belongs in a `#[cfg(test)] mod tests` in
the owning crate, not in `server/tests`.

A test that does **low-level DB work that cannot go through the `backend`
fixture** — bootstrap-admin DDL (`CREATE ROLE`/`CREATE DATABASE`), a
from-scratch `unique_postgres_url()` database, or driving both engines in one
body (cross-backend interop) — is instead a bare `#[tokio::test]` carrying
`// guard:low-level-db — <reason>` (no backend template). Home it by what it
proves: a one-database low-level test in its dialect dir, a cross-backend one in
a generic module.

Finally, a templated test MUST **use** its injected `#[case] backend` — a
`let _ = backend;` (or `#[case] _backend`) discard is rejected. Either use the
backend, or drop the template for a `// guard:low-level-db` bare test per above.

Pure synchronous `#[test]` unit tests are never flagged.

## NixOS integration

The narrow `nixosModules.jaunder` module is the building block for custom NixOS
deployments; it owns only Jaunder's systemd service:

```nix
{
  imports = [ inputs.jaunder.nixosModules.jaunder ];

  services.jaunder = {
    enable = true;
    bind = "127.0.0.1:3000";
    db = "sqlite:/var/lib/jaunder/data/jaunder.db";
    prod = true;
  };
}
```

For the supported single-host composition, import `nixosModules.jaunder-stack`:

```nix
{
  imports = [ inputs.jaunder.nixosModules.jaunder-stack ];

  services.jaunder.stack = {
    enable = true;
    hostName = "jaunder.example.com";
    database = "sqlite"; # or "postgresql" for the host's local instance
  };
}
```

The [NixOS deployment runbook](docs/DESIGN.md#nixos-deployment-stack) owns
operator procedures, including PostgreSQL boundaries, UI/SSH access, retention,
persistence and backup, and optional Basic Auth. Inline Basic Auth usernames
match `[A-Za-z0-9._-]+`. Inline `username`/`passwordHash` remain compatible,
with algorithm inference; the literal evaluated hash enters generated Caddy
configuration and the Nix store. Production deployments should prefer
`basicAuth.credentialsFile`, an absolute runtime filename string, with both
inline fields empty and file-only `credentialsFileAlgorithm` (`bcrypt` by
default, or `argon2id`). Never read or embed those contents in Nix. File mode
requires configured systemd >=260 and uses a private `LoadCredential` copy with
native NixOS Caddy startup/reload parsing and provisioning, without a custom
content validator. Supply native account entries and use `caddy hash-password`
through stdin for the selected algorithm. Both ordinary reload and restart can
refresh/rotate credentials; source replacement alone does not activate them.
Failed reload retains parsed authentication, not necessarily the old credential
mount; a successful empty-file reload instead activates deny-all, and malformed
hash internals are not necessarily startup errors. The runbook owns the native
recipe, provisioning, recovery and runtime trust boundary details.

Do not set `JAUNDER_CAPTURE_DIR` in production. That is test-only and should
stay in the interactive VM or e2e test node config. The `jaunder` CLI is
installed for the `jaunder` user via `users.users.jaunder.packages`.

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
  tests" above. For a self-contained run, prefer `dev pg-run`.
