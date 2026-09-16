# Present Profile fields consistently

## Outcome

The Profile card presents Username with the same labelled field language as
Display Name and Bio while making its immutable status clear through native
read-only behavior. The redundant card description is removed.

## Load-bearing decisions

- Render Username as a labelled, read-only input alongside the editable Profile
  fields.
- Use `readonly`, not `disabled`, so the Username remains focusable, selectable,
  and exposed as a form value without implying that it can be edited.
- Use the existing shared form-field classes so Username follows the same layout
  and visual language as Display Name and Bio.
- Preserve the canonical lowercase Username exactly as returned by the Profile
  data boundary.
- Remove the card description “Your display name and bio.” without replacement.
- Retain the page title “Profile” and page subtitle “Your details.”
- Do not change Profile persistence, validation, update dispatch, or Default
  Post Format behavior.

## Acceptance

- `/profile` presents Username, Display Name, and Bio as consistently labelled
  fields in the Profile card.
- The Username control contains the authenticated User's canonical Username and
  has native read-only semantics.
- A User can focus, select, and copy the Username but cannot edit it.
- The Profile card does not render “Your display name and bio.”
- Display Name and Bio remain editable and continue to persist through Update
  Profile.
- Browser coverage proves the Username presentation and the absence of the
  redundant copy.

## Boundaries

- Username remains immutable; this issue adds no rename capability or storage
  mutation.
- Do not change Username normalization, identity, URL, authentication, or
  protocol behavior.
- Do not redesign other settings cards or form primitives.
