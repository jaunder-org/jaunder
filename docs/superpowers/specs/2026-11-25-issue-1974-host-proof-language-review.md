# Issue #1974: language review of `tools/host-proof`

## Outcome

Each of host-proof's six executables, and its repository contract tests, gets a
recorded language and home verdict under ADR-0261's maintainability criterion.
The verdicts land as an amendment to ADR-0261, projected into
`docs/ARCHITECTURE.md`'s pending-moves text, with one follow-up issue per move.
This cycle changes no tool code.

When the follow-ups land, `tools/host-proof` and `packages.host-proof` no longer
exist. That overtakes #1974's original goal of splitting the crate so the CI
verifier stops rebuilding for unrelated edits.

## Why a review, not a split

ADR-0261 keeps a development tool in Rust only if it needs product crates it
genuinely links, or Rust-specific parsing (`syn`, wasm sections). #1977 applied
that test to four other tools, not to host-proof. host-proof's `src` holds about
14.4k lines including inline tests, and most of it fails the test, so carefully
re-homing it would be wasted work. The human chose the review on 2026-11-25.

## Proposed verdicts

**`e2e-evidence`**, about 4k lines plus the shared trace core

- _Use:_ CI's E2E verdict (`dev e2e-reconcile`, `ci.yml:438`) and
  `dev validate`.
- _Verdict:_ **Python in `tools/dev`.** It is JSON, tarball and trace
  reconciliation, which ADR-0261 assigns to Python.

**`trace-analysis`**, about 5.6k lines in `traces/*` plus its binary

- _Use:_ manual only. It provides `analyze`, `run` and `boot-phases`, documented
  in `docs/observability.md`.
- _Verdict:_ **Python in `tools/dev`.** The trace parser and the boot-phase
  outcomes it shares with `e2e-evidence` are ported with `e2e-evidence`. The
  `analyze`/`run`/`boot-phases` commands are then ported on top of that shared
  core. The human wants these application-trace diagnostics used more, not
  retired (2026-11-25). They are separate from `dev`'s Perfetto trace, which
  `tools/dev/dev/chrome.py` already writes.

**`wasm-audit`**, about 1.4k lines

- _Use:_ `budget` runs in `ci-validate-hermetic` and compares the raw size of
  the selected wasm file against a ceiling. `report [--breakdown]` is manual
  size attribution.
- _Verdict:_ **Python in `tools/dev`.** The budget is a file size. The breakdown
  delegates wasm parsing to `twiggy`, subject to the probe below, so the
  amendment revises ADR-0261's "wasm sections" example to "unless a maintained
  tool already parses it".

**`csr-prerequisite`**, about 120 lines, plus 380 lines of span writer

- _Use:_ `dev test-local` validates the CSR bundle before
  `cargo build -p jaunder`.
- _Verdict:_ **Retire.** `server/build.rs:69-77` runs the same Vite-manifest and
  system-theme validators, and fails closed when `JAUNDER_CSR_BUNDLE_DIR` is
  declared. The product build is the product interface.

**`admit-inputs`**, 27 lines

- _Use:_ `dev test-local` and `e2e-local` refuse untracked files that a flake
  source would silently omit.
- _Verdict:_ **Joins `repository-policy` as an executable.** It is a thin CLI
  over `repository_policy::checkout`, which repository-policy keeps in Rust to
  admit its own source. The new rule: a thin CLI over logic that a Rust owner
  keeps for itself joins that owner.

**`source-diagnostics`**, about 1.3k lines

- _Use:_ `cache-safety-probe`. It is ADR-0252's guard that coverage and E2E
  final verdicts stay non-substitutable and excluded by Cachix's `pushFilter`,
  but no gate or CI job runs it.
- _Verdict:_ **Python in `tools/dev`, run by a gate.** It is `nix eval`, JSON
  and regex work. A guard nobody runs protects nothing, so the follow-up wires
  it into `ci-validate-host`. The human chose this over retirement, which would
  have dropped ADR-0252's guard (2026-11-25).

**Contract tests in `tools/host-proof/tests`**, about 1.8k lines

- _What they check:_ the devShell tmpdir, the e2e VM script, hook and workflow
  callers, the e2e matrix topology and the performance workflow. None of them
  exercises a host-proof executable.
- _Verdict:_ **pytest under `tools/dev`.** They check YAML, Nix and shell
  contracts.

