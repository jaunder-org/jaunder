# Repeatable Production Deployment Baseline Harness Implementation Outline

> Execute with `jaunder-iterate`, delegating individual tasks through
> `jaunder-dispatch`. This outline exists because the approved spec introduces a
> durable xtask/Nix harness interface, authentication-continuity checks,
> cross-backend recovery, and shared E2E contracts.

## Scope

In:

- An opt-in host xtask command family for discovery and two-revision
  acceptance-capability runs.
- Isolated persistent NixOS deployment profiles for SQLite and PostgreSQL behind
  one stable HTTPS origin.
- Expanded typed demo fixtures plus one shared Playwright create/verify flow in
  the ordinary E2E matrix and baseline harness.
- Same-backend restart/reboot plus all four compatible-schema restore
  directions.
- Exact revision/package/runtime identity, schema-governed upgrade claims,
  sanitized JSON-authoritative evidence, a reusable runbook, an initial
  discovery report, and a controlled two-revision acceptance demonstration.

Out:

- Mandatory gate expansion, performance/soak work, public DNS/ACME,
  release-channel or downgrade policy, cross-schema restore, live data, and
  unrelated product fixes.

## Task outline

- [x] Task 1: Establish the shared representative-data and behavior contract
  - Contract: extend the existing `demo` profile with a versioned
    backend-neutral seeded manifest; keep its current corpus and use disjoint
    stable identities for a Playwright-owned operation manifest. The shared flow
    owns fixed checks for login, web Post creation/visibility/scheduling,
    HTML/Markdown/Org, Media, AtomPub discovery/Collection/Member/Media
    operations, unchanged session/App Password use, fresh App Password
    mint/use/revoke, and Atom/RSS/JSON Syndication Feeds. Its read-only verifier
    consumes both manifests; canonical values and Media hashes—not
    backend-assigned numeric IDs—are identity authorities.
  - Verification: focused dual-backend fixture tests and
    `devtool run -- cargo xtask e2e-local production-baseline-flow.spec.ts`
    during iteration; the PR's existing `{sqlite,postgres}×{chromium,firefox}`
    CI lanes are the acceptance proof that the exact shared flow joined the
    ordinary matrix; then `devtool run -- cargo xtask check`.

- [x] Task 2: Define the command and durable evidence interfaces
  - Contract: add `production-baseline discover --revision <sha>` and
    `production-baseline accept --source <sha> --target <sha>` under the
    existing xtask CLI/dispatch pattern. Produce a `ResolvedRevision` carrying a
    full commit in `jaunder-org/jaunder` and immutable flake reference; reject
    unresolved input and an acceptance pair whose inputs resolve to the same
    full commit. Package building remains Task 3 ownership.
  - Contract: require the executing xtask, shared Playwright flow, test-support
    binary, and both manifests to share one clean upstream harness commit.
    Record that commit and manifest versions/hashes; reject dirty or
    mixed-revision execution before lifecycle mutation.
  - Contract: acquire one exclusive host run lease before build or lifecycle
    mutation; a second invocation fails before touching shared proxy, disk,
    workspace, or output state. Stale ownership is diagnosed and cleaned only
    through bounded, identity-checked cleanup.
  - Contract: `docs/production-baseline.schema.json` version 1 owns fixed check
    IDs, harness/product/runtime identities, backup format and schema versions,
    lifecycle outcomes and per-action/check durations, gaps, findings, and
    failure classes. Passing and failed durable output admits only validated
    `summary.json` and generated `summary.md`; sanitizer/evidence failure
    publishes nothing outside the restricted workspace.
  - Verification: CLI/revision/distinctness and clean-harness tests with
    controlled repositories; lease contention/stale-owner tests; schema,
    JSON-to-Markdown parity including durations, allowlist, all-outcome
    redaction, and canary tests; then `devtool run -- cargo xtask check`.

- [ ] Task 3: Provide the isolated production VM lifecycle
  - Contract: add a package-selection seam to the Jaunder NixOS module while
    preserving the existing default. The lifecycle adapter alone consumes
    `ResolvedRevision`, builds that immutable revision's supported
    `packages.jaunder` installable, and returns its exact installable URI,
    derivation, output-path, NAR-hash, executable-hash, systemd, and
    running-executable identities. New named opt-in `packages`/`apps` own only
    the harness and VM profiles; no baseline lifecycle output enters `checks`.
  - Contract: backend-specific persistent disks, the supported PostgreSQL
    bootstrap path, and an external proxy expose Jaunder only at one stable
    local HTTPS origin. Service restart and whole-VM reboot preserve Jaunder,
    Media, and database state; proxy cutover changes targets without changing
    browser origin.
  - Verification: opt-in smoke execution for both profiles proves module
    startup, HTTPS redirect/termination and secure cookies, package/runtime
    identity, service restart, VM reboot, and bounded cleanup; then
    `devtool run -- cargo xtask check`.

