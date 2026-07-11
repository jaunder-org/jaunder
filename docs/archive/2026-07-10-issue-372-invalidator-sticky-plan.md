# Plan — #372: `Invalidator::sticky` (error-aware)

**Spec:** `docs/superpowers/specs/2026-07-10-issue-372-invalidator-sticky.md`
**Branch/worktree:** `worktree-issue-372-invalidator-sticky`

> **Rework (post-review):** #346 flagged that `MemberChecklist` swallows a
> `list_audience_members` error into an empty member set. The first cut of
> `sticky` (`Signal<Option<T>>`, `unwrap_or_default`) **enshrined that bug**.
> Per direction, folded the #346(a) fix in: `sticky` is now **error-aware**
> (`Signal<Option<Result<T, String>>>`, surfaces the error), and
> `MemberChecklist` renders it three-way. #346(b) (`list_my_subscribers`) stays
> separate.

No ADR (sibling in the `Invalidator` family under ADR-0060). Commit grouping
(amended into the existing 3 commits): A primitive / B convert + style-guide / C
archive spec+plan.

## Task 1 — Add error-aware `Invalidator::sticky` to `web::reactive`

In `web/src/reactive.rs`, inside the existing `cov:ignore` block in
`impl Invalidator` (after `patched`):

```rust
pub fn sticky<T, Fut, E>(&self, fetch: impl Fn() -> Fut + Send + Sync + 'static)
    -> Signal<Option<Result<T, String>>>
where
    T: Clone + serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    E: Clone + std::fmt::Display + serde::Serialize + serde::de::DeserializeOwned + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<T, E>> + Send + 'static,
{
    let resource = self.resource(fetch);
    let signal = RwSignal::new(None::<Result<T, String>>);
    Effect::new(move |_| {
        if let Some(result) = resource.get() {
            signal.set(Some(result.map_err(|e| e.to_string())));   // surface, never swallow
        }
    });
    signal.into()
}
```

Update the module doc to reflect the error-surfacing behavior.

**Verify:** host + wasm clippy pass (the `E: Display` / no-`Default` bounds line
up).

_Spec: AC1._

## Task 2 — Convert `MemberChecklist` + surface the error (#346a)

In `web/src/audiences/mod.rs`, replace the `members_res` +
`RwSignal::new(None)` + `Effect` with
`let member_ids = members.sticky(move || list_audience_members(audience_id));`,
and match it three-way so a fetch error renders `<p class="error">` instead of
an empty checklist:

```rust
match member_ids.get() {
    None            => "Loading members…",
    Some(Err(e))    => view! { <p class="error">{e}</p> },   // #346
    Some(Ok(ids))   => …checklist…,
}
```

**Verify:** host + wasm clippy pass; `members_res` gone; the error arm renders
`p.error`.

_Spec: AC2._

## Task 3 — web-style-guide §9 note

`Invalidator::sticky(fetch) -> Signal<Option<Result<T, String>>>` for the
invalidator-driven sticky case; **surface the `Err`, never swallow to a
default** (#346); constant-source stays `Signal::derive`.

_Spec: AC4._

## Task 4 — Gate green + clean history

- `cargo xtask check` (fixes formatting + fast checks), then
  `cargo xtask validate` green (via `devtool run --`) — existing audiences
  nix-e2e as the regression guard.
- Amend the changes into the existing 3 commits (fixup/autosquash); force-push.
  PR body: addresses #346(a), does **not** `Close` #346 ((b) is separate).

**Verify:** validate exits 0.
