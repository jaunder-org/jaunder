# Spec — #372: `Invalidator::sticky` — flat sticky-retention helper (error-aware)

**Issue:** [#372](https://github.com/jaunder-org/jaunder/issues/372) **Status:**
proposed **Depends on:** #359 (`Invalidator`) and #348 (`Invalidator::patched` /
`ListState`), both merged. **Also addresses:** #346 (a) — `MemberChecklist`
swallows a `list_audience_members` fetch error into an empty member set. Folded
in per direction: the sticky-coupled member-error surfacing lands here. #346 (b)
— `list_my_subscribers`' `get_user` `Err`/`Ok(None)` swallowed paths +
characterization tests — is **not** part of the sticky API and stays "#346
proper" (a separate agent). This PR therefore _addresses_ but does not `Close`
#346.

## Problem

The sticky-signal retention idiom (web-style-guide §9) — copy a resolved
`Resource` into a signal, updating only when it resolves, so a refetch retains
the last value instead of flashing to "Loading…" — was open-coded at several
sites. #348 already absorbed most of them (`AudiencesPage`'s list → `patched`;
its subscriber roster → a one-line `Signal::derive`). The last open-coded sticky
in the converged audiences vertical is `MemberChecklist::member_ids`:

```rust
let members_res = members.resource(move || list_audience_members(audience_id));
let member_ids = RwSignal::new(None::<Vec<i64>>);
Effect::new(move |_| {
    if let Some(result) = members_res.get() {
        member_ids.set(Some(result.unwrap_or_default()));   // <- #346: error swallowed to empty
    }
});
```

The `unwrap_or_default` is a **bug** (#346): a transient `list_audience_members`
error becomes an _empty_ member set, so every subscriber renders an "Add" button
— silently misrepresenting "nobody is a member." The audience **list** surfaces
its `Err` (renders `<p class="error">`); the members list must be consistent,
not default to empty.

## Approach

Factor it into **`Invalidator::sticky`**, the flat peer of #348's
`Invalidator::patched` — both consume an invalidator-driven resource; `patched`
into a keyed store, `sticky` into a retained flat **result**. Crucially,
`sticky` **surfaces the error** (it does not swallow it — that was the bug):

```rust
// web::reactive, inside the existing `cov:ignore` block in `impl Invalidator`
pub fn sticky<T, Fut, E>(&self, fetch: impl Fn() -> Fut + Send + Sync + 'static)
    -> Signal<Option<Result<T, String>>>
where
    T: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
    E: Clone + Display + Serialize + DeserializeOwned + Send + Sync + 'static,
    Fut: Future<Output = Result<T, E>> + Send + 'static,
```

- `None` until the first resolve; then, on each resolve,
  `Some(result.map_err(|e| e.to_string()))` — `Some(Ok(v))` on success,
  `Some(Err(msg))` on failure. Retained across a _pending_ refetch (the `Effect`
  guard fires only on `Some`, so a mutation-triggered refetch never blanks to
  "Loading…").
- The fetch returns `Result<T, E>`; `self.resource(fetch)` is over the whole
  `Result` (as `patched`), `T` is the unwrapped success value (`member_ids`:
  `T = Vec<i64>`). Bounds == `patched`'s minus the store: `E: Display` (for
  `to_string`) is **back** vs the earlier swallowing draft, `T: Default` is
  **gone** (no `unwrap_or_default`).
- Client-only reactive plumbing (like `resource`/`action`/`patched`):
  `cov:ignore`'d, exercised by the audiences e2e.

`MemberChecklist` becomes a one-liner plus a three-way render that **surfaces
the error**:

```rust
let member_ids = members.sticky(move || list_audience_members(audience_id));
// …
match member_ids.get() {
    None            => "Loading members…",
    Some(Err(e))    => view! { <p class="error">{e}</p> },   // #346 fix
    Some(Ok(ids))   => …checklist…,
}
```

## Acceptance criteria

Observable unless marked _(structural)_.

1. **Primitive exists** _(structural)._ `web::reactive` gains
   `Invalidator::sticky(fetch) -> Signal<Option<Result<T, String>>>`: `None`
   until first resolve, then `Some(Ok(v))` / `Some(Err(msg))`, retained across a
   pending refetch. It sits with `resource`/`action`/`patched` and is
   `cov:ignore`'d (client-only), e2e-exercised.

2. **`MemberChecklist` uses it, and surfaces the error** _(structural, + fixes
   #346a)._ The hand-rolled `members_res` + `RwSignal::new(None)` + `Effect` are
   gone, replaced by one `members.sticky(…)` call and a three-way match: a
   `list_audience_members` error renders `<p class="error">`, **not** an empty
   checklist. No other component or the subscriber roster changes.

3. **Toggle behavior preserved** (regression). The existing audiences e2e still
   passes: a membership toggle re-fetches only that audience's members (one
   `list_audience_members` round-trip), the list is never re-fetched, no
   checklist is left stuck on "Loading members…", and the #348 no-remount
   handles hold. (The success path is unchanged; only the _error_ path changes —
   from empty to a surfaced error — and there is no fault injection for
   `list_audience_members` in the e2e harness, so that path is covered
   structurally by the three-way match, not by a new e2e.)

4. **Style guide** _(structural)._ web-style-guide §9 documents
   `Invalidator::sticky` and the **surface-the-error, never swallow** rule
   (constant-source resources still use `Signal::derive`).

## Non-goals

- **#346 (b) — `list_my_subscribers` server-side error paths + tests** — the
  `get_user` → `Err` / `Ok(None)` swallowed-to-raw-id fallbacks are not part of
  the sticky API; they remain "#346 proper" (a separate agent). This PR does not
  `Close` #346.
- **No `home.rs` / `TimelineState` change** — its paginated
  `resolve`/`fail`/`adopt` state machine is a separate, richer concern, handled
  when that page is converted.
- **No new ADR** — `sticky` is a sibling in the `Invalidator` family already
  recorded by ADR-0060; the style-guide note suffices.
- **No new e2e** — behavior-preserving on the success path; the error path has
  no harness fault injection and is covered by the three-way match.
- **No constant-source helper** — the subscriber roster's `Signal::derive`
  stays.

## Risks / to verify

- **Generic bounds.** Same as `patched`'s minus the keyed store — `E` needs
  `Display` (for `to_string`); `T` drops `Default` (no `unwrap_or_default`).
  Verify on both web targets (host + wasm clippy).
- **Coverage.** `sticky` is `cov:ignore`'d; the `MemberChecklist` three-way
  match adds an `#[component]`-exempt arm — the gate should stay green.
