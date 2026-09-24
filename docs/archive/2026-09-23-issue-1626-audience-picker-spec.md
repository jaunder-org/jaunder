# Issue #1626 — One audience control with lossless target selection

## Outcome

The Post composer presents one compact Audience control instead of a disclosure
containing a second dropdown. An author can select any union of Public,
Subscribers, and their Named audiences; saving and reopening a Post preserves
every checked target, including targets currently dominated by Public.

## Load-bearing decisions

- The open control offers independent checkboxes for Public, Subscribers, and
  each available Named audience. Neither Public nor Subscribers replaces another
  checked choice. A selected Public target does not erase narrower selections.
- Private is the empty selection, not a checkbox that can coexist with other
  targets. With none checked, the closed control reads **Private**, and the open
  control explains that only the author can see the Post. A visible **Clear
  all** action deliberately returns to Private; no choice is silently changed
  when the author checks or unchecks a different target.
- The closed control names the widest selected scope: **Public** if checked,
  otherwise **Subscribers** if checked, otherwise **1 audience** or **<n>
  audiences** for Named-only selections, otherwise **Private**. These are
  summaries, not transformations: opening the control reveals every checked
  target, including narrower ones dominated by Public. The trigger remains
  compact on small screens; the open panel labels each target and its checked
  state accessibly.
- The full selected set is the editing contract across create, update, load, and
  reopen. If the current web request/response model or underlying persistence
  cannot round-trip Public + Subscribers + Named, change that boundary rather
  than approximating it in the UI. Private persists under the existing
  empty-target semantics; no Private-plus-other representation is introduced
  (ADR-0020, ADR-0207).
- Existing site Default Audience initializes new Posts, including Private; an
  existing Post opens with its complete saved set. A pending or failed Default
  Audience or current-Post audience load must not make a placeholder selection
  saveable; failure surfaces an error and prevents the write. A late default
  response must not overwrite the author's intervening choice. An unresolved or
  failed Named-audience load must not be mistaken for an empty choice or allow a
  write that silently loses named targets.

## Acceptance

- On `/posts/new`, the inline Home composer at `/app`, and the existing-Post
  editor, there is one Audience trigger and no nested base select. The closed
  summary follows Private → Named-only count → Subscribers → Public precedence
  for the selected set, with singular/plural Named count, while opening it
  reveals every selected target even where the summary names only the widest
  scope.
- The author can check Public, Subscribers, and multiple Named audiences
  together, save, reopen, and find every one still checked. Removing Public
  leaves the previously checked narrower targets intact; checking it again does
  not alter them. These combinations are proven through the running application,
  not just presentation state.
- Clear all produces Private; save and reopen still show Private. Moving from
  Private to a Named-only selection does not require selecting a built-in
  audience first. Existing Posts with a Named-only or multi-target set open and
  save without dropping a target.
- Defaults are respected: pending or failed Default Audience/current-Post
  selection retrieval cannot submit a placeholder Public or Private choice, and
  a delayed default does not overwrite an author's intervening selection.
  Loading or failed Named-audience retrieval never causes an accidental Private
  write or loss of an existing selection. Authorization, visibility, and
  Public-only syndication behavior remain unchanged for the resulting target
  set.
- Keyboard and assistive-technology users can open the control, identify and
  toggle each target, use Clear all, and understand the Private empty state. The
  control's state is conveyed without relying only on color.
- Comparable before/after screenshots at desktop (1280×800) and narrow (390×844)
  viewports show `/posts/new` with the control closed and open, including a
  multi-target selection. Include a Private empty-state pair and the inline Home
  composer where the compact layout differs.
- Focused host tests cover target-set conversion, round-trip, and late-load
  state transitions. Both-backend HTTP integration tests cover create, update,
  and audience retrieval through the changed web contract for Public +
  Subscribers + multiple Named targets and empty/Private. Playwright coverage
  exercises save/reopen, load failures, and visible controls on the real browser
  surface.

## Boundaries

- No change to who a target admits, Named-audience membership, Default Audience
  policy, AtomPub's accepted target sets, or storage representation unless
  needed to make the web editing contract lossless.
- No new audience type, no Private-plus-other combination, and no redesign of
  the separate Audiences management page.
