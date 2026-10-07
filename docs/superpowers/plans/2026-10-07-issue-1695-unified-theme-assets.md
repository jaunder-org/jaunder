# Issue #1695: Unified styling assets implementation outline

> Execute with `jaunder-iterate`; delegate bounded tasks through
> `jaunder-dispatch` when useful, with one writer in this checkout. This outline
> exists because the approved spec changes package production, durable
> ownership/references, startup, and public asset URLs.

Approved contract:
[unified Theme Packages and styling assets spec](../specs/2026-10-07-issue-1695-unified-theme-assets.md).

## Scope

In: system-managed bundled Theme Packages; shared application/theme content
lifecycle; release-safe documents and thumbnail consumers; isolated upgrade,
rollback, recovery, cache-transition, and visual-parity evidence.

Out: image-sizing changes, production operations, visual redesign, theme-copy
UI, custom permission relaxation, and CSR/Media backing-store migration. The
mutable non-manifest asset inventory is required; unrelated redesigns are not.
The tasks below are dependent slices of one asset contract, not independent
subsystems or authorization for parallel implementation writers.

## Task outline

- [ ] Task 1: Establish shared compiled artifact and package contracts.
  - [x] Extract the shared immutable content-response primitive, preserving
        streamed 200 and conditional 304 behavior; focused checks pass on both
        backends. System and thumbnail consumers will adopt it in task 4.
  - [x] Share stable packaged-default header selection and typed immutable
        content addressing across publication, public/draft presentation, and
        thumbnails. Compiler-minted content views couple exact digest/MIME/bytes
        without duplicating the owning revision or changing identity framing.
  - Contract: one compiler-minted package revision/asset representation and one
    exact digest/MIME/bytes representation. Trusted application CSS has a closed
    system-only role, not a bypass in the untrusted package validator. Custom
    mutation APIs continue to accept only operator/author-owned catalog inputs.
  - Verification: existing package parser/compiler, public presentation,
    conditional HTTP, and thumbnail tests remain unchanged and green.
  - [ ] Capture reproducible visual baselines before the first
        presentation-affecting edit.

- [ ] Task 2: Produce bundled packages and application styling artifacts.
  - Convert Studio, Terminal, and Reader presentation source to portable package
    inputs; compile through the same package boundary used by custom
    publication. Separate private/trusted application styling from scoped public
    presentation without changing appearance or widening custom CSS permissions.
  - Contract: one deterministic system artifact inventory maps stable theme
    names and the application-CSS role to exact bytes, MIME, revision, and
    content digests. The host and Nix use the same producer; no consumer guesses
    filenames or reimplements package hashing. Artifact production precedes CSR
    shell generation/server embedding without a binary-build dependency cycle.
  - Qualification input contract: define three deterministic fixture variants
    for the completed implementation: A (unaltered), B-app (only a bounded
    application-CSS presentation declaration differs), and B-theme (only a
    bounded Studio-package presentation declaration differs). Commit the fixture
    definitions as test inputs, not changes to the shipped styles. Each variant
    passes through the real artifact producer and server build.
  - Verification: all shipped packages validate; unchanged bytes retain
    identity; changed application/public-theme bytes change only their
    corresponding addresses. Verify artifact admission and Nix source-closure
    inputs. Normal release derivations cannot select fixture variants; test that
    their inventory excludes qualification CSS.

- [ ] Task 3: Add atomic system publication to the existing content lifecycle.
  - Add paired SQLite/PostgreSQL migrations and exact storage operations for
    system-owned package revisions and current application/package references.
    Reuse the content eligibility, digest locks, installer, and collector rather
    than creating a second system blob store or cleanup service.
  - Contract: system ownership is explicit, cannot be supplied by custom catalog
    callers, and is outside custom quotas. References for one installed system
    inventory advance in one `WriteScope`; custom live references remain intact.
    Replaced content retains its deadline and original bytes. Sorted digest
    locks cover installation, reference changes, and collection; file I/O
    remains outside the short database transaction. Commit-indeterminate
    outcomes retain recoverable content and cannot be mistaken for confirmed
    readiness.
  - Verification: both-backend tests for initial/repeated install, upgrade,
    rollback, shared system/custom digests, quota isolation, failure atomicity,
    retention boundaries, and orphan/uncertain-commit reconciliation. Exercise
    the existing backup formats with retained system bytes and eligibility.

- [ ] Task 4: Wire startup and all document/serving consumers to the inventory.
  - Install and reconcile current system artifacts before readiness; inject only
    the exact handles/typed artifact values each subsystem needs (ADR-0016).
    Resolve bundled and custom selections through the shared package
    presentation path, preserving existing tokens, fallback, inheritance, and
    custom controls.
  - Contract: generated CSR shell, public projector, authenticated entry shell,
    and CSR theme-link adoption use the same immutable identities. Startup
    errors refuse serving. The existing `/theme/{digest}` contract serves both
    system and custom content through shared response semantics; no old digest
    is redirected to new bytes. Both retired stylesheet paths return 404 before
    SPA fallback.
  - Thumbnail transport remains database-independent, consumes the same compiled
    inventory and response primitives, and loses its separate `ServeEmbed`
    mount.
  - Verification: real router/document/startup tests on both backends, package
    transition tests, thumbnail command integration, immutable 200/304 checks,
    legacy-route 404 checks, and generated-shell/staging/Nix parity checks.

