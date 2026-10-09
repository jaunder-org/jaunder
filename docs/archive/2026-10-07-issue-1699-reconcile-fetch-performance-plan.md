# Issue #1699 implementation outline

> Execute with **jaunder-iterate**; use **jaunder-dispatch** only for authorized
> bounded tasks. This outline exists because diagnostics cross client modules
> and must preserve privacy, cancellation and primary operation outcomes.

## Scope

In: the
[approved spec](2026-10-07-issue-1699-reconcile-fetch-performance-spec.md) —
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
- [x] **4. Instrument user workflows and complete correlation.**
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
    warning-free; both independent reviews approved that correction. The actual
    confirmed executor and all five push/pull/delete/keep-local/keep-remote row
    owners now have batch/row timers and allowlisted action/decision
    projections. Five permanent tests cover direct roots, eligibility/no-op
    results, native return identity/error/quit, disabled field laziness,
    retained outcome aggregation, displayed order/foreign filtering/duplicate
    selections, append-before-cancel, and refresh failure. The real executor
    calls actual push owners in both modes; existing successful/failing mutation
    regressions also ran enabled (45/45, no diagnostic warnings). Pure suite:
    496/496; compilation warning-free; 76 native reconcile defuns equivalent
    after stripping only diagnostic wrappers. Final live evidence is recorded
    below. Both batch/row reviews approved; the Standards wording note (action
    known at start, only decision initially unknown) is corrected in README.
    Explicit conflict commands and separate scratch finish/cancel/discard
    callbacks now have the seven remaining label groups. Remote staging and
    actual snapshot/Ediff setup are separate `merge.stage` owners. Six permanent
    tests replay existing native prompt/mutation/staging/startup/drift/partial/
    racing-edit assertions off/on, verify real command→batch→row parentage,
    independent retained-scratch callback roots, privacy and exact native
    error/quit payloads for all seven labels. The full runner caught a duplicate
    test load; replay now loads legacy contracts lazily only during focused
    execution. Pure suite: 502/502; compilation warning-free; all 76 native
    reconcile defuns still equivalent. Both independent conflict/merge reviews
    approved. Task 4's permanent owner/lifecycle and correlation contracts are
    delivered; complete live populations, coverage and owner/field inventory
    evidence are recorded under Task 5.
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
- [x] **5. Reconcile evidence, document usage, and verify the branch.**
  - Delivered evidence after upstream reconciliation: README maps all 52
    expanded labels to 68 actual production boundaries and named permanent
    proofs; the source-reader census reconciles both directions. The full pure
    population passes 532/532; the full live population passes 46/46 with
    diagnostics disabled and enabled. Production byte-compilation is
    warning-free. The fresh authoritative `ci-validate test-checks` lane passes
    its hermetic Emacs producer, artifact lift and consumer: 4,205 covered
    points and 223 accepted ignored points; producer outcome is success across
    all 18 modules. No branch suppressions were added. Deferred fields and batch
    projections have permanent regressions requiring their own live Edebug
    counters. All three retained reviewers close the post-rebase Media owner
    findings: actual acquisition/reuse, fallback installation and
    original/fallback path safety own spans; ordinary composition adds no
    duplicate facade span. Five new tests demonstrate controlled red/green,
    off/on behavior, standalone roots, exact native conditions and topology.
  - Privacy evidence after the Media-owner fix: the complete enabled live
    population retains 7,512 events, 3,756 paired spans, 543 roots and 47
    labels; the five labels absent from live execution have pure
    actual-lifecycle proofs. Strict schema, bounds and immediate-parent/LIFO
    topology checks pass, with a maximum event size of 183 bytes. The runtime
    input audit passes 46/46 live tests and checks 201 distinct actual
    credential/account/origin/path/header inputs and authored sentinels without
    leakage. Short authored words that coincide with admitted literal enums are
    governed by the closed-schema and permanent producer proofs, not claimed as
    unique runtime sentinels. Temporary reproduction and bounded result files
    are in `.xtask/issue-1699/` and do not ship.
  - Fresh whole-tree verification: the operator-approved direct
    `validate --no-e2e` run passes all 67 steps on clean `a66d645c`, including
    current Rust coverage producer, population reconciliation, reports, gate and
    consumer: 80,510 executable lines, zero failures, guard violations or CRAP
    failures. The governed run passed 66/67 with a dependency failure matching
    the session's confirmed stale offline Cargo-source configuration. A direct
    run selecting the current flake-derived source home passes the same gate
    without changing tools, source, offline policy or budgets. Infrastructure
    follow-up #1705 records that mismatch; exact workflow propagation remains
    untraced. No Rust or timeout source changes were made. The earlier archived
    Rust replay is diagnosis history, not the current authoritative proof.
  - Current local scale evidence: the real-filesystem 1,000-Post replay observes
    exactly 1,000 validations in 0.421s after upstream reconciliation.
    Controlled selected-pull flows over 1,000 Posts take 1.049s for three
    selections and 2.649s for ten, retaining 160/440 Collection GETs and 6/20
    Member GETs. These use synthetic network responses, plain Org and no Media;
    they are not production timings. Permanent pure regressions retain the
    work-count bound.
  - Acceptance reconciled: all three final whole-branch reviews found no code or
    documentation defects. The operator explicitly accepted the demonstrated
    test speedup as sufficient before merge and deferred production timing to
    [follow-up #1706](https://github.com/jaunder-org/jaunder/issues/1706). The
    governing spec records the exact decision; no production result is claimed.
    That follow-up depends on delivery of #1699 but does not block this PR,
    merge or release. Task 5 is complete under the approved acceptance change;
    the other requirements and all privacy/safety tests remain unchanged. Final
    archive, intended-tree proof and PR/CI are delivery stages, and merging
    still requires explicit per-PR approval.
  - Depends on 1–4. Add the exhaustive label-to-test/field-producer table to
    `elisp/README.md`, with option/controls, privacy, sharing, retention and
    remaining Collection cost. Each expanded spec label names an actual pure or
    live test; generic core tests cannot stand in for absent instrumentation.
  - Verification: check complete label and field-producer inventories in both
    directions; run the full pure and live Emacs populations, formatting and
    byte-compilation, then applicable repository/authoritative Emacs coverage
    gates. Compare final diagnosed-path work counts at 1,000 Posts and retain
    the operator-approved production-replay deferral in the governing spec and
    #1706. Clearly distinguish local proof from any later production timing.
    Temporary `.xtask` tools do not ship.

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
