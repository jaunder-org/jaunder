# Issue #1733 — Settings and Account hubs replace sidebar configuration links

## Outcome

A signed-in user's sidebar lists only the places where they read and write
Posts. Configuration moves behind two controls at the right end of every page
header, after page actions such as the timeline sort button: a **Settings** cog
and an **Account** avatar. Each control opens a hub page that links to the
existing configuration pages. On a phone, the top strip no longer scrolls
through 17 links to reach Passkeys or Sign out.

## Approach and tradeoffs

**Sidebar catalog.** The signed-in sidebar keeps Home, Compose, Manage Posts,
Drafts, Scheduled, History, Media and Audiences. Themes, Sessions, Passkeys,
Settings (`/profile`), Invites and the four operator `/admin/*` items leave it.
The footer drops the avatar, username and Sign out, and keeps the RSS link. The
anonymous sidebar is unchanged.

**Hubs.** There are two new private routes. Each renders a `Topbar` and grouped,
linked rows, every row showing a title and a one-line description:

| Hub         | Groups and rows                                                                                                                                                                                                           |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `/settings` | _Publishing_: Profile and posting defaults (`/profile`), Themes. _Invitations_: Invites, when `may_issue_invitation` allows. _Site administration_ (operators only): Configure Backups, Site Settings, SMTP Relay, WebSub |
| `/account`  | Username header; Email (`/profile/email`), Passkeys, Sessions; a Sign out link (`/logout`)                                                                                                                                |

`/profile` goes under Settings, not Account. Besides the username card, it holds
Default Audience, Content License and Default Post Format, and the sidebar
labels it "Settings" today. Splitting that page is out of scope.

Row visibility uses the same viewer rules as today's sidebar, in a pure,
host-tested catalog. Existing page URLs and server-side authorization are
unchanged. Hiding a link is presentation, not access control.
`APPROVED_PRIVATE_ROUTES` gains `Settings → /settings` and `Account → /account`.

**Corner controls in `Topbar`.** The controls render last in the `Topbar` right
slot (`j-topbar-right`), in normal flow and driven by the session signal. Every
app page, public timelines included, already renders the reactive `Topbar` after
mount. The one projector-injected masthead, Local `/`, is anonymous-only:
signed-in visitors are redirected to `/app` before paint. Because the controls
sit inside the header, they share Home's sticky chrome, sit below warning
banners and wrap with the header at narrow widths. No shell-level positioning is
needed.

- **First paint.** The controls render only for a signed-in viewer. The
  anonymous projector paint has none, so on public routes `html.authed` reserves
  their exact width before first paint with a pseudo-element in the header's
  right slot, which steps aside once the real controls exist. The sort button
  therefore does not move when they fill in (ADR-0044 D4). The pure
  `topbar::render` twin is unchanged. The class is not updated by in-app
  login/logout; a stale value costs at most an empty reservation until the next
  document load, never an overlap.
- **Markup.** Both controls are links (`<a>`, `aria-label="Settings"` /
  `"Account"`), not menus. The matching control carries `aria-current="page"` on
  its hub and on that hub's sub-pages.
- **Themes.** On themed public pages the controls sit in the masthead, inside
  the themed area, as today's signed-in sidebar links already do. They are not a
  new Style Contract hook. Protected anchored chrome (ADR-0188) is for mutation
  controls and is not needed for navigation links.
- **Cleanup.** The `html.authed` reservation for the sidebar footer is dropped
  with the avatar row.

**Sub-page orientation.** Each page reached from a hub shows a "← Settings" or
"← Account" link in its header. `Topbar` derives it from the hub catalog and the
current path, so no page declares its own parent.

**Tests.** Browser tests that relied on the removed links keep what they prove:

- **Login-success signal.** `SEL.logoutLink` in the shared helpers changes to
  the Account control.
- **Logout.** Flows that test logout from the UI go through `/account`; other
  flows navigate to `/logout` directly.
- **Navigation.**
  - Admin, Passkeys and Profile navigation through `.j-nav-item` / `.j-nav a`
    and `passkeys-nav-link` moves to the hub rows.
  - `.j-sb-foot` username checks move to the Account hub header.
  - The exact signed-in nav count/href list in `auth.spec.ts` is updated.

## Acceptance

Proof to execute:

1. **Host tests.**
   - The sidebar catalog no longer contains the moved destinations.
   - Hub catalogs return the right rows for member and operator viewers, and for
     each Registration Policy (the existing Invites matrix moves with the item).
   - Every hub page and row is a private route, and back-link ownership is
     exact.
2. **Browser tests.**
   - A signed-in member sees the controls after the sort button on Home and on a
     public timeline (`/~<user>`), and on a private page (`/drafts`). This
     includes the in-app login flow without a reload.
   - Settings shows no Site administration group for a member; an operator sees
     all four admin rows, and each navigates to its page.
   - Account → Sign out signs the user out, and the controls disappear.
   - Account → Passkeys reaches the Passkeys page, which shows "← Account".
   - The controls carry `aria-current` on their hub and sub-pages.
   - An anonymous viewer sees no controls.
3. **Layout shift.** The signed-in projector probes (`authed-cls`, and
   `timeline-cls` on `/~<user>`) stay green with the header's sort control added
   as a probe target, measured after the corner controls fill in.
4. **`visual-proof`.**
   - Before/after pairs: Home (signed-in operator) and a public timeline, each
     at 1280×760 and 390×844.
   - After-only: `/settings` (operator) and `/account` at both sizes.

Deliverables to commit:

- Code, tests and CSS.
- `docs/flows/README.md`: route graph gains `/settings` and `/account`; the
  operator-sidebar and sign-out edges are replaced.
- `docs/coverage/csr-e2e-matrix.md`: the sidebar Themes citation is updated.
- A `docs/web-style-guide.md` note: account and settings navigation is the
  `Topbar` corner controls, not sidebar items or per-page actions.

## Boundaries

- No redesign of the remaining narrow-screen content strip (bottom tabs,
  hamburger menu). File it as a follow-up if wanted.
- No dropdown menus or settings shell. Existing pages are not restructured
  beyond the back link; `/profile` is not split.
- No change to Style Contract hooks, page URLs, authorization or the anonymous
  chrome.