- [ ] Task 5: Prove deployment transitions and unchanged visual presentation.
  - Extend the existing isolated production-baseline coordinator/Playwright
    bridge with a bounded cache-transition proof. It already switches immutable
    packages behind stable Caddy origin while retaining a browser context;
    ordinary `e2e-local` starts one fresh server/storage lifecycle and cannot
    itself prove a two-release transition. Keep this proof distinct from #1419's
    final qualification and reuse its lifecycle, privacy, and evidence controls.
  - Input ownership: this task adds a qualification-only mode that builds A,
    B-app, and B-theme from task 2's fixtures at one clean committed revision
    containing tasks 1–4's complete unified implementation. The existing
    internal qualification package/VM construction seam supplies these distinct
    immutable packages; do not pretend the existing
    distinct-source/target-commit acceptance command can provide this matrix. No
    production runtime flag, public package option, or ordinary release output
    selects fixture variants. Evidence records the resolved source commit,
    fixture digests, system asset inventories, derivation/package identities,
    and executing binary identities.
  - Contract: run A → B-app → A and A → B-theme → A behind the same origin,
    preserving a browser context throughout each sequence. Assert that B-app
    changes only application styling identity/presentation and B-theme changes
    only the selected Studio package identity/presentation; original bytes stay
    readable. Use real handlers, not `page.route`, cache clearing, fake cache
    logic, or a new application test endpoint.
  - Contract: application-CSS transitions run on Local and Home; selected public
    package transitions run on Local and an author permalink, not Home. Prove
    cached A bytes and normal B document navigation without clearing the cache,
    plus old-digest readability and rollback. Respect declared document-load
    allowances; assertions use stylesheet readiness and computed presentation.
  - Verification: isolated host qualification proves real package switches,
    cache continuity, compatible rollback, and restored digest readability on
    both backends. Require changed artifact identities/computed presentation;
    unchanged A/B CSS or fresh contexts are not transition evidence. Ordinary
    presentation flows use focused e2e-local followed by applicable lanes.
    Compare before/after narrow/wide Local, permalink, and Home screenshots for
    all public bundled themes and one custom package. Retain actual Safari
    evidence if available; otherwise name the device/browser evidence gap.

- [ ] Task 6: Reconcile asset inventory, documentation, and release evidence.
  - Record every mutable non-manifest asset's cache/identity disposition,
    including favicon and thumbnail consumers. Stable mutable assets must have
    an explicit policy rather than accidental heuristic or immutable caching.
  - Contract: completed implementation replaces the architecture's Committed
    direction prose with verified current reality; ADR/glossary/operator
    guidance agrees with startup and legacy-URL behavior. No generated ADR index
    edits.
  - Verification: spec conformance, custom-theme security and immutable
    CSR/Media regression evidence, formatting/doc gates, final standard/spec
    review, and CI. Report the landed startup/upgrade/recovery contract for
    #1419's final release-candidate qualification without changing its claim or
    running live production work. Present visual pairs with the PR review
    handoff.

## Risk checks and acceptance coverage

- Tasks 1/2/4 cover spec acceptance 1 and package authority/selection parity.
- Tasks 3/4/5 cover 2–6: real cache transitions, exact HTTP identity, backend
  parity, retention and recovery, and database-independent thumbnail delivery.
- Tasks 1/5/6 cover 7/8: baseline-before-mutation, final visual pairs, and
  honest actual-Safari evidence. Task 6 covers 9's complete bounded asset
  disposition.
- Preserve custom validation limits, CSS containment, deterministic revision
  framing, trusted controls, and private-surface styling. No new lint or
  coverage waiver is authorized. CSS movement is not permission to relax visual
  tests.
- Migrations preserve existing custom catalogs and references; system
  publication must not invent author identities, consume reserved custom names,
  reset custom quotas, or let shared-digest collection remove a live
  system/custom resource.
- System artifact inputs must reach the host/Nix shell and server producers.
  Update affected source-closure/invalidation proofs; do not broadly re-admit
  unrelated server sources into the wasm compiler closure.
- Recovery tests extend `server/tests/misc/backup_interop.rs` beyond database
  rows to actual retained styling blobs, eligibility, and served old digests.
  Thumbnail command proof must exercise the real supplied browser as well as its
  existing host transport tests; scripted CDP alone is not visual evidence.
- Use focused Rust proof via `devtool run -- cargo xtask test-local -- ...` and
  ordinary browser-flow proof via
  `devtool run -- cargo xtask e2e-local <spec-or-file:line>`. The
  cache-transition proof uses the existing opt-in
  `cargo xtask production-baseline` lifecycle extended in task 5, not a promise
  that e2e-local swaps releases. Commit/push gates and independent deliverable
  review remain owned by `jaunder-iterate`/`jaunder-commit`; CI supplies
  hermetic PR-boundary evidence.
