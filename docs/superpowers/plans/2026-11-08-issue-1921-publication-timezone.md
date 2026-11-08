# Issue #1921 Implementation Outline

> Execute with `dev-cycle-iterate`. Outline needed because the approved
> `docs/superpowers/specs/2026-11-08-issue-1921-publication-timezone.md`
> requires coordinated SQLite/PostgreSQL Post and revision migrations, AtomPub
> and web API changes, and an Emacs DATE/`JAUNDER_DATE_TZ` round trip. No
> implementation starts until this outline is independently reviewed and
> approved.

## Execution summary

First prove the highest-risk local Emacs invariant: an unchanged DATE can retain
the exact UTC instant, including fractional seconds and a later DST-fold
occurrence, while its wall time and `JAUNDER_DATE_TZ` move to the latest editing
zone. Then persist that zone with prior-state revisions, carry it through web
and AtomPub, render all approved Published displays, and finish Emacs
push/pull/reconciliation. Keep wire, storage, and local state incomplete until
the end-to-end round trip is proven; no intermediate slice is feature-complete.

## Task outline

- [x] **1. Prove and implement local DATE/zone instant preservation.**
  - Contract: first add a narrow regression at the existing ERT date-conversion
    seam. Compare civil DATE fields against the saved instant rendered in the
    recorded old zone; when unchanged, retain the exact marker, fractional
    precision, and later-fold identity while re-rendering DATE in the latest
    zone. Changed DATE follows ADR-0182. Keep this helper local to Emacs; raw
    Org remains ADR-0155 final-value validation.
  - Verification: fractional-second, later-fold, changed-date, and gap/fold ERT
    proofs via `traced-run -- emacs --batch -Q -l elisp/scripts/run-tests.el`.
    Resolve this risky invariant before database, protocol, or browser
    expansion.

- [x] **2. Persist and validate timezone state with prior-state revisions.**
  - Contract: store a required supported named IANA `timezone` on Posts and
    immutable revision snapshots. Accept aliases with a named `iana_name`,
    reject numeric offsets and `Etc/Unknown`, and retain exact accepted
    spelling. Paired SQLite/PostgreSQL migrations explicitly backfill UTC and
    report the fallback count; backup/restore round-trips current-schema data
    without inventing ADR-0174 cross-version conversion. Zone-only updates are
    changes; the inserted revision stores the complete OLD state/zone and
    current Post stores the new zone. Other revision-producing paths preserve
    timezone.
  - Verification: add validation, migration/store/revision tests on both
    backends, and current-schema backup/restore tests. Focused proof:
    `traced-run -- cargo xtask test-local -- -p storage` covers the storage
    package only; server storage integration coverage is in the `jaunder`
    package in Task 3.

- [ ] **3. Carry editor timezone through web and AtomPub read/write paths.**
  - Contract: `PostInputs.timezone` is optional JSON input. On each browser
    create/edit submission capture and validate the supported IANA device zone;
    capture failure explicitly sends UTC and shows a warning. Test that behavior
    separately from old-client API omission (create UTC, update preserves saved
    zone). `<j:timezone>` uses `https://jaunder.org/ns/atompub` and is
    advertised as the version-1 `timezone` feature. Explicit transport timezone
    takes precedence over a recognized named `JAUNDER_DATE_TZ` header for the
    saved zone. Raw Org `JAUNDER_DATE_UTC` remains a final-publication
    assertion; marker-free raw Org stays supported; structured Atom
    `<published>` retains precedence over header lifecycle. Present
    empty/invalid input rejects. Never change the publication instant merely
    because timezone changed.
  - Verification: API/AtomPub tests cover capture vs omission, validation,
    precedence, readback, and zone-only no-op behavior. The focused `web::posts`
    filter does not cover AtomPub. Use
    `traced-run -- cargo xtask test-local -- -p jaunder` for the full product
    package, including server integration tests, alongside Task 2's storage
    package tests.

