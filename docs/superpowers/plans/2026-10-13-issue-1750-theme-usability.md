# Issue #1750: Theme usability implementation outline

- Status: Approved outline; human said “approved” through the established
  Speaker.
- Human outline approval governs the pre-metadata document SHA-256
  `ae201d07737b6dc5637bf80d112e398a6eb93704b85b2e13403aa9170c93fe66`. Execution
  still requires Lead admission and the prerequisites below.
- Execute with `jaunder-iterate`, using `jaunder-dispatch` for bounded tasks.
- Trigger: public ZIP-import optional-name/default and validation-order
  behavior.
- Authoritative contract:
  [current spec](../specs/2026-10-13-issue-1750-theme-usability.md).

## Scope and approval provenance

In: shared validated-name access, compatible importer behavior, ZIP-first UI,
activation feedback, Advanced disclosure, reachability, maintained docs and
proof. Out: new parser/endpoint/schema/storage/auth/engine/protocol, automatic
retries, implicit naming fallbacks, remote discovery and new snapshot variants.

Human said **“approved”** through the established Speaker in direct response to
JAU-1750-SPEC-001's manifest-default/narrow-importer proposal, SHA-256
`7ea0ca96897eb47d46c82ec0059072592237dde122515dbdb70329792e6fb84c`. Reviewed
spec compatibility precision before metadata edits has SHA-256
`a5787a2d21923af248b4fa1d9b46cca0c3f52e9596e230225ab458e997d149f5`. Its
absent-versus-explicit-name clarifications preserve callers and validation;
Manager qualifies them as nonmaterial, not direct human approval of that hash.
Approval hashes identify their original documents, not resulting metadata bytes.

## Key contracts and ordered tasks

- [x] 1. Compatible ZIP import with validated manifest-default catalog naming.
  - Contract: shared package validation supplies name metadata; absent multipart
    `name` uses it, valid explicit `name` wins, invalid/blank explicit input
    fails. Preserve response shape, scope-first admission, bounded archive
    consumption, effective catalog-name validation and no partial import/rename
    workaround.
  - Proof: shared-validator/name tests plus real-router multipart integration
    cases for omission, overrides, manifest/filename disagreement, invalid
    names, malformed packages, collisions, ordering, limits and authorization.
    Database-touching tests retain `#[apply(backends)]` parity. Focused lanes:
    `cargo xtask test-local -- -p host theme_package` and
    `cargo xtask test-local -- -p jaunder theme_import_zip`, via `devtool run`.
- [x] 2. Small complete ZIP-import/use UI tracer, dependent on task 1.
  - Contract: default ZIP-only form; blank Advanced override omits `name`.
    Catalog choice is not activation. Explicit use publishes then selects the
    same custom Theme ID/scope only on confirmed publication. Built-in/inherit
    choices use selection alone. Keep operator Site default deliberate.
  - Proof: maintained `theme-management.spec.ts` helpers drive import, catalog
    choice, successful use, fresh signed-out author presentation and private
    Studio preservation; prove pending duplicate/scope changes are blocked.
    Focused lane:
    `devtool run -- cargo xtask e2e-local theme-management.spec.ts`.
- [ ] 3. Complete failure, Advanced, reachability, guidance and visual proof.
  - Contract: publication of an already-selected Theme ID advances its revision;
    later selection failure is not rollback. Database/transport/decode
    uncertainty stops chaining, rereads catalog/selection, and reports reread
    errors honestly. Keep accessible progress/errors outside Advanced and all
    controls reachable.
  - Proof: host-tested state transitions; browser stage failures/uncertainty,
    already-selected publication, keyboard disclosure/focus/status, recovery,
    operator scope and bottom reachability. Update current theme/flow guidance.
    Finish matched 390×844 and 1440×900 BEFORE/AFTER pairs and wide signed-out
    `/~user`; retain existing backend/browser integration-boundary proof policy.

## Prerequisite and risk checks

- Accepted actual BEFORE capture is required before task 2 presentation
  mutation. The latest consumed attempt produced only initial `/themes` images
  at 1440×900 and 390×844; selected-draft, scroll and public captures remain
  outstanding. No runtime is ongoing. Capture uses current unmodified
  management/editor counterparts, standard author data, a minimal schema-1 ZIP,
  device scale 1 and light mode; not original Tailwind. No runtime execution is
  authorized by this metadata-preparation assignment.
- Keep API wire compatibility and authorization ahead of archive consumption;
  default naming must not duplicate ZIP parsing/manifest validation.
- Preserve ADR-0184/0197, private/no-store drafts, isolated previews, quotas,
  immutable revisions, public/private boundaries and existing inheritance.
- Keep pure decisions host-tested and views thin; no lint/coverage suppression.
- Before implementation: cold outline review, disposition of every finding, then
  human outline approval. Lead owns acceptance and subsequent proof admission.
