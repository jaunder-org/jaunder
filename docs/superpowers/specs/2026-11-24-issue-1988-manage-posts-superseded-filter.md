# Issue #1988 — Manage Posts discards superseded list results

## Outcome

On Manage Posts (`/posts/manage`), only the response to the most recent list
request may change what the page shows. When a User applies filter A and then
filter B before A's response arrives, A's response is discarded whenever it
arrives. Once B settles, the list, the "Loading…" indicator and the error slot
describe filter B, and A's late arrival never changes them. "Loading…" stays up
until B itself settles.

## Approach and tradeoffs

Today one effect starts a `list_managed_posts` request on every change to the
state filter, audience filter, applied search, cursor, or reload counter, and
writes whichever response **arrives last**. Two overlapping requests therefore
race: if A resolves after B, A's rows overwrite B's and A's completion also
clears "Loading…" while B is still outstanding.

The fix gives each list request a generation number:

| Step                | What happens                                                                                        |
| ------------------- | --------------------------------------------------------------------------------------------------- |
| request starts      | the page state advances its list generation, sets loading, clears the error, and hands back a claim |
| response arrives    | the page state compares the claim with the current generation                                       |
| claim is current    | the outcome is written and loading clears — exactly today's behaviour                               |
| claim is superseded | the outcome is dropped: no rows, no error, no loading change                                        |

"Outcome" covers all three completions of one load: the list result or its
error, "Sign in to manage Posts." when the session has no User, and a session
reconcile error. The generation is advanced and read untracked, so the load
effect does not subscribe to it.

The begin/finish transitions live on the existing host-tested Manage Posts page
state, so the latest-wins rule is unit-tested without a browser; the component
keeps only the effect and the `spawn_local` dispatch. This mirrors the
generation claims the drafts list and the theme/navigation code already use.

Superseded requests are not cancelled on the network; they complete and are
ignored. Cancellation would save server work but adds abort plumbing for a
bounded, read-only query — not worth it.

Simplest credible alternative: a Leptos `LocalResource` keyed on the filters. It
does not solve this. In reactive_graph 0.2.14 an async derivation runs its
fetches one at a time: a filter change waits for the in-flight fetch to finish
(and paints its result) before starting the next. So applying B while A is slow
would delay B and briefly show A's rows. Its version check only guards against
hand-set values, not rival fetches. The generation claim instead lets B start
immediately and drops A whenever it arrives.

## Acceptance

- Host unit test on the Manage Posts page state: a completion whose claim was
  superseded by a later begin changes neither rows, error nor loading, for a
  list result, a list error and a sign-in/reconcile outcome alike; the current
  claim's success sets rows and clears loading; its failure sets the error and
  clears loading.
- New deterministic browser test in `end2end/tests/manage-posts.spec.ts`, after
  the initial unfiltered load has finished:
  1. Hold, via `page.route` on `**/api/posts/list_managed_posts`, only the
     request whose JSON body `search` is the first term (a term that matches the
     seeded Posts).
  2. Apply a second search that matches nothing; wait for "No Posts match these
     filters.".
  3. Release the held request and wait for its `requestfinished` event.
  4. Click "Select all matching" and wait for its response and for the button to
     re-enable. That round trip was sent after the held response reached the
     browser, so the browser has processed the held response by then. It is not
     a list request, so it cannot repaint rows and hide a stale write.
  5. Assert the search box reads the second term, zero rows are shown, and "No
     Posts match these filters." is visible.

  The test must fail against the current code.

- The existing "selects across pages and applies atomic bulk operations" test
  keeps passing unchanged.
- Proof runs: `dev test-local` for the unit test,
  `dev e2e-local manage-posts.spec.ts` for both browser tests.

## Boundaries

- Only the Manage Posts list load changes. "Select all matching", the
  confirmation snapshot and bulk execution keep their current behaviour (they
  already disable conflicting controls while pending).
- No server, storage or API change; no request cancellation.
- While B is in flight the previous rows stay on screen and "Next page" stays
  enabled. Clicking it pairs B's filters with the old page's cursor. That is
  existing behaviour, excluded here and filed as #2093.
- No change to the existing browser test's steps.