- [ ] **4. Render all approved publication displays from the saved zone.**
  - Contract: propagate timezone through `PostRecord`, current/revision history
    DTOs and rendered projections. Format publication instants in-zone on public
    cards/permalinks, scheduled list, edit scheduled notice, current history and
    each revision's Published value using that revision's zone. Audit
    timestamps, UTC ordering and permalink dates retain existing semantics;
    projected and reactive public markup stay coincident.
  - Verification: cover Jiff DST abbreviations, timeline/permalink projection,
    scheduled/edit notices and current/revision history; run
    `traced-run -- cargo xtask e2e-local end2end/tests/posts.spec.ts` and
    `traced-run -- cargo xtask e2e-local end2end/tests/history.spec.ts`.

- [ ] **5. Preserve Emacs Org instant identity through publish and reconcile.**
  - Contract: capture and validate the latest named device zone before entry
    construction; a missing/invalid Emacs capture explicitly uses UTC and shows
    a warning, tested separately from successful capture. Send the zone as
    `j:timezone`. For local existing Posts only, compare DATE civil fields with
    DATE rendered from saved `JAUNDER_DATE_UTC` in recorded old
    `JAUNDER_DATE_TZ`. Unchanged DATE carries the exact saved UTC marker
    (precision and fold identity included) and is re-rendered in the new zone.
    Changed DATE is interpreted in the latest zone under ADR-0182. Missing or
    invalid baseline requires pull/reconcile or an actionable stop before PUT.
    Only confirmed writes replace local `JAUNDER_DATE_TZ` and paired DATE. Pull
    adopts server timezone without recapturing device timezone. Raw Org ingress
    keeps its separate ADR-0155 final-value assertion and marker-free
    compatibility.
  - Verification: ERT covers unchanged fractional-second/later-fold values,
    changed DATE, scheduled times, invalid/missing marker, capture fallback
    warning, failed/stale conditional write, pull and reconciliation. Focused
    commands: `traced-run -- emacs --batch -Q -l elisp/scripts/run-tests.el`;
    `traced-run -- cargo build -p jaunder`;
    `traced-run -- env JAUNDER_TEST_BINARY=target/debug/jaunder emacs --batch -Q -l elisp/scripts/run-integration-tests.el`.

- [ ] **6. Add bounded local-original recovery evidence.**
  - Contract: build only the read-only `JAUNDER_ID` → `JAUNDER_DATE_TZ` local
    manifest extractor. Preserve exact valid alias spelling; report local
    duplicates/missing/invalid values. Report server IDs absent from originals
    only when a separately authorized read-only Post-ID inventory is supplied.
    No production access, DB update, import, timestamp rewrite or revision
    rewrite is part of this task; original path/sample may arrive later and is
    not a blocker to fixture-backed extractor design.
  - Verification: fixture tests cover exact ID joins, alias spelling, duplicate
    and invalid values, and the explicit absent-server-inventory limitation.

- [ ] **7. Integrate, conform, document, and ship the approved change.**
  - Contract: inspect the complete spec diff and all implementation boundaries;
    reconcile every acceptance criterion, current docs/ADR needs, migrations,
    backups, API, AtomPub, rendered surfaces and Emacs round trip. No broad
    green intermediate claim while end-to-end protocol/storage state is
    incomplete. Finish approved work and applicable focused tests before the
    project pre-PR gate; then use normal review/ship flow. Merge requires its
    separate explicit PR/head approval.
  - Verification: run `traced-run -- just validate-no-e2e` when required by the
    named risk or missing equivalent; run focused `e2e-local` proofs above for
    the changed browser paths. Let CI provide its required platform/backend
    matrix. Apply `jaunder-review` and `jaunder-ship`; do not merge under this
    outline.

## Risk checks

- Never re-interpret an unchanged Org DATE using a new zone. Local Emacs marker
  comparison is not server raw-Org bookkeeping; preserve ADR-0155's final
  effective publication assertion and structured Atom precedence.
- All persisted state, revision snapshots, backup paths and storage tests remain
  dual-backend. Old-schema backup conversion is not introduced.
- Pull adopts server zone; only an authored successful write captures the latest
  editor zone. Conditional-write conflicts leave local DATE/zone untouched.
- No original-file path/sample, production Post-ID inventory, production read,
  import or migration execution is assumed authorized.
- Gate and commit execution follows `jaunder-commit`; retain one checkout
  executor/writer. This outline is not spec approval, implementation authority,
  merge approval, or cleanup authority.
