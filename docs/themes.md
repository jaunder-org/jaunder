# Creating and deploying themes

Jaunder custom themes are CSS packages for the public site. They style Jaunder's
versioned semantic markup; they cannot replace markup, add JavaScript, or add
server behavior.

Theme management lives at `/themes`:

- One workspace edits **User** or **Site** settings through tabs. Operators can
  keep personal and site selections separate; switching tabs preserves
  independent editor buffers. Ordinary users manage only their own User theme.
- User theme controls visitors' public user pages; Site theme controls public
  site pages and user pages following it. Neither changes Home or private
  Studio.
- **Available themes** combines bundled choices with only the current owner's
  custom themes. The whole current tile is highlighted only after confirmed
  reads. Its **Use** button's pressed state is independent of the editor target.
  Confirmed author inheritance marks **Follow site theme**; a confirmed unset
  site setting marks Studio, the resolver's default. Failed reads say **Current
  theme could not be checked** and mark no card current.
- Author and site themes never cross owners implicitly. Move a theme between
  owners or instances by exporting and importing its Theme Package.

## Bundled defaults and application styling

Studio, Terminal and Reader are system-managed bundled Theme Packages. Selecting
a bundled name follows the revision shipped by the installed Jaunder release.
They use the same package compiler, immutable digest delivery and
retained-content lifecycle as custom packages, but cannot be edited or deleted
through custom catalogs and do not consume their quotas. Site/author precedence
and Studio fallback are unchanged.

Protected application CSS is a separate, non-selectable system role sharing that
delivery lifecycle. Custom CSS remains scoped to public presentation and cannot
style trusted controls or private surfaces. Home never loads the selected public
Theme Package. See [application asset lifecycle](application-assets.md) for the
complete inventory, cache policy, upgrade/rollback and fresh-target restore
rules.

## Choose a starting point

The workspace has two sections:

1. **Available themes** is a responsive grid. Custom cards offer **Use**,
   **Edit**, and per-item **Copy**; built-in cards are read-only and offer
   **Preview** and **Copy**. Copy immediately copies that saved source without a
   chooser. **Create theme** opens a local Name/Style/Logo/Header form. **Upload
   theme ZIP** opens the native ZIP picker; a chosen file uploads once, while
   cancel writes nothing. The validated manifest name seeds automatic
   allocation, never the filename.
2. **Theme editor** edits Name, Style (CSS), Logo, and Header image together.
   One **Preview** renders all current unsaved fields in isolation; **Save
   changes** persists the complete form. A new theme remains private and
   unpublished. An inactive existing theme saves a draft without advancing its
   published revision or changing selection. Saving a changed active theme also
   publishes that revision; public pages using it update without another Use
   action, and the existing selection is preserved.

**Copy** is a permanent per-item action on eligible bundled and custom cards. It
copies saved content, not unsaved editor buffers, and preserves the package and
saved presentation. The confirmed result is a new private, unpublished,
unselected theme. Any unavailable or unauthorized image rejects the whole Copy.
An uncertain outcome rechecks owner state without claiming success or
automatically replaying Copy. Follow site theme is inheritance, not a Copy
source. Copy leaves the source editor buffer untouched; only after confirmation
does the editor open the new private copy.

**Preview** renders the complete current editor form—including unsaved Name,
Style and image choices—in an isolated visual preview. It does not save, publish
or select. Switching choice/owner or saving clears obsolete output; loading and
errors never leave another theme's preview shown. Preview is explicit rather
than eager work for every card or catalog refresh and does not automatically
retry.

This is a rendered preview, not a packaged PNG thumbnail. Installed Theme
Packages currently contain no standard thumbnail field; a repository's
`preview.png` is not a package member. Built-in cards remain immutable and
read-only, without exposing editable custom CSS. Follow site theme remains
inheritance guidance, not a theme preview input. Built-in and inheritance Use
actions change selection only.

The current-theme summary is based on confirmed reads; unknown reads never imply
a selected theme. Saving a changed active theme advances its public revision
directly. Using another custom theme publishes and then selects its captured
Theme ID; if selection fails after publication, the published revision is not
rolled back.

