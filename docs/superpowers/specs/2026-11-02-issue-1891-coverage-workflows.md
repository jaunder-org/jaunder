# #1891 — Coverage and doctest workflows

## Outcome

Replace coverage/doctest-specific host producer orchestration with direct Just
workflows. Nix remains producer and hard producer-status gate; one narrow host
policy leaf applies the source-aware coverage rules COTS cannot express.
Preserve coverage/doctest populations, backend parity, failure verdicts, and CI
evidence.

## Public commands and calls

`just coverage` and `just doctests` are the public single-command entrypoints.
Just recipes own the short producer → Nix gate → (coverage only) host policy
sequence; they do not call the Rust validation-lane orchestrator. Each public
recipe first requires a clean checkout, failing closed if
`git status --porcelain --untracked-files=all` fails or reports changes. Keep
this as one small Just precondition, not a copied Rust policy/framework.

- `just coverage` → `coverage-workflow`: Nix `coverage` producer output link at
  `.xtask/gcroots/coverage` → Nix `coverage-gate` →
  `cargo xtask coverage-policy --out .xtask/gcroots/coverage` → propagate either
  nonzero verdict.
- `just doctests` → `doctests-workflow`: Nix `doctests` producer output link at
  `.xtask/gcroots/doctests` → Nix `doctests-gate` → propagate its verdict.

Recipes use existing Nix checks and pinned Just/runtime; keep the Just
definitions short. The policy command is a leaf: it reads the realized
report/source, does not invoke Nix, producer/gate orchestration or Just, and
returns only the policy verdict. It cannot recurse.

During this migration, existing Rust aggregate callers may temporarily invoke
`coverage-workflow` / `doctests-workflow` directly via the existing
`crate::sh::step` primitive. This is bounded caller adaptation, not the public
workflow or a permanent dependency: no caller routes through `ci-validate` back
to public `just coverage`. #1889 owns eventual removal of Rust gate
aliases/catalog/result machinery and CI cutover to direct Just. That cutover
does not block #1891 implementation or require unlanded imports. Until it lands,
existing callers remain functional.

CI's target command is
`nix develop .#ci --accept-flake-config -c just coverage`; its GitHub job result
remains the aggregate verdict. Until #1889 makes that cutover, current
`ci-validate coverage` retains its clean-tree precheck and invokes the internal
recipe. Diagnostic uploads remain evidence only. Do not preserve/upload
`.xtask/coverage-result.json`: no reader exists. Retain producer status,
diagnostics and storage snapshot needed for diagnosis.

## Deletions and retained policy

Delete the coverage/doctest host producer/gate `build_check` calls and workflow
functions in `xtask/src/steps/nix.rs`; status parsing/rendering and
status-to-`StepResult` adapters; producer-derived coverage phase records;
status/report/diagnostic copy/lift plumbing; `CommandResult.coverage` and
coverage-only report summaries without readers. Remove the corresponding
workflow-only `TestCheck` adapters. Generic child exit reporting may remain
temporarily for current callers. #1889 owns the later retirement of the shared
validation catalog/results; it does not shelter these #1891-specific adapters.

Keep only the source-aware coverage policy: reason-bearing real-comment
`cov:ignore`/blocks; fail-closed AST recognition of literal
`unreachable!("msg")` with the A1 covered-line guard; and source-span
`crap:allow` at the exclusive CRAP threshold 30. Nix `coverage-gate` rejects
absent/invalid/non-success producer status before this policy runs;
`doctests-gate` rejects invalid/non-`ok` status. Producer status alone is never
a coverage pass.

COTS owns instrumentation, report and CRAP score generation. Consume existing
pinned `cargo llvm-cov report --text` per-source line rows and cargo-crap JSON.
Retain the current bounded line-table extraction: nonblank count means
executable, nonzero means covered, source text feeds marker matching. LLVM JSON
is region/segment-oriented and supplied research does not prove a version-pinned
equivalent line mapping for gaps, multiline/multiple regions, expansions and
instantiations. `llvm-cov show` is line-oriented but does not eliminate
extraction from the current producer report. Do not invent a JSON segment
reducer or generic report framework. Keep LCOV internal to cargo-crap.

Minimum producer output: coverage `status.json`, `coverage-report.txt`,
`crap-report.json`, diagnostics; doctests `status.json`, diagnostics. Producer
output links live under `.xtask/gcroots/{coverage,doctests}`; no Rust copy/lift
layer.

## Acceptance

1. Direct `just coverage`/`just doctests` each run clean-tree admission,
   producers and hard gates in order, propagate failure, and never round-trip
   through `cargo xtask ci-validate`.
2. Verify Nix status failure prevents coverage policy; policy fails for
   missing/malformed/empty/no-executable reports, uncovered unexempt lines, A1,
   malformed markers, or CRAP >30; doctest gate failure is nonzero.
3. Remove the issue-specific producer/status/phase/copy/report adapters listed
   above. Any temporary Rust callers invoke internal Just workflow recipes
   through existing `sh::step` only; no new executor/framework. #1889 owns
   permanent gate/catalog/result retirement and CI Just cutover.
4. Test line-table attribution with representative pinned llvm-cov output for
   executable/non-executable rows, zero/nonzero counts, gaps,
   multiline/multiregion rows, expansions and instantiations; verify A1 and
   malformed/unsupported-report fail-closed behavior. Test marker grammar,
   structural fail-closed behavior, source-span `crap:allow` and threshold 30.
   No heavy proof is part of spec preparation.
5. Preserve root-workspace census and SQLite/PostgreSQL parity; separate
   auxiliary `xtask`/`tools` and doctest fence populations; retain job-result
   verdict and diagnostic-only status/artifact uploads. No
   `.coverage-result.json` copy/upload.

## Ownership and architecture

#1889's approved spec/outline owns retirement of the Rust gate/catalog/results,
final CI Just cutover, and the narrow superseding ADR draft plus
`docs/ARCHITECTURE.md` projection. ADR-0192/0203's Rust-owned catalog
composition is superseded only as specified by #1889; retain their real
clean-tree admission, four lane/job results, per-ref verdict and population
guarantees. #1891 supplies the coverage/doctest Just workflows and bounded
current-caller adaptation only. No new ADR or catalog framework is introduced
here. ADR-0028's execution-domain reconciliation follows #1889's assigned ADR
work; Nix still never executes host xtask, and the coverage policy remains
host-side.

#1890 continues to own server-function verifier/inventory/trace and E2E capture;
coordinate only actual shared-file edits. #1884/#1885/#1886 populations remain
excluded. Implementation outline/risk coordination follows #1889's approved
outline; no separate #1891 outline is proposed absent a new material risk
trigger.

The Manager-relayed Just-default preference provenance is human response
`c9e04f6d-1312-4eba-a4fb-d48a5a564587` (`issue1889-spec-v1`), Manager message
`136935bc-0788-4553-a11a-96cc88657e64`. Approval of this exact #1891 spec:
Manager relay `fccee738-df53-4839-b430-75853e2f0569` (`issue1891-spec-v4`),
“Approve spec and begin”. This authorizes the specified workflow design only; no
implementation is included in this preparation commit. No separate #1891 outline
is currently warranted absent a new material risk. Detailed trace, COTS/LLVM
research, ADR evidence, tracker API reads and review refs are in ignored
`.xtask/issue-1891-spec-evidence.md`.
