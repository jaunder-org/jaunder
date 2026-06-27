# ADR-0027: Scheduled Publishing — Time-Gated Visibility and Restart-Durable Go-Live

* Status: accepted
* Deciders: mdorman, Claude
* Date: 2026-06-27

## Context and Problem Statement

Post visibility was **inconsistent**: most public reads in `storage/src/posts.rs`
gated only on `published_at IS NOT NULL`, while the feed-window query already gated
`published_at <= now`. A future-dated `published_at` therefore leaked onto the post
page, profile, and tag pages immediately, even though the feeds correctly hid it.
This blocked scheduled publishing (set a future publish time) and the Emacs
`#+DATE:` flow that depends on it. Audience-based visibility (who may see a post)
is already settled by [ADR-0020](0020-content-visibility-and-subscription-model.md);
**time-based** visibility (when a post becomes public) was unspecified and applied
unevenly. Implemented for issue #70 (Unit A of the Emacs front-end epic).

## Decision Drivers

* One uniform rule for *when* a post is public — no per-surface drift.
* Future-dated go-live must reach **cached feeds** even across a server restart
  that straddles the publish time (the silent failure mode: live on its permalink
  but never entering the feeds).
* The publish action must express an explicit time, not just a boolean, so a
  schedule (and backdating) round-trips through storage, the web form, and AtomPub.
* Time-visibility is **orthogonal** to ADR-0020 audience-visibility; both gate the
  same reads independently.

## Decision Outcome

**Three post states derived purely from `published_at`:** *draft* (`NULL`),
*scheduled* (`NOT NULL AND > now`), *live* (`NOT NULL AND <= now`).

1. **Uniform time-gate (the invariant).** Every public read gates
   `published_at IS NOT NULL AND published_at <= now`. `now` is threaded as an
   explicit `DateTime<Utc>` parameter (mirroring `list_published_in_window`), never
   read from the clock inside the query — so the boundary is deterministically
   testable. **Any new public read must take `now` and apply this gate.**
2. **Query-time visibility is the source of truth.** On-demand HTML pages flip the
   instant `now >= published_at` with no background job — the gate does it. Only
   **cached feeds** are materialized and need a nudge.
3. **Restart-durable go-live in the feed worker.** A `go_live_pass` runs each tick
   *before* the queue drain: steady state enqueues feed regen for posts whose
   `published_at` lands in the in-memory `(last_tick, now]` window; on the first
   pass after start (`last_tick` unset) a **feed-relative startup catch-up**
   enqueues any cached feed whose surface has a live post newer than that feed's
   `generated_at`, then seeds `last_tick = now`. The tick handles **only**
   future-dated → live transitions; **immediate and backdated publishes enqueue
   their own feed regeneration on the write path**, so the tick never reasons about
   backdating.
4. **Publish is an explicit optional timestamp, not a bool.** Storage update takes
   `PublishUpdate { Unpublish, Publish { at: Option<DateTime<Utc>> } }`
   (`Publish { at: None }` keeps an existing timestamp or stamps `now`). AtomPub
   create/update honor the entry's `<published>`; the web compose form has an
   optional publish-at control (local time → UTC in the browser).
5. **Slug freeze stays at schedule time** — unchanged: once `published_at` is
   non-NULL the slug is final, which is what makes a scheduled post's URL stable.

## Consequences

* Good: one rule for time-visibility; future-dated posts are uniformly hidden until
  due and appear instantly after; the restart-straddle gap is healed.
* Good: backdating and scheduling share one code path; the schedule round-trips over
  the web UI and AtomPub.
* Bad: the `now` parameter is now load-bearing on every public read — a new read
  that forgets it silently reintroduces the leak. Mitigated by the boundary tests.
* Bad: `last_tick` is in-memory, so correctness leans on the startup catch-up; it
  depends on cached feeds carrying `generated_at` (they do).
* Neutral: time-visibility composes with, but is independent of, ADR-0020 audience
  visibility — both predicates gate the same queries.
* Deferred (#15): full scheduled-post management UI (scheduled list, in-place
  reschedule, pull-back-to-draft).
