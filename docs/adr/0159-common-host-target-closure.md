# ADR-0159: Close `common` and `host` by target reachability

- Status: accepted
- Date: 2026-08-27
- Issue: [#847](https://github.com/jaunder-org/jaunder/issues/847) (subsumes
  [#855](https://github.com/jaunder-org/jaunder/issues/855))

## Context

`common` is a dual-target crate, yet it has accumulated machinery that reaches
only host consumers. That makes the crate boundary describe historical
convenience rather than the target graph. CSR does reach the narrow Syndication
Feed grammar — `FeedFormat`, `FeedSurface`, and `canonicalize` — but it reaches
neither protocol's implementation surface. `host` already exists as the
host-focused sibling under [ADR-0058](0058-host-crate-layering.md), but its
ownership rule needs a mechanical dependency-floor check and a precise exception
for the `sqlx` bridge.

The move must not blur the two protocol surfaces: an authenticated AtomPub
Collection serializes native source for editors, while a public Syndication Feed
serializes rendered HTML for readers
([ADR-0015](0015-atompub-serialization-surfaces.md)). Their crate home may
change; their contracts and bytes may not.

## Decision

#847 subsumes #855. For items currently in `common`, `common` owns a type or
operation only when a CSR or other dual-target consumer reaches it; `host` owns
every such type and machinery with no such consumer. This is a
target-reachability rule, not a claim that all server- or storage-only code
belongs in `host`, nor that semantic classification can be inferred
mechanically.

Move AtomPub wholesale to `host`, including its document, Service Document, RSD,
namespace, extension, XML, and serialization machinery. Move host-only outbound
Syndication Feed path parsing, both closed configuration registries, settings,
events, windows, representation models, and rendering to `host`; retain its
CSR-reached `FeedFormat`, `FeedSurface`, and `canonicalize` grammar in `common`.
Move `RenderOutput`, `Password`, password-hashing errors, and
`StoredPasswordHash` to `host`. Rendering, ETag construction, and Password
hash/verify become module-qualified `host` free functions rather than inherent
methods or unqualified imports.

Keep `ProfferedPassword` and its validation, `RenderedHtml`, `PostFormat`, ETag,
Org normalization, croner, `BackupSchedule`, and the feed grammar in `common`:
client and server share their one `FromStr` validation path where applicable.
Host-owned module-qualified rendering and sanitization free functions use
ammonia to establish the `RenderedHtml` invariant, then use the existing
gate-policed trusted reconstruction seam to mint the common-owned value.
`common/sqlx` is the sole ownership-forced optional host bridge: orphan-rule
trait ownership requires it despite `common`'s otherwise dual-target dependency
purity, and it does not make its bridged domain types host-only.

There are no compatibility aliases, re-exports, or deprecated paths. Every
caller migrates to the new qualified home in one cutover.

A new cargo-metadata gate enforces two graph invariants: `host` has no runtime
workspace dependency other than `common` (`macros` is its existing build-time
exception), and the exact target/feature-resolved CSR closure excludes `host`,
`storage`, `server`, and `common/sqlx`. External dependencies remain allowed
when these invariants and the existing wasm build pass. The gate deliberately
cannot decide semantic host-only classification: review owns that judgement. The
decision claims no bundle-size benefit.

## Consequences

- `common` remains dual-target because its actual consumer graph requires it,
  while `host` becomes the explicit home of host-only protocol and password
  machinery.
- Existing crate-home and call-shape statements in
  [ADR-0018](0018-constant-time-authentication.md),
  [ADR-0023](0023-atompub-jaunder-wire-extensions.md),
  [ADR-0058](0058-host-crate-layering.md),
  [ADR-0063](0063-domain-value-newtype-convention.md),
  [ADR-0065](0065-client-side-domain-validation.md),
  [ADR-0072](0072-timestamps-cross-boundary-as-utcinstant.md),
  [ADR-0073](0073-url-crate-for-absolute-url-normalization.md),
  [ADR-0079](0079-rendered-html-sanitization.md),
  [ADR-0089](0089-upstream-atom-document-io.md),
  [ADR-0090](0090-media-references-extracted-at-render.md),
  [ADR-0095](0095-doctest-gate-enumerates-the-fence-population.md),
  [ADR-0102](0102-config-key-closed-registry.md), and
  [ADR-0112](0112-role-tagged-site-urls.md) remain historical records; their
  dated notes identify the current ownership. The current view is
  [ARCHITECTURE.md](../ARCHITECTURE.md).
