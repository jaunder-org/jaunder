# CSR flow-to-e2e matrix

This is the canonical, reviewable mapping from stable user-visible CSR flows to
Playwright behavior. A CSR flow is a mounted-app capability, not an
implementation module, helper, `#[server]` function, or protocol-only endpoint.
It is intentionally hand-maintained: a spec path is evidence only after review
confirms its setup, navigation, and assertions exercise the flow. It is neither
generated from route names nor enforced by static name-resolution.

The host coverage gate does not measure wasm-only components; see
[ADR-0050](../adr/0050-stateless-coverage-gate.md) and
[ADR-0070](../adr/0070-web-vertical-wasm-only-component-files.md).
Server-function coverage remains a separate evidence surface under
[ADR-0081](../adr/0081-empirical-server-fn-flow-coverage.md):
[`server-fns.json`](server-fns.json) is its sole committed artifact and records
the checked covered server-function set plus orphan reasons. It does not persist
per-test attribution and is not user-visible CSR-flow-to-spec evidence.

[`docs/flows/README.md`](../flows/README.md) is the checked route/journey index
that consumes these heading anchors. Flow documents link the relevant heading
below and do not copy the server-function snapshot or infer request-to-test
attribution from it.

## Candidate inventory

`web/src/app/component.rs` is the mounted candidate inventory. Every
`ParentRoute`, child `Route`, and the `Routes` fallback is assigned once below.
Paths grouped in one row are one stable user-visible flow; a route still appears
only in that row.

