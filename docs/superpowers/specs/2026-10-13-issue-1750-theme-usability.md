# Issue #1750: Theme usability

- Status: Approved spec; human said “approved” through the established Speaker.
- Issue: <https://github.com/jaunder-org/jaunder/issues/1750>
- Human approval governs the original manifest-default/narrow-importer proposal:
  `7ea0ca96897eb47d46c82ec0059072592237dde122515dbdb70329792e6fb84c`.
- Reviewed compatibility precision, before these metadata edits:
  `a5787a2d21923af248b4fa1d9b46cca0c3f52e9596e230225ab458e997d149f5`. Manager
  accepts it as nonmaterial, not direct human approval of that hash.
- Evidence: static intake and partial initial BEFORE images at both viewports.
  Selected-draft/scroll/public captures and proof remain outstanding; no runtime
  is ongoing. Metadata preparation does not authorize execution.

## Outcome

Import and use Theme Package ZIPs without source editors or unreachable actions.

## Load-bearing decisions

- The default import form exposes a ZIP picker and import action, not CSS,
  manifest, asset JSON, empty-draft creation, or presentation-media editors.
- Omitted multipart `name` uses existing shared validated `theme.json.name`.
  Choose server-side after package validation, never from the filename. Valid
  supplied `name` stays authoritative; invalid supplied names, including blank,
  fail without fallback. Blank Advanced override omits the `name` field.
- Keep trimmed nonempty catalog labels ≤100 Unicode scalar values.
  Studio/Terminal/Reader remain reserved (ASCII case-insensitive). Reject
  collisions; offer override, never filename fallback, truncation or suffix.
- Import remains a private, unpublished, unselected draft. It does not change
  public selection. Catalog choice opens that entry; it is not activation.
- One obvious “Use this theme” action publishes the chosen custom draft and,
  only after confirmed publication, selects its Theme ID in the chosen scope.
  Publication and selection remain separate existing operations, not atomic.
- Show publishing/applying progress and truthful stage-specific feedback.
  Publishing an already-selected Theme ID immediately advances its public
  revision. Selection failure cannot undo this; report publication and current
  selection from rereads, not definite nonactivation or rollback.
- Only a `Confirmed` publication result permits selection. `CommitIndeterminate`
  database acknowledgement or transport/decoding uncertainty after dispatch
  stops chaining and triggers existing catalog/selection rereads. Explain
  uncertainty, including failed rereads; never infer rollback, success or
  nonactivation. Publication rejection prevents selection. No automatic retries;
  pending actions cannot duplicate operations or target a changed scope.
- Default to “Your public pages”; expose “Site default” only to operators.
  Preserve explicit site switching, authorization and separate catalogs. Bundled
  selection and author inheritance use existing paths, never publication.
- Explain author-public versus site/inheriting-author effects. Home and private
  management retain protected application styling, not public-theme previews.
- Advanced is initially closed, labeled and keyboard-operable with expanded
  state; source/package/media controls, preview/export/rename/delete are opt-in.
- Activation/status stay outside Advanced. Accessible live feedback is near its
  action; visible errors explain a next action. All controls/feedback, including
  the expanded editor's bottom, are scroll- and keyboard-reachable.
- Preserve ADR-0184 package/security/lifecycle rules and ADR-0197 compatibility.

## Acceptance

- Prove absent `name` defaults from manifest despite filename disagreement.
  Valid explicit names override both; old clients/responses stay compatible.
  Invalid/blank names fail without fallback; blank Advanced omits `name`.
- Reject invalid packages before defaults, including blank manifest names.
  Catalog-invalid defaults/overrides and collisions give actionable override
  errors. Missing ZIP fails; never show false import/activation or fallback.
- Catalog choice leaves selection unchanged. Confirmed activation uses the new
  revision on a fresh signed-out author route. Already-selected custom
  publication advances its public revision even if subsequent selection fails.
  Preserve inheritance, bundled recovery without publishing, and operator Site
  default.
- Cover rejection, `CommitIndeterminate`, and lost/undecodable responses after
  dispatch at both stages; prove chain stopping, rereads/errors, truthful
  feedback, no automatic retry and no duplicate submission.
- Observe keyboard disclosure, focus, live status and bottom-control
  reachability at 390×844 and 1440×900 CSS pixels, device scale 1, light mode.
- At both viewports, `/themes` BEFORE shows the existing initial management
  page, imported draft selected, and exposed editor at attempted bottom scroll.
  AFTER shows ZIP-only default, imported-draft choice, and expanded Advanced
  editor. Match standard-profile author, minimal ZIP, data and scope; capture
  BEFORE before presentation mutation. Pair signed-out `/~user` after activation
  at 1440×900; verify private `/themes` remains protected.
- Host-test pure naming/state; prove theme-management flow under matrix policy.

## Boundaries

- Allow server import/shared validated metadata and optional existing `name`.
- Preserve callers/responses; no new parser/API/schema/storage/auth/engine.
- No new protocol/client ZIP parser/duplicate validation/import-rename.
- No discovery/atomicity/snapshots; update docs. Tailwind ZIP unavailable.
