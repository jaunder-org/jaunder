# Spec — #347: manual refresh for the once-fetched subscriber roster

**Issue:** [#347](https://github.com/jaunder-org/jaunder/issues/347)
**Branch/worktree:** `worktree-issue-347-roster-refetch` (branched from
post-#346/#383 `main`)

## Problem

`AudiencesPage`'s subscriber roster is fetched **once** at page load (a
constant-source resource, `web/src/audiences/mod.rs`):

```rust
let subscribers_res = crate::server_resource(|| (), |()| list_my_subscribers());
let subscribers: RosterSignal = Signal::derive(move || subscribers_res.get());
```

If the author gains a subscriber while `/audiences` is open, that subscriber
does not appear in the assignment checklists until a full page reload. The
subscribe happens in **another user's session**, so the author's page has no
client-side signal of it — true liveness would need server push (out of scope).
This gives the author an in-page way to pull a fresh roster without reloading.

## Resolved decisions (design interview)

1. **Manual refresh (option B), not accept-and-reload, not refetch-on-focus.**
   Accept was the lower-effort close but leaves the mid-session case to a full
   reload; refetch-on-focus is new infra with no precedent (no
   `visibilitychange`/focus listener in the web crate) and fires surprise
   refetches. A user-triggered refresh matches the established `Invalidator`
   idiom (the audience list and members already use it).
2. **`Invalidator` + `Invalidator::sticky` — and fix `sticky` to preserve the
   error type.** Convert the roster from a constant-source `server_resource` +
   `Signal::derive` to an `Invalidator`-driven `sticky` resource — the same
   shape #372 gave the members path, and the `subscribers` customer #372
   explicitly anticipated. #372's `sticky` **stringified** the fetch error
   (`result.map_err(|e| e.to_string())`) solely to mirror `patched`'s
   `ListState::Error(String)` (per #372's spec: "Bounds == `patched`'s"). But
   `sticky` returns a raw `Result` to the caller (unlike `patched`'s display
   enum), and the error `E` already satisfies every bound a `Signal` needs — it
   must, to sit in the `Resource<Result<T, E>>` — so the stringify was
   gratuitous and lossy, foreclosing any structured handling. This change makes
   `sticky` **generic over `E`** (`-> Signal<Option<Result<T, E>>>`, dropping
   the `map_err` and the now-unused `Display` bound), preserving the structured
   error. `RosterSignal` therefore **stays `WebError`** (unchanged from #346),
   and the members path (`member_ids`) regains its structured error too.
   Behavior-transparent — the roster node already `format!`s the error, and the
   members arm now stringifies at the render site (`{e.to_string()}`; a Leptos
   `{…}` interpolates `IntoRender`, which `String` has and `WebError` lacks),
   rendering identically. `sticky` is `#[client_only]` (client-only coverage,
   #370) with exactly these two callers.
3. **A refresh icon in the "Your audiences" card head.** A single page-level
   control (the roster is shared across every checklist; `sticky` updates them
   all flash-free). Rendered as an icon (new `Icons::REFRESH` glyph) — not a
   text button — with an accessible label ("Refresh subscribers") since an
   icon-only control needs a name. On click it `notify()`s the roster
   invalidator.
4. **No new loading/spinner UI.** `sticky` keeps the current roster visible
   during the refetch (no flash). A failed refetch resolves `Some(Err)` →
   surfaced by #346's existing page-level error node (consistent with the
   members path); the checklists suppress, per #346. Behavior on a failed
   refresh is therefore already specified by #346 and unchanged.

## Approach (implementation shape — see the plan for detail)

- **`Invalidator::sticky`** (`web/src/reactive.rs`): change the return to
  `Signal<Option<Result<T, E>>>` — drop `.map_err(|e| e.to_string())` and the
  `Display` bound — so it preserves the caller's error type. The roster node
  already `format!`s the error; the `member_ids` arm gains a one-token
  `{e.to_string()}` (a Leptos `{…}` needs `IntoRender`, which `String` has and
  `WebError` lacks). `sticky` is `#[client_only]` (#370).
- **`RosterSignal`** (`web/src/audiences/mod.rs`): **unchanged** from #346 —
  `Signal<Option<WebResult<Vec<SubscriberSummary>>>>` (structured `WebError`),
  which now flows straight from the fixed `sticky`.
- **`Icons`** (`web/src/render/mod.rs`): add
  `pub const REFRESH: &'static str = …;` (a circular-arrow glyph in the existing
  `0 0 20 20` viewBox convention).
- **`AudiencesPage`** (`web/src/audiences/mod.rs`): replace the constant-source
  `subscribers_res` + `Signal::derive` with
  `let roster = Invalidator::new(); let subscribers: RosterSignal = roster.sticky(list_my_subscribers);`.
  `provide_context` is unchanged (still a `RosterSignal`). Add the control to
  the "Your audiences" `j-card-head`: a
  `<button aria-label="Refresh subscribers" on:click=move |_| roster.notify()>`
  wrapping an **inline `<svg>`** — the `Icon` component lives in the
  `target_arch`-gated `pages/ui.rs`, unreachable from this dual-target module,
  so the markup is inlined (byte-identical to `Icon`) and only the
  `Icons::REFRESH` glyph data is imported; the component relocates to `web::ui`
  under #312. The accessible name lives on the **button** (the `<svg>` has
  none), following the `aria-label="Remove tag"` precedent in `pages/ui.rs`.
- **`MemberChecklist`**: the roster gating is unchanged; its **members** error
  arm stringifies at the render site (`{e.to_string()}`) now that `sticky`
  yields a structured `WebError`. The page-level roster error node is unchanged
  (it already `format!`s the error).

## Acceptance criteria

- **AC1 — mid-session refresh works.** With `/audiences` open, **one audience
  created** (so a checklist is mounted) and the roster initially showing "No
  active subscribers yet.", after a second user subscribes to the author and the
  author clicks the refresh control, the new subscriber appears as an "Add"
  candidate in that audience's checklist — **without a page reload**.
- **AC2 — accessible, discoverable control.** The refresh control is a
  `<button>` with the accessible name "Refresh subscribers" (an icon child) in
  the "Your audiences" card head, reachable by role+name
  (`getByRole("button", { name: "Refresh subscribers" })`) rather than a brittle
  DOM-path selector.
- **AC3 — existing behavior preserved.** The initial load, the genuinely-empty
  roster message, and #346's error-surfacing (including a failed _refresh_
  resolving `Some(Err)` → the page-level error node, not an empty roster) are
  unchanged. The audience-list and members refetch behavior (#348/#359/#372) is
  untouched.

**Design guarantee (not independently tested).** _No flash on refresh_: `sticky`
never resets the signal to `None` after the first resolve (it only overwrites
with the next resolved value, `reactive.rs`), so a refresh retains the current
roster until the new one arrives — no blank/loading flash in the checklists.
This is a structural property of `sticky`, not a separately-asserted AC (a
sub-frame transient with no deterministic observation point — the treatment #346
gave its loading-flash guarantee).

## Out of scope

- **Live/push updates** — no server push or polling; refresh is user-triggered
  only.
- **Refetch-on-focus** — considered and rejected (decision 1).
- No change to the audience-list resource or its `patched`/`ListState` handling
  (#348/#359), or to #346's error-handling shape. (The `Invalidator::sticky`
  error-type fix **is** in scope — it also restores the members path's
  structured error, but is behavior-preserving: the members error node renders
  identically.)

## Testing

An e2e in `end2end/tests/audiences.spec.ts`: register author, open `/audiences`,
create one audience ("Friends"), and assert its checklist shows "No active
subscribers yet." (roster fetched empty at load). In a second
`browser.newContext()`, register user X and `subscribeTo` the author. Back on
the author's page, assert X is still **absent** (the once-fetched roster hasn't
updated), click the control via
`getByRole("button", { name: "Refresh subscribers" })`, then assert X appears as
an "Add" candidate in the Friends checklist — no reload. Deterministic (a real
subscribe event), no fault injection. The two-context + `subscribeTo` shape
mirrors the existing CRUD test (`audiences.spec.ts:36-40`).
