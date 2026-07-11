# Spec — #346: `AudiencesPage` surfaces a `list_my_subscribers` fetch error instead of an empty roster

**Issue:** [#346](https://github.com/jaunder-org/jaunder/issues/346)
**Branch/worktree:** `worktree-issue-346-roster-error` (rebased onto post-#381
`main`) **Follow-on filed:**
[#383](https://github.com/jaunder-org/jaunder/issues/383) (generalize the
fault-injection pattern to other verticals' error branches) — `blocked-by #346`.

## Problem

In `web/src/audiences/mod.rs`, `AudiencesPage` seeds the shared subscriber
roster from a resource that swallows a fetch error into an **empty** roster:

```rust
let subscribers_res = crate::server_resource(|| (), |()| list_my_subscribers());
let subscribers = Signal::derive(move || {
    subscribers_res.get().and_then(Result::ok).unwrap_or_default() // Err/None -> empty Vec
});
provide_context(subscribers);
```

The roster is provided via context as `Signal<Vec<SubscriberSummary>>` and
consumed inside every `MemberChecklist`. A transient `list_my_subscribers` error
therefore renders as "No active subscribers yet." on every audience row — a
fetch failure masquerading as "you have no subscribers." The
`unwrap_or_default()` also collapses the pre-resolve (`None`) state into empty,
so the same lie flashes during the initial load. The audience-**list** resource,
by contrast, surfaces `Err` as `<p class="error">` via `ListState::Error`; the
roster should be consistent.

## Resolved decisions (design interview)

1. **Surface the roster error once, at page level** (Design A). A shared,
   page-level fetch has one failure; render one error node in the "Your
   audiences" card, mirroring the audience-list `ListState::Error` sibling-node
   idiom — not one copy per audience row. This stays visible even when the
   author has **zero** audiences (no rows ⇒ no checklists to carry a per-row
   error).
2. **Checklists render their subscriber UI only when the roster genuinely
   resolved.** "Not yet resolved" (loading) and "errored" both suppress the
   checklist's subscriber section, so the misleading "No active subscribers
   yet." never shows for either. Fixing the loading→empty flash falls out of
   this by necessity; no loading UI is _added_ (the audiences card already shows
   its own "Loading…" for the list — a second one would be redundant). The
   visible page-level node is **error-only**.
3. **A genuine empty roster is unchanged.** When `list_my_subscribers` resolves
   with zero subscribers, each checklist still shows "No active subscribers
   yet." — that is correct and must be preserved as distinct from the
   error/loading suppression.
4. **Automate the error path via Playwright route interception** (Option 2). No
   fault-injection harness exists today (verified: no
   `page.route`/`fulfill`/`abort` anywhere in `end2end/`). Introduce it here —
   intercept the `POST` to `list_my_subscribers` (match by URL
   **substring/regex**, per the suite's `url.includes` convention, not an exact
   glob) and `fulfill({ status: 500, body: "boom" })` so the client `Resource`
   resolves `Err`. This works because the app is **CSR** (`mount_to_body` +
   `leptos/csr` in `web/src/lib.rs` / `web/Cargo.toml`), so the roster fetch is
   a real client-side `POST` the route can intercept — under SSR/hydrate it
   would resolve server-side and interception would be a silent no-op. File #383
   to generalize the pattern across the other untested fetch-error branches.

## Approach (implementation shape — see the plan for task breakdown)

The roster context carries the **full resolved state**,
`Signal<Option<WebResult<Vec<SubscriberSummary>>>>` — mirroring the shape #381
just landed for the members path
(`member_ids: Signal<Option<WebResult<Vec<i64>>>>`), so the checklist matches
both async inputs with the identical `None / Some(Err) / Some(Ok)` idiom, and
one signal is the single source of truth for both the page-level error node and
the per-checklist suppression. (A collapsed `Option<Vec>` was considered and
rejected: it discards the error at the context boundary, forcing a redundant
second derivation for the page node and breaking the symmetry with
`member_ids`.)

- **`AudiencesPage` roster context**: drop
  `.and_then(Result::ok).unwrap_or_default()` for a **pass-through**
  `Signal::derive(move || subscribers_res.get())`, typed
  `Signal<Option<WebResult<Vec<SubscriberSummary>>>>`: `None` while loading,
  `Some(Err(e))` on error, `Some(Ok(vec))` once resolved. It stays a
  constant-source `Signal::derive` (not a sticky `RwSignal`, not an
  `Invalidator` — clear of #347's and #372's scope). `WebError` is already
  `Clone` (it lives in `sticky`'s signal), so the signal is viable.
- **Page-level error node**: in `AudiencesPage`, read the same roster signal and
  render on `Some(Err(e))` a sibling `<p class="error">` in the "Your audiences"
  card. The text carries a **static, subscribers-specific prefix** — e.g.
  `"Couldn't load your subscribers: " {e}` — so it is distinguishable from the
  sibling audience-list and create-form `p.error` nodes, which share that class.
  Silent on loading / empty / loaded.
- **`MemberChecklist`**: consume
  `Signal<Option<WebResult<Vec<SubscriberSummary>>>>`. Post-#381 the checklist
  matches `member_ids` three ways — `None` ⇒ "Loading members…", `Some(Err)` ⇒
  the **members** error node, `Some(Ok(ids))` ⇒ the roster branch. The roster
  gating nests **inside the `Some(Ok(member_ids))` arm**, replacing today's
  `if subscribers.is_empty()` (`mod.rs:411`), matching the roster with the same
  idiom: `None | Some(Err(_))` ⇒ render nothing (loading or error — the error is
  shown once at page level); `Some(Ok(subs))` empty ⇒ "No active subscribers
  yet."; `Some(Ok(subs))` non-empty ⇒ the add/remove list.

**Coordination:** #381 (issue #372) **merged 2026-07-11**; it landed
`Invalidator::sticky` (which retains the members `Result`, so the _members_
error already renders) and the three-way `member_ids` match. This branch is
**rebased onto post-#381 `main`** and targets that match. The roster fix is
independent of the sticky helper and touches only the roster half of the same
function.

## Acceptance criteria

- **AC1 — error surfaces once.** When `list_my_subscribers` resolves `Err`, the
  Audiences page shows a `p.error` node whose text contains "Couldn't load your
  subscribers" (pinned substring — `p.error` alone is shared with the
  list/create errors), in the "Your audiences" card, and it appears even when
  the author has **zero** audiences.
- **AC2 — no empty-lie on error.** When `list_my_subscribers` resolves `Err`,
  **no** `MemberChecklist` renders "No active subscribers yet." and none renders
  an add/remove list.
- **AC3 — genuine empty preserved.** When `list_my_subscribers` resolves with
  zero subscribers, each `MemberChecklist` renders "No active subscribers yet."
  and there is **no** roster `p.error` node.
- **AC4 — happy path preserved.** When `list_my_subscribers` resolves with ≥1
  subscriber, each `MemberChecklist` renders the add/remove list (existing
  behavior).
- **AC5 — automated error test.** A Playwright test intercepts the
  `list_my_subscribers` POST (substring/regex match) and forces
  `{ status: 500, body: "boom" }`, asserting AC1 (pinned substring) and AC2; the
  existing positive-path coverage (AC3/AC4) is retained.

**Design guarantee (not independently tested).** _No loading flash_: the
checklist renders its subscriber section only on `Some(Ok(_))` — `None`
(loading) and `Some(Err)` (error) both render nothing — so no checklist shows
"No active subscribers yet." before the roster first resolves. AC2 exercises the
identical suppression on the error arm; the loading arm is a transient
pre-first-resolve state with no deterministic observation point, so it is a
guarantee of decision 2 rather than a separately-asserted AC.

## Out of scope

- The **members** swallow (`list_audience_members` / `member_ids`) — landed by
  #372/#381.
- **Other components'** fetch-error branches (audience list, posts, profile, …)
  — #383.
- Making the roster **refetchable** mid-session (invalidator-driven) — #347.
  This change keeps it a constant-source resource; it only stops swallowing the
  error.

## Note for planning

The route-interception fault-injection pattern is **unproven in this repo**
(nothing in `end2end/` has done it). The plan's implementation should
**spike-verify** that a single forced 500 (with a body) drives the client
server-fn `Resource` to `Err` and renders the error branch, before building
AC1/AC5 assertions on top of it.