- [ ] Task 4: Implement the discovery recovery workflow
  - Contract: with the Task 2 lease held, every run creates a new workspace and
    fresh SQLite/PostgreSQL source disks, runs every Task 1 creation/check, then
    cuts the stable origin back to each source after service restart and again
    after whole-VM reboot to run the read-only continuity checks with the
    original browser cookie and App Password. It creates one backup per source
    and restores each into fresh SQLite and PostgreSQL targets running the same
    package/schema. The stable origin follows each target; unchanged
    credentials, fresh login, fresh App Password mint/use/revoke, and every
    fixed feed check run after each restore. Interrupted state cannot resume
    into passing evidence.
  - Contract: record each backup hash, manifest `format_version`, and observed
    source/target schema versions. Reject unsupported format or schema before
    target mutation. Backup files and raw state remain only in the restricted
    workspace; every product, harness, infrastructure, lifecycle, verifier,
    lease, sanitizer, or evidence failure is non-passing.
  - Verification: controlled opt-in discovery exercises both source lifecycles,
    all four restore directions, fixed check population, compatibility
    rejection, fresh-run behavior, and complete evidence safety; then
    `devtool run -- cargo xtask check`. The real upstream-identified discovery
    report is Task 6.

- [ ] Task 5: Implement the two-revision acceptance-capability workflow
  - Contract: accept only a validated distinct `ResolvedRevision` pair and
    create fresh source state. Repeat both source deployments and
    restart/reboot, switch each service declaratively to the target package,
    prove target runtime identity, run every
    continuity/read-only/fresh-authentication check, then back up the target
    schema and complete all four target-package restore directions.
  - Contract: record package activation for every distinct revision; claim a
    binary change only when executable hashes differ, and schema migration only
    when schema versions differ and target startup performs it. An optional
    pre-upgrade-backup probe against a mismatched target expects ADR-0174
    rejection and is not required recovery success.
  - Verification: controlled immutable revisions cover equal-revision rejection,
    byte-identical package activation, changed executable classification,
    unchanged-schema upgrade, changed-schema classification, and all four target
    recovery directions; then `devtool run -- cargo xtask check`. The
    clean-upstream non-release qualification report is Task 6.

- [ ] Task 6: Publish operator guidance and qualified harness evidence
  - Contract: `docs/production-baseline.md` documents prerequisites, commands,
    topology, safe cleanup, output interpretation, reruns, and failure
    distinctions. Every observed discovery defect or operational blocker has one
    individual Milestone #22 issue with reproduction evidence and a preliminary
    blocking, accepted, or deferred disposition for #1419 to confirm.
  - Verification: after Tasks 1–5 are committed and that implementation commit
    is pushed to the #1450 branch, run the clean upstream harness twice from
    fresh state: discovery against `f6b26c2f9e68c82504b4758778ab2bb706e7cca7`,
    then non-release acceptance qualification from that source to the pushed
    harness implementation commit. Generate and commit both dated JSON/Markdown
    pairs; verify every discovery finding's issue link, reproduction evidence,
    and preliminary disposition; confirm schema/render parity and all-outcome
    evidence safety. Final release-candidate work remains #1419 after this
    prerequisite lands. Finish with `jaunder-commit`.

## Cross-task contracts

- Task 1 owns manifest versions, semantic identities, the full fixed
  behavior-check registry, and the shared read-only verifier consumed by Tasks 4
  and 5.
- Task 2 owns CLI argument types, validated `ResolvedRevision` and distinct-pair
  types, exclusive run lease, evidence model/schema, renderer, allowlist,
  redaction policy, and failure classes. It resolves identity but does not build
  packages.
- Task 3 alone consumes `ResolvedRevision` to build immutable packages/profiles
  and owns the lifecycle adapter; Tasks 4 and 5 request transitions without
  reproducing VM, proxy, disk, backend, or package-build mechanics.
- Task 4 defines and executes the discovery graph and produces raw structured
  results. Task 5 reuses that graph and adds declarative package upgrade plus
  target-schema recovery.
- Task 6 renders and publishes the clean upstream discovery and non-release
  qualification results from Tasks 4 and 5; handwritten success claims are not
  evidence. Issue #1419 later invokes the landed Task 5 interface against a
  target containing merged #1450 for final release-candidate evidence.

## Risk checks

- Preserve ADR-0028 and the host-only xtask invariant: host xtask may invoke
  Nix; Nix never invokes xtask.
- Keep the harness opt-in under explicitly invoked `packages`/`apps`, never
  `checks`; keep it outside `cargo xtask check`, `prepush`, `validate`,
  `nix flake check`, and the mandatory E2E matrix.
- Preserve the NixOS module's existing default package and production service
  behavior for callers that do not select another immutable package.
- Keep test-support outside the production Jaunder package/service closure.
- Do not create Git worktrees or build evidence from dirty/live checkout
  contents; resolve immutable repository commits directly.
- Preserve backend parity and exact-schema compatibility from ADR-0174; never
  reinterpret a compatibility rejection as recovery success or product failure.
- Preserve one stable HTTPS origin across proxy cutovers; never re-scope,
  reinject, or remint the credential used for continuity proof.
- Enforce the evidence allowlist and canary/forbidden-field scan for every
  durable outcome; sanitizer or evidence failure publishes no report.
- Keep deployment lifecycle mechanics out of Playwright while ensuring the same
  behavior verifier runs in ordinary E2E and baseline phases.
- Update `docs/ARCHITECTURE.md`, the hand-maintained operator navigation that
  owns the new runbook, NixOS module documentation, command help, and affected
  test inventories. Do not edit the promotion-owned ADR table in
  `docs/README.md`. No new ADR is expected unless implementation discovers a
  decision not already governed by ADR-0028, ADR-0142, ADR-0174, or the approved
  spec.
