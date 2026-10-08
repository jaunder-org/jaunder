# Issue #1699 implementation outline

> Execute with **jaunder-iterate**; use **jaunder-dispatch** only for authorized
> bounded tasks. This outline exists because diagnostics cross client modules
> and must preserve privacy, cancellation and primary operation outcomes.

## Scope

In: the
[approved spec](../specs/2026-10-07-issue-1699-reconcile-fetch-performance.md) —
identity-indexed Local Post Link proof, realistic scale/flow regressions, opt-in
diagnostic core, exhaustive operation integration and evidence.

Out: shared batch freshness, server/schema/protocol changes, new Media policy,
asynchronous execution, payload logging, saved logs, and unrelated refactors. No
backend behavior changes; use existing live Emacs fixtures for consumer proof.

## Task outline

- [x] **1. Remove all-to-all Post-link validation.**
  - Contract: join by exact Post ID before filesystem work; retain all competing
    evidence rather than overwriting duplicates. Valid unique proofs preserve
    canonical href, current file/root and ID/slug/filename checks. Ambiguous or
    stale proof cannot invent a replacement. No batch-freshness caching.
  - Verification: real-file 100/1,000-Post regression with canonical alternate
    hrefs and unrelated image URL; validation counts stay at most one per Post.
    Exercise valid links, duplicates/ambiguity, stale/missing/out-of-root files,
    partial inventories and exact byte preservation. Upgrade the actual selected
    pull fixture's public permalink evidence without changing request counts.
- [x] **2. Deliver the diagnostic core and user controls.**
  - Contract: `elisp/jaunder-debug.el` owns the option `jaunder-debug` (off),
    `jaunder-debug-show`, `jaunder-debug-clear`, `jaunder-debug-disable`,
    operation context, field validation/encoding, and bounded read-only buffer.
    It has no dependency on feature modules; callers explicitly load its macro
    surface.
  - Integration surface: `jaunder--with-debug-operation` accepts a literal
    label, lazily evaluated initial fields and an operation body; a guarded
    `jaunder--debug-fields` form adds final status/count/decision fields to that
    span. All field expressions and diagnostics bypass execution when disabled.
    These are the only event-writing routes; callers never format raw values.
  - Verification: `elisp/test/jaunder-debug-test.el` and
    `elisp/test/jaunder-debug-acceptance-test.el` cover disabled sentinels,
    every spec allowlist/range, event size/ASCII limits, nesting/standalone
    roots, return/signal/quit preservation, buffer lifecycle, clear without
    creation, fresh IDs after clear, eviction-marker accounting, and
    sink/warning failure.
- [x] **3. Instrument acquisition, transformation and filesystem work.**
  - Depends on 1 and 2. Own every `transport.*`, `service.*`, `atom.*`, `org.*`,
    `member.*`, `post-link.*`, `media.*`, and `pull.*` label from the spec.
    Instrument real owning boundaries, not just their interactive callers.
  - Contract: initial fields are literal enums or safe aggregate counts;
    terminal updates use only the core allowlist. Do not pass payloads/error
    messages to the diagnostic core. Path/hash/verification spans distinguish
    local delay from transport. The replacement-proof span isolates the
    confirmed bottleneck.
  - Verification: real-boundary pure tests cover every owned label, with secret
    and content sentinels at dynamic field producers. Retain Media trust/no-
    overwrite, conditional revalidation, partial success and source invariants.
    Run focused live publish/pull workflows with diagnostics both off and on.
  - Evidence: 467/467 pure tests and warning-free compilation. Five focused live
    publish/pull/reconciliation tests passed both off and on against freshly
    provisioned servers; all 24 task-3 labels were observed, with no live
    credential/account/origin leakage. Reproduction and parked evidence:
    `.xtask/issue-1699/task3-live-proof.el` and
    `1791410827438-2657327.{out,err}` in the same evidence directory. These
    temporary tools do not ship; authoritative coverage remains task 5.
