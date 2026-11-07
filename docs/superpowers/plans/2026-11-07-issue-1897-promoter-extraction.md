# Python ADR Promoter Extraction Implementation Outline

> Execute with dev-cycle-iterate. Outline needed because the approved change
> crosses parser/source fidelity, privileged controller recovery, Nix package
> selection, and two independently gated PRs. Human approved this outline and
> implementation via `2af04086-a4e5-4c71-877c-4961f01b4053` /
> `issue-1897-python-outline-v1`, for exact outline SHA-256
> `8bb64a64b0b8d06454ddfe3c3ad2380bd59c667c07e74078ffe0de264db19fa4` and design
> commit `f2bdfaff9b204e3ef7177c908d6060955a908831`. This authorizes
> implementation within the approved design and outline; PR 1 and PR 2 retain
> their separate human merge gates. It grants no privilege expansion or cleanup
> authority.

## Execution summary

Work only to the approved design in
[`2026-11-07-issue-1897-promoter-extraction.md`](../specs/2026-11-07-issue-1897-promoter-extraction.md).
First retire the highest-risk assumption: Marko source-span edits preserve exact
source bytes and the pinned Nix output is selected from the active checkout.
Then establish the sole Python mutation owner and controller behavior with fake
Git/`gh` seams and complete fixture-based mutation proof. Deliver PR 1, observe
a trusted-main workflow operation and ordinary PR/ref/check result, then prepare
PR 2 as a separately reviewed and human-merge-gated retirement. A failed PR 1
Nix-selected path blocks PR 2 until repaired or reverted.

## Task outline

- [ ] Task 1: Select the pinned package from the active checkout and prove
      Marko's source fidelity.
  - Contract: exact locked Python/Marko/pytest/GitHub/Git/Prettier choices;
    Marko GFM `Link`/`Image` spans patched into original bytes through a tested
    character-to-byte boundary map; no Markdown serialization or fallback
    scanner. Keep the three approved eligibility differences explicit.
  - Verification: offline Nix evaluation/build and Just recipe selection of the
    exact current-checkout package; source-span fixtures for tables, images,
    Unicode, normalized line endings/form-feed/NUL, code, reference links and
    all three eligibility differences.
- [ ] Task 2: Make Python the single mutation owner and prove complete promotion
      and README synchronization.
  - Contract: Python implements both mutations; legacy `promote` and
    `sync-readme` are fixed delegates; stage/admission order and existing byte
    behavior remain as specified. Use the approved fake Git seam and temporary
    tracked fixture repository.
  - Verification: compare complete expected bytes, assigned numbers, staged
    paths, heading/status/citations, link edits, README markers/curated titles
    and warnings; exercise `sync-readme`; run existing Rust ADR format/parity/
    link gates on resulting output. The existing probes are not this proof.
- [ ] Task 3: Port controller transitions and preserve operational safeguards.
  - Contract: approved fake `gh`/Git adapters; preserve authority, coalescing vs
    dequeue recovery, immutable head/diff, exact identity/check correlation,
    safe refusal vs error, lease verification, visible failures and exit
    outcomes. No live GitHub or privileged call is a unit-test seam.
  - Verification: transition, error, expected-OID lease refusal/postcondition,
    and every outcome tests against the approved contracts.
- [ ] Task 4: Deliver PR 1 with one writer and temporary controller
      compatibility.
  - Contract: package/Nix/Just/workflow and tests; remove Rust mutation bodies;
    preserve `adr promote`/`sync-readme` delegates and temporary `adr promoter`
    controller delegate. Separate PR review and human merge gate.
  - Verification: PR 1 acceptance and checks; then observe one trusted-main
    workflow operation and its ordinary PR, ref and check result before PR 2.
- [ ] Task 5: Migrate actual callers and retire the old controller in PR 2.
  - Contract: update controller rerun guidance and remaining callers to Just;
    remove obsolete Rust controller/state/GitHub/Git adapters/tests and
    temporary `adr promoter` CLI/help/dispatch/delegate. Keep only the two fixed
    mutation/recovery delegates. Separate PR review and human merge gate.
  - Verification: confirm caller inventory is migrated, PR 2 acceptance passes,
    and complete the issue only after the second gated merge and acceptance.

## Risk checks

- No direct promotion or privileged live operation substitutes for fake-adapter
  unit proof; promotion is acceptance and merge queue alone writes `main`.
- Keep the ADR proposed until automated promotion; update architecture prose to
  reflect verified implementation at each delivery boundary, distinguishing
  remaining prospective work. Do not rewrite accepted ADR Decision text or
  broaden permissions.
- PR 2 is blocked until the PR 1 trusted-main observation succeeds; keep PR 1
  compatibility if its selected path fails.
