# Issue #1977: which remaining tools should stay in Rust

## Outcome

Each of the four remaining candidates in #1977 gets a recorded verdict and
reason. The language policy changes so that maintainability decides the
language, and it names when shell is allowed. Each approved move becomes its own
follow-up issue. No tool code changes in this cycle.

## Criterion

The question is ongoing maintainability: how readable and changeable the code is
for what it actually does. Startup and compile time no longer separate the
languages: ADR-0258 serves Rust host tools as prebuilt Nix packages, and Python
runs from the checkout. Port cost is noted but does not decide a verdict.

A tool earns Rust when it:

- links product crates it genuinely needs, such as a storage benchmark driving
  `storage`; or
- needs Rust-specific parsing, such as `syn` over Rust source or wasm section
  parsing.

A format written by Rust and read in another language does not make the reader
Rust. The writer publishes a JSON Schema generated from its own types, and each
reader validates against that schema. The coverage and doctest status readers on
#1977's "Rust clearly pays" list are not reviewed here.

Process sequencing, subprocess calls (`gh`, `git`, `nix`), JSON and YAML
shaping, statistics and state machines are Python's ground. When a host tool
links product code only to learn a fact (an inventory, a manifest, a format
version), that is a missing or unused product command or file interface. Fix the
interface instead of linking.

The #1976 pilot (ADR-0249) is the calibration: about 390 non-test Rust lines
became 232 lines of Python.

## Verdicts

| Candidate                                  | Size (non-test / test lines) | Verdict                                             |
| ------------------------------------------ | ---------------------------- | --------------------------------------------------- |
| `e2e-local` lifecycle (`tools/host-proof`) | ~2,350 / ~1,160              | Move into `tools/dev` as Python                     |
| `pr-observer`                              | ~5,270 / ~5,350              | Rewrite in Python, starting with a feature audit    |
| `perf-data` (host side of `performance`)   | ~2,000 incl. tests           | Move to Python, against a generated schema          |
| `qualification`                            | ~5,790 / ~2,500              | Retire. Replace it with a scheduled CI upgrade test |

**e2e-local.** It runs a fixed sequence: collector, server, seed, Playwright,
with cleanup and signal handling. That is the pg-run shape. The Rust needs two
Tokio runtimes as a sync/async bridge, error types that carry the session back
for cleanup, and ~400 lines of per-branch failure records. Python's `Popen`,
`killpg` and `ExitStack` say the same thing directly. It shares no product
types. It is the only user of tokio, processkit and xshell in host-proof. The
move must preserve:

- INT/TERM exit as 130/143, and a second signal escalates.
- Readiness checks `runtime.json` against the child's pid and start time.
- Collector retry when its port is taken.
- The capture is copied only after its writers stop.
- `verify-no-panics` still runs.
- stdout is reserved for the JSON report.
- The whole process tree is reaped on every exit path.
- The aarch64-darwin start-time token (via `libproc`), the main cost the move
  accepts.

It drops the test-only `selection` path in `e2e_provenance.rs` (`:249-274`).

**pr-observer.**

- It calls `gh api` as a subprocess about 15 ways, putting a deadline on each
  call through tokio and processkit.
- It parses untyped JSON by hand (`str_at`, `required_string`), so Rust types
  add little.
- It hand-rolls RFC3339 dates.
- It reimplements GitHub Actions' job graph and matrix expansion from `ci.yml`
  to classify each check as required, transitively required or optional. It also
  string-matches a `jq` command in `ci.yml` and then runs it itself.
- It matches failures against other PRs by log signature.

All of that is Python's ground. Much of the hard-to-follow code is scope, not
language, so the rewrite begins by auditing which features still earn their
place. Its 190 tests are the behavior inventory the rewrite covers or
deliberately drops. Behavior that is kept still honors ADR-0087, 0135, 0195,
0198, 0220 and 0240, including the exit-code and JSON contract that `pr-land.sh`
relies on.

**perf-data.** The host CLI links no product code. Its dependencies are `serde`,
`serde_json`, `clap`, `anyhow`, `thiserror` and `tempfile`. Its work is
assembly, cross-checks, statistics, comparison and the `gh` baseline import, all
Python's ground.

The `performance` library it is built on is shared with two writers that must
stay Rust:

- **`test-support`** seeds the benchmark data through `storage`. It takes the
  dataset plan from `performance` and writes the dataset manifest
  (`test-support/src/performance.rs:33`).
- **`perf-producer`** drives `storage`, writes the storage fragments, and
  validates browser fragments inside the VM guest (`nix/checks.nix:1292`).

The manifest already has a second, hand-written reader in TypeScript
(`end2end/tests/browser-performance.ts:219-272`).

So the split is:

- **Rust keeps** the writer side: the dataset plan, the manifest and fragment
  types, `test-support`'s seeder and `perf-producer`. That part of `performance`
  publishes JSON Schemas generated from its own types for the dataset manifest,
  the fragments and the combined result.
- **Python** (the `perf-data` commands, as a `dev` module or a package next to
  it) does assembly, comparison and baseline import, validating against those
  schemas.
- **TypeScript.** The browser benchmark's manifest checks validate against the
  same schema rather than restating it by hand.

The generated schemas are committed, and a check fails when they drift from the
Rust types (ADR-0109).

The move must preserve:

- each fragment's manifest and positions are cross-checked;
- noncanonical runs are rejected for the baseline;
- for baseline import: admission before download, provenance bound to the run's
  head SHA and attempt, medium-profile canonical-only results, and an atomic
  write;
- comparison stays advisory only.

**qualification.**