| Candidate                                                                                                                      | Disposition                                                                         |
| ------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------- |
| `ParentRoute path=""`                                                                                                          | [Application shell and boot state](#application-shell-and-boot-state)               |
| `/`                                                                                                                            | [Public reading](#public-reading)                                                   |
| `/app`                                                                                                                         | [Authenticated cockpit](#authenticated-cockpit)                                     |
| `/register`, `/login`, `/logout`                                                                                               | [Authentication](#authentication)                                                   |
| `/register?invite_code=...`                                                                                                    | [Invitation registration](#invitation-registration)                                 |
| `/profile`, `/profile/email`, `/verify-email?token=...`                                                                        | [Profile and email verification](#profile-and-email-verification)                   |
| `/sessions`                                                                                                                    | [App password management](#app-password-management)                                 |
| `/audiences`                                                                                                                   | [Audiences, subscriptions, and visibility](#audiences-subscriptions-and-visibility) |
| `/invites`                                                                                                                     | [Invitation registration](#invitation-registration)                                 |
| `/admin/backups`, `/admin/site`, `/admin/websub`                                                                               | [Administration](#administration)                                                   |
| `/posts/new`, `/drafts`, `/posts/:post_id/edit`, `/history`, `/posts/:post_id/history`, `/posts/:post_id/history/:revision_id` | [Post authoring lifecycle](#post-authoring-lifecycle)                               |
| `/media`                                                                                                                       | [Media management](#media-management)                                               |
| `/themes`                                                                                                                      | [Theme management](#theme-management)                                               |
| `/forgot-password`, `/reset-password?token=...`                                                                                | [Password reset](#password-reset)                                                   |
| `/tags/:tag`, `/:username/tags/:tag`                                                                                           | [Tag browsing](#tag-browsing) — canonical user path: `/~:username/tags/:tag`        |
| `/:username`                                                                                                                   | [Public reading](#public-reading) — canonical user path: `/~:username`              |
| `/~:username/:year/:month/:day/:slug`                                                                                          | [Public reading](#public-reading)                                                   |
| `Routes` fallback                                                                                                              | Out of scope: an error rendering, not a stable user capability.                     |

Stable navigation surfaces outside the route table are assigned to their
containing flows: the shell navigation exposes the route rows above; post links,
tag chips, editor controls, and media controls are behavior inside their
respective flows. The syndication discovery links and AtomPub HTTP endpoints are
protocol surfaces, not mounted CSR entry points; they are out of scope here.

## Covered flows

### Application shell and boot state

**Paths and entry points:** the shared `ParentRoute` shell; first CSR mount and
shell navigation.

**Evidence:** [`end2end/tests/theme.spec.ts`](../../end2end/tests/theme.spec.ts)
checks the mounted root/theme behavior; and
[`end2end/tests/authed-flash.spec.ts`](../../end2end/tests/authed-flash.spec.ts)
checks authenticated shell state after the CSR mount.

### Public reading

**Paths and entry points:** `/`, the mounted `/:username` matcher with canonical
`/~:username` entry, `/~:username/:year/:month/:day/:slug`, and post links.

**Evidence:** [`end2end/tests/posts.spec.ts`](../../end2end/tests/posts.spec.ts)
creates posts and asserts timelines, permalinks, deletion, and author timelines;
[`end2end/tests/unicode-slug.spec.ts`](../../end2end/tests/unicode-slug.spec.ts)
exercises encoded permalink paths; and
[`end2end/tests/theme.spec.ts`](../../end2end/tests/theme.spec.ts) proves
viewer-independent site and author presentation precedence across public routes.

### Authenticated cockpit

**Paths and entry points:** `/app`; authenticated shell navigation.

**Evidence:**
[`end2end/tests/authed-flash.spec.ts`](../../end2end/tests/authed-flash.spec.ts)
asserts the anonymous bounce and authenticated cockpit content;
[`end2end/tests/posts.spec.ts`](../../end2end/tests/posts.spec.ts) asserts the
authored timeline shown by the cockpit; and
[`end2end/tests/navigate.spec.ts`](../../end2end/tests/navigate.spec.ts) clicks
the authenticated Feed link and asserts in-app cockpit navigation without a
document reload.

### Authentication

**Paths and entry points:** `/register`, `/login`, and `/logout`; registration,
login, and shell logout controls.

**Evidence:** [`end2end/tests/auth.spec.ts`](../../end2end/tests/auth.spec.ts)
asserts registration and login rendering, validation, submission, failure,
client-side navigation, and logout.
[`end2end/tests/invite.spec.ts`](../../end2end/tests/invite.spec.ts) also
exercises invitation-constrained registration.

### Profile and email verification

**Paths and entry points:** direct entry to `/profile` and `/profile/email`, and
direct token URL entry to `/verify-email?token=...`.

**Evidence:**
[`end2end/tests/profile.spec.ts`](../../end2end/tests/profile.spec.ts) asserts
profile editing and default-post-format behavior;
[`end2end/tests/email.spec.ts`](../../end2end/tests/email.spec.ts) asserts email
status and change behavior plus invalid verification-token error handling; and
[`end2end/tests/theme.spec.ts`](../../end2end/tests/theme.spec.ts) asserts
operator and author theme controls, authorization visibility, persistence, and
the `Site default` reset.

### App password management

**Paths and entry points:** `/sessions`; app-password create, display, and
revoke controls.

**Covered evidence:**
[`end2end/tests/atompub.spec.ts`](../../end2end/tests/atompub.spec.ts) creates,
displays, and revokes App Passwords through `/sessions`. The external Protocol
Client's AtomPub requests are protocol-only evidence and remain outside this CSR
flow.

### Audiences, subscriptions, and visibility

**Paths and entry points:** `/audiences`; audience controls and profile
subscription controls.

**Evidence:**
[`end2end/tests/audiences.spec.ts`](../../end2end/tests/audiences.spec.ts)
asserts audience creation, membership, refresh, and error rendering.
[`end2end/tests/visibility.spec.ts`](../../end2end/tests/visibility.spec.ts)
asserts audience/public visibility, subscription, and masked-permalink behavior.

### Invitation registration

**Paths and entry points:** `/invites` and the `/register?invite_code=...`
invite-code registration path.

**Evidence:**
[`end2end/tests/invite.spec.ts`](../../end2end/tests/invite.spec.ts) creates an
invitation through the UI, extracts its code, completes registration through the
invite-code route, and asserts the invite-only policy fallback.

### Administration

**Paths and entry points:** `/admin/site`, `/admin/backups`, and
`/admin/websub`; administrator shell navigation.

**Evidence:**
[`end2end/tests/admin-site.spec.ts`](../../end2end/tests/admin-site.spec.ts)
asserts site-setting loads, updates, authorization, and configuration banners.
[`end2end/tests/backup.spec.ts`](../../end2end/tests/backup.spec.ts) asserts
backup schedule, mode, retention, destination, and save behavior.
[`end2end/tests/websub.spec.ts`](../../end2end/tests/websub.spec.ts) asserts hub
configuration, authorization, phase filtering, pagination, exact redrive, and
stale-selection rejection.

### Post authoring lifecycle

**Paths and entry points:** `/posts/new`, `/drafts`, `/posts/:post_id/edit`,
`/history`, `/posts/:post_id/history`, and
`/posts/:post_id/history/:revision_id`; composer, draft-list, permalink, editor,
sidebar History, and active owner Post History controls.

**Evidence:** [`end2end/tests/posts.spec.ts`](../../end2end/tests/posts.spec.ts)
asserts draft creation, publish, edit, slug freeze, unpublish, deletion,
scheduling, audience selection, and in-app transitions between each screen.
[`end2end/tests/history.spec.ts`](../../end2end/tests/history.spec.ts) asserts
sidebar and owner-Post entry points, Current state, complete immutable detail,
cursor append, semantic no-op suppression, and Deleted Post owner access.

### Media management

**Paths and entry points:** `/media`; its rendered Attach media control, and
file controls embedded in the post editor and cockpit composer.

**Evidence:** [`end2end/tests/media.spec.ts`](../../end2end/tests/media.spec.ts)
asserts that the route renders its Attach media control, and separately asserts
file selection and upload behavior in the post editor and cockpit composer.

### Password reset

**Paths and entry points:** direct entry to `/forgot-password` and direct token
URL entry to `/reset-password?token=...`; the forgot-password form.

**Evidence:**
[`end2end/tests/password_reset.spec.ts`](../../end2end/tests/password_reset.spec.ts)
asserts reset token/error and client validation behavior.

### Tag browsing

**Paths and entry points:** `/tags/:tag`, the mounted `/:username/tags/:tag`
matcher with canonical `/~:username/tags/:tag` entry, and post tag chips.

**Evidence:** [`end2end/tests/posts.spec.ts`](../../end2end/tests/posts.spec.ts)
asserts site and per-user tag listings, empty tags, and tag-edit transitions.
[`end2end/tests/timeline-cls.spec.ts`](../../end2end/tests/timeline-cls.spec.ts)
measures both tag-route timeline presentations, while
[`end2end/tests/feeds.spec.ts`](../../end2end/tests/feeds.spec.ts) asserts
client-side navigation from a tag chip.

### Public custom theme presentation

**Paths and entry points:** fresh public author routes and client-side
navigation back to the site route. Site selection, author override/inheritance,
and invalid-selection fallback are resolved by the existing storage precedence
suite.

**Evidence:** [`end2end/tests/theme.spec.ts`](../../end2end/tests/theme.spec.ts)
publishes the shared hostile-but-valid package, then proves its immutable
stylesheet, package defaults, cold anonymous load, and custom identity survive
rendering. The fixture uses fixed positioning, viewport inset, extreme
`z-index`, transforms, filters, visible overflow, and an oversized positioned
descendant; the authenticated owner's trusted action remains visible and
receives pointer hit-testing above it. In-app navigation removes all custom
presentation from the private site surface.

### Theme management

**Paths and entry points:** `/themes`; authenticated sidebar Themes navigation.
Authors manage their own catalog. Operators can switch to the server-authorized
site catalog. The private Studio route never admits a custom stylesheet into its
own document; the real public preview is isolated.

**Evidence:**
[`end2end/tests/theme-management.spec.ts`](../../end2end/tests/theme-management.spec.ts)
proves the authenticated mounted route's Studio root, keyboard-addressable
author and operator scope controls, accessibility, parent-document stylesheet
isolation, and the author lifecycle through CSS import, rename, invalid editor
feedback, lossless asset editing, fixed and pooled presentation Media, shuffle,
isolated responsive preview, publish and custom selection, fresh-context
survival, export, atomic delete fallback, and ZIP re-import. Endpoint ownership,
multipart transport, and detailed error mapping remain separately evidenced by
the transport suite below.

## API transport evidence

### Theme management API transport

**Paths and entry points:** the authenticated `/api/themes/` server-function
transport; it supports the mounted `/themes` route and is also covered directly
where browser UI setup cannot prove transport-specific headers and multipart
ordering.

**Evidence:**
[`end2end/tests/theme-api.spec.ts`](../../end2end/tests/theme-api.spec.ts)
`theme API transport preserves author ownership, private responses, imports, mutations, and preview isolation`
uses a browser context's authenticated `page.request` transport to cover every
author theme endpoint: create/list/import_css/import_package/import_zip,
get_draft/get_presentation/get_selection, replace_css/rename/publish/select,
replace_binding/replace_pool/shuffle, export/remove, and preview. Each confirmed
mutation is followed by an observable catalog, draft, presentation, or selection
reread; removal proves selection fallback. It also covers private `no-store`
responses, preview isolation, owner-only draft assets, anonymous denial, and
ordered multipart ZIP routing/authentication/validation.

| Endpoint                       | Evidence disposition |
| ------------------------------ | -------------------- |
| `/api/themes/list`             | API transport        |
| `/api/themes/get_draft`        | API transport        |
| `/api/themes/get_presentation` | API transport        |
| `/api/themes/get_selection`    | API transport        |
| `/api/themes/create`           | API transport        |
| `/api/themes/import_package`   | API transport        |
| `/api/themes/import_zip`       | API transport        |
| `/api/themes/import_css`       | API transport        |
| `/api/themes/replace_css`      | API transport        |
| `/api/themes/export`           | API transport        |
| `/api/themes/rename`           | API transport        |
| `/api/themes/remove`           | API transport        |
| `/api/themes/publish`          | API transport        |
| `/api/themes/select`           | API transport        |
| `/api/themes/replace_binding`  | API transport        |
| `/api/themes/replace_pool`     | API transport        |
| `/api/themes/shuffle`          | API transport        |
| `/api/themes/preview`          | API transport        |

## Maintenance workflow

When a mounted route, stable CSR entry point, or Playwright behavior changes,
update the affected candidate row and flow section in the same change. Add a new
flow heading before adding the first candidate to it. If review finds no
behavioral spec for a flow, record the duplicate search and selection evidence
here, then create or reuse exactly one milestone-6 `Task` follow-up; do not
claim coverage from a filename, a server function, or the server-function
snapshot. Future flow documents link the affected heading rather than copying
its evidence list.
