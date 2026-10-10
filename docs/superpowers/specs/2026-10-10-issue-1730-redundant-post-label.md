# Remove redundant Post wording in revision history (#1730)

## Outcome

Post Revision history tables show only the linked numeric Post ID in the “Post”
column, without repeating “Post” in every row. The column heading continues to
identify what those numbers mean.

## Load-bearing decisions

- Apply the change to both the owner-wide History page (`/history`) and an
  individual Post History page (`/posts/{post_id}/history`).
- Keep the “Post” column heading unchanged.
- Each row's Post link shows the full numeric ID without a “Post” prefix.
- Preserve each link's destination: the corresponding Post History page.
- Preserve snapshot titles, slugs, timestamps, state labels, Inspect actions,
  row order, pagination, and authorization behavior.
- This is a presentation correction, not a change to Post identity or the
  owner-only Post Revision policy in ADR-0136.

## Acceptance

- Populated tables on both routes retain the “Post” heading and display
  numeric-only Post links for their revision rows.
- A Post link still opens the correct Post History page through in-app
  navigation, including history for a Deleted Post.
- Focused browser regression coverage verifies the visible link text and
  destination; run the history spec through the focused local e2e lane.
- Capture comparable Before/After screenshots of populated tables on both routes
  before and after presentation changes. Use the same seeded owner, revision
  data, application styling, and desktop viewport (1280 × 800).
- Screenshot pairs remain in transient run/session storage and are presented
  together for review; they are not committed visual snapshot baselines.

## Boundaries

- Do not remove “Post” from unrelated UI wording or rename domain concepts.
- Do not change spacing, table layout, other columns, or responsive styling.
- No API, storage, schema, authorization, or navigation-policy changes.
- No new ADR or glossary entry is needed for this reversible label change.
- Commit the spec while work is in progress; retire it after conformance review
  according to the repository's artifact lifecycle.