The **Theme tools** disclosure contains developer-focused package authoring and
export/import tools; it is separate from the everyday editor. **Images**
provides explicit presentation-media operations, including header-pool rotation.
**Published versions** separately selects a custom theme's current published
version using **Use published version**; it does not publish newer draft edits.
These tools do not replace the normal form's Preview and Save changes. Upload
uses the manifest name for automatic allocation; explicit API names remain
strict.

Catalog labels are trimmed, nonempty, and limited to 100 Unicode scalar values.
Studio, Terminal and Reader are reserved case-insensitively. Automatic Upload
and Copy allocation uses the base when available, otherwise the lowest available
` 2`, ` 3`, etc. Reserved bases are automatic seeds, never final labels. Numeric
endings are literal (`Name 2` copies to `Name 2 2`); only a generated stem may
shorten to fit the scalar limit. Explicit names never truncate or gain suffixes,
and invalid names/collisions still reject. Malformed packages still fail. One
sticky live status reports progress, confirmed success, rejection or uncertainty
outside the detail disclosures.

The existing multipart import API remains compatible: `scope` and `archive`
retain their meanings; `name` is optional. Only an omitted `name` defaults from
validated manifest metadata. An explicitly supplied invalid or blank name fails
without fallback; valid supplied names and existing response shapes are
unchanged. Omitted-name automatic allocation occurs transactionally with
insertion, and returns the actual allocated ID/name. Package validation precedes
manifest-default naming.

Creating or importing by any starting point remains private and unselected; it
never changes public selection. Use plain CSS while establishing a layout, and a
complete package for portable fonts, assets and defaults.

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

Uploading the ZIP into Studio still creates a private, unselected draft. Review
and edit it in the normal editor; use its card **Use** action when ready.
Repository automation does not import, publish, or select a theme.

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
| `navigation-rail`    | Public navigation rail and its supporting chrome               |
| `site-brand`         | Jaunder brand link within the navigation rail                  |
| `navigation-search`  | Search affordance within the navigation rail                   |
| `primary-navigation` | The public navigation landmark                                 |
| `hero`               | Stable container for header image and masthead                 |
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
| `post-title`         | Post title heading or `#` permalink fallback                   |
| `post-summary`       | Optional Post summary                                          |
| `post-body`          | Post body; its sanitized semantic descendants are styleable    |
| `post-footer`        | A Post footer                                                  |
| `tag-list`           | Optional tag list                                              |
| `tag`                | One tag link                                                   |
| `source-attribution` | Optional source attribution                                    |
| `continuation`       | Optional pagination continuation                               |

A Post with a permalink but no visible Rendered Title shows a `#` link named
`Permalink` in the `post-title` heading. This navigation fallback also covers an
empty sanitized title; it does not become authored title data. Posts without a
visible title and without a permalink omit the heading.

Highlighted Post-body `pre code` has fifteen closed, scoped `j-syn-*` token
hooks. In addition to `comment`, `keyword`, `string`, `number`, `function`,
`type`, `variable`, `constant`, `operator`, and `punctuation`, the additive
roles are `function-call`, `diff-plus`, `diff-minus`, `heading`, and `quote`.
Each has a matching `--j-syn-*` CSS variable with a built-in default. Existing
packages remain valid without overrides; a public theme may choose, for example,
`:root { --j-syn-function-call: #573c8b; }`. Choose overrides with at least
4.5:1 contrast against the theme's code background. Built-in defaults mix each
theme's semantic color with its ink to keep small monospace text legible. Do not
style an authored hook outside `[data-jaunder-part="post-body"] pre code`: a
permitted forged class may survive sanitization, but only code tokens should
acquire color.

The contract guarantees these concepts, landmarks, route presence, cardinality,
and accessible source order. Every public route has one `hero`; its optional
decorative `header-image` precedes and shares that stable container with the
`masthead`, so themes may layer the image behind masthead content without
inferring wrapper or sibling structure. The optional Site Tagline is plain text
within the existing `masthead` concept; target it through that hook and ordinary
descendant selectors rather than expecting a separate tagline hook. The contract
does not otherwise guarantee wrapper depth or incidental sibling positions.
Owner-only Post Actions controls are not Style Contract content: Jaunder mounts
them in a trusted sibling outside this surface, tethered to a protected
Post-header slot. Theme CSS cannot directly style or suppress those controls,
but a theme that removes or clips a Post/header can remove their visual anchor;
select **Studio** in `/themes` to recover the controls.

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
- Posts with and without titles, summaries, tags, and applicable continuation;
- the avatar and author handle that the current renderer always emits;
- static CSS and layout treatment for optional avatar, author-handle, and
  source-attribution hooks without fabricating unsupported DOM states;
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

