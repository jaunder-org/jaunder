# Tooling artifact preparation/import outline (#1752)

> Execute with `jaunder-iterate`; use `jaunder-dispatch` for any separately
> authorized implementation delegation. This outline exists because compiled
> artifact/source boundaries and the #1754 baseline handoff carry architecture
> and multi-agent correctness risk. The approved
> [spec](../specs/2026-10-11-issue-1752-static-dependency-reuse.md) is
> authoritative.

## Scope

In: tools-clippy's tooling dependency path, correct Crane export/import and
source invalidation, preserved shared tools consumers, reproducible baseline.
Out: product-cache expansion, per-unit migration, smaller static verdicts,
custom fingerprints/provenance, host admission/retention owned by #1753.

## Task outline

- [ ] **Diagnose one real artifact path before choosing the repair.**
  - Trace effective Cargo target/profile/features/configuration and every pinned
    setup/configure hook through tools dependency export and consumer import.
  - Prefer existing archive/log evidence. If unavailable, use a dependency-free
    nested tools-workspace reproduction with the same pinned Crane producer and
    import hooks; compare actual phase directories, nonempty archive contents
    and where the consumer imports/uses them. No full graph needed to diagnose
    target alignment. Prove or disprove the suspected mismatch explicitly.
  - Verification: bounded same-pinned-hook reproduction or retained real archive
    evidence; no unapproved evaluation-time host execution or cache purge.
  - Spec: acceptance 1–2; establishes the input to the conditional next slice.
- [ ] **Deliver the narrow confirmed dependency/export correction.**
  - If diagnosis is disproved, investigate the remaining real-source dependency
    coupling with the same bounded evidence. If no beneficial defect remains,
    record the justified no-change branch and enter the final slice's baseline
    measurement and real-consumer verification before handoff. Skip only
    candidate-specific regressions, comparisons and lower-cost proof.
  - For a confirmed defect, repair the shared tooling preparation/export/import
    seam in `nix/packages.nix` using existing Crane/Cargo authorities. Verify
    ordinary, documentation and qualification consumers retain their exact
    profiles/features/native/embedded inputs; final sources remain complete.
  - Verification: focused nonempty artifact consumption/freshness regression and
    source-boundary probes covering unrelated versus relevant source, manifests,
    lockfiles, build scripts, embedded inputs, flags and native dependencies.
  - Spec: acceptance 3–5; review this coherent boundary before enlarging proof.
- [ ] **Prove tools-clippy reuse within the unchanged full static verdict.**
  - Only if the same confirmed defect remains, prepare/import compatible
    dev/all-targets dependency artifacts through `nix/checks.nix`; do not assume
    release artifacts suffice or duplicate the authoritative lint definition.
  - Separate compiler/target/profile/features/flags/native compatibility from
    archive transport. Real local sources must compile/lint; changed inputs must
    rebuild affected work. Preserve offline homes and full static-code verdict.
  - Verification: focused real sandbox tools-clippy execution with observed
    unaffected dependency freshness; then one applicable full static-code
    boundary proof under its separately granted window. Host behavior remains
    subject to existing host checks; no release/clippy timing conflation.
  - Spec: acceptance 1, 3–5, 7, 9; no new authoritative subset check.
- [ ] **Measure, review and hand off the reproducible baseline.**
  - Both branches require baseline/diagnostic measurements and real-consumer
    verification. An improved candidate adds matched comparisons and lower-cost
    proof; the no-change branch does not stop after the dependency-free probe.
  - Execute the approved measurement ceiling with one restored input per arm,
    pre-run output validity and recorded resolved-feature graph changes. Stop or
    postpone contaminated timing; exact commands/windows remain
    coordinator-owned.
  - Use #1753's session observer/evidence with explicit host/Nix attribution
    limits. Its wrapper-free single-job calibration is resource exploration, not
    default/isolated throughput or warm-cache proof. Do not duplicate it.
  - Have the coordinator operate the agreed second existing checkout; never
    mutate that checkout. #1754 receives the preserved release/default devtool
    revision, exact command/configuration, identities, cache state, measurements
    and real-consumer proof (or explicit unchanged-baseline disposition).
  - Verification: cumulative Standards/Spec review, focused correctness and
    matched measurement evidence, then applicable final certification. Publish
    durable conclusions to current docs/issue; retain logs/probes transiently.
  - Spec: acceptance 3, 6–10; final conformance precedes working-doc retirement.

## Risk checks

- Confirm actual export/import before fixing a hypothesized target mismatch;
  nonempty archive plus Cargo freshness, not a derivation identity alone.
- Keep dependency dummy inputs distinct from complete final consumer sources.
  Compiler/profile/features/native mismatches must rebuild, never mask stale
  work.
- Shared docs/qualification consumers and #1734 source/provenance admission
  remain intact; no generic Linux-only wiring or unsupported Darwin reuse claim.
- No coverage/lint/CRAP suppression, new per-unit policy or reduced population.
  Reopen material conflicting ADR/scope choices rather than silently changing
  them.
- Heavy proof, hook-backed commits and certification require scheduled windows;
  no merge/deployment authority follows from spec or outline approval.
