# Public Theme Selectors

Issue: #21

Supersedes the browser-local design archived at
`docs/archive/2026-09-02-issue-21-theme-selector-spec.md`. That document remains
a frozen record of the rejected design.

## Outcome

The operator selects a default public theme for the Jaunder site, and each
author may override that theme for their own publication pages. Jaunder persists
both settings in the database and resolves one deterministic theme for every
public URL. Presentation never varies by visitor, browser, or session.

## Ownership and precedence

- Jaunder keeps its existing domain model: a User is the publication; this work
  does not introduce a Blog entity.
- The operator-owned site theme applies to aggregate and non-author-specific
  public pages, including the site timeline and site tag pages.
- An authenticated author's optional theme override applies to that author's
  public User page, author tag pages, and post permalinks.
- When an author has no override, their pages inherit the current site theme.
  Choosing `Site default` deletes the override, so later operator changes flow
  through automatically.
- If the operator is also an author, `/profile` exposes both settings as
  separate, programmatically named controls.

## Persistence and authorization

- The accepted built-in values are exactly Terminal, Studio, and Reader, backed
  by one closed serializable theme type.
- The site default uses the existing typed `site_config` store. Only an
  authenticated operator may call its settings read or mutation interface. The
  viewer-independent public resolver may read the site theme internally and does
  not expose the stored value through an authenticated-user response.
- The author override uses the existing typed `user_config` store. An
  authenticated author may read or mutate only the row keyed by their own
  authenticated User identity; no client-supplied owner identifier is trusted.
- Both stores already use generic key/value tables shared by SQLite and
  PostgreSQL, so no schema migration is required. Both mutations use the
  existing `WriteScope`/`MutationOutcome` contract.
- A missing or invalid site-theme value resolves to Studio. A missing or invalid
  author-theme value resolves to the current resolved site theme. Database read
  errors do not use either fallback.
- Each segmented-button choice dispatches its persisted mutation immediately;
  there is no Save button. Visible success is reported only after
  `MutationOutcome::Confirmed`. Confirmed and `CommitIndeterminate` outcomes
  both invalidate and revalidate persisted state. After either outcome, the
  control adopts the successfully reread persisted value, including inherited
  `Site default`. If rereading fails, it retains the last confirmed selection
  and shows the read error. `CommitIndeterminate` remains visibly error-like
  even when rereading succeeds and never claims persistence; rollback-confirmed
  failures remain error-like.

## Rendering

- The public projector resolves the effective theme from the public route's
  ownership plus site and optional author configuration.
- The effective theme is part of the public projector/CSR seed and the pure
  shell-rendering input. Public markup contains the final `.j-root[data-theme]`
  before CSS paint, and CSR mounts with the same value.
- Every public navigation response carries the destination route's
  server-resolved effective theme alongside its page data, and the client
  applies that value when committing the destination route.
- Public responses remain viewer-independent and cacheable. Their ETags derive
  from final representation bytes, so changing an effective theme changes the
  representation and ETag without a separate cache subsystem.
- Authenticated cockpit and private application surfaces retain the Studio
  application theme. The settings controls update their own selected state but
  do not turn theme into a viewer-specific cockpit preference.
- The browser-local `jaunder_theme` read/write path, storage telemetry contexts,
  and unknown-local-identifier behavior are removed completely.

## Acceptance

- A non-operator cannot see the site-theme control, and direct site-theme read
  or mutation requests are rejected by server-side operator authorization.
- An authenticated author can see and mutate only their own theme override;
  anonymous and cross-author mutation requests are rejected.
- `/profile` exposes accessible segmented-button controls: `Site theme` for an
  operator and `Your pages theme` for every author. The author control includes
  `Site default`, Terminal, Studio, and Reader; each button reports its state
  with `aria-pressed`.
- Confirmed choices update the selected button state without a page reload and
  survive a fresh authenticated browser context without local storage.
- Choosing `Site default` deletes the author override. A later operator theme
  change then changes that author's public pages without another author write.
- A fresh anonymous browser receives the site theme on the site timeline and
  site tag pages.
- A fresh anonymous browser receives an author's override on that author's User
  page, author tag pages, and post permalinks; an author without an override
  receives the current site theme on those routes.
- Initial projector markup and the CSR-mounted tree use the same effective
  theme, preventing a default-theme flash during boot. Public client navigation
  recomputes the theme for the destination route rather than retaining a stale
  source-page theme.
- Missing or corrupt values follow the fallback rules above. Site-theme or
  author-theme storage read failures surface as errors rather than rendering a
  false default.
- Confirmed and commit-indeterminate mutations revalidate their setting and
  adopt a successfully reread value. A failed reread retains the last confirmed
  selection and displays the read error. Commit-indeterminate results remain
  visibly error-like and never claim whether the write committed.
- Identical public content and effective theme produce identical cacheable
  bytes/ETags; changing the effective theme changes the rendered representation
  and ETag.
- Typed storage, authorization, mutation outcome, pure rendering, projector,
  navigation, and browser coverage prove these contracts with SQLite/PostgreSQL
  parity where storage behavior is involved.

## Boundaries

- Custom theme creation, assets, layout controls, CSS editing, and theme
  lifecycle remain in #1341 and do not block this issue.
- This issue adds no Blog aggregate, per-viewer theme, browser-local fallback,
  or compatibility storage path.
- This issue does not change the three shipped CSS theme definitions.
- Public cache expiry remains governed by the existing cache policy; no purge or
  versioning subsystem is introduced.