**Edit** opens a custom theme's editor without activation; **Use** activates the
custom theme's saved content. Built-in themes are immutable: their card offers
read-only Preview and Use, and they cannot be edited. The normal editor exposes
Name, Style (CSS), Logo and Header image together. **Theme tools** holds
developer-oriented package and management operations; **Images** exposes
separate presentation-media controls, including header-pool rotation. Expanded
controls remain reachable by ordinary scrolling and keyboard navigation.

The developer **Theme Package** editor changes the manifest, shared stylesheet
and lossless asset list separately from normal **Save changes**. **Manage
Theme** contains export, publication and removal actions. Owner Media bindings
are instance-local and are not included in an exported ZIP.

Draft edits remain private until saved. Preview is an explicit read-only render
of the current unsaved form; saving an inactive theme preserves its prior
published revision and public selection. For each existing image binding,
**Keep** preserves its saved value and original ownership/provenance, including
a valid existing Site binding. **Theme default** restores the packaged default;
**No image** explicitly hides the image, even when a default exists. Selecting a
fixed Header image replaces a rotating header-image pool; the UI warns about
that replacement.

Keep exported, known-good ZIP packages in source control outside Jaunder. That
provides reviewable history and a portable rollback input; the Studio UI does
not select historical published revisions directly.

## Saving, publishing and selecting

The everyday editor's **Save changes** saves the complete form. For an active
custom theme, a changed saved revision is published immediately and public pages
using that theme update without a separate Use action. For an inactive existing
theme, Save changes its draft while preserving the prior published revision and
selection. Creating a theme starts a private, unpublished theme. Save's
`applied` receipt describes whether the public revision advanced; `false` does
not imply the target is inactive.

The card's **Use** action for a custom theme publishes its saved draft and then,
once publication is confirmed, selects the same captured Theme ID and scope.
Publication and selection are distinct operations and are not atomic:
publication may succeed even if selection fails. Built-in choices and **Follow
site theme** change selection only. **Published versions** separately lets the
user choose an already published revision through **Use published version**; it
is not an editor Save/Publish action.

Pending actions cannot duplicate an operation or change their captured
scope/target. Confirmed completion is distinguished from rejection and
indeterminate acknowledgement. Uncertain writes stop chaining; catalog and
selection are reread for current state, but those reads do not prove the write's
outcome. Failed rereads leave state unknown. The UI never automatically replays
a write. Custom public themes do not affect Home or private Studio, which retain
protected application styling. Selecting a site theme affects site routes and
authors using the site theme; a User theme affects that author's public routes.

An indeterminate database acknowledgement, lost response, or undecodable
response after dispatch stops chaining and reports uncertainty, not success,
rollback or definite nonactivation. Catalog and selection are reread; failed
rereads explicitly leave current state unknown. No automatic retry runs. Review
that feedback and verify current public presentation before a manual retry; if
rereads failed, reopen Themes to obtain current state. Choosing a bundled theme
or author inheritance uses selection alone and never publishes a custom draft.
Home and private management retain protected application styling regardless of
public selection.

For a cautious rollout, preserve a known-good Theme Package, edit and preview
changes before saving, then verify the public result from a fresh signed-out
browser context. If needed, select a built-in or other published theme as a
separate recovery action.

Deleting a selected site theme falls back to **Studio**. Deleting a selected
author theme restores site inheritance. Previously issued immutable CSS and
asset URLs remain available for the retention window, so fresh cached public
HTML is not broken by deletion.

## Related implementation documentation

- [Theme management routes and endpoint census](flows/theme-management.md)
- [Current custom-theme architecture](ARCHITECTURE.md#custom-public-themes)
- [Application asset inventory and cache lifecycle](application-assets.md)
- [Custom-theme architectural decision](adr/0184-css-package-public-themes.md)
