# Custom Public Themes

Issue: #1341

## Outcome

Operators and authors can create, edit, preview, publish, import, export,
select, and remove custom public themes alongside Jaunder's built-in Terminal,
Studio, and Reader themes. A custom theme is ordinary CSS plus packaged local
assets; it styles one versioned semantic public document rather than replacing
its markup or executing code.

This work extends #21's ownership and precedence. The operator owns a site theme
catalog and selects the site default. Each author owns a separate catalog and
may select one of their themes as the override for their own public pages or
inherit the site default. A Theme Package is portable between those catalogs,
but a stored theme and its Media bindings have exactly one owner.

The CSS capability prototype and its Firefox results are preserved on
`prototype/issue-1341-css-capability`; they establish that the same semantic
HTML supports the required visual range without a typed token language or custom
templates.

## Public Style Contract

The themeable boundary is
`[data-jaunder-theme-surface][data-jaunder-style-contract="1"]`, exactly once
inside `.j-root` on every cacheable public document. An unthemeable parent
`[data-jaunder-theme-clip]` establishes paint containment and a lower stacking
context; transformed selectors cannot select that parent. Authenticated owner
mutation controls render in a sibling trusted-chrome stacking context above the
clip. The themed document may contain inert anchors for those controls, but its
positioned, fixed, filtered, or overflowing descendants remain clipped below
them and cannot style, hide, move, or overlay the controls. Studio, editors, and
administration pages expose neither boundary and never load custom theme CSS.

Style Contract version 1 guarantees these exact hooks:

| Hook                                     | Element and cardinality                                                                              |
| ---------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `data-jaunder-part="primary-navigation"` | One `nav` per public document.                                                                       |
| `data-jaunder-part="masthead"`           | One `header` per public document.                                                                    |
| `data-jaunder-part="site-title"`         | One textual site-identity element in the masthead.                                                   |
| `data-jaunder-part="logo"`               | Zero or one decorative `img` in the masthead.                                                        |
| `data-jaunder-part="header-image"`       | Zero or one decorative `img` after the masthead and before main content.                             |
| `data-jaunder-part="main"`               | One `main` per public document.                                                                      |
| `data-jaunder-part="post-list"`          | One on timeline and tag routes; absent on a permalink.                                               |
| `data-jaunder-part="post"`               | One `article` for each rendered Post.                                                                |
| `data-jaunder-part="post-header"`        | One `header` in each Post.                                                                           |
| `data-jaunder-part="avatar"`             | Zero or one `img` in each Post header.                                                               |
| `data-jaunder-part="author-name"`        | One in each Post header.                                                                             |
| `data-jaunder-part="author-handle"`      | Zero or one in each Post header.                                                                     |
| `data-jaunder-part="published-time"`     | One `time` in each published Post.                                                                   |
| `data-jaunder-part="post-title"`         | Zero or one heading in each Post.                                                                    |
| `data-jaunder-part="post-summary"`       | Zero or one summary in each Post.                                                                    |
| `data-jaunder-part="post-body"`          | One body container in each Post; semantic descendants of `RenderedHtml` remain styleable by element. |
| `data-jaunder-part="post-footer"`        | One `footer` in each Post.                                                                           |
| `data-jaunder-part="tag-list"`           | Zero or one list in each Post footer.                                                                |
| `data-jaunder-part="tag"`                | One link for each rendered tag.                                                                      |
| `data-jaunder-part="source-attribution"` | Zero or one in each Post footer.                                                                     |
| `data-jaunder-part="continuation"`       | Zero or one on a paginated list route.                                                               |

The contract guarantees those meanings, landmark elements, cardinalities, route
presence, and accessible source order. It does not guarantee incidental wrapper
depth, sibling positions beyond the stated landmark order, or positional
selectors such as `:nth-child`. Built-in themes use the same hooks.

CSS owns layout. Public rendering emits no inline layout declarations.
Renderer-owned dynamic presentation values, such as an avatar hue, may use
explicitly documented typed custom properties; authored CSS or package data
never enters a `style` attribute. The textual site identity remains present when
a logo is configured, and both role images use empty alternative text rather
than replacing content.

## Theme Package

