# Creating and deploying themes

Jaunder custom themes are CSS packages for the public site. They style Jaunder's
versioned semantic markup; they cannot replace markup, add JavaScript, or add
server behavior.

Theme management lives at `/themes`:

- every signed-in author manages a private **Author catalog**;
- operators can switch to the separate **Site catalog**;
- author and site themes never cross catalogs implicitly. Move a theme between
  catalogs or instances by exporting and importing its Theme Package.

## Choose a starting point

The Studio page supports three starting points:

1. **Create empty draft** creates a zero-asset draft.
2. **Import CSS draft** wraps a stylesheet in a generated version-1 manifest.
3. **Import ZIP draft** imports a complete Theme Package.

All three create private, unselected drafts. Creating or importing a theme does
not change any public page.

Use plain CSS while establishing a layout. Use a Theme Package when the theme
needs fonts, images, packaged logo/header defaults, or portable source control.

## Maintain a theme repository

Keep the portable Theme Package source at the repository root:

```text
.
├── theme.json
├── style.css
├── assets/              # optional; every member is declared by theme.json
│   ├── body.woff2
│   └── logo.webp
├── preview.png          # committed canonical thumbnail
├── .github/
│   └── workflows/
│       └── theme.yml    # calls the reusable Jaunder workflow
├── src/                 # optional authoring or preprocessor source
└── README.md
```

