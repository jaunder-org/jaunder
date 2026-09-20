# Issue #1590 Inline-rendered Post titles — Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for bounded tasks.
> This outline exists because the approved spec changes both database schemas,
> trusted-markup decode invariants, full Post Revisions, backup/restore
> behavior, and three public Syndication Feed representations.

## Scope

In:

- One host-owned inline title renderer and one validated common Rendered Title
  type implementing the approved closed HTML and visible-text contracts.
- Atomic persistence for current Posts and full Post Revisions on SQLite and
  PostgreSQL.
- Schema migration and backup/restore validation.
- Shared web Post article headings and Atom/RSS/JSON Syndication Feed
  projections, including feed identity and durable cache invalidation.
- Focused, backend-parity, e2e, and existing visual-baseline proof.

Out:

- Title authoring/extraction, slugs, document metadata, AtomPub, editor and
  history surfaces, independently clickable title links, embedded title media,
  and automatic rewrites after future parser upgrades.

## Task outline

- [x] Task 1: Establish the Rendered Title domain and render aggregate
  - Contract: `common` owns a trusted `RenderedPostTitle` value with no general
    raw-string constructor. Host ammonia validates persisted SQL bytes and
    constructs title values; server-authored DTO bytes are trusted by CSR exactly
    as rendered body HTML. `host::render` renders source formats and invokes
    common's host-only ammonia title sanitation and plain-text projection
    boundaries.
  - Contract: `PostRenderOutput` privately owns authored title, body, format,
    Rendered Title, rendered body, and media references. Storage create/update
    inputs consume that aggregate and receive only read accessors for binding
    and comparison, making source/derivative mismatches unrepresentable.
  - Verification: exhaustive table-driven tests pin retained tags,
    flattening/removal rules, entities, whitespace, malformed source, empty
    output, and all three formats. Native tests prove SQL rejection through
    ammonia, CSR trusts server DTO bytes without acquiring sanitizer dependencies,
    and no public trusted-markup or aggregate mismatch doors exist.

- [x] Task 2: Install and enforce persisted title state as one slice
  - Depends on: Task 1's value and aggregate contracts.
  - Contract: nullable Rendered Title columns are added to `posts` and
    `post_revisions` in both dialects. No production instances exist, so the
    migration does not repair legacy rows. New titled writes persist a derivative;
    titleless rows stay null and content-free authored titles store a present
    empty fragment.
  - Contract: in the same slice, strict `PostRecord` and full-revision decoding,
    inserts, updates, canonical no-op comparison, prior-state capture, seeders,
    and backend fakes consume `PostRenderOutput`. Backup schema/domain coverage
    includes both columns. Presence violations retain structural failure;
    invalid payloads follow ADR-0174 restore-and-report but fail typed reads
    through the host ammonia policy before an unescaped sink. Lightweight history metadata remains
    unchanged.
  - Verification: `#[apply(backends)]` tests cover create/update, format and
    title transitions, semantic no-op, complete prior-state snapshots,
    titleless/empty distinctions, nullable current and revision migration,
    exact backup round trips, invalid presence, and every invalid fragment class
    on SQLite and PostgreSQL.

- [x] Task 3: Paint persisted Rendered Titles on shared web Post surfaces
  - Depends on: Tasks 1–2.
  - Contract: content-weight DTOs carry rendered bytes for timeline/permalink
    painting while authored title remains only where source and metadata
    consumers already need it. The public projector and CSR use one pure heading
    renderer and never parse or sanitize authoring source. Empty fragments omit
    the article heading; links stay flattened inside the existing permalink.
  - Verification: component and wire tests prove exact markup, malicious-byte
    rejection, empty omission, titleless behavior, and projector/CSR
    coincidence. Focused regressions prove slug derivation and browser document
    titles still use authored title syntax unchanged.

- [x] Task 4: Project titles correctly into every Syndication Feed format
  - Depends on: Tasks 1–2.
  - Contract: feed assembly carries the persisted fragment and its visible-text
    projection. Atom emits `type="html"`; RSS and JSON Feed emit marker-free
    plain text and omit empty titles. AtomPub remains authored source. Every
    affected serializer input joins the semantic fingerprint, serializer
    revisions advance, and existing durable feed caches are invalidated once.
  - Verification: exact serializer tests cover formatted and empty titles for
    Atom/RSS/JSON; fingerprint tests prove every byte-affecting title change
    changes identity; cache tests prove pre-feature representations cannot be
    served as current. Existing-surface regression tests prove AtomPub
    create/get/update round-trips authored Markdown, Org, and HTML title syntax.

- [x] Task 5: Prove integrated browser behavior and close documentation
  - Depends on: Tasks 1–4.
  - Contract: public timeline and permalink fixtures exercise Markdown, Org, and
    HTML title markup without widening visual-snapshot policy. The accepted ADR
    draft, `CONTEXT.md`, and `docs/ARCHITECTURE.md` remain synchronized with
    delivered behavior.
  - Verification: always run the targeted assertions with
    `devtool run -- cargo xtask e2e-local <spec-or-file:line>`. If pixels
    intentionally change, update the existing public-timeline Chromium/Firefox
    baselines only through
    `devtool run -- cargo xtask e2e-local --update-visual-snapshots`, review the
    changed PNGs, then run unfiltered `devtool run -- cargo xtask e2e-local` as
    the required post-update verification. Run
    `devtool run -- cargo xtask check` as the broad integration diagnostic.

## Risk checks

- No raw title or restored invalid fragment can enter an unescaped HTML sink.
- No caller can independently construct or mismatch authored and rendered title
  state.
- Host-only ammonia validates persisted Rendered Title bytes; CSR trusts
  server-authored bytes through the narrow common-owned DTO reconstruction hook,
  and host/sqlx dependencies remain outside the wasm closure.
- Migration and persistence land together: no executable intermediate state can
  read, write, or serve a stale/null derivative for a titled Post.
- New-write persistence keeps authored and rendered title state atomic for
  immutable Revisions as well as current Posts on both backends.
- Empty canonical output is distinct from a missing derivative at SQL, backup,
  and DTO boundaries.
- Parser upgrades do not silently rewrite persisted title or revision bytes.
- Timeline DTO growth preserves ADR-0097's content-weight split; source title is
  not duplicated onto every row without an existing consumer.
- Whole-title permalink anchors never acquire nested anchors or active media.
- Atom uses its HTML text contract; RSS/JSON never receive secondary source
  markup; AtomPub remains authored source.
- Feed fingerprints, serializer revisions, and durable caches change together.
- Storage schema, SQL projections, revision capture, backup fixtures, raw test
  rows, and both dialect implementations remain column-for-column consistent.
- Any lint suppression requires explicit user approval; commits contain no
  `Co-Authored-By` trailer.