- **History.** It was added 2026-09-12 (#1450, PR #1469) for #1419, which is
  still open. It has run twice: a discovery on 2026-09-12 and the #1748 proof on
  2026-10-15. It also carries a `qualification` feature in `host` and in
  `system-artifacts`.
- **What it links, and the existing alternative for each:**
  - The theme inventory, with its sha256 digests and `/theme/{digest}` URLs,
    checked through `load_system_artifact_inventory`: the deployed server's
    responses, compared with goldens.
  - The CSR outputs: Vite's `manifest.json`.
  - ETags: the deployed server's response headers.
  - One backup-format constant: readable from a backup file.
- **What it does by hand.** It boots QEMU, Caddy and `curl` probes itself. That
  is the NixOS VM test driver's job, and its test scripts are Python. The repo
  already has 7 driver test definitions; one of them generates every E2E lane.
- **The replacement.** A scheduled CI job uses that driver to deploy an earlier
  build through the NixOS module, upgrade it to the current commit, and compare
  the result against golden files that each commit regenerates.
- **The follow-up's own spec decides:**
  - which earlier build to upgrade from;
  - what the golden set contains;
  - where goldens are stored;
  - whether one test can deploy two immutable revisions.
- **Styling.** The synthetic `a`/`b-app`/`b-theme` fixtures (ADR-0239) are
  dropped. Their rules are already tested in ordinary CI on both backends:
  install and retention (`server/src/commands/lifecycle.rs:1375`,
  `storage/src/theme_asset_manager.rs`), presentation across upgrade and
  rollback (`storage/src/public_theme.rs:1445`), immutable 200 and exact-ETag
  304 (`server/tests/theme_content.rs`), retired `/style/...` routes, restore
  (`server/tests/misc/backup_fixture.rs`), and Home isolation
  (`end2end/tests/theme-presentation.spec.ts:67`). The upgrade test keeps one
  browser across the upgrade, and checks that every stylesheet URL in the served
  pages returns 200 with immutable headers and that the earlier build's
  stylesheet URLs still resolve.

  The approved capability loss: a regression in how stylesheets change across an
  upgrade surfaces only on a run where CSS really changed between the two
  builds, not on every run.

- It retires `tools/qualification` and both `qualification` features when the
  replacement lands.

## Language policy (`CONTRIBUTING.md` "Code conventions")

Replace "Rust … by default for infrastructure", and allow Python "when it
produces more focused" code, with the criterion above:

- Infrastructure is Python, unless it links product crates it needs or needs
  Rust-specific parsing.
- Product facts come from product commands or files.
- A cross-language format is validated against a JSON Schema generated from its
  Rust writer's types.

This is a criterion-based policy change made through ADR review, not the
"blanket language-policy exception" that milestone 33 excludes.

Shell is named narrowly, for host scripts in `scripts/` only. Shell inside Nix
builders and `run:` steps in GitHub workflows is out of scope. Bash is allowed
when the script:

- runs commands and checks their results;
- uses traps only to remove temporary files or to report which step failed;
- gets concurrency only from a standard tool, such as bounded `xargs -P` or a
  `tee` the script waits for.

Signal forwarding, supervising long-lived children, or data shaping beyond `jq`
means Python. All six current scripts conform:

- `pr-land.sh` (polling is delegated to `dev pr-watch`);
- `pr-cleanup.sh`;
- `build-csr-assets.sh`;
- `e2e-workflow` (`xargs -P`, plus a `tee` it waits for);
- `git-add`;
- `test-theme-repository-release-version.sh` (`RETURN` cleanup traps).

These amend ADR-0249's "no shell exception is added". ADR-0249 rejected bash for
exactly the capability this rule excludes: forwarding signals to a child.

## Deliverables (committed)

- An ADR draft (`jaunder-adr`, numberless) recording:
  - the criterion;
  - the four verdicts with their reasons;
  - the shell rule, which amends ADR-0249.

  The criterion and the shell rule are projected into `docs/ARCHITECTURE.md`
  (`jaunder-adr-projection`). The per-tool detail stays in the ADR. ADR-0028
  needs no change: ADR-0240 and ADR-0246 have already superseded its tool
  placements.

- The `CONTRIBUTING.md` language-policy change.

## Tracker actions

- File four follow-up issues, one per move, each with the preservation list
  above and the ADRs it would amend:
  - e2e-local: ADR-0246 `:64-69`, ADR-0247, ADR-0258.
  - pr-observer: ADR-0240, ADR-0258.
  - perf-data: ADR-0241, ADR-0251, ADR-0258, and the `performance` description
    in `docs/ARCHITECTURE.md`.
  - qualification: ADR-0239, ADR-0225, ADR-0247, ADR-0251, ADR-0258.
- Comment on the existing issues:
  - **#1974:** e2e-local and its supervision code leave host-proof.
    `csr-prerequisite` and `admit-inputs` stay. host-proof sheds tokio,
    processkit and xshell.
  - **#1975:** retiring `qualification` removes one consumer of
    `repository-policy`'s git helpers.
  - **#1419:** its acceptance run waits on the replacement upgrade test.
  - **#1977:** the verdicts.
- #1973 is closed and unchanged, because `diagnostic-build` stays in Rust.

## Acceptance

- The ADR draft and the `CONTRIBUTING.md` change are on the branch, and
  `dev check-no-test` passes.
- The four follow-ups exist, and the comments on #1974, #1975, #1419 and #1977
  are posted.
- Conformance review finds a verdict and reason for each candidate, the policy
  criterion, and the shell decision.

## Boundaries

- No tool code is moved, deleted or refactored here.
- No other tools are reviewed.
- Follow-up designs (the pr-observer audit, the upgrade test's golden set)
  belong to their own specs.
