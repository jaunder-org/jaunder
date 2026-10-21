# Theme management

Matrix: `matrix:docs/coverage/csr-e2e-matrix.md#theme-management-api-transport`

## Routes

- `route:/themes`

`/themes` is an authenticated Studio route. It begins in the session owner's
User setting. Operators use the **User** and **Site** tabs to address either
independent setting in one workspace; their editor buffers remain separate when
switching tabs. Ordinary users see only their User setting. The server remains
authoritative for selected scope and rejects site operations without operator
status.

The route loads catalog, selection, draft and presentation data through the
owner-authorized API. Its custom-theme preview is isolated from Studio: draft
CSS is returned only to a sandboxed preview document and is never inserted into
the private route's stylesheet cascade. The **Available themes** section places
bundled choices and the current owner's custom catalog in a responsive grid;
separate **Create theme** and **Upload theme ZIP** actions remain in the page
toolbar. No other owner's private catalog is discovered. The entire confirmed
current tile is highlighted; its primary button is explicitly named **Use
<theme>** with `aria-pressed` from the confirmed live selection, not the editor
choice. Unknown reads never invent a current tile. **Edit** opens a custom
editor without activation; **Preview** shows a bundled theme read-only. Built-in
themes are immutable but remain selectable through **Use**; inheritance has no
editor or Copy source. Copy is a per-item action for eligible bundled/custom
themes and uses the saved source directly, without a chooser. Expanded semantics
identify the explicit editor target without a second tile highlight or visible
badges. The normal **Theme editor** keeps Name, Style and Logo/Header together
in one unsaved form. Explicit Preview submits the current form to
`preview_editor` and renders it in the isolated sandbox without writing. Save
submits the complete form through `save_editor`: a new theme remains private and
unpublished; an inactive existing theme updates its draft without advancing its
public revision; an active theme updates its draft and published revision
without changing selection. Custom tile Use publishes then selects its captured
saved Theme ID, even if it is already in use. Copy leaves the source editor
buffer untouched and opens the destination editor only after confirmed
completion; it does not activate the copied theme. Uploading and opening editors
never activate a theme. The current-theme summary is based on confirmed reads;
unknown reads never imply a selected theme. `save_editor`'s `applied=false`
receipt is not evidence that the target is inactive.

Upload opens the native ZIP picker; choosing a file uploads once, while cancel
writes nothing. UI Upload always omits multipart `name`. The validated manifest
name seeds automatic owner-local allocation, never the filename. Copy uses the
saved catalog label (canonical package name for a bundled theme). A collision or
reserved base receives the lowest available ` 2`, ` 3`, etc.; existing numeric
endings are literal. Only generated stems shorten to fit the 100-Unicode-scalar
limit. Explicit-name API imports and developer package create/rename stay
strict.

Copy sends one `copy_theme` request with captured scope and source from that
theme's per-item action; it does not open a source chooser or modify the editor
buffer. Copy preserves the latest saved package and presentation, excludes
unsaved source buffers, and creates a private, unpublished, unselected
destination. Only after confirmation does the editor open that destination.
Unavailable or unauthorized image bindings reject the whole operation; no
partial copy is claimed. Rejection and uncertain acknowledgement remain
error-like, revalidate catalog/selection once, and never replay Copy, publish or
select. Pending requests block owner switches and duplicates; switching settings
retains the independent User/Site editor buffers.

Explicit Edit opens a private draft without changing public selection. Built-in
choices remain immutable and read-only, never exposing editable custom CSS.
Built-in cards offer a read-only **Preview** through `preview_installed`. The
normal editor's explicit `preview_editor` renders all current unsaved fields in
the same semantic renderer and titled sandboxed iframe, never inserting custom
CSS into Studio. This is rendered content, not a packaged thumbnail PNG. Follow
site theme has no author stylesheet and remains inheritance guidance, not an
installed-theme preview input. Owner/choice/save-generation guards clear
obsolete output and reject late results. Loading/error states are local to
preview; theme choices remain available and no automatic retry or card-wide
fanout runs. **Theme tools** contains developer package operations; **Images**
exposes separate presentation-media actions; **Published versions** offers **Use
published version** independently of the normal Save changes form. A single
sticky polite live status remains visible while scrolling. The standard scroll
region makes Save and expanded bottom controls reachable.

## Endpoint census

