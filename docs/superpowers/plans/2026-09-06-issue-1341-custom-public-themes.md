# Custom Public Themes Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for independently
> owned slices. This outline exists because issue #1341 adds schema, filesystem
> content, storage concurrency, authorization, an untrusted CSS/archive
> boundary, public cache identity, and cross-projector/CSR contracts.

Authoritative spec:
`docs/superpowers/specs/2026-09-06-issue-1341-custom-public-themes.md`

## Scope

In:

- The version-1 public Style Contract and trusted paint boundary.
- Host-only Theme Package validation/transformation and content identities.
- Owner catalogs, drafts, published revisions, selection, quotas, retention,
  garbage collection, backup/restore, and SQLite/PostgreSQL parity.
- Exact Media role bindings and deterministic header pools.
- Immutable public assets, projector/CSR presentation, authenticated preview,
  Studio management, and browser evidence.

Out:

- Custom templates, JavaScript, server hooks, external resources, visual-token
  schemas, page builders, per-viewer selection, and request-random images.
- Changes to Syndication output or the `RenderedHtml` sanitization contract.
- Reclamation before the approved cache-safe retention deadline.

## Task outline

- [ ] Task 1: Land Style Contract version 1 for built-in public themes.
  - Contract: pure public render leaves emit the exact `data-jaunder-part` hook
    table from the spec inside one versioned theme surface; an unthemeable paint
    containment/low-stacking parent keeps authenticated owner chrome in a higher
    sibling context; public layout leaves inline declarations; private routes
    remain Studio-only.
  - Verification: route fixtures prove exact hook counts, landmark element
    types, route presence, accessible source order, retained textual site
    identity, empty decorative-image alt text, and projector/CSR coincidence;
    authenticated flash/layout-shift checks and an actual Chromium/Firefox smoke
    cover all built-ins at desktop and mobile widths.

- [x] Task 2: Implement the bounded host-only Theme Package compiler.
  - Contract: `host` owns the closed ZIP/manifest parser, media/font validation,
    parser-backed CSS scoping and global-name rewriting, RFC 8785
    canonicalization, and the three approved non-circular digest encodings. It
    exposes opaque `ValidatedThemePackage` and `CompiledThemeRevision` values.
    No ZIP, CSS, image, font, filesystem, or database dependency enters `common`
    or the wasm graph.
  - Verification: adversarial parser tests cover every archive, limit, MIME,
    URL, selector, at-rule, font/keyframe, canonicalization, and digest
    boundary; deterministic byte fixtures plus dependency/license and common
    wasm-closure gates prove the host-only boundary.

- [x] Task 3: Establish the typed theme aggregate and relational contract.
  - Contract: `common` owns wasm-safe `ThemeId`,
    `PublicThemeSelection::{BuiltIn, Custom}`, and role-tagged public
    digest/value types. Paired additive migrations create owner catalog, draft,
    immutable revision, content eligibility, role/pool, selection, quota, and
    retention rows and backfill legacy built-in site/author selections. The
    current config path remains readable until Task 5's deployable cutover. A
    complete `ThemeStorage` trait and both backend implementations own
    catalog/draft/revision/binding primitives inside caller-owned `WriteScope`
    transactions. AppState constructs the store; consumers receive exact traits
    through server context.
  - Verification: migration upgrades and schema parity; `#[apply(backends)]`
    CRUD/revision/reference primitives; case-insensitive per-owner name
    uniqueness, all built-in-label collisions, and same-name different-owner
    allowance; explicit restore ordering, typed-column inventory, and relational
    backup fixture shape.

- [x] Task 4: Materialize, publish, retain, and collect immutable theme content.
  - Contract: `ThemeAssetManager` consumes compiler-minted values and owns this
    idempotent protocol: install exact immutable blobs under sorted content
    locks before database visibility; then, in one short `WriteScope`, call the
    single quota-admission primitive and mint published/retained serving
    eligibility. Confirmed failure may remove only newly installed blobs after
    an unreferenced recheck; commit-indeterminate outcomes retain blobs for
    reconciliation. Startup reconciliation and opportunistic collection account
    for orphan files. Collection rechecks references and deadline under locks,
    commits eligibility/quota detachment before unlink, and retries failed
    unlinks.
  - Contract: quota acquisition order is site-wide quota row, owner quota row,
    then content rows in ascending digest order; every
    create/import/draft-growth/publish/detach/collection path uses that owner.
    SQLite relies on its write-first transaction, and PostgreSQL takes those
    rows in the same order. Draft-only hashes never receive public-serving
    eligibility. Backup and restore move immutable CSS/assets together with
    eligibility and retention rows; restore validates every referenced blob
    before any restored content becomes public.
  - Verification: dual-backend atomic publication, quota boundaries, competing
    owners, same-blob deduplication, detach, retention-clock, and collection
    tests; injected failure/crash tests at materialize, transaction, confirmed
    failure, commit-indeterminate, eligibility detach, and unlink boundaries;
    restart reconciliation proves every legal state is either referenced and
    readable or unreferenced and safely reclaimable. Both-backend full
    rows-plus-content backup/restore round trips cover missing and corrupt blob
    failure without public eligibility.

