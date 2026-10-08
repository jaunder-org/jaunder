# Issue #1695: Unified Theme Packages and styling assets

## Outcome

Built-in themes, including the default Studio theme, are bundled Theme Packages
rather than a parallel stylesheet mechanism. Application CSS and published theme
assets share one content-addressed serving, cache, and retention contract, so a
refreshed document after deployment cannot reuse a previous release's styling
under the same URL.

This addresses the confirmed stale-asset reproduction in
[#1695](https://github.com/jaunder-org/jaunder/issues/1695), including its
reporter comment confirming that cache clearing resolved the iPhone symptom. It
does not change the Post image-sizing policy established by #1630.

## Load-bearing decisions

### Packages and presentation

- Studio, Terminal, and Reader ship as system-managed Theme Packages. Their
  presentation uses the same package validation, compilation, revision, and
  resolution pipeline as custom themes, not a built-in-only CSS shortcut.
- Existing built-in selection names remain valid. A selection follows the
  installed release's bundled revision, without operator intervention or a
  conversion into an author/operator-owned custom theme.
- Bundled packages are selectable but not editable or deletable through custom
  catalogs. They do not consume custom-theme quotas. Editing their source means
  producing a separate custom package, not modifying system-owned content; a new
  UI for copying bundled packages is not required by this issue.
- Preserve site/author precedence, Studio fallback, deterministic public
  presentation, and custom-theme ownership, publication, and permission rules.
- Application CSS includes structural and protected-control styling. It remains
  system-authored and non-selectable, but uses the same asset installation,
  digest addressing, serving, and retention mechanism as package content.
- Shared asset delivery does not grant custom packages global styling powers.
  Their Style Contract scope, containment, resource validation, and separation
  from trusted controls remain enforced. Private application surfaces keep
  Jaunder-owned styling rather than loading author-selected public CSS.

### Installation and retention

- The binary contains everything needed to install its bundled packages and
  application CSS; deployment requires no external theme provisioning.
- Before accepting requests, startup installs and verifies the current bundled
  packages and application assets in the shared persistent content store.
  Missing/corrupt artifacts, failed installation, or failed eligibility
  reconciliation prevent startup; there is no embedded stylesheet fallback.
- Repeated startup is idempotent. Current system content remains referenced;
  replacing a release atomically advances system references without changing
  existing bytes at any digest address.
- Superseded system content follows the existing shared retention rule: collect
  only when unreferenced and after the one-year asset lifetime plus five-minute
  public-document freshness window. Upgrades and rollbacks preserve eligibility
  for already-issued digest references.
- System ownership is distinct from custom catalog ownership. System content
  cannot exhaust author/operator publication quotas; its inventory and reference
  lifetime are determined by shipped artifacts and the shared retention policy.
- Backup/restore preserves retained content and serving eligibility through the
  existing backup contract. Starting a restored instance installs its current
  release without discarding retained earlier content.
- SQLite and PostgreSQL provide equivalent installation, reference advancement,
  eligibility, retention, and recovery behavior.

### HTTP and document consumers

- Every emitted built-in/application stylesheet reference names the digest of
  the served bytes. All published styling assets use the same HTTP behavior,
  including `public, max-age=31536000, immutable`, MIME, validators, and
  conditional requests. An address never serves replacement bytes.
- Static CSR shells, anonymous projected documents, authenticated entry paths,
  public theme transitions, and theme-thumbnail rendering consume the same
  generated artifact identities and package presentation contract.
- Thumbnail/check/package commands remain database-independent. Their existing
  loopback-only transport may provide command-lifetime content, but must use the
  shared artifact and HTTP contract rather than an embedded `/style` service.
- Remove `/style/jaunder.css` and `/style/jaunder-themes.css`: they return 404,
  never compatibility aliases or the SPA shell. New documents never emit them.
- Already-issued legacy documents may be temporarily unstyled until refreshed.
  Removing a route does not evict a browser's existing cached response. Existing
  public HTML's five-minute freshness remains; no instant cache purge is
  claimed.
- Existing content-addressed CSR and Media behavior remains unchanged. Audit
  other mutable non-manifest assets and record their disposition rather than
  silently granting immutable caching to a stable filename.

## Acceptance

1. Each bundled theme is a valid package and resolves to an immutable revision
   and digest stylesheet through the same presentation path as a custom theme.
   Existing selection tokens, defaults, and inheritance survive upgrade.
2. A real browser caches release A's application and theme styling, then the
   same origin switches to release B. A normal fresh document navigation loads
   B's styling without clearing caches. Prove application CSS changes on both
   Local and Home entry paths. Independently prove selected bundled public-theme
   CSS changes on Local and an author permalink; Home must not load the selected
   public Theme Package as a consequence of this test or implementation.
3. Unchanged content retains its address; changed bytes get a new address. Old
   digest requests retain original bytes across upgrade/rollback, with correct
   200/304 headers and empty conditional bodies. Legacy stylesheet requests
   return 404 and are absent from all generated documents.
4. Both backends prove idempotent install, failure-before-serving, system/custom
   ownership and quota isolation, atomic release-reference changes, and
   reference/deadline-controlled collection. Custom theme safety tests remain.
5. Isolated backup/restore and upgrade/rollback proofs retain old styling assets
   and start with the installed release's usable default and application CSS.
6. Thumbnail rendering remains database-independent and uses the same styling
   artifacts. CSR, Media, and published custom-theme immutable contracts remain.
7. Comparable before/after screenshots show no intended visual change: Local, an
   author permalink, and Home at narrow and wide viewports, including all
   bundled themes on public surfaces and one custom Theme Package. Protected
   controls and Post image fit remain usable and unchanged.
8. Record actual Safari deployment-transition evidence when available; otherwise
   explicitly retain the Safari/iPhone verification gap. Playwright WebKit,
   Chromium, or Firefox results are not labeled actual Safari evidence.
9. Record a bounded inventory of mutable non-manifest assets and document each
   asset's identity/cache disposition, including the public favicon and any
   additional stylesheet consumers. No stable filename silently receives
   immutable caching; any remaining mutable resource has an explicit policy.

## Boundaries

No image-sizing workaround, imported #1677 stash, visual redesign, new theme
editing/copy UI, custom-theme security relaxation, or CSR/Media storage
migration. No production access, live upgrade, or restore is authorized by this
spec. Milestone #22's final production qualification remains owned by claimed
#1419 and must exercise the final release candidate after this change lands.

The architectural decision is recorded in
[the unified styling assets draft](../../adr/drafts/unified-theme-and-application-assets.md).
Architecture/storage/public-URL changes require an approved short implementation
outline after this spec is approved.