- [ ] **4. Instrument user workflows and complete correlation.**
  - Depends on 2 and 3. Own every `config.*`, `auth.*`, `author.*`, `publish.*`,
    `delete.*`, `report.*`, `inventory.*`, `reconcile.*`, `conflict.*`,
    `merge.*` label; no spec inventory label is left outside tasks 3/4.
  - Progress: `config.resolve` and `auth.lookup` wrap their actual owners,
    without dynamic fields. Four focused off/on, privacy, condition and
    disabled-work proofs pass; the complete pure suite is 471/471 and
    byte-compilation is warning-free. `author.new`, `author.complete`, and
    `author.cancel` now wrap their real command lifecycles. Three additional
    off/on proofs cover both creation paths, actual
    publishing/write-back/rename, local deletion, input-buffer exit, callback
    correlation, and native error/quit partial effects; the pure suite is
    474/474 and byte-compilation is warning-free. Publishing and deletion now
    cover all eight required labels, including both validation, conditional
    update, and checkpoint owners. `publish.recover` owns durable intent
    preparation/matching; its replay transfer/retries are timed by
    `publish.create`. Nine focused proofs compare native return values,
    wire/key/If-Match/delay sequences, unchanged and changed recovery,
    checkpoint/rename partial effects, standalone calls, and disabled diagnostic
    factories. Spec review exposed topology and standalone-root evidence gaps;
    follow-up tests now assert exact root labels and immediate parent-label
    edges, reject cycles and misordered terminals, require intent completion
    before sibling transfer, and directly exercise intent, write-back, rename
    and no-op rename. The pure suite is 483/483; production forms remain
    structurally identical after removing diagnostic wrappers. Report open and
    refresh, local discovery, complete Collection pagination, each page, and the
    complete identity join now wrap their six actual owners without fields. Six
    further proofs cover real report rendering/classification, retained
    marks/results, direct helper roots, exact requests/progress, native
    wire/read/join/partial-render errors and quits, invalid pages, disabled
    factories, privacy, and diagnostic retention after report-buffer exit. The
    pure suite is 489/489; compilation is warning-free and all 110
    inventory/reconciliation function forms are identical after removing
    diagnostic wrappers. A batch-level regression exposed terminal diagnostics
    releasing pending input despite the enclosing owner's `inhibit-quit`:
    completion now leaves that input for the owner to acknowledge after its
    result checkpoint. Two red/green regressions cover native value identity,
    body/sink input, and the real executor recording before cancellation and
    refreshing with input cleared. The pure suite is 491/491; compilation is
    warning-free. Batch/row, conflict, and merge owners remain pending.
  - Contract: outer command/batch/row spans nest existing lower-level
    operations; direct helper calls remain independent roots. Ediff setup and
    later finish/ cancel/discard are separate calls, not a fictitious
    continuously running span. Business blocked/no-op/partial outcomes use
    decision fields, while signal/quit outcomes describe actual call
    termination. Existing progress, prompts, retry/recovery and mutation
    ordering are unchanged.
  - Verification: pure and live tests cover real entry points and standalone
    boundaries, nested batch/row timing, conflict/create recovery, and
    error/quit propagation. Buffer tests separately exercise reconciliation,
    authoring, merge scratch and Ediff-view exit without deleting retained
    diagnostics.
- [ ] **5. Reconcile evidence, document usage, and verify the branch.**
  - Depends on 1–4. Add the exhaustive label-to-test/field-producer table to
    `elisp/README.md`, with option/controls, privacy, sharing, retention and
    remaining Collection cost. Each expanded spec label names an actual pure or
    live test; generic core tests cannot stand in for absent instrumentation.
  - Verification: check complete label and field-producer inventories in both
    directions; run the full pure and live Emacs populations, formatting and
    byte-compilation, then applicable repository/authoritative Emacs coverage
    gates. Compare final diagnosed-path work counts at 1,000 Posts and request
    affected-Post production replay. Clearly distinguish local proof from that
    manual timing confirmation. Temporary `.xtask` tools do not ship.

## Risk checks

- **Privacy:** finite spec labels/keys/enums only. Never format an unvalidated
  value, stringify arbitrary errors, log paths/URLs, or emit secrets through a
  diagnostic failure. Unknown enums become literal `unknown`; invalid events
  follow the fixed warning policy. Fault the warning itself in preservation
  tests.
- **Semantics:** diagnostic completion cannot replace a primary return, error,
  or `quit`, including cleanup failure. Never retry an uncertain mutation or
  weaken fresh Collection, Member ETag, local digest or clean-buffer checks.
- **Disabled cost:** lazy macros guard every field expression, clock, encoder,
  ID mutation and buffer allocation; wrapping alone must not change execution.
- **Bounds:** test saturation, invalid types/statuses, oversized IDs/events,
  10,000-event retention plus its separate marker, and clock movement without
  producing negative elapsed milliseconds. Clear must not recycle IDs.
- **Loading:** the primitive is feature-independent; explicitly load macros
  before byte-compilation/use and check fresh-load paths for dependency cycles.
- **Coverage:** every spec acceptance bullet maps to tasks 1–5 above. Preserve
  root-workspace/backend invariants; this change is the Emacs Protocol Client,
  not a reason to alter server, storage, e2e or coverage policy.

## Non-obvious verification lanes

Pure suite: `devtool run -- emacs --batch -Q -l elisp/scripts/run-tests.el`.
Live suite: use `elisp/scripts/run-integration-tests.el` with its existing
`JAUNDER_TEST_BINARY` fixture, never production credentials. The authoritative
pure/live coverage verdict is the `elisp-coverage-producer` and host consumer in
`cargo xtask validate --no-e2e`/CI, not mocked-network elapsed time.
