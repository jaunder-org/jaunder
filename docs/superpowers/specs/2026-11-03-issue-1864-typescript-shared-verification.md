# Outcome

Give the existing `devtool check tsc` population a dedicated hermetic Nix leaf
and separate `cargo xtask validate` verdict, while retaining the named host
check and the complete ordered `devtool check --all` inventory. Type-checking
remains `tsc --noEmit -p end2end/tsconfig.json`; this does not move Playwright
execution into Nix.

# Approach and tradeoffs

The project config has no `files`, `include`, or `exclude`, so TypeScript's
default discovery under `end2end` applies: `.ts`, `.tsx`, `.d.ts` outside
`node_modules`, plus imported declarations; JS is not enabled. Record the actual
compiler file list in proof. The dedicated leaf directly admits the config,
compiler project sources/imports and package/lock inputs, and invokes the
existing `devtool check tsc` definition with the pinned compiler, Node and
provisioned type dependencies.

Keep using the existing `e2ePackage`/Nix-matched Playwright dependency path.
That package copies all of `end2end`, so even a narrow direct source filter has
transitive dependency on unrelated end2end files; their changes may invalidate
the TS leaf. Accept this coupling rather than adding package-pruning
infrastructure. A separate leaf is not a promise of complete derivation-input
isolation: it still separates the TS verdict and gives it a directly reviewable
source filter.

Extend the static catalog by one `typescript` group containing `tsc`; keep
`devtool check tsc` valid. Remove only `tsc` from `code`. `--all` must retain
the prior complete order and membership, inserting the TypeScript group where
`tsc` previously appeared (after `byte-compile`, before `cargo-deny`), with no
duplicates. The Nix sequence becomes `static-docs`, `static-typescript`,
`static-code`; code retains its other checks, including TypeScript formatting
and `e2eCensusContract.ts`. This narrowly supersedes ADR-0178's two-group
partition and two-leaf layout; `devtool` remains the one command-definition
owner.

For timing, reuse the current subprocess output streams and
`StepResult`/`PhaseRecord` consumer model—do not extend app OpenTelemetry or the
test-local Perfetto tracer. Measure only the `tsc` child with a monotonic clock
in `devtool check tsc`; emit one bounded structured timing event to stderr after
it exits, including check identity, elapsed milliseconds and outcome. The host
xtask consumer parses the event from that check's current child-process output.
For Nix, accept it as current-request execution evidence only when observed in
the live output stream of that `nix build` invocation and positively
attributable to the builder running; never recover execution time by reading a
stored build log or `nix log`. If the current request cannot be positively
correlated, report execution duration unavailable. Continue reporting the whole
host step duration, Nix request duration, derivation identity and store
reuse/realization separately. A reused/substituted output without a correlated
current execution event has no current tsc duration; historical output is never
current-request timing.

Keep `cargo xtask check` host behavior, the full validation aggregate, hooks,
workflow topology and GitHub required-check selection unchanged. The Linux Nix
leaf is consumed by the existing x86_64-linux xtask validation path. The cost is
a third leaf/group and a small per-check event parser plus continued `end2end`
dependency coupling. Keeping tsc only in `static-code` is simpler but cannot
provide a separate TS Nix verdict.

# Acceptance

- The dedicated leaf runs the same named `devtool check tsc` command and
  compiler project as the host. A deliberately invalid project TS input fails
  the host check and Nix verdict; restoring it passes both. `devtool check tsc`
  still works directly; the complete ordered `--all` inventory matches the
  pre-change identities/order exactly once.
- Record the actual compiler file list and prove ignored `end2end/node_modules`
  is provisioned from the declared Nix dependency path, not scanned as checkout
  source. Check derivation identity changes for a relevant project TS input and
  remains stable for an unrelated Rust input outside the direct and transitive
  dependency closures (e.g. `server/src/lib.rs`). Document that another
  `end2end` change can still invalidate the leaf via `e2ePackage`; do not claim
  otherwise. Demonstrate valid local-store reuse and report
  identity/classification separately from execution.
- A focused `devtool` test uses a controlled child command to prove the
  monotonic duration event reports the `tsc` child interval and its actual
  outcome (success and failure). A host-consumer test proves the event from the
  current child process becomes an execution `PhaseRecord`. A Nix-consumer test
  feeds a timing event from the current live builder stream and confirms it is
  surfaced; feeding the same event only from a retained historical log must not
  count as current execution. A subsequent reused-output request has request
  duration and reuse evidence but no current tsc execution duration. If live
  builder correlation is unavailable in a real request, the result is explicitly
  unavailable, not inferred.
- `cargo xtask validate --no-e2e` retains the three separate static verdicts and
  the existing test checks; the full validation path remains authoritative. Do
  not change hook routing, workflow topology, or GitHub required-check
  selection.

# Boundaries

In scope: the existing tsc population, its dedicated Nix leaf and host
validation consumer, catalog partition/order preservation, timing event and
current-request observation, focused contract tests, and the ADR/architecture
projection. Out of scope: TypeScript formatting, `e2eCensusContract.ts`,
Playwright runs, other families (including ERT), dependency pruning, generic
cache/timing infrastructure, Nix invoking host-only `xtask`,
hook/CI/required-check routing changes, and implementation of other families.
This proposed spec is not implementation, outline, commit, or merge approval.