- [x] Task 5: Cut selection and public presentation over to custom identities.
  - Contract: one final wasm-safe published-presentation DTO contains built-in
    or custom identity, current published revision, immutable stylesheet URL,
    and optional route-resolved logo/header URLs. `resolve_public_theme` returns
    that shape while preserving #21 site/author precedence, missing/corrupt
    fallbacks, and database-error propagation. A selected stable Theme ID
    advances on publish without another selection mutation. Task 5's paired
    cutover migration removes legacy selection rows, config keys, accessors, and
    generated endpoint assumptions only when the new resolver and every profile,
    tracing, seed, page-state, test, and built-in caller switch together; no
    fallback or alias remains.
  - Verification: dual-backend resolver tests cover precedence, inheritance,
    unpublished/deleted themes, publish advance, malformed values, and read
    failures; serde fixtures pin the final DTO; symbol-aware caller migration
    and built-in API/browser regressions prove the old path is gone.

- [ ] Task 6: Add race-free image roles, pools, shuffle, and atomic removal.
  - Contract: package bindings validate against the candidate revision. For
    Media operations, acquire all `MediaContentLocks` in ascending content-hash
    order before `WriteScope`; inside it, acquire
    `MediaStorage::lock_media_reference` locks in stable exact-`MediaRef` order,
    verify persisted session-derived ownership, and update bindings/references.
    Hold content guards through the confirmed post-transaction decision and
    extend the existing delete/reclaim predicate rather than adding another.
  - Contract: one storage-owned `remove_theme` orchestration uses that sequence
    to authorize the owner and atomically update site/author selection fallback,
    catalog visibility, revision retention metadata, and all Media binding rows.
    Physical reclamation follows only confirmed database detachment;
    commit-indeterminate callers revalidate the complete state.
  - Contract: one pure selector owns the approved versioned pool encoding,
    canonical route formatter, duplicate policy, digest-modulo choice, and
    persisted shuffle seed.
  - Verification: digest/pool byte vectors; dual-backend
    fixed/mixed/pool/shuffle and candidate-binding tests; concurrent bind/remove
    versus Media delete/reclaim; atomic-removal rollback and
    commit-indeterminate revalidation; cross-owner, owner-report, and backup
    coverage.

- [ ] Task 7: Serve and render deterministic published custom presentation.
  - Contract: public handlers resolve a hash through Task 4's
    published-or-retained eligibility—not raw file existence—then serve compiled
    CSS/assets with stored MIME, `nosniff`, ETag, and one-year immutable
    caching. Draft-known hashes are anonymous-inaccessible. Projector and web
    render/navigation code consume Task 5's DTO without changing it. Initial
    HTML links custom CSS before paint; initial CSR mount and each public
    navigation atomically adopt the destination stylesheet and role images;
    private navigation removes them and uses Studio.
  - Verification: strict-path, MIME, cache, conditional-request, and retained
    content integration tests; a known draft hash is inaccessible before
    publish, public after publish, and readable through retention; projector
    byte/ETag determinism and CSR coincidence; fresh-load/pushState smoke proves
    no theme flash, stale stylesheet/image, or custom CSS on private routes.

- [ ] Task 8: Expose complete owner-authorized management and preview APIs.
  - Contract: a new web theme vertical owns catalog CRUD, CSS/ZIP import/export,
    draft/assets/defaults, publish, selection, atomic removal, Media
    binding/pool, shuffle, and preview endpoints. Session identity is the sole
    owner input; operator/site and author/catalog authorization are distinct;
    cross-owner misses are existence-masked. Every draft-derived response is
    `private, no-store`; export uses safe attachment disposition and omits local
    Media bindings. Preview renders the eligible site's or author's real Style
    Contract without selection.
  - Contract: rate and one-in-flight permits are keyed only from the
    authenticated principal and acquired before multipart body read,
    decompression, or compile. They release after success, validation failure,
    cancellation, and commit-indeterminate completion.
  - Verification: API/router tests cover every auth class, ownership mask,
    multipart/limit/error mapping, MutationOutcome revalidation, all draft
    headers, preview isolation, and atomic publish failure. Controlled overlap
    proves same-owner saturation changes no state, different owners remain
    independent, rate limits precede expensive work, and every exit releases its
    permit. Export checks adversarial-name filename safety, disposition,
    `private, no-store`, archive membership, and absence of bound Media IDs.
    Server-fn registrar entries, flow endpoint ownership, CSR evidence rows, and
    focused Playwright transport checks land in this task's commit.

- [ ] Task 9: Build the accessible Studio theme-management journey.
  - Contract: one mounted private route lets an author manage their catalog and
    an operator manage the site catalog, with programmatically distinct scopes.
    It supports create/rename/delete, CSS editing, package import/export, asset
    management, preview/publish, built-in/custom selection and author
    inheritance, fixed logo/header binding, explicit header pools, and
    `Shuffle assignments`. Confirmed/indeterminate mutations reuse existing
    revalidation semantics; private UI never evaluates custom CSS.
  - Verification: component/page-state tests plus Playwright journeys cover
    keyboard/accessibility state, author/operator authorization, failures and
    rereads, package round-trip, preview, publish/select/delete fallback,
    binding/pool/shuffle, responsive preview, and survival in a fresh browser
    context.

