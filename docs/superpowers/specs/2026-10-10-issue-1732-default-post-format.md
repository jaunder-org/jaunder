# Respect Default Post Format in new-Post composers (#1732)

## Outcome

Home’s inline composer and `/posts/new` initialize their authoring format from
the current User’s saved Default Post Format. A new Post is interpreted and
stored using the format the composer shows, not an unrelated Markdown default.

## Load-bearing decisions

- Apply the saved preference when a new composer opens. Markdown, Org, and Html
  are all supported, as defined by `CONTEXT.md`.
- Resolve the preference before enabling submission. Do not present Markdown as
  a resolved selection while the saved preference is still loading.
- If loading fails, show an actionable error with a retry control and keep
  submission unavailable until the preference has been resolved successfully. Do
  not silently convert a failed preference lookup into a Markdown fallback.
- Once initialized, an explicit author-selected format belongs to that
  composition. Async completion or revalidation must not overwrite it.
- Successful submission clears the composition while preserving its selected
  format for successive Posts, consistent with current composer behavior.
- A preference change does not rewrite an already-open composition. Opening a
  fresh composer uses the currently saved preference.
- Editing an existing Post continues to use that Post’s stored authoring format,
  independently of the User’s current preference.

## Acceptance

- Browser regression tests exercise both Home’s inline composer and
  `/posts/new`, proving that a non-Markdown saved preference is selected on
  entry and that submission persists the displayed authoring format.
- Cover all three supported saved preference values across the focused tests;
  Markdown remains valid rather than becoming a special failure case.
- A delayed preference response cannot enable a premature submission or expose a
  falsely resolved Markdown selection. Resolving it enables normal composing and
  submission without discarding any entered content.
- A failed preference request produces the visible retryable error, prevents
  submission, and recovers to the saved format after a successful retry.
- An explicit format change remains selected and governs submission. After a
  successful inline submission, the next composition retains that selection.
- Existing-Post editing retains the stored format even when the saved preference
  differs. Changing the preference affects a newly opened composer, not an
  existing composition.
- Host tests cover any introduced format-initialization state transitions;
  focused browser tests prove the actual async wiring and persisted result.
- Capture comparable before/after views of Home and `/posts/new` for the same
  User with an Org preference at a desktop viewport, plus loading/failure views
  of any added presentation. Keep captures and run evidence in ignored/session
  storage and surface them for review; they are not tracked deliverables.

## Boundaries

No preference schema, storage policy, AtomPub interpretation, existing-Post
format conversion, audience-default policy, or unrelated composer layout
changes. Reuse the existing preference contract. No new architecture decision or
implementation outline is required for this bounded bug fix.
