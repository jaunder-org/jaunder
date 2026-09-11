# Issues #1435 and #1436: Button consistency and placement

## Outcome

Profile form actions communicate the same hierarchy, and the full-page composer
keeps its save and publish actions beside the options they affect instead of
pinning them to the bottom of a tall viewport.

## Load-bearing decisions

- Treat each profile card as an independent form. Its sole mutation is its
  primary action.
- Keep **Update Profile** unchanged and render **Save** for Default Post Format
  with `j-btn is-primary`.
- Preserve the full composer’s existing hierarchy: **Save draft** remains
  neutral and **Publish** remains primary.
- Remove the full composer action wrapper’s automatic top margin. The actions
  follow the Media section in the aside’s normal flex flow and inherit its
  existing spacing.
- Apply the placement change only to the full-page new-post composer. Compact
  composition and persisted-post editing retain their current layouts.
- Do not introduce a new layout primitive or global CSS rule for a single call
  site.

## Acceptance

- On `/profile`, **Update Profile** and Default Post Format **Save** have the
  same primary visual treatment.
- Profile button disabled states, validation, dispatch, persistence, and
  feedback remain unchanged.
- On `/posts/new` (the Compose page), **Save draft** and **Publish** appear
  directly after the Media section rather than at the viewport bottom.
- The compose actions retain their neutral/primary distinction, order, disabled
  state, and save/publish behavior.
- After applying a publication time, the resulting **Schedule** or **Publish**
  action remains in the same normal-flow row directly after Media, with its
  label, primary treatment, disabled gate, and dispatch behavior unchanged.
- The placement remains close to the options on both ordinary and tall portrait
  viewports.
- Existing profile and compose end-to-end behavior remains green; actual browser
  inspection confirms both visual outcomes.

## Boundaries

- No server endpoint, storage, domain type, route, copy, or
  accessibility-semantic change.
- No profile-card spacing or footer redesign beyond the requested button
  variant.
- No changes to compact composer, permalink editor, or unrelated settings forms.
- No ADR or `CONTEXT.md` update: this applies established button and flex-layout
  conventions without changing architecture or domain language.