- [ ] Task 10: Close cross-surface evidence and documentation.
  - Contract: extend the existing theme precedence suite rather than creating a
    second convention; complete route journey and aggregate e2e evidence; retain
    the approved ADR/architecture/glossary projection; remove obsolete
    built-in-only comments, fixtures, endpoints, and paths exposed by the clean
    cutover.
  - Verification: targeted server/web/storage suites, both backends and both
    supported browsers for the complete end-to-end contract. A hostile-but-valid
    custom theme uses fixed/inset positioning, extreme z-index, transforms,
    filters, and overflowing descendants; Chromium and Firefox prove trusted
    owner controls remain visually above, hit-testable, and unaffected by its
    selectors. Documentation gates and the repository verification lane selected
    by `jaunder-iterate` precede the final `jaunder-commit`/ship gates.

## Ordering and handoff

- Tasks 1 and 2 are independent and may execute in parallel with disjoint file
  ownership: Task 1 owns public markup/CSS; Task 2 owns package compilation and
  host-only dependencies.
- Task 3 consumes only Task 2's validated value contract. Task 4 consumes Tasks
  2 and 3. Task 5 consumes Tasks 3 and 4. Task 6 consumes Tasks 3–5. Task 7
  requires Tasks 1, 4, 5, and 6. Task 8 requires Tasks 2–7; Task 9 consumes Task
  8's complete wire API. Task 10 follows all behavior slices.
- Tasks 3–8 are serialized at their shared storage/DTO/composition seams.
  Parallel delegation remains appropriate within a task only after the
  integration owner assigns disjoint backend, pure-function, or test files
  against the named contract.
- Each completed checkbox advances through `jaunder-iterate`; each independently
  reviewable slice reaches `jaunder-commit` with its focused evidence. No commit
  carries a `Co-Authored-By` trailer.

## Cross-task contracts

- Host parsing has one minting door for `ValidatedThemePackage` and
  `CompiledThemeRevision`; downstream storage never reconstructs validation or
  trusts client-supplied paths, MIME, hashes, owners, or quota totals.
- `ThemeId`, `PublicThemeSelection`, role-tagged digests, and the final
  published presentation DTO are wasm-safe values in `common`; archive bytes,
  authored CSS, storage rows, quota state, and drafts never enter `PageSeed` or
  the CSR graph.
- `ThemeStorage` owns relational primitives inside caller-owned `WriteScope`
  transactions. `ThemeAssetManager` owns immutable filesystem operations and
  reconciliation. The publish protocol is materialize → transactional
  quota/eligibility commit → confirmed-failure cleanup or indeterminate
  retention; collection is transactional eligibility detach → unlink/retry.
- Public handlers require a published-or-retained eligibility row in addition to
  matching content bytes. Draft/preview storage and URLs remain separate and
  owner-authorized even when their bytes hash to known public content.
- One storage-owned removal orchestration composes selection fallback, catalog
  detach, retention state, and Media reference release in one transaction under
  the established outer content-lock/inner MediaRef-lock order.
- One typed content URL builder owns public CSS/asset paths. One typed route
  formatter and one pool selector own header-assignment bytes. Projector, CSR,
  preview, and tests consume those owners rather than re-derive strings.
- Theme storage/services are constructed at the composition root and injected
  per consumer; they are not exposed as a heterogeneous service locator through
  AppState.

## Risk checks

- Keep new parsers and decoder dependencies host-only; run dependency and wasm
  graph gates after manifest changes.
- Preserve injective, versioned digest framing and the asset → transformed CSS →
  revision order; never place a revision digest inside bytes that define itself.
- Enforce actual streamed/decompressed/decoded limits and acquire abuse permits
  before expensive work.
- Preserve the database/filesystem state machines on ordinary failure, process
  interruption, and commit-indeterminate outcomes; reconciliation must never
  delete possibly referenced bytes or make draft-only bytes public.
- Use the single site → owner → sorted-content quota lock order and the outer
  sorted `MediaContentLocks` → inner sorted `lock_media_reference` order on both
  backends.
- Extend the existing Media deletion predicate rather than creating a second
  reference check; Post Media extraction remains unchanged.
- Keep custom paint below trusted owner controls, all custom fetches
  same-origin, and every private/draft response owner-authorized and
  `private, no-store`.
- Preserve anonymous byte identity: selected revision, stylesheet, image roles,
  pool assignment, and shuffle seed reach final HTML/ETag without viewer,
  session, or browser inputs.
- Update every exported `Theme`/`PublicPresentation` caller through symbol-aware
  references; remove obsolete config keys and generated endpoint aliases in the
  same cutover.
- Treat package format, Style Contract hooks, digest fixtures, public URLs, and
  pool-selection vectors as compatibility tests, not implementation details.
