# #783 — keep authors on a Post after unpublishing

Issue: [#783](https://github.com/jaunder-org/jaunder/issues/783). Milestone:
Correctness & data integrity.

## Outcome

After an author successfully unpublishes a Post from its permalink page, the
application keeps the author on that Post at its returned draft permalink. The
transition remains client-side and immediately renders the Post's draft state.

## Load-bearing decisions

- A commit-confirmed successful unpublish navigates to the canonical permalink
  returned by the mutation rather than to the Drafts list.
- The destination is the Post after the lifecycle transition. Its permalink may
  move from the publication date to the creation date; this is expected.
- When the destination differs from the current URL, navigation replaces the
  current browser-history entry. The obsolete published permalink must not be
  left as the immediately previous entry because it no longer identifies the
  unpublished Post.
- When both permalinks have the same date-based URL, the page refreshes its Post
  resource in place. The author must see draft state rather than stale published
  state even though routing itself has no destination change to observe.
- Only a commit-confirmed success changes navigation or page state. Failed or
  commit-indeterminate unpublish attempts preserve the current page and existing
  error behavior.
- Existing owner-only draft visibility remains authoritative: the resulting
  draft permalink is viewable by its author and masked as not found for others.
- No ADR or glossary change is required. This is a local, reversible UX policy
  using existing Post lifecycle and permalink concepts.

## Acceptance

- Unpublishing a Post whose publication and creation dates differ leaves the
  author on the returned creation-date draft permalink without a full document
  reload.
- The navigation replaces the published permalink in browser history.
- Unpublishing a Post whose publication and creation dates produce the same URL
  leaves the author on that URL and renders the draft banner and draft actions
  without a full document reload.
- The prior Drafts-list redirect is no longer the successful unpublish behavior.
- Existing tests continue to prove that the mutation returns the canonical draft
  permalink and that non-authors cannot view it.

## Boundaries

- Do not change permalink derivation, slug preservation, publication-state
  storage, or the unpublish server contract.
- Do not change draft visibility, the Drafts list, publish navigation, delete
  navigation, or scheduled-Post management beyond the shared unpublish result.
- Do not add redirects from abandoned published permalinks or browser-history
  recovery policy.