The portable representation is a ZIP archive with exactly `theme.json`,
`style.css`, and zero or more regular files below `assets/`. Member names are
UTF-8 forward-slash paths. Import rejects NULs, backslashes, empty or dot
components, absolute/drive/UNC paths, traversal, symlinks and other non-regular
entries, encryption, duplicate canonical names, local/central header
disagreement, and every unexpected member. It streams every entry without
extracting to an archive-chosen path, counts actual bytes read rather than
trusting headers, and enforces centralized compressed, expanded, file-count,
per-file, CSS-AST, and nesting limits before any persistent write.

`theme.json` version 1 has this closed shape; unknown or missing fields fail:

```json
{
  \"schema\": 1,
  \"name\": \"Paper\",
  \"style_contract\": 1,
  \"assets\": {
    \"assets/body.woff2\": \"font/woff2\",
    \"assets/logo.webp\": \"image/webp\",
    \"assets/header-one.avif\": \"image/avif\"
  },
  \"defaults\": {
    \"logo\": \"assets/logo.webp\",
    \"header\": [\"assets/header-one.avif\"]
  }
}
```

`assets` is required and may be empty. Its keys are every package asset exactly
once; its values are one of `font/woff2`, `image/png`, `image/jpeg`,
`image/webp`, or `image/avif`. `defaults` is required and may be empty. `logo`
is one declared image path; `header` is a non-empty duplicate-free array of
declared image paths. Either key may be absent. JSON strings are valid UTF-8,
and the manifest's canonical form is RFC 8785 JSON Canonicalization Scheme. The
fixed `style.css` member is the sole authored stylesheet entry point.

Declared type, detected bytes, and the stored serving type must agree. Raster
dimensions, total decoded pixels and animation-frame counts, and WOFF2
decompressed table sizes have independent centralized limits. SVG, HTML,
JavaScript, source maps, executable content, and unknown types are rejected.
Browser interpretation is bound by the stored detected type and `nosniff`, not
by an open-ended claim that valid media cannot be parsed as any other format.

A plain `.css` upload is the zero-asset shorthand: Jaunder creates a new draft
with a generated version-1 manifest and the uploaded file as `style.css`.
Package import also creates a draft; neither operation publishes or selects the
result. Export writes the current validated editable state in the same package
format, without owner-local Media bindings.

Within one owner catalog, custom display names are case-insensitively unique and
cannot impersonate a built-in theme label. Stored identity is an opaque stable
Theme ID, so renaming does not break selection.

## CSS validation and isolation

Jaunder treats CSS as untrusted structured input. Import, draft save, preview,
and publish use one bounded parser-backed validation and transformation
boundary:

- syntax errors, native nesting, and every at-rule except top-level `@font-face`
  and `@keyframes` plus nestable `@media`, `@supports`, and `@container`
  conditionals are rejected rather than repaired;
- `@import`, external URLs, protocol-relative URLs, `data:`/`blob:` URLs, and
  URL-bearing values that cannot be completely parsed are rejected in every
  declaration and custom-property token stream;
- every accepted URL resolves to a declared package asset, and publication
  rewrites it to that asset's immutable same-origin content URL;
- every style selector is parsed and scoped below
  `[data-jaunder-theme-surface][data-jaunder-style-contract="1"]`; `html`,
  `body`, and `:root` map to that boundary, while an unscopable selector is
  rejected;
- conditionals retain the same selector, declaration, nesting, and URL checks;
  `@font-face` and `@keyframes` are top-level only;
- authored font-family and keyframe identifiers receive a prefix derived from
  the canonical source-package digest, and every `font`/`font-family` and
  `animation`/`animation-name` reference is parser-rewritten. An ambiguous,
  undeclared, or custom-property-hidden reference is rejected;
- the transformed stylesheet is deterministically serialized by Jaunder. The
  authored bytes remain private editable package input and are never served to
  visitors.

The boundary prevents executable content, external fetches, cross-surface CSS,
namespace collisions, unbounded server parsing, and gross image/font decode
expansion; substring filtering is not acceptable. An authenticated theme owner
is nevertheless trusted for the visual usability and residual browser render
cost of otherwise valid CSS on the public pages they control. This feature does
not claim that arbitrary CSS can be made inexpensive without reducing it to a
closed property language.

## Draft, preview, and publication

A new or imported custom theme begins as a mutable private draft. Its owner can
edit CSS, add or remove package assets, change package image defaults, and
preview it against the real Style Contract. Author previews use the author's
public page; site previews use the site timeline. Every draft-derived read
enforces catalog ownership server-side—including metadata/editor state,
transformed preview CSS, assets, preview HTML/data, and ZIP export—and returns
`Cache-Control: private, no-store`. Draft IDs are not bearer capabilities.
Export uses attachment disposition and a server-generated safe filename. No
draft operation changes the selected public theme.

