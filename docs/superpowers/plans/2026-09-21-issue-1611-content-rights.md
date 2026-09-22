# Content Rights Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for bounded tasks.
> This outline exists because the approved spec adds a persisted dual-backend
> setting, couples User mutations to the transactional feed outbox, and changes
> three public Syndication Feed wire formats.

## Scope

In:

- Add the closed User-wide Content License and its All Rights Reserved default.
- Make Display Name and Content License changes atomically invalidate every
  affected public feed representation.
- Project Copyright Declarations into public Post markup and Atom, RSS, and JSON
  Syndication Feed items.
- Add account controls, exact protocol/backend/integration/e2e coverage, the
  existing Local snapshot update, and review-only visual proof.
- Land the approved glossary, ADR draft, architecture, design, and Style
  Contract projections with the behavior they describe.

Out:

- Schema migrations, per-Post rights state, custom licenses, site-owned notices,
  AtomPub changes, authored-content mutation, or new committed visual states.

## Task outline

- [x] Task 1: Establish the typed Content License and durable User setting
  - Contract: `common` owns a closed `ContentLicense` whose stored/wire values
    are `all-rights-reserved` or the spec's exact SPDX identifiers and whose
    public label, optional SPDX identifier, and optional canonical URL exactly
    match the approved table. `UserConfigKey` admits the value; absent rows and
    old backups resolve to All Rights Reserved; ordinary `user_config`
    backup/restore preserves and validates explicit values without a migration.
  - Verification: dual-backend config tests cover absence/default, every value,
    update, close-and-reopen persistence, invalid restore data, and
    backup/restore; enum/config-key census and round-trip tests cover the
    expanded closed registry. Focused proof:
    `devtool run -- cargo xtask test-local -- -p storage user_config`.

- [x] Task 2: Make feed-visible User mutations transactional
  - Contract: one storage-owned operation updates the Display Name or Content
    License and, only when its feed-visible value changes, derives all Site,
    User, Site Tag, and User Tag URLs from the User's currently active,
    published, Public Posts and their complete tag union. It calls the existing
    `affected_feed_urls`/`enqueue_many` seams inside the same `WriteScope`;
    enqueue failure rolls back the User/configuration update. Profile server
    functions receive only their exact storage/outbox dependencies.
  - Verification: both backends prove complete 4-surface × 3-format fanout,
    tag/path deduplication, no-public-Post and semantic-no-op behavior, and
    rollback on injected enqueue failure. Dual-backend HTTP integration tests
    prove authentication, validation, persistence, and typed mutation failure
    for the Content License endpoint and preserve existing profile behavior.
    Focused proof:
    `devtool run -- cargo xtask test-local -- -p jaunder -E 'test(/^web::profile/)'`.

- [x] Task 3: Emit the exact Syndication Feed rights contract
  - Contract: the format-neutral feed item carries immutable creation year,
    current author name with Username fallback, and typed Content License; feed
    regeneration resolves those values without changing title, summary, or
    rendered-body bytes. Atom emits `rights` plus an RFC 4946 license link when
    applicable; RSS emits the approved Dublin Core and Creative Commons
    namespace elements; JSON emits the exact `_jaunder` object and nullable
    license shape. All Rights Reserved emits no license URL element/link.
  - Verification: serializer tests assert exact XML namespaces/elements and JSON
    keys/types for All Rights Reserved and every Creative Commons mapping;
    regeneration tests prove current-name/current-license behavior and unchanged
    authored/rendered fields. AtomPub regression tests assert unchanged Service
    Documents, Collections, and Member Entries. Existing cache fingerprint,
    publisher-generation, and duplicate-safe at-least-once WebSub behavior
    remains authoritative. Focused proof:
    `devtool run -- cargo xtask test-local -- -p host -E 'test(/^feed::/)'`.

- [x] Task 4: Present and manage Content Rights on the web
  - Contract: `/profile` exposes the typed selector, canonical license links,
    and retroactivity warning. Every public Post renderer emits
    `© YEAR NAME · LABEL` inside the existing `post-footer` Style Contract
    surface and links only a Creative Commons label; Local, User, Site Tag, User
    Tag, and permalink projector/CSR markup remain identical.
    Home/settings/admin gain no global legal footer, and the Style Contract
    gains no new compatibility hook.
  - Verification: pure render tests cover name fallback, exact text/linking,
    source order, and escaping; e2e proves setting persistence and retroactive
    HTML/feed presentation for existing Posts. Update the existing Local
    baselines only through
    `devtool run -- cargo xtask e2e-local --update-visual-snapshots`, then prove
    the focused flows with
    `devtool run -- cargo xtask e2e-local profile.spec.ts` and retain
    review-only Local/permalink desktop/compact before-and-after pairs through
    `visual-proof`, covering both All Rights Reserved and one linked Creative
    Commons choice.

## Cross-task contracts

- `ContentLicense` is the sole parser and metadata authority used by storage,
  web, and all serializers; no task duplicates the label/SPDX/URL table.
- The transaction in Task 2 is the only mutation path that earns feed events;
  Tasks 3 and 4 consume current state and never enqueue independently.
- Copyright Declaration fields are projection data, never Post or Post Revision
  persistence. AtomPub serializers and source-oriented DTO behavior stay
  byte-for-byte unchanged.
- The declaration remains a child of the existing `post-footer` Style Contract
  surface; no new stable hook or contract version is introduced.

## Risk checks

- A Content License or Display Name commit cannot succeed while its affected
  feed-event enqueue fails; no SQL transaction spans rendering or HTTP.
- Fanout considers all currently public Posts and every distinct current Tag,
  not one page, the hybrid feed window, drafts, deleted Posts, or private
  audiences. Scheduled Posts pick up current rights when their ordinary go-live
  event regenerates feeds.
- Feed serializers use library extension APIs and assert output; they do not
  patch serialized XML strings. RSS namespace declarations and Atom license
  relation placement are tested literally.
- The exact declaration is derived from immutable `created_at`, never
  `published_at`, browser time, or Post Revision state.
- Missing config rows remain valid across existing databases and backups; an
  invalid explicit stored/restored value fails closed rather than becoming All
  Rights Reserved.
- Public projector bytes remain viewer-independent, themeable, and cache-safe;
  declaration markup exposes no new Style Contract hook or
  wrapper/sibling-position guarantee.
- Existing Local snapshots are reviewed and updated; permalink and compact
  comparisons remain transient visual-proof artifacts, not new snapshot
  baselines.
- No lint suppression, coverage exemption, protocol expansion, or unrelated
  cleanup enters the branch without explicit approval.
