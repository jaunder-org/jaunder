# Operator-configurable Local identity implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for an individual
> task when useful. This outline exists because the approved behavior changes
> public Syndication Feed metadata and publisher-cache invalidation contracts.

## Scope

In:

- Typed optional Site Tagline configuration and one atomic site-identity update.
- Local projector/CSR identity and metadata coincidence.
- Site and site-tag Syndication Feed descriptions in RSS, Atom, and JSON.
- Operator Site Settings controls and focused browser proof.

Out:

- Schema migrations; `site_config` remains the closed key/value registry.
- Home, User/User-tag feed descriptions, per-User or per-Post taglines, and
  cross-tab live updates.
- New Style Contract concepts or a contract-version change.

## Task outline

- [x] Task 1: Establish the typed identity and publisher persistence contract.
  - Contract: add optional `SiteTagline` to `SiteIdentity` and `site.tagline` to
    `SiteConfigKey`; parsing follows the approved scalar-count and
    line-separator rules, while invalid stored data reads as absent without
    repair.
  - Contract: one identity mutation writes title, optional tagline, and guarded
    base URL in one `WriteScope`, and atomically advances publisher generation
    and invalidates cached feeds so an older generation cannot commit afterward.
  - Verification: focused common value/serde tests plus dual-backend
    `SiteConfigStorage` and `PublisherStorage` tests for round-trip, malformed
    storage, rollback, snapshot coherence, and cache fencing. CLI command-path
    tests cover set/unset for title, tagline, and base URL, proving validation,
    the base-URL passkey guard, generation advance, cache deletion, and stale
    fencing. Run:
    - `devtool run -- cargo xtask test-local -- -p common site::tests`
    - `devtool run -- cargo xtask test-local -- -p storage site_config`
    - `devtool run -- cargo xtask test-local -- -p storage publisher`
    - `devtool run -- cargo xtask test-local -- -p jaunder commands::site_config`

- [x] Task 2: Carry one resolved identity through projected and reactive Local.
  - Contract: the Local public presentation/seed carries the resolved
    `SiteIdentity`; projected body, serialized seed, reactive adoption, and
    document/Open Graph metadata all consume that same value rather than making
    independent reads.
  - Contract: the configured title owns the prominent Local heading and the
    existing `site-title` semantic hook; the optional tagline remains escaped
    text within the existing `masthead` concept. Do not add a version-1 Style
    Contract hook without a separate architectural decision.
  - Verification: focused host web/projector tests cover default/configured
    identity, absent/escaped tagline markup, seed round-trip, head metadata,
    existing hook cardinality, and projector/reactive coincidence. Run:
    - `devtool run -- cargo xtask test-local -- -p web local`
    - `devtool run -- cargo xtask test-local -- -p web app::render`
    - `devtool run -- cargo xtask test-local -- -p jaunder projector`

- [x] Task 3: Route the Site Tagline into applicable Syndication Feeds.
  - Contract: `PublisherSnapshot` supplies one coherent identity; feed metadata
    maps the tagline to `description` only for `FeedSurface::Site` and
    `FeedSurface::SiteTag`. Existing serializers remain the native RSS
    `description`, Atom `subtitle`, and JSON Feed `description` adapters.
  - Verification: focused regeneration/renderer tests cover all four feed
    surfaces, all three formats, omission when absent, escaped protocol output,
    semantic fingerprint/ETag change, and regeneration after identity-cache
    invalidation. Run:
    - `devtool run -- cargo xtask test-local -- -p host feed::`
    - `devtool run -- cargo xtask test-local -- -p jaunder -E 'test(/^feed::/)'`

- [x] Task 4: Expose and prove the aggregate operator workflow.
  - Contract: the existing Site Settings card loads, validates, sets, changes,
    and clears the tagline alongside title and base URL through one action;
    rollback-confirmed and commit-indeterminate feedback retain their existing
    distinct meanings and base-URL warning revalidation remains intact.
  - Verification: extend `end2end/tests/admin-site.spec.ts` for persisted
    set/change/clear behavior and navigation to the resulting Local title and
    tagline; retain Local mount/layout-shift and theme regressions. Run
    `devtool run -- cargo xtask e2e-local admin-site.spec.ts`.
  - Documentation: keep `CONTEXT.md`, the architecture view, Site Settings flow
    documentation, and public-theme guidance aligned with the delivered
    behavior; no ADR is required unless implementation needs a new Style
    Contract concept or other unapproved architecture choice.

## Risk checks

- The passkey RP-host guard and all three identity writes share the same
  transaction; validation completes before that transaction begins.
- Publisher generation advance and feed-cache deletion are in the identity
  transaction, including CLI `site-config set/unset` paths for identity keys.
- Projector and CSR never fetch identity independently for the same Local paint;
  the projected bytes and seed describe one snapshot.
- Public HTML may remain cache-fresh for five minutes; do not add bespoke
  cross-tab or cache-bypass behavior.
- Tagline text reaches Maud/Leptos and feed libraries as text, never through a
  raw HTML door, and telemetry carries neither title nor tagline values.
- User and User-tag feeds remain description-free; Home and non-Local mastheads
  retain their current behavior.
- Existing `masthead` and `site-title` Style Contract meaning, route presence,
  cardinality, and accessible source order remain valid.
- No lint suppression lands without explicit user approval.