Only root `theme.json`, `style.css`, and the optional closed `assets/` tree are
package input. Documentation, workflow files, `preview.png`, and authoring
sources are support files and are ignored. The `assets/` tree contains only
regular, non-symlink files declared by the manifest; see
[Theme Package format](#theme-package-format) for the complete package and
security rules.

`style.css` is the portable source of truth. A repository may generate it from
Sass or another tool, but owns that toolchain and must run it before Jaunder,
then separately fail its own automation if the generated result differs from the
committed `style.css`. Jaunder never executes repository build hooks and does
not standardize a preprocessor or its dependencies.

The database-independent commands operate on the repository directory and use
the same validation and transformation rules as Studio:

```sh
jaunder theme check .
jaunder theme thumbnail . --browser "$BROWSER" --output preview.png
jaunder theme package . --output theme.zip
```

`check` consumes only the canonical package members and ignores repository
support files. `thumbnail` atomically creates or replaces its explicitly named
PNG so the committed artifact can be regenerated in place. It requires the
supplied, externally installed Chromium-compatible executable: Jaunder does not
discover, download, or bundle a browser. `package` then writes a deterministic
importable ZIP and fails rather than replacing an existing output; keep ZIPs as
workflow and release artifacts, not repository files. None of these commands
opens a database, contacts a Jaunder server, or changes catalog state.

The thumbnail is a source-controlled presentation artifact. It renders Jaunder's
fixed, versioned Style Contract fixture at exactly 1200×800, with deterministic
fixture content, package-local assets, loaded local fonts, and animation and
caret rendering disabled. The fixture covers representative navigation,
masthead, Post, metadata, tag, and continuation hooks; it is not a substitute
for the manual route, responsive, accessibility, or browser checks below.

Use the reusable
[`.github/workflows/theme-repository.yml`](../.github/workflows/theme-repository.yml)
from a caller workflow pinned to an immutable workflow commit. It owns Jaunder's
canonical `packages.theme-thumbnail-environment` contract (pinned Chromium,
fonts, fontconfig, locale, time zone, device scale, and browser flags);
`devShells.theme-thumbnail` exposes the same environment for local work. Callers
must not recreate those browser or font pins. Run repository-owned preprocessing
and its committed `style.css` drift check before calling it:

```yaml
jobs:
  theme:
    uses: jaunder-org/jaunder/.github/workflows/theme-repository.yml@<immutable-workflow-commit>
    with:
      jaunder-revision: <immutable-jaunder-commit>
      theme-directory: .
```

`jaunder-revision` is required; `theme-directory` defaults to `.`. The reusable
workflow checks source, creates a temporary canonical thumbnail and fails if it
differs from committed `preview.png`, then uploads the deterministic
`theme-package.zip` in the `theme-package` artifact on branch and pull-request
runs. On a SemVer tag with a `v` prefix (`vMAJOR.MINOR.PATCH`, optionally
followed by a prerelease and build metadata) it creates a GitHub release and
attaches that exact `theme-package.zip`; stable tags create published releases,
and prerelease tags such as `v1.2.3-rc.1` create prereleases. Core version
numbers and numeric prerelease identifiers cannot have leading zeroes. Callers
need `contents: write` for this path (`contents: read` otherwise). A rerun may
verify or restore a missing asset, but it refuses to replace different bytes at
an existing version: changed packages receive a new tag.

The release tag is the Theme repository's distribution version. Manifest
`schema` and `style_contract` values are Jaunder compatibility versions, not the
Theme's release version. Theme Package schema 1 and Style Contract 1 were
introduced in Jaunder 1.0.0, so packages targeting both require Jaunder 1.0.0 or
later. Schema 1 deliberately has no release-version field; a versioned release
URL identifies the distribution while the deterministic ZIP bytes identify the
exact package. Installed-version metadata or update discovery would require a
separately designed manifest revision.

Importing the ZIP into Studio still creates a private, unselected draft. Preview
it, explicitly publish it, then explicitly select the published theme;
repository automation does not import, publish, or select a theme.

## Theme Package format

A Theme Package is a ZIP archive with exactly this shape:

```text
theme.json
style.css
assets/
  body.woff2
  logo.webp
  header-one.avif
```

`assets/` is optional. Every file below it must be declared in `theme.json`.
Directories, duplicate paths, traversal paths, symlinks, encryption, and extra
root files are rejected.

A version-1 manifest has this closed shape:

```json
{
  "schema": 1,
  "name": "Paper",
  "style_contract": 1,
  "assets": {
    "assets/body.woff2": "font/woff2",
    "assets/logo.webp": "image/webp",
    "assets/header-one.avif": "image/avif"
  },
  "defaults": {
    "logo": "assets/logo.webp",
    "header": ["assets/header-one.avif"]
  }
}
```

Rules:

- All five top-level fields are required: `schema`, `name`, `style_contract`,
  `assets`, and `defaults`. `assets` and `defaults` may be empty.
- `schema` selects the closed `theme.json` data shape. Version 1 is the only
  accepted value.
- `style_contract` selects the public document hooks and CSS behavior the theme
  targets. Version 1 is the only accepted value. It is separate from `schema` so
  the package format and the styling interface can evolve independently. Schema
  1 and Style Contract 1 require Jaunder 1.0.0 or later.
- `name` must contain non-whitespace text.
- Supported asset types are `font/woff2`, `image/png`, `image/jpeg`,
  `image/webp`, and `image/avif`.
- `defaults.logo` names one declared image.
- `defaults.header` is a non-empty, duplicate-free array of declared images.

- `style.css` is the only stylesheet entry point.
- SVG, HTML, JavaScript, source maps, and undeclared or mislabeled assets are
  rejected.

Package URLs are relative paths such as `url(assets/body.woff2)`. Jaunder
rewrites them to immutable same-origin URLs when it validates the package.
Absolute, protocol-relative, `data:`, and `blob:` URLs are rejected.

Published CSS and package assets are served at immutable
`/theme/<sha256-digest>` URLs. Owner-authorized draft previews use
`/theme/draft/<theme-id>/<asset-path>`; those draft URLs are private and
`no-store`.

## Style Contract version 1

Every selector is scoped below:

```css
[data-jaunder-theme-surface][data-jaunder-style-contract="1"]
```

Write selectors against semantic `data-jaunder-part` hooks, not wrapper depth,
positional selectors, or incidental classes. `html`, `body`, and `:root` map to
the theme surface. For example:

```css
:root {
  --paper: #f7f2e7;
  --ink: #27231d;
  color: var(--ink);
  background: var(--paper);
}

[data-jaunder-part="main"] {
  max-width: 72rem;
  margin-inline: auto;
}

[data-jaunder-part="post"] {
  display: grid;
  gap: 0.75rem;
  padding-block: 2rem;
  border-block-end: 1px solid color-mix(in srgb, currentColor 20%, transparent);
}
```

The stable hooks are:

| Hook                 | Meaning                                                        |
| -------------------- | -------------------------------------------------------------- |
| `primary-navigation` | The public navigation landmark                                 |
| `masthead`           | The public masthead, including the optional Local Site Tagline |
| `site-title`         | Textual Local site identity                                    |
| `logo`               | Optional decorative package or Media logo                      |
| `header-image`       | Optional decorative package, Media, or pooled header image     |
| `main`               | The main-content landmark                                      |
| `post-list`          | Timeline or tag-route Post list; absent on a permalink         |
| `post`               | One rendered Post article                                      |
| `post-header`        | A Post header                                                  |
| `avatar`             | Optional author avatar                                         |
| `author-name`        | Author display name                                            |
| `author-handle`      | Optional author handle                                         |
| `published-time`     | Publication time                                               |
| `post-title`         | Optional Post title heading                                    |
| `post-summary`       | Optional Post summary                                          |
| `post-body`          | Post body; its sanitized semantic descendants are styleable    |
| `post-footer`        | A Post footer                                                  |
| `tag-list`           | Optional tag list                                              |
| `tag`                | One tag link                                                   |
| `source-attribution` | Optional source attribution                                    |
| `continuation`       | Optional pagination continuation                               |

The contract guarantees these concepts, landmarks, route presence, cardinality,
and accessible source order. The optional Site Tagline is plain text within the
existing `masthead` concept; target it through that hook and ordinary descendant
selectors rather than expecting a separate tagline hook. It does not guarantee
wrapper depth or incidental sibling positions. Owner-only Post Actions controls
are not Style Contract content: Jaunder mounts them in a trusted sibling outside
this surface, tethered to a protected Post-header slot. Theme CSS cannot
directly style or suppress those controls, but a theme that removes or clips a
Post/header can remove their visual anchor; select **Studio** in `/themes` to
recover the controls.

Jaunder accepts standard declarations and CSS custom properties, subject to the
same URL and global-name checks. `anchor-name` is reserved for Jaunder's trusted
Post Actions controls and Theme Packages cannot declare it. Supported at-rules
are top-level `@font-face` and `@keyframes`, plus nestable `@media`,
`@supports`, and `@container`. `@import`, native nesting, unscopable selectors,
external resources, and hidden font/keyframe references are rejected. Jaunder
prefixes authored font-family and keyframe names to prevent collisions.

## Test a draft

Import and draft saves run the same bounded parser-backed validator used by
preview and publication. A successful save therefore proves that the archive,
manifest, CSS syntax, selectors, URLs, and assets satisfy the platform rules; it
does not prove the visual design is usable.

Select the draft in the catalog and choose **Preview draft**. Preview uses the
real public renderer in a sandboxed document and does not change public
selection.

Before publication, check at least:

- the site home page, an author timeline, a tag route, and a permalink;
- Posts with and without titles, summaries, avatars, tags, and attribution;
- narrow and wide viewports;
- keyboard navigation, visible focus, contrast, zoom, and reduced-motion
  behavior;
- configured logo/header defaults and every explicit header-pool entry;
- a fresh signed-out browser context after publication;
- that Studio remains visually unchanged and that theme Post/header layout keeps
  owner Actions controls visibly anchored; if it removes or clips that ancestor,
  selecting Studio remains the supported recovery.

A header pool is deterministic for a route and the current theme state. Use
**Shuffle assignments** to deliberately remap routes; it is not per-request
randomness.

For contributors changing Jaunder's theme implementation, the focused platform
checks are:

```sh
devtool run -- cargo xtask test-local -- -p host theme_package
devtool run -- cargo xtask e2e-local theme-management.spec.ts
```

These commands test Jaunder itself. Studio import and preview remain the
canonical validation path for a particular Theme Package.

## Modify a theme

Selecting a catalog entry opens its editor. Available operations include:

- **Rename** changes the catalog label without changing the stable Theme ID.
- **Save CSS** replaces the authored stylesheet while preserving the manifest
  and package assets.
- **Save complete draft package** changes `theme.json`, `style.css`, and the
  lossless asset list together.
- Presentation controls choose packaged defaults, an explicit package image,
  owned Media, no image, or an explicit header pool.
- **Export ZIP** downloads the current editable package. Owner Media bindings
  are instance-local and are not included.

Draft edits are private. They do not alter the currently published revision or
public selection. Preview again after every material edit.

Keep exported, known-good ZIP packages in source control outside Jaunder. That
provides reviewable history and a portable rollback input; the Studio UI does
not select historical published revisions directly.

## Publish and deploy

Deployment has two explicit steps:

1. Choose **Publish** to validate the complete draft and create an immutable,
   content-addressed revision.
2. Choose the published custom theme under **Public selection**.

Only published themes appear as custom selection options. Selecting an author
theme affects that author's public routes. Selecting a site theme affects site
routes and authors who inherit the site selection. Authors can choose **Inherit
site selection** instead of an override.

Publishing a new revision of an already selected theme advances its stable Theme
ID, so public pages adopt the new revision without another selection change. A
failed publish leaves the previous published revision and public presentation
unchanged.

For a cautious rollout:

1. export the current known-good package;
2. save and preview the new draft;
3. publish during a low-traffic window;
4. verify from a fresh signed-out browser context;
5. if necessary, immediately select a built-in theme or another published theme;
6. re-import the known-good ZIP, publish it, and select it for package-level
   rollback.

Deleting a selected site theme falls back to **Studio**. Deleting a selected
author theme restores site inheritance. Previously issued immutable CSS and
asset URLs remain available for the retention window, so fresh cached public
HTML is not broken by deletion.

## Related implementation documentation

- [Theme management routes and endpoint census](flows/theme-management.md)
- [Current custom-theme architecture](ARCHITECTURE.md#custom-public-themes)
- [Custom-theme architectural decision](adr/0184-css-package-public-themes.md)
