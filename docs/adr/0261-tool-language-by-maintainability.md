# ADR-0261: Infrastructure language is chosen by maintainability

- Status: accepted
- Date: 2026-11-23
- Issue: [#1977](https://github.com/jaunder-org/jaunder/issues/1977)
- Amended: 2026-11-25 — verdicts for `tools/host-proof` (#1974)

## Context

`CONTRIBUTING.md` said to use Rust "by default for infrastructure", allowing
Python "when it produces more focused, easier-to-maintain code". Most host tools
became Rust when the scripts were moved uniformly
([ADR-0028](0028-devtool-vs-xtask-boundary.md)), without asking what each one
did.

Two things changed that. [ADR-0258](0258-dev-runs-nix-built-host-tools.md)
serves Rust host tools as prebuilt Nix packages, so compile and startup time no
longer separate the languages. And the #1976 pilot
([ADR-0249](0249-pg-run-python-unix-socket.md)) showed what a default had cost:
about 390 non-test Rust lines of `pg-run` became 232 lines of Python, because
much of the Rust existed only to be Rust.

#1977 reviewed the remaining candidates: the `e2e-local` lifecycle, the
`pr-observer`, `qualification`, and the host side of `performance`
(`perf-data`).

## Decision

**Criterion.** The deciding question is ongoing maintainability: how readable
and changeable the code is for what it actually does. Port cost is noted but
does not decide a verdict. A development or CI tool is Python unless it:

- links product crates it genuinely needs, such as a storage benchmark driving
  `storage`; or
- needs Rust-specific parsing, such as `syn` over Rust source or wasm section
  parsing.

Process sequencing, subprocess calls (`gh`, `git`, `nix`), JSON and YAML
shaping, statistics and state machines are Python's ground.

_Clarified by #1974._

- **Specialized parsing only counts when no maintained tool already does it.**
  The Rust-specific parsing exception holds only when no maintained tool already
  does the parsing. For example, `twiggy` reads wasm sections and names and
  demangles Rust symbols. A tool that only consumes its output is Python.
- **A thin command-line wrapper joins its Rust owner.** If a tool is a thin
  command-line wrapper over logic that a Rust owner keeps for its own use, it
  joins that owner rather than being ported.
- **Repository contract tests are Python.** These are tests that check YAML, Nix
  and shell contracts rather than a Rust interface.

**Product facts come from product interfaces.** A host tool that links product
code only to learn a fact, such as an inventory, a manifest or a format version,
is a sign of a missing or unused product command or file. Fix that interface
instead of linking.

**Cross-language formats.** A format written by Rust and read in another
language does not make the reader Rust. The writer publishes a JSON Schema
generated from its own types, committed with a drift check
([ADR-0109](0109-cross-language-literal-agreement.md)), and every reader
validates against it.

**Shell** is allowed only for host scripts in `scripts/`. Shell inside Nix
builders and `run:` steps in GitHub workflows is out of scope. A bash script
conforms when it:

- runs commands and checks their results;
- uses traps only to remove temporary files or to report which step failed;
- gets concurrency only from a standard tool, such as bounded `xargs -P` or a
  `tee` the script waits for.

Signal forwarding, supervising long-lived children, or data shaping beyond `jq`
means Python. This amends ADR-0249's "no shell exception is added". ADR-0249
rejected bash for exactly the capability this rule excludes. Five of the six
current scripts conform: `pr-land.sh`, `pr-cleanup.sh`, `build-csr-assets.sh`,
`e2e-workflow` and `git-add`. `test-theme-repository-release-version.sh` does
not: it cuts a regex and a `run:` block out of a workflow file with `sed` by
fixed indentation, which is data shaping beyond `jq`. It moves to Python
([#2070](https://github.com/jaunder-org/jaunder/issues/2070)).

**Verdicts.**

- **`e2e-local` moves into `tools/dev` as Python.** It is a fixed process
  sequence (collector, server, seed, Playwright) with cleanup and signal
  handling, the `pg-run` shape. The Rust needs two Tokio runtimes as a
  sync/async bridge, error types that carry the session back for cleanup, and
  about 400 lines of per-branch failure records. It shares no product types, and
  it is host-proof's only user of tokio, processkit and xshell.
- **`pr-observer` is rewritten in Python, starting with a feature audit.** It
  calls `gh api` as a subprocess, walks untyped JSON by hand, hand-rolls RFC3339
  dates, reimplements GitHub Actions' job graph and matrix expansion, and
  string-matches a `jq` command in `ci.yml`. Rust types add little there, and
  much of the hard-to-follow code is scope rather than language. Behavior that
  is kept still honors [ADR-0087](0087-xtask-github-pr-observation.md),
  [ADR-0135](0135-pr-watch-actionable-handoff.md),
  [ADR-0195](0195-dynamic-pr-check-classification.md),
  [ADR-0198](0198-active-workflow-incomplete-pr-evidence.md),
  [ADR-0220](0220-explicit-one-shot-pr-reenqueue.md) and
  [ADR-0240](0240-just-frontdoors-host-pr-observer.md), including the exit-code
  and JSON contract that `pr-land.sh` relies on.
- **`perf-data` moves to Python, against generated schemas.** The host CLI links
  no product code. The `performance` writer side stays Rust: the dataset plan,
  the manifest and fragment types, `test-support`'s seeder and `perf-producer`,
  which drive `storage`. It publishes JSON Schemas for the dataset manifest, the
  fragments and the combined result. Python assembles, compares and imports
  baselines against them, and the browser benchmark's TypeScript manifest checks
  validate against the same schema.
- **`qualification` is retired.** It links product code only to learn facts the
  deployed server and the build already expose, and it boots QEMU, Caddy and
  `curl` probes by hand. A scheduled CI job replaces it: the NixOS VM test
  driver deploys an earlier build through the NixOS module, upgrades it to the
  current commit, keeps one browser across the upgrade, and compares the result
  against golden files that each commit regenerates. The synthetic
  `a`/`b-app`/`b-theme` styling fixtures
  ([ADR-0239](0239-qualification-workflow-ownership.md)) are dropped. Their
  rules are already tested in ordinary CI on both backends.

**`tools/host-proof` verdicts** (_added by #1974_). Each of the crate's
executables and its contract tests gets a verdict. The crate's original split
(#1974) is dropped: once these moves land, the crate and `packages.host-proof`
are retired.

- **`e2e-evidence` and `trace-analysis` move into `tools/dev` as Python.**
  - _What they are:_ the E2E verdict (lane completeness, duration budget, boot
    decomposition, server-function flow coverage) and the application-trace
    diagnostics. Both are JSON, tarball and trace reconciliation.
  - _One trace core:_ the span parser, boot-phase outcomes and capture readers
    serve both. The `trace-analysis` commands are ported on top of that core and
    kept for wider use; they are separate from `dev`'s own Perfetto trace.
  - _Product facts:_
    - The capture archive layout from `host::capture` is pinned under
      [ADR-0109](0109-cross-language-literal-agreement.md).
    - The `syn` inventory of server functions stays in `repository-policy`,
      which emits it as JSON against a schema generated from its types.
- **`wasm-audit` moves into `tools/dev` as Python.**
  - _The CI budget_ is the raw size of the selected wasm file.
  - _The manual breakdown_ delegates parsing and demangling to `twiggy`. The
    #1974 probe showed that `twiggy top -f json` attributes every byte of the
    unstripped `csrWasm`. Python rolls its items up by crate and walks the
    section table.
  - _Product fact:_ the Vite manifest key that selects the wasm is pinned under
    ADR-0109.
- **`csr-prerequisite` is retired.** `server/build.rs` runs the same
  Vite-manifest and system-theme validators, and fails closed on a declared
  bundle. Retiring it also retires host-proof's Rust OpenTelemetry writer, its
  only user.
- **`admit-inputs` joins `repository-policy`.** It is a 27-line command over
  `repository_policy::checkout`, which that package keeps for its own source
  admission.
- **`source-diagnostics` (the cache-safety probe) moves into `tools/dev` as
  Python and runs in `ci-validate-host`.** It is
  [ADR-0252](0252-substitutable-static-verdicts.md)'s guard that final verdicts
  stay out of Cachix. No gate ran it before, so it guarded nothing.
- **The repository contract tests in `tools/host-proof/tests` become pytest.**
  These are the devShell, e2e VM script, hook, workflow and matrix-topology
  tests. None of them tests a host-proof executable.

## Consequences

- `CONTRIBUTING.md`'s language rule states this criterion and the shell rule.
- Each move is its own follow-up issue, which amends the ADRs it touches:
  - `e2e-local`: [ADR-0246](0246-retire-xtask-devtool-endpoints.md),
    [ADR-0247](0247-host-tool-json-report-contract.md), ADR-0258.
  - `pr-observer`: ADR-0240, ADR-0258.
  - `perf-data`: [ADR-0241](0241-performance-workflow-ownership.md),
    [ADR-0251](0251-dev-runner-replaces-just.md), ADR-0258, and the
    `performance` description in `docs/ARCHITECTURE.md`.
  - `qualification`: ADR-0239, [ADR-0225](0225-nix-cached-host-tooling.md),
    ADR-0247, ADR-0251, ADR-0258.
- The `tools/host-proof` verdicts are follow-ups under #1859, each amending the
  ADRs it touches:

  | Issue                                                       | Move                                                            | ADRs it amends                                                         |
  | ----------------------------------------------------------- | --------------------------------------------------------------- | ---------------------------------------------------------------------- |
  | [#2100](https://github.com/jaunder-org/jaunder/issues/2100) | `e2e-evidence` and `trace-analysis`                             | ADR-0246, ADR-0247, ADR-0258                                           |
  | [#2101](https://github.com/jaunder-org/jaunder/issues/2101) | `wasm-audit`                                                    | [ADR-0106](0106-wasm-raw-size-budget.md), ADR-0246, ADR-0247, ADR-0258 |
  | [#2102](https://github.com/jaunder-org/jaunder/issues/2102) | `admit-inputs`, `csr-prerequisite` and the OpenTelemetry writer | [ADR-0251](0251-dev-runner-replaces-just.md), ADR-0258                 |
  | [#2103](https://github.com/jaunder-org/jaunder/issues/2103) | cache-safety probe                                              | ADR-0252, ADR-0258                                                     |
  | [#2104](https://github.com/jaunder-org/jaunder/issues/2104) | contract tests                                                  | none                                                                   |
  | [#2105](https://github.com/jaunder-org/jaunder/issues/2105) | retire the crate                                                | ADR-0246's "focused host proof" concern                                |

- Accepted losses from the host-proof verdicts:
  - `csr-prerequisite`'s separately named early failure; the server build fails
    with the same validators instead.
  - Its two sub-phase spans.
  - Any Rust process joining a `dev` trace.
  - The one-JSON-stdout report form of the moved tools. Their verdicts, exit
    codes and committed snapshots are kept.
- Until each follow-up lands, the current tool stays in place.
- `e2e-local` in Python loses the Rust aarch64-darwin start-time token from
  `libproc`, and must otherwise preserve the readiness, signal, cleanup and
  report behavior listed on its follow-up.
- Retiring the styling fixtures is an accepted capability loss: a regression in
  how stylesheets change across an upgrade surfaces only on a run where CSS
  really changed between the two builds, not on every run.
- #1419's acceptance run waits on the upgrade test.
- ADR-0028 needs no change. ADR-0240 and ADR-0246 have already superseded its
  tool placements.