Publish validates the complete draft and atomically materializes immutable
content. Three non-circular digests are defined:

1. The source-package digest hashes fixed ASCII domain
   `jaunder-theme-source-v1`, the unsigned 64-bit big-endian asset count,
   length-prefixed RFC 8785 manifest bytes, length-prefixed authored-CSS bytes,
   then each asset's length-prefixed normalized path, stored detected type, and
   exact bytes in lexicographic path order. It namespaces authored font and
   keyframe identifiers.
2. Each asset URL contains the raw full SHA-256 of its exact bytes. The
   transformed CSS contains those asset URLs, and its URL contains the raw full
   SHA-256 of the deterministic transformed CSS bytes.
3. The published-revision digest hashes fixed ASCII domain
   `jaunder-theme-revision-v1`, the unsigned 64-bit big-endian asset count,
   length-prefixed canonical manifest bytes, the raw 32-byte transformed-CSS
   digest, then each length-prefixed asset path and type plus its raw 32-byte
   content digest in lexicographic path order.

Every variable-length field, including each path, type, manifest, stylesheet,
and asset body, has its own unsigned 64-bit big-endian byte length. Digests are
fixed raw 32-byte fields. These encodings, not ZIP entry order or metadata, are
the identity contract. CSS is served as `text/css; charset=utf-8`; assets use
only their stored detected type; all responses send
`X-Content-Type-Options: nosniff`, full-digest ETags, and
`Cache-Control: public, max-age=31536000, immutable`.

A theme with no published revision cannot be selected. Before publish commits,
every package-backed fixed or pooled image binding must resolve in the candidate
revision; a missing path rejects the publish and leaves the prior revision and
presentation unchanged. A successful publish advances the stable Theme ID to the
new immutable revision, so a selected theme updates without a separate selection
mutation. Prior revision bytes and issued content URLs are never changed in
place.

## Presentation Media

Package assets travel with the Theme Package. Owner Media does not: it remains
instance data. A binding mutation acquires the existing lock for the exact Media
key and, within the same storage mutation, verifies the persisted
`(user_id, source, sha256, filename)` row belongs to the theme owner before
inserting the binding and updating pool state. The guarded delete/reclaim
decision uses that same lock, includes theme bindings in its conditional
reference check and owner report, and therefore cannot race a new binding. No
client-supplied owner identity or preflight lookup is authoritative.

Each theme has owner-local bindings for the two Style Contract image roles:

- `logo`: packaged default, one packaged image, one owned Media item, or absent;
- `header`: packaged default, one packaged image, one owned Media item, or an
  explicit non-empty pool containing packaged images and/or owned Media.

The header pool never implicitly includes the owner's full Media collection. Its
version-1 assignment contract is:

1. Encode a package entry as byte tag `0x00` plus its length-prefixed normalized
   path. Encode a Media entry as tag `0x01` plus length-prefixed source
   discriminant, raw SHA-256 bytes, and UTF-8 filename.
2. Reject duplicate encoded entries and sort the remaining entry encodings
   lexicographically. The pool revision is SHA-256 of `jaunder-theme-pool-v1`,
   the unsigned 64-bit big-endian entry count, and each length-prefixed entry.
3. Obtain route identity only from the typed public route formatter: its
   canonical UTF-8 path, with no scheme, authority, fragment, or non-rendering
   query input. A future rendering query parameter requires a new assignment
   contract.
4. Hash `jaunder-theme-assignment-v1` followed by length-prefixed route
   identity, raw published-revision digest, raw pool-revision digest, and the
   persisted 32-byte shuffle seed. Interpret the full digest as an unsigned
   big-endian integer modulo the pool length and select that canonical entry.

The same route and state therefore select the same image on every backend and
across upgrades. `Shuffle assignments` replaces only the persisted seed in one
confirmed mutation, deliberately remapping routes while remaining deterministic
thereafter.

The chosen immutable asset or Media URL appears in final public HTML. It
participates in the existing body-derived representation ETag; changing a
revision, binding, pool, or shuffle seed changes affected representation bytes.
Removing a binding or theme releases its guarded Media references.

## Persistence, authorization, and fallback

