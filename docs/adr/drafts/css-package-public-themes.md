# ADR-DRAFT: Custom public themes are scoped CSS packages

- Status: proposed
- Date: 2026-09-06
- Issue: [#1341](https://github.com/jaunder-org/jaunder/issues/1341)

## Context

[#21](https://github.com/jaunder-org/jaunder/issues/21) established one
server-resolved public Theme for each route: an operator-owned site default and
an optional author-owned override.
[ADR-0041](../0041-public-projector-and-csr-client.md) requires the anonymous
projector to produce byte-identical, CDN-cacheable HTML for a route and
presentation state. The existing built-in themes vary CSS variables over a
shared document, but issue #1341 must support a much broader WordPress-like
visual range, local fonts and images, portable import/export, and safe
owner-authored styling.

A CSS capability prototype demonstrated that one accessible semantic document
can support the required layout range. A typed visual-token language would
constrain that range without removing the need to validate CSS. Custom
templates, JavaScript, and server hooks would instead create executable
rendering systems, broaden the trust boundary, and make the shared projector/CSR
document contract conditional.

Raw CSS is not safe by default. Selectors can escape their intended surface and
URL-bearing values can create unreviewed external requests. Mutable asset URLs
would also undermine long-lived caching. WordPress-style request-random header
selection conflicts with ADR-0041 because separate renders of the same route
could produce different bytes.

## Decision

Jaunder custom public themes are versioned Theme Packages: a ZIP archive with
`theme.json`, one `style.css` entry point, and an allowlisted set of
package-local WOFF2 or raster-image assets. They target a versioned public Style
Contract made of accessible semantic HTML and stable concept hooks. Custom CSS
is scoped inside an unthemeable paint-containment/low-stacking boundary;
authenticated owner mutation controls render in a sibling trusted stacking
context above it. Theme Packages contain no templates, JavaScript, server hooks,
or external resources. Built-in themes use the same Style Contract.

The package has a closed member and manifest grammar. Import streams actual
bytes under compressed, expanded, file-count, per-file, AST, nesting, image
decode, and font expansion limits. It rejects non-UTF-8 or non-forward-slash
names, absolute/drive/UNC or empty/dot/traversal paths, symlinks and other
non-regular or encrypted entries, duplicate normalized names, ZIP-header
disagreement, unknown members or manifest fields, and declared/detected serving
type disagreement. SVG, HTML, JavaScript, source maps, executable content, and
unknown types are rejected.

Jaunder parses and transforms authored CSS before preview or publication. The
initial at-rule set is closed to `@font-face`, `@keyframes`, `@media`,
`@supports`, and `@container`; unsupported or unscopable constructs, nesting,
imports, external or opaque URLs, and undeclared assets fail closed. The
transform scopes selectors beneath the Style Contract root, namespaces global
font and keyframe identifiers, parser-rewrites their references, and rewrites
package URLs to same-origin content-addressed URLs. An unresolved reference is
rejected. Only the deterministic transformed stylesheet is served publicly.

This boundary prevents executable content, external fetches, cross-surface
effects, namespace collisions, and unbounded server/archive or decoded-asset
work. Authenticated owners remain trusted for visual usability and residual
browser rendering cost on the public pages they control; ordinary CSS is not
misrepresented as a closed inexpensive property language.

A stored custom theme belongs either to the site operator or to one author. It
has a mutable private draft and immutable published revisions. Every
draft-derived read, including export, requires catalog ownership, uses
`private, no-store`, denies anonymous requests, and never mutates the public
selection. Publish is an atomic validation boundary. Raw full SHA-256 content
hashes identify each exact asset and transformed stylesheet; separate versioned,
domain-separated, length-prefixed canonical encodings identify the source
package and published revision. Asset URLs resolve before the stylesheet and
revision digests, avoiding a circular identity. Publish validates every
package-backed image binding against the candidate revision and fails atomically
if a path is absent. Publishing then advances the stable Theme ID to the new
revision, so a selected theme updates without a second selection mutation while
prior revision bytes remain unchanged.

Existing #21 site/author precedence selects either a built-in Theme or an
owner-valid custom Theme ID; private Studio surfaces never load custom CSS. A
theme with no published revision cannot be selected. Removing a selected site
theme atomically resets the site selection to Studio; removing a selected author
theme atomically deletes the author override and restores site inheritance.
Missing or corrupt selections follow those fallbacks, while database read
failures remain errors. Removal detaches the catalog and Media bindings.
Published content remains readable while referenced and through the one-year
asset lifetime plus five-minute public-HTML freshness window after detachment;
garbage collection requires both no reference and the elapsed deadline.

Atomic admission enforces per-owner theme/revision/logical-byte quotas,
site-wide revision/deduplicated-byte quotas, per-principal operation rate, and
one in-flight package operation per owner. Detached content remains
quota-charged until collection, preventing cache-safe retention from becoming
unbounded disk allocation.

Style Contract version 1 admits decorative `logo` and `header` presentation
roles. A role can use a package default, a package image, or exact Media owned
by the theme owner. `header` may instead use an explicit pool. Pool selection
uses a versioned, domain-separated, length-prefixed SHA-256 contract over the
typed canonical route, published revision, canonical duplicate-free pool, and
persisted shuffle seed. An explicit shuffle mutation replaces only that seed.
Selection can therefore vary across routes but is stable for identical route and
theme state, and the selected URL remains part of the body-derived public ETag.

Theme Media bindings are persisted presentation references, distinct from the
Post Media references derived from sanitized `RenderedHtml` by
[ADR-0090](../0090-media-references-extracted-at-render.md). Binding and guarded
deletion share the existing exact Media-key lock: ownership verification and
binding insertion occur in one storage mutation, and deletion conditionally
checks theme references under the same lock. Owner-local Media bindings are not
exported in a Theme Package.

## Consequences

Theme authors get ordinary CSS expressiveness, portable local assets, safe draft
preview, and atomic publication without creating a second template or execution
engine. A plain stylesheet can be imported as a zero-asset package. The public
projector and CSR client retain one shared semantic document and one
deterministic presentation input.

The package schema, Style Contract, CSS transform, digest encodings, image-pool
assignment, and asset routes become versioned compatibility surfaces. Their
parsers need adversarial tests, resource limits, path normalization,
backend-parity storage, and backup/restore coverage. Published revisions and
asset URLs are immutable; edits create new revisions rather than changing cached
bytes. Cache-safe retention consumes storage and quota after catalog removal;
content-address deduplication limits duplication, and eligible content is
reclaimed only after its advertised lifetime.

CSS can rearrange existing semantic content but cannot add data, widget
instances, or behavior. Package-local SVG and arbitrary content types remain
excluded from the initial asset allowlist. A future expansion of executable
content, external resources, Style Contract concepts, or request-varying
presentation requires another decision rather than weakening this boundary.
