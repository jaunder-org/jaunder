# ADR-DRAFT: Infrastructure language is chosen by maintainability

- Status: proposed
- Date: 2026-11-23
- Issue: [#1977](https://github.com/jaunder-org/jaunder/issues/1977)

## Context

`CONTRIBUTING.md` said to use Rust "by default for infrastructure", allowing
Python "when it produces more focused, easier-to-maintain code". Most host tools
became Rust when the scripts were moved uniformly
([ADR-0028](../0028-devtool-vs-xtask-boundary.md)), without asking what each one
did.

Two things changed that. [ADR-0258](../0258-dev-runs-nix-built-host-tools.md)
serves Rust host tools as prebuilt Nix packages, so compile and startup time no
longer separate the languages. And the #1976 pilot
([ADR-0249](../0249-pg-run-python-unix-socket.md)) showed what a default had
cost: about 390 non-test Rust lines of `pg-run` became 232 lines of Python,
because much of the Rust existed only to be Rust.

#1977 reviewed the remaining candidates: the `e2e-local` lifecycle, the
`pr-observer`, `qualification`, and the host side of `performance`
(`perf-data`).

## Decision

**Criterion.** The deciding question is ongoing maintainability: how readable
and changeable the code is for what it actually does. Port cost is noted but
does not decide a verdict. Infrastructure is Python unless it:

- links product crates it genuinely needs, such as a storage benchmark driving
  `storage`; or
- needs Rust-specific parsing, such as `syn` over Rust source or wasm section
  parsing.

Process sequencing, subprocess calls (`gh`, `git`, `nix`), JSON and YAML
shaping, statistics and state machines are Python's ground.

**Product facts come from product interfaces.** A host tool that links product
code only to learn a fact, such as an inventory, a manifest or a format version,
is a sign of a missing or unused product command or file. Fix that interface
instead of linking.

**Cross-language formats.** A format written by Rust and read in another
language does not make the reader Rust. The writer publishes a JSON Schema
generated from its own types, committed with a drift check
([ADR-0109](../0109-cross-language-literal-agreement.md)), and every reader
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
rejected bash for exactly the capability this rule excludes. All six current
scripts conform: `pr-land.sh`, `pr-cleanup.sh`, `build-csr-assets.sh`,
`e2e-workflow`, `git-add` and `test-theme-repository-release-version.sh`.

**Verdicts.**

- **`e2e-local` moves into `tools/dev` as Python.** It is a fixed process
  sequence (collector, server, seed, Playwright) with cleanup and signal
  handling, the `pg-run` shape. The Rust needs two Tokio runtimes as a
  sync/async bridge, error types that carry the session back for cleanup, and
  about 400 lines of per-branch failure records. It shares no product types.
- **`pr-observer` is rewritten in Python, starting with a feature audit.** It
  calls `gh api` as a subprocess, walks untyped JSON by hand, hand-rolls RFC3339
  dates, reimplements GitHub Actions' job graph and matrix expansion, and
  string-matches a `jq` command in `ci.yml`. Rust types add little there, and
  much of the hard-to-follow code is scope rather than language. Behavior that
  is kept still honors [ADR-0087](../0087-xtask-github-pr-observation.md),
  [ADR-0135](../0135-pr-watch-actionable-handoff.md),
  [ADR-0195](../0195-dynamic-pr-check-classification.md),
  [ADR-0198](../0198-active-workflow-incomplete-pr-evidence.md),
  [ADR-0220](../0220-explicit-one-shot-pr-reenqueue.md) and
  [ADR-0240](../0240-just-frontdoors-host-pr-observer.md).
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
  ([ADR-0239](../0239-qualification-workflow-ownership.md)) are dropped. Their
  rules are already tested in ordinary CI on both backends.

## Consequences

- `CONTRIBUTING.md`'s language rule states this criterion and the shell rule.
- Each move is its own follow-up issue, which amends the ADRs it touches:
  - `e2e-local`: [ADR-0246](../0246-retire-xtask-devtool-endpoints.md),
    [ADR-0247](../0247-host-tool-json-report-contract.md), ADR-0258.
  - `pr-observer`: ADR-0240, ADR-0258.
  - `perf-data`: [ADR-0241](../0241-performance-workflow-ownership.md),
    [ADR-0251](../0251-dev-runner-replaces-just.md), ADR-0258.
  - `qualification`: ADR-0239, [ADR-0225](../0225-nix-cached-host-tooling.md),
    ADR-0247, ADR-0251, ADR-0258.
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