Custom themes are a dedicated typed aggregate, not opaque values stored in
`site_config` or `user_config`. Persistence records owner, stable Theme ID,
name, mutable draft, immutable published revisions, package assets, image-role
bindings, pool revision, and shuffle seed. The schema and storage behavior are
identical for SQLite and PostgreSQL and participate in backup, restore, and
schema validation.

All mutations use the existing `WriteScope`/`MutationOutcome` contract. An
operator alone manages site-owned themes and bindings. An authenticated author
alone manages their author-owned themes and bindings. A custom theme may be
selected only at its matching ownership level; portability across levels is by
export and import, not a cross-owner database reference.

The #21 effective-theme precedence is unchanged, but its closed built-in value
becomes a typed selection of either a built-in Theme or an owner-valid custom
Theme ID. Database read failures remain errors. Missing or corrupt site
selection resolves to Studio; missing or corrupt author selection inherits the
resolved site theme.

Removal is one transaction. Removing the selected site custom theme resets the
site selection to Studio. Removing the selected author custom theme deletes the
author override, restoring inheritance. The mutation detaches the catalog record
and releases its guarded Media bindings only after selection/reference updates
commit.

Issued content remains readable while any live revision references it and for at
least 31,536,300 seconds—the one-year immutable asset lifetime plus the
five-minute public-HTML freshness window—after the last revision reference is
detached. Garbage collection requires both no live reference and the elapsed
retention deadline, so removal cannot break a still-fresh public document.

Theme creation, import, draft growth, and publication are atomically admitted
against centralized per-owner limits for active themes, retained revision count,
and logical retained bytes, plus site-wide limits for retained revision count
and deduplicated physical bytes. Detached blobs continue charging their owner
until garbage collection. Package parsing and publication also have
per-principal request-rate limits and one in-flight operation per owner. A limit
rejection leaves the draft, current published revision, selection, and content
store unchanged.

## Acceptance

- Operators and authors can create, edit, preview, publish, rename, export,
  import, select, and remove only themes in their respective catalogs.
- Imported packages and plain CSS become private drafts; an invalid archive,
  manifest, stylesheet, selector, URL, asset, ownership claim, or unsupported
  Style Contract version fails closed with an actionable error and no partial
  write.
- Every draft-derived response uses server-side owner authorization and
  `private, no-store`; preview uses the actual semantic public renderer, changes
  no selection, and exposes no draft bytes to an anonymous or cross-owner
  request.
- Publishing is atomic and yields immutable, content-addressed CSS and asset
  URLs with fixed validated content types and `nosniff`. A failed publish leaves
  the previous published revision and public presentation unchanged. ZIP
  metadata/order variations with identical canonical content yield identical
  identities, while any canonical path, type, or byte change yields a new
  identity.
- Custom CSS produces materially different layouts from one Style Contract
  document without altering accessible landmark/source order, reaching trusted
  owner mutation controls, or affecting Studio pages. Global font and keyframe
  names cannot collide with Jaunder styling.
- Package defaults, fixed owned Media, and an explicit mixed header pool render
  correctly. Identical canonical routes and theme state choose identical pool
  images; different routes can choose different images; shuffling changes the
  persisted mapping and cache identity.
- Referenced Media cannot be deleted through the guarded path until its theme
  binding is removed. Cross-owner Media binding is rejected server-side.
- Removing a theme atomically detaches selection, catalog visibility, and
  guarded Media references while leaving issued CSS and asset URLs readable
  through the defined retention deadline, after no fresh public document can
  reference them.
- Site/author selection, inheritance, deletion fallback, public navigation,
  projector/CSR coincidence, ETag/304 behavior, immutable asset caching, backup
  and restore, and SQLite/PostgreSQL parity are covered at their existing test
  seams.
- Browser coverage proves the author and operator workflows, an anonymous fresh
  load, responsive custom layout in Chromium and Firefox, inaccessible draft
  assets, no flash to another theme, and no custom CSS on private surfaces.

## Boundaries

- No custom HTML templates, page builders, widget instances, JavaScript, server
  hooks, browser-local selection, per-viewer presentation, external theme
  resources, or typed visual-token language.
- Themes cannot add data or interactive navigation behavior. New Style Contract
  concepts require an explicit future contract version rather than incidental
  selector compatibility.
- True per-request random images are excluded because they contradict anonymous
  byte-identical caching. Deterministic pools plus explicit shuffle provide
  variation without that conflict.
- Syndication output and authored `RenderedHtml` sanitization are unchanged;
  theme image bindings are presentation references, not Post Media references.