### Product facts the ports must not copy

ADR-0261 says product facts come from product interfaces. Two cross into Python,
and each is pinned with ADR-0109's cross-language literal agreement: the value
is defined once on each side, and a test fails when the two differ.

- The CSR wasm's Vite manifest key, now chosen by `host::vite_manifest`. The
  wasm budget needs it.
- The capture archive layout, now defined by `host::capture`. The ports read
  `instance-N/diag.log` and the stream files from it. `e2e_local.py` already
  hard-codes `instance-{index}`, and the port joins it to the same pinned
  constant.

Server-fn coverage needs the `syn` inventory, with each function's endpoint,
which stays in `repository-policy`. The reconcile job runs the Nix-built, cached
`repository-policy` package to emit it as JSON, validated against a schema
generated from the Rust types. This keeps "same commit, same verdict": the
inventory is still derived from the commit's own `web/src`.

### Accepted losses

- `csr-prerequisite`'s early, separately named failure. The server build fails
  instead, and with the same validators.
- Its two OpenTelemetry sub-phase spans. Dropping the Rust OpenTelemetry writer
  also means no Rust child process joins a `dev` trace any longer.
- The JSON report form of `e2e-evidence`, `wasm-audit` and the probe. Their
  verdicts, exit codes, fail-closed order and the committed snapshot bytes are
  preserved, and the report form follows `dev` conventions, as #2066 did for
  `e2e-local`.

## This cycle's work

1. **Run the twiggy probe** in scratch; the evidence goes to the PR, not the
   tree.
   - Run `twiggy` (nixpkgs) on the unstripped `.#csrWasm`.
   - Compare its per-function sizes and crate attribution with
     `dev wasm-audit report --breakdown`.
   - Check that the section accounting still adds up to the whole file.
   - The result decides between two options: Python plus twiggy, or retiring
     `--breakdown` and documenting `twiggy top` instead.
2. **Amend ADR-0261** with the verdicts, the two rule clarifications (thin CLI
   joins its owner; a maintained parser supersedes the wasm example) and, per
   move, the ADRs that move will amend:

   | Move                                                     | ADRs it will amend           |
   | -------------------------------------------------------- | ---------------------------- |
   | All moves                                                | ADR-0246, ADR-0247, ADR-0258 |
   | Dropping the OpenTelemetry writer, moving `admit-inputs` | ADR-0251                     |
   | The cache-safety probe                                   | ADR-0252                     |
   | `wasm-audit` (budget constant's home)                    | ADR-0106                     |

   Project the verdicts into ARCHITECTURE's pending-moves sentence and its
   host-proof description, following #1977's precedent. Update the wasm-sections
   example at CONTRIBUTING.md:2524.

3. **File follow-up issues** through `jaunder-issues` as sub-issues of #1859 in
   milestone 33, each with its list of behaviour to preserve:
   - `e2e-evidence`, plus the trace core, to Python. This includes the inventory
     JSON command and schema and the pinned capture constants.
   - `trace-analysis` commands to Python on the shared core.
   - `wasm-audit` to Python. Add twiggy to the devShell if it is used. Preserve
     the budget failure message and the unstripped `csrWasm` package.
   - `admit-inputs` into `repository-policy`, `csr-prerequisite` retired, the
     OpenTelemetry writer dropped. Retire `tools/host-proof` and
     `packages.host-proof` once the other follow-ups are done.
   - The cache-safety probe to Python, run in `ci-validate-host`.
   - The contract tests to pytest.
4. **Close #1974.** Retitle it and rewrite its body to the review, then close it
   as completed when this PR merges. Record the new residuals in #1859's
   baseline.

## Acceptance

- The ADR-0261 amendment gives every executable and the contract tests a
  verdict, its reason under the criterion, the accepted losses above and the
  ADRs each move amends. ARCHITECTURE and CONTRIBUTING reflect it.
- The twiggy probe result is in the PR description, and the wasm breakdown
  verdict follows from it.
- The follow-up issues exist as #1859 sub-issues with preservation lists, and
  #1974 links them.
- `dev check` passes.

## Boundaries

- No tool code changes in this cycle.
- Out of scope:
  - re-reviewing #1977's verdicts;
  - `repository-policy`'s own language (its `syn` use qualifies);
  - process supervision;
  - the `ux-sandbox` duplication.