| Endpoint                                 | Status        | Surface                                                                                                      |
| ---------------------------------------- | ------------- | ------------------------------------------------------------------------------------------------------------ |
| `endpoint:/api/themes/list`              | API transport | Lists the authenticated author's or operator's owned theme catalog.                                          |
| `endpoint:/api/themes/get_draft`         | API transport | Reads an owner-authorized editable draft without exposing it to another catalog owner.                       |
| `endpoint:/api/themes/get_presentation`  | API transport | Reads owner-authorized logo/header bindings, pool members, and shuffle seed without Media owner IDs.         |
| `endpoint:/api/themes/get_selection`     | API transport | Reads the site selection or the author's explicit override/inheritance state.                                |
| `endpoint:/api/themes/create`            | API transport | Creates a validated custom-theme draft in the selected owner catalog.                                        |
| `endpoint:/api/themes/import_css`        | API transport | Creates an unselected private version-1 no-asset draft from bounded plain CSS.                               |
| `endpoint:/api/themes/import_package`    | API transport | Replaces a draft from a complete validated package/editor payload.                                           |
| `endpoint:/api/themes/copy_theme`        | API transport | Copies one authorized saved source into a private, unpublished, unselected same-library draft.               |
| `endpoint:/api/themes/import_zip`        | API transport | Imports a bounded ZIP Theme Package as a new unselected private draft.                                       |
| `endpoint:/api/themes/replace_css`       | API transport | Replaces one owned draft's stylesheet while preserving its manifest and package assets.                      |
| `endpoint:/api/themes/export`            | API transport | Exports the exact owner-authorized draft package without relational Media bindings.                          |
| `endpoint:/api/themes/rename`            | API transport | Renames an owned catalog entry under case-insensitive catalog uniqueness rules.                              |
| `endpoint:/api/themes/remove`            | API transport | Removes an owned theme and releases its guarded presentation bindings atomically.                            |
| `endpoint:/api/themes/publish`           | API transport | Validates and materializes the complete draft as an immutable published revision.                            |
| `endpoint:/api/themes/select`            | API transport | Selects a built-in or owned custom theme, or clears an author override to inherit.                           |
| `endpoint:/api/themes/replace_binding`   | API transport | Replaces a fixed logo or header binding after owner and Media checks.                                        |
| `endpoint:/api/themes/replace_pool`      | API transport | Replaces the explicit header image pool and its deterministic shuffle seed.                                  |
| `endpoint:/api/themes/shuffle`           | API transport | Replaces only the persisted header-pool shuffle assignment.                                                  |
| `endpoint:/api/themes/preview`           | API transport | Renders an owned draft through the Style Contract renderer without changing selection.                       |
| `endpoint:/api/themes/preview_editor`    | API transport | Renders the complete current unsaved editor input through the isolated Style Contract preview.               |
| `endpoint:/api/themes/save_editor`       | API transport | Saves all normal editor fields together; publishes a changed revision only when the target remains selected. |
| `endpoint:/api/themes/preview_installed` | API transport | Reads a selected built-in or owned saved custom draft into the isolated renderer without applying it.        |
| `/theme/draft/{theme_id}/{path}`         | API transport | Serves an owned draft asset only to its author owner or an operator owning its site draft.                   |

The mounted `/themes` journey consumes this complete census. **Custom tile Use**
captures scope and Theme ID, publishes, and applies that same ID only after
`Confirmed` publication. Pending actions cannot change that captured target or
dispatch duplicates. Bundled selection and author inheritance use selection
alone. Publication and selection remain separate operations: publication of an
already-selected ID advances its public revision even if the later selection
fails.

Typed rejection prevents chaining. `CommitIndeterminate` or transport/decoding
uncertainty after dispatch stops chaining without an automatic retry, success,
rollback or definite-nonactivation claim. Catalog and selection rereads describe
current state, not proof of an uncertain write's outcome; failed rereads report
unknown state and a next action. The current-theme summary uses existing reads:
known names, **Using the site theme** for confirmed author inheritance,
**Checking current theme…** while loading, or **Current theme could not be
checked** on failure or a missing custom-theme name. A confirmed unset site
setting marks Studio as current, following the existing public-theme resolver
default. A failed or unread selection never implies that fallback. Failed or
pending selection reads disable ready-theme recovery actions, never implying
inheritance. **Use site theme** and explicit bundled **Use** buttons are
selection-only. **Published versions** separately offers **Use published
version** when published custom themes exist; it selects an existing version
without saving or preparing editor changes. Mutation feedback uses one sticky
live region. **Getting your theme ready…** → **Applying your theme…** → **Your
theme is now in use.** preserves the separate stages. Only confirmed completion
has success styling; progress is neutral, and rejection, uncertainty and local
failures use error styling. A single polite, atomic live status follows the
action that produced it. Other confirmed/indeterminate authoring mutations
request owner-state rereads rather than asserting that those reads succeeded.

Installed previews authenticate through the same owner gate, including Site
operator authorization for built-ins, and return private/no-store data. One
principal operation permit covers each request. Custom reads preserve draft
validation and image ownership; built-ins read installed metadata plus eligible,
digest-verified immutable CSS and existing system image defaults. Missing rows
and storage/file/integrity/metadata/UTF-8 errors remain errors, not successful
Studio or prior-preview fallback. The legacy custom `preview` wire and behavior
remain available.

Site-owned operations require an operator; author-owned operations require the
authenticated author, and every draft, package, preview, asset, and binding read
remains owner-scoped. Custom styles affect public site/author routes according
to precedence; Home and private management always keep protected application
styling.
