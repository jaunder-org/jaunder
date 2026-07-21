# Spec — #329: converge the `timeline` vertical onto the file-level host/wasm split

**Status:** awaiting approval. **Parent:** #303 (umbrella). **Decision
records:** `docs/adr/0070-web-vertical-wasm-only-component-files.md` (supersedes
ADR-0056), with `docs/web-style-guide.md` §8 as the layout template;
`docs/adr/0055-*` (extraction discipline / no host stubs) carried forward.

## Problem

`web/src/pages/timeline.rs` (151 lines) is the **shared reactive
paginated-timeline machinery** (#181) — not a routed page. It is consumed by two
pages, `pages/home.rs` (`/` public Local timeline) and `pages/cockpit.rs`
(`/app` authed Feed). It defines:

- `pub(crate) struct TimelineState` — a `#[derive(Clone, Copy)]` bundle of six
  `RwSignal`s: `rows`, **two** cursor signals
  (`next_cursor_created_at: RwSignal<Option<UtcInstant>>`,
  `next_cursor_post_id: RwSignal<Option<PostId>>`), `has_more`, `loading_more`,
  and `error: RwSignal<Option<String>>`, with `adopt`/`resolve`/`fail`
  transition methods (`Default`/`adopt` wrapped in `// cov:ignore`).
- `pub(crate) fn spawn_load_more<F, Fut>(state, fetch)` — wasm-only generic
  paginator (`leptos::task::spawn_local`).
- `#[component] pub(crate) fn TimelineRows(...)` — the shared scroll region:
  post list (or the "No posts yet." placeholder) + "Load more" button, rendering
  `crate::posts::PostCard`.

The module defines **no `#[server]` fns and no wire types of its own** — it
re-uses `crate::posts::{PostCard, TimelinePage, TimelinePostSummary}` and is
generic over the caller's list fn (`list_local_timeline` / `list_home_feed`). It
lives in `pages/` (technology-grouped), the layout ADR-0070 §5 dissolves; it is
explicitly named there as a vertical needing a new dir.

Two type-modeling weaknesses in `TimelineState` also surface on the move:

- The keyset cursor is stored as **two independent signals** that must always be
  set, read, and cleared as a pair — they can silently drift. A keyset cursor is
  one logical value: `(created_at, post_id)` together.
- Load state is `loading_more: bool` **plus** `error: Option<String>` — the pair
  admits the illegal `loading_more == true && error == Some(_)` state.

## Decisions (interview-resolved)

1. **New `web/src/timeline/` vertical (ADR-0070 §5).** The machinery becomes a
   sibling vertical — not folded into `posts/` — because both future consumers,
   #317 (cockpit) and #319 (home), import it as a shared sibling; burying it in
   the already-large `posts/` would contradict the ADR-0070 per-vertical dir
   list. It is a **degenerate** vertical: it has no `#[server]` fns and no wire
   types, so there is **no `api.rs` and no `server.rs`** — matching the issue's
   "composite, no 1:1 `#[server]` module" note. This is the first
   **server-less** vertical (no wire surface at all); ADR-0070 §1 lists `api.rs`
   but `web-style-guide.md` §8 only marks `server.rs`/`component.rs` omittable —
   so this cycle adds a one-line §8 note that `api.rs` is likewise omitted when
   a vertical has no server surface, keeping the layout doc in sync.

2. **Three files: `mod.rs` (wiring) / `state.rs` (ungated, host-tested) /
   `component.rs` (wasm-only).**
   - `mod.rs` — module wiring only (ADR-0070 §1): `mod state;` (ungated) +
     `#[cfg(target_arch = "wasm32")] mod component;` + the re-exports. No items
     of its own.
   - `state.rs` — the **pure, host-compiled, host-tested** value types: the
     `TimelineCursor` newtype (its `from_page` pairing + `into_query` split —
     the genuinely error-prone logic) and the `LoadStatus` enum (its accessors).
     The row operations themselves stay trivial reactive-layer calls in
     `component.rs` (`rows.set(page.posts)` on a seed/re-fetch,
     `rows.update(|r| r.extend(page.posts))` on load-more) — a pure
     `apply_rows`/`PageMode` abstraction over `Vec` replace-vs-append was tried
     and dropped as a Middle Man (`Replace` just returns its input). Cfg-free;
     coverage-measured; real `#[cfg(test)]` unit tests replace the current
     `cov:ignore` markers.
   - `component.rs` — the wasm-only reactive layer: `TimelineState` (the signal
     bundle, now holding the pure types), `spawn_load_more`, and the
     `#[component] TimelineRows`. Declared
     `#[cfg(target_arch = "wasm32")] mod component;`; **zero cfg gates inside
     the file** (ADR-0070 §1/§2); free to call `leptos::task::spawn_local` /
     browser code directly.

3. **`TimelineCursor` newtype — bundle the two cursor signals.** Introduce
   `pub struct TimelineCursor { pub created_at: UtcInstant, pub post_id: PostId }`
   in `state.rs` — `pub` fields so the wasm-only paginator in `component.rs` can
   destructure it back into the `(Option<UtcInstant>, Option<PostId>)` the posts
   list fns take. (The pure `state.rs` items are `pub` and re-exported from
   `mod.rs`, not `pub(crate)`: their only host consumer is `#[cfg(test)]` — the
   `component.rs` consumer is wasm-only — so `pub(crate)` would fire host
   `dead_code`; a re-exported `pub` item is crate-public API and escapes it.
   This is the `posts/mod.rs` precedent, ADR-0070 §6.) `TimelineState` stores
   the cursor as **one** `RwSignal<Option<TimelineCursor>>` in place of the two
   `Option` signals, making "one set, the other not" unrepresentable. The cursor
   is built from the posts `TimelinePage`'s flat fields
   (`next_cursor_created_at` / `next_cursor_post_id`,
   `web/src/posts/api/listing.rs`) at the page-application boundary — it stays
   **timeline-local**; the posts wire DTO is unchanged (a wire-level
   `Option<TimelineCursor>` on `TimelinePage` is flagged to #569, not done
   here). **`has_more` is retained as its own `RwSignal<bool>`, sourced from the
   page — not collapsed into `cursor.is_some()`.** The server can emit
   `has_more == true` with an absent cursor in the degenerate `page_size == 0`
   case (`listing.rs` `page_from_rows`), so `has_more` is not a pure function of
   cursor presence and keeping it avoids that trap. `fail()` continues to set
   `has_more = false` **and** now clears the cursor (`None`) so the two stay
   consistent on failure (the current `fail()` leaves the cursor signals set).

4. **`LoadStatus` enum — replace `loading_more` + `error`.** Introduce
   `pub enum LoadStatus { Idle, InFlight, Failed(String) }` in `state.rs` with
   pure accessors `is_in_flight(&self) -> bool` and
   `into_failure(self) -> Option<String>` (owned, so the reactive callers —
   which hold a cloned `LoadStatus` from `read_signal!` — can return the
   `String` directly rather than re-matching the `Failed` arm inline).
   Transitions are plain variant assignment (`LoadStatus::InFlight` / `Idle` /
   `Failed(msg)`) at the wasm call sites — no dedicated transition methods — and
   host `#[cfg(test)]` constructs **every arm directly** and asserts the
   accessors over it, so each state (including `InFlight`) is reachable and
   assertable on the host, not only from inside the wasm-only paginator.
   `TimelineState` stores one `RwSignal<LoadStatus>` in place of the
   `loading_more: bool` + `error: Option<String>` pair. The `String` payload
   stays a user-facing display message (deliberate `String`, not a domain type).
   **This is a real rewire of the two consumers, not an import swap:** the error
   is displayed by the _callers_, not `TimelineRows` — `home.rs` and
   `cockpit.rs` each build `read_error = move || read_signal!(state.error)` and
   emit `<p class="error">{…}</p>`; those blocks call `.into_failure()` instead.
   And `TimelineRows` itself reads `loading_more` (button label + disabled
   state), so its reads switch to `read_signal!(state.status).is_in_flight()`.

5. **Extraction discipline (ADR-0070 §6 / ADR-0055).** The pure types and
   transition logic in `state.rs` are host-tested and coverage-measured; the
   wasm-only `component.rs` leaves the host coverage denominator entirely (not
   dead-but-exempt). **No new `cov:ignore` and no `#[component]`-exemption
   reliance** is added to keep the reactive code green; no fake host stub is
   introduced.

6. **Rewire the consumers + module decls.** `home.rs` and `cockpit.rs` switch
   their imports from `crate::pages::timeline::{…}` to `crate::timeline::{…}`,
   backed by a gated re-export in `timeline/mod.rs`
   (`#[cfg(target_arch = "wasm32")] pub use component::{TimelineRows, TimelineState, spawn_load_more};`),
   **and** update their `read_error` closures to call `.into_failure()` on the
   status (Decision 4) rather than the deleted `state.error` signal.
   `pub mod timeline;` is added to `web/src/lib.rs` and the
   `pub(crate) mod timeline;` is **removed** from `web/src/pages/mod.rs`. No
   `<Route>` line changes (timeline is unrouted); `HomePage`/`CockpitPage`
   registrations are untouched.

7. **Deferred couplings left intact.** `TimelineRows` keeps importing
   `crate::pages::signal_read::read_signal` (the inlining of that macro is #304)
   and keeps the `crate::posts::{PostCard, TimelinePage, TimelinePostSummary}`
   dependency (no posts↔timeline cycle: posts does not import timeline). The
   pure `web::render` projector's `SiteTimeline` first-paint path is **not**
   part of this vertical (it is the shared host-side projector, already ungated
   and host-tested); this issue does not move it.

## Target end state (acceptance floor — observable)

1. `web/src/pages/timeline.rs` **no longer exists**; its
   `pub(crate) mod timeline;` at `web/src/pages/mod.rs` is deleted;
   `web/src/lib.rs` declares `pub mod timeline;` (matching the sibling
   verticals; `pub` so the re-exported pure items stay reachable on the host
   build).
2. `web/src/timeline/` contains exactly `mod.rs`, `state.rs`, `component.rs` (no
   `api.rs`, no `server.rs`). `mod.rs` has **no items of its own** beyond `mod`
   declarations and re-exports.
3. `TimelineState` holds **one** `RwSignal<Option<TimelineCursor>>` (not two
   cursor signals), a retained `has_more: RwSignal<bool>`, and **one**
   `RwSignal<LoadStatus>` (not `loading_more: bool` + `error: Option<String>`).
   `TimelineCursor` and `LoadStatus` are defined in `web/src/timeline/state.rs`.
4. `web/src/timeline/state.rs` is **ungated** (host-compiles) and carries
   `#[cfg(test)]` unit tests exercising: `TimelineCursor::from_page` (the
   both-or-neither pairing) and `TimelineCursor::into_query` (the split back to
   `(Option<UtcInstant>, Option<PostId>)`); and each `LoadStatus` arm
   (`Idle`/`InFlight`/`Failed`) constructed and asserted on the host via
   `is_in_flight` / `into_failure`. It introduces **no `cov:ignore`** for that
   logic.
5. `target_arch = "wasm32"` appears in the vertical **only on `mod` declarations
   and their paired `pub use`** (ADR-0070 §2), never on an item inside
   `state.rs`, `component.rs`, or `mod.rs`. `component.rs` contains **zero cfg
   attributes**.
6. `home.rs` and `cockpit.rs` import the machinery via `crate::timeline::…`; no
   `crate::pages::timeline::…` path remains anywhere in the tree.
7. Each of `timeline/mod.rs`, `state.rs`, `component.rs` opens with a `//!` doc
   naming its ADR-0070 role (wiring / pure host-tested types / wasm-only UI).
8. `cargo xtask validate` green, **including `wasm-clippy`** (load-bearing gate
   surface for the now-wasm-only UI, ADR-0070 §Consequences) and the
   timeline-exercising e2e flows.

## Shape of the work

- Create `web/src/timeline/{mod.rs, state.rs, component.rs}`; add
  `pub mod timeline;` to `web/src/lib.rs`.
- **`state.rs` first (test-first where practical):** define `TimelineCursor` and
  `LoadStatus`; extract the page-application logic from `adopt`/`resolve`/`fail`
  into pure, host-tested functions/methods; add unit tests. Resolve the
  `has_more`-vs-`cursor.is_some()` question here.
- **`component.rs`:** move `TimelineState` (re-typed onto the new signals),
  `spawn_load_more`, and `TimelineRows` essentially verbatim, delegating their
  transition bodies to `state.rs`; drop the `cov:ignore` markers made
  unnecessary by the wasm gate.
- **`mod.rs`:** wiring + the gated re-export block.
- **Rewire:** repoint `home.rs`/`cockpit.rs` imports and the `spawn_load_more`
  call sites, and rewrite their `read_error` closures onto `LoadStatus::Failed`;
  delete `pages/timeline.rs` and its `pages/mod.rs` decl.
- Update `docs/web-style-guide.md` §8 to note `api.rs` is omitted for a
  server-less vertical (this is the first such case).
- Run `cargo xtask check` while iterating; `wasm-clippy` after the gate flip;
  `cargo xtask validate --no-e2e` locally and let CI's e2e matrix gate the flows
  (local e2e VM is reaped here).

## Out of scope

- Moving `App`/Router out of `pages/mod.rs` — that is **#330**.
- Dissolving `pages/ui.rs` / `web::render` — that is **#312**; this issue only
  stops timeline from _living in_ `pages/`, it does not remove `pages/` itself.
- Inlining the `read_signal!` macro — that is **#304**; timeline keeps importing
  it from `crate::pages::signal_read`.
- Any wire-level cursor newtype on the posts `TimelinePage` DTO — flagged to
  **#569**; this issue keeps `TimelineCursor` timeline-local.
- Converging the `home`/`cockpit` verticals themselves (#319 / #317) — this
  issue touches `home.rs`/`cockpit.rs` only as far as the move forces (import
  paths + the `LoadStatus` error-read rewire), not a full vertical convergence.

## Verification

`cargo xtask validate` (static + `wasm-clippy` + coverage + e2e). The
load-bearing behavioral checks are the timeline pagination + first-paint flows
already covered by `end2end/tests/{posts,authed-flash,feeds,visibility}.spec.ts`
(local `/` Local timeline, `/app` Feed, "Load more" pagination, owner pre-paint)
— each must stay green through the move. Because the UI is now wasm-only,
`wasm-clippy` (`-p web -p client`, host + Nix mirror) is load-bearing type-check
surface for this vertical, not just host clippy. The new `state.rs` unit tests
are the host-side proof that the cursor/status modeling is correct.
