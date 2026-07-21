# Timeline Vertical Convergence (#329) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Spec:** `docs/superpowers/specs/2026-07-21-issue-329-timeline-vertical.md` —
the "what/why." This plan is the "how"; it references the spec by section rather
than restating it.

**Goal:** Move the shared timeline pagination machinery out of
`web/src/pages/timeline.rs` into a new server-less `web/src/timeline/` vertical
(ADR-0070), extracting its pure value model into an ungated host-tested
`state.rs` — introducing a `TimelineCursor` newtype and a `LoadStatus` enum that
make the old drift-prone signal bundle's illegal states unrepresentable.

**Architecture:** Three files — `mod.rs` (wiring), `state.rs` (ungated,
host-tested pure types + fold logic), `component.rs` (wasm-only reactive
`TimelineState`, `spawn_load_more`, `TimelineRows`). No `api.rs`/`server.rs`:
the vertical has no `#[server]` fns or wire types (it re-uses
`crate::posts::{TimelinePage, TimelinePostSummary, PostCard}`). The two
consumers (`pages/home.rs`, `pages/cockpit.rs`) repoint their imports at
`crate::timeline` and read failures off `LoadStatus::Failed`.

**Tech Stack:** Rust, Leptos (CSR/wasm), `cargo xtask` gate.

## Review header

**Scope — in:** the file move; the `TimelineCursor` + `LoadStatus`
type-modeling; the consumer rewire forced by dropping `state.error`; a one-line
`web-style-guide.md` §8 note that `api.rs` is omittable for a server-less
vertical. **Scope — out:** `#330` (App/Router move), `#312` (`pages/`
dissolution), `#304` (`read_signal!` inlining — kept as-is), a wire-level cursor
on the posts `TimelinePage` DTO (deferred to `#569`), full `home`/`cockpit`
vertical convergence (`#319`/`#317`).

**Tasks:**

1. Capture the deferred wire-cursor concern on #569 (separable concern, filed
   first).
2. `state.rs` — pure `TimelineCursor` / `LoadStatus` / `apply_rows`, host-tested
   (TDD).
3. UI cutover — `component.rs` + wiring, rewire both consumers, delete
   `pages/timeline.rs`, §8 doc note.

**Key risks/decisions:**

- **`pages` is wasm-only** (`lib.rs:33`; `signal_read.rs:6` — "`pages` compiles
  wasm-only since #300"), so the consumers may import the wasm-gated
  `TimelineRows`/`TimelineState` freely. The load-bearing gate for this
  vertical's UI is **`wasm-clippy`** (runs inside `cargo xtask check`,
  `static_checks.rs:76`), not host clippy.
- **Host dead-code trap (ADR-0070 §6):** `state.rs`'s pure items are consumed by
  the wasm-only `component.rs` + `#[cfg(test)]`; on the host non-test build
  they'd read as dead. They are therefore `pub` **and re-exported from
  `mod.rs`** (the blessed wasm-only-pure-helper recipe) so they stay reachable
  public API.
- **`has_more` retained** (not collapsed into `cursor.is_some()`) — the server
  can emit `has_more == true` with an absent cursor at `page_size == 0`
  (`posts/api/listing.rs` `page_from_rows`). `fail()` now clears the cursor so
  the two stay consistent.
- **Atomic cutover:** the last consumer moving off `pages/timeline.rs` makes
  that file dead code, so the rewire **and** the delete land in one commit (Task
  3).

## Global Constraints

- No `Co-Authored-By` trailer on commits.
- `target_arch = "wasm32"` appears **only on `mod` declarations and their paired
  `pub use`** — never on an item inside a leaf file (ADR-0070 §2).
  `component.rs` has zero internal cfgs.
- Pure, host-testable logic lives in **ungated, host-tested** files, extracted
  before any gate; **no new `cov:ignore`**, no `#[component]`-exemption
  reliance, no fake host stub (ADR-0070 §6 / ADR-0055).
- Per-commit gate: the pre-commit hook runs the full `cargo xtask check` (fmt +
  clippy + **wasm-clippy** + Nix coverage/tests). Run `cargo xtask check` first
  so it passes clean (**jaunder-commit**). Local e2e is reaped on this host —
  run `cargo xtask validate --no-e2e` for the full-gate step and let CI's e2e
  matrix gate the flows.

---

### Task 1: File the deferred wire-cursor concern on #569

Separable concern surfaced in the spec (a wire-level `Option<TimelineCursor>` on
the posts `TimelinePage` DTO). It belongs to the existing posts-DTO-rename issue
#569, not this cycle — capture it there so it isn't lost.

**Files:** none (tracker only).

- [x] **Step 1: Add the note to #569**

```bash
gh issue comment 569 --repo jaunder-org/jaunder --body "Deferred from #329 (timeline convergence): the timeline keyset cursor is now modeled as a timeline-local \`TimelineCursor { created_at: UtcInstant, post_id: PostId }\` in \`web/src/timeline/state.rs\`. Consider giving \`TimelinePage\` an \`Option<TimelineCursor>\` on the wire (replacing the flat \`next_cursor_created_at\`/\`next_cursor_post_id\` pair) when reworking the post DTOs here, per ADR-0065's pervasive-newtype direction."
```

- [x] **Step 2: Verify** the comment posted
      (`gh issue view 569 --repo jaunder-org/jaunder --comments`). No commit (no
      repo change).

---

### Task 2: Pure `state.rs` — `TimelineCursor`, `LoadStatus`, `apply_rows`

The ungated, host-tested heart of the vertical. Written test-first.
`pages/timeline.rs` is left untouched this task; the new module compiles and
tests on the host in isolation.

**Files:**

- Create: `web/src/timeline/state.rs`
- Create: `web/src/timeline/mod.rs`
- Modify: `web/src/lib.rs:44-48` (add `pub mod timeline;` in alpha order, after
  the `test_support` line, before `pub mod topbar;`)

**Interfaces:**

- Consumes: `common::ids::PostId` (`PostId::from(i64)`),
  `common::time::UtcInstant` (`FromStr`), `crate::posts::TimelinePage` (fields
  `posts`, `next_cursor_created_at: Option<UtcInstant>`,
  `next_cursor_post_id: Option<PostId>`, `has_more: bool`).
- Produces (all `pub`, re-exported from `mod.rs`):
  - `struct TimelineCursor { pub created_at: UtcInstant, pub post_id: PostId }` +
    `TimelineCursor::from_page(&TimelinePage) -> Option<TimelineCursor>`
  - `enum LoadStatus { Idle, InFlight, Failed(String) }` (derives `Default` →
    `Idle`) + `LoadStatus::is_in_flight(&self) -> bool`,
    `LoadStatus::error_message(&self) -> Option<&str>`
  - `enum PageMode { Replace, Append }`
  - `fn apply_rows<T>(current: Vec<T>, incoming: Vec<T>, mode: PageMode) -> Vec<T>`

- [x] **Step 1: Write the failing tests + wiring**

Create `web/src/timeline/state.rs` with **only** the `#[cfg(test)] mod tests`
block below (the `use super::*;` will reference not-yet-defined items — that is
the RED). Also create `web/src/timeline/mod.rs` with just `mod state;` (no
re-export yet) and add `pub mod timeline;` to `web/src/lib.rs` (alpha order,
after the `test_support` line, before `pub mod topbar;`), so the module is
reachable. The test module pins every branch: `from_page` (both components →
`Some`; neither and each partial → `None`), `is_in_flight` / `error_message`
across all three arms, and `apply_rows` in both modes.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn instant() -> UtcInstant {
        "2026-07-19T10:30:00Z".parse().unwrap()
    }

    fn page(
        next_cursor_created_at: Option<UtcInstant>,
        next_cursor_post_id: Option<PostId>,
        has_more: bool,
    ) -> TimelinePage {
        TimelinePage { posts: Vec::new(), next_cursor_created_at, next_cursor_post_id, has_more }
    }

    #[test]
    fn cursor_from_page_needs_both_components() {
        assert_eq!(
            TimelineCursor::from_page(&page(Some(instant()), Some(PostId::from(7)), true)),
            Some(TimelineCursor { created_at: instant(), post_id: PostId::from(7) }),
        );
        assert_eq!(TimelineCursor::from_page(&page(None, None, false)), None);
        assert_eq!(TimelineCursor::from_page(&page(Some(instant()), None, true)), None);
        assert_eq!(TimelineCursor::from_page(&page(None, Some(PostId::from(7)), true)), None);
    }

    #[test]
    fn load_status_accessors_cover_each_arm() {
        assert!(!LoadStatus::Idle.is_in_flight());
        assert!(LoadStatus::InFlight.is_in_flight());
        assert!(!LoadStatus::Failed("boom".into()).is_in_flight());

        assert_eq!(LoadStatus::Idle.error_message(), None);
        assert_eq!(LoadStatus::InFlight.error_message(), None);
        assert_eq!(LoadStatus::Failed("boom".into()).error_message(), Some("boom"));
    }

    #[test]
    fn apply_rows_replaces_or_appends() {
        assert_eq!(apply_rows(vec![1, 2], vec![3, 4], PageMode::Replace), vec![3, 4]);
        assert_eq!(apply_rows(vec![1, 2], vec![3, 4], PageMode::Append), vec![1, 2, 3, 4]);
    }
}
```

- [x] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p web timeline::state` Expected: FAIL — compile error,
`TimelineCursor` / `LoadStatus` / `apply_rows` / `PageMode` not defined.

- [x] **Step 3: Implement the model above the test module**

Prepend the types + logic to `state.rs` (above the `#[cfg(test)] mod tests`),
and add the re-export to `mod.rs`
(`pub use state::{apply_rows, LoadStatus, PageMode, TimelineCursor};` — this is
also what keeps them reachable, hence non-dead, on the host build).

`state.rs` module doc + impl (prepend above the test module):

```rust
//! Timeline pagination — the pure, host-tested value model (ADR-0070 §6): the
//! `TimelineCursor` newtype, the `LoadStatus` enum, and the row-fold helper. The
//! reactive `TimelineState` that wraps these in signals lives in the wasm-only
//! `component.rs`; everything here is ungated and coverage-measured.

use common::ids::PostId;
use common::time::UtcInstant;

use crate::posts::TimelinePage;

/// A keyset pagination cursor: the `(created_at, post_id)` pair a timeline page
/// hands back to fetch the next page. Bundling the two — which always move
/// together — makes "one set, the other not" unrepresentable (they were two
/// independent `Option` signals before #329).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimelineCursor {
    pub created_at: UtcInstant,
    pub post_id: PostId,
}

impl TimelineCursor {
    /// Build a cursor from a page's flat next-cursor fields: `Some` only when
    /// **both** components are present. A partial pair (which the server never
    /// emits) collapses to `None` rather than a half-cursor.
    #[must_use]
    pub fn from_page(page: &TimelinePage) -> Option<Self> {
        match (page.next_cursor_created_at, page.next_cursor_post_id) {
            (Some(created_at), Some(post_id)) => Some(Self { created_at, post_id }),
            _ => None,
        }
    }
}

/// The load state of a timeline: idle, a load-more in flight, or a failed fetch
/// carrying its display message. Replaces the old `loading_more: bool` +
/// `error: Option<String>` pair, which admitted the illegal "loading *and*
/// errored" combination.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum LoadStatus {
    #[default]
    Idle,
    InFlight,
    Failed(String),
}

impl LoadStatus {
    /// Whether a load-more is in flight (drives the button's disabled state).
    #[must_use]
    pub fn is_in_flight(&self) -> bool {
        matches!(self, Self::InFlight)
    }

    /// The failure message to display, if the last load failed.
    #[must_use]
    pub fn error_message(&self) -> Option<&str> {
        match self {
            Self::Failed(message) => Some(message),
            _ => None,
        }
    }
}

/// Whether a fetched page replaces the current rows (a seed or re-fetch) or is
/// appended to them (load-more).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageMode {
    Replace,
    Append,
}

/// Fold a fetched page's rows into the rows already shown. `Replace` swaps them
/// (first paint / re-fetch); `Append` extends them (load-more). Generic so the
/// merge is host-testable without constructing `TimelinePostSummary` fixtures.
#[must_use]
pub fn apply_rows<T>(current: Vec<T>, incoming: Vec<T>, mode: PageMode) -> Vec<T> {
    match mode {
        PageMode::Replace => incoming,
        PageMode::Append => {
            let mut current = current;
            current.extend(incoming);
            current
        }
    }
}
```

Final `web/src/timeline/mod.rs` after this step:

```rust
//! The timeline vertical (#329, ADR-0070): shared cursor-paginated timeline
//! machinery used by the public Local timeline (`home`) and the authed `/app`
//! cockpit. Module wiring only.
//!
//! A server-less vertical — no `#[server]` fns or wire types of its own (it
//! re-uses `crate::posts::{TimelinePage, TimelinePostSummary, PostCard}`), so
//! there is no `api.rs`/`server.rs`: only the pure host-tested `state` and (from
//! Task 3) the wasm-only reactive `component`. The `pub use` keeps the pure
//! items reachable on the host build, where `component` is compiled out.

mod state;
pub use state::{apply_rows, LoadStatus, PageMode, TimelineCursor};
```

- [x] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p web timeline::state` Expected: PASS (3 tests). If
`apply_rows`/`PageMode`/`TimelineCursor` trip a host dead-code lint, the
`pub use` in `mod.rs` is missing an item — that re-export is what keeps them
live on the host build.

- [x] **Step 5: Run the gate**

Run: `cargo xtask check` Expected: green (host clippy, wasm-clippy,
coverage/tests). `state.rs` is ungated and fully covered by its tests — no
`cov:ignore`.

- [x] **Step 6: Commit**

```bash
git add web/src/timeline/state.rs web/src/timeline/mod.rs web/src/lib.rs
git commit -m "feat(web/timeline): pure TimelineCursor + LoadStatus state model

Stand up the timeline vertical's ungated, host-tested value model (ADR-0070
§6): a TimelineCursor newtype bundling the keyset (created_at, post_id) pair,
a LoadStatus enum replacing the loading_more+error pair, and a generic
row-fold helper. Re-exported from mod.rs so they stay reachable on the host
build where the wasm-only component is compiled out. No behaviour change yet
— pages/timeline.rs is still the live machinery."
```

Run `cargo xtask check` first so the hook passes clean (**jaunder-commit**).

---

### Task 3: UI cutover — `component.rs`, rewire consumers, delete `pages/timeline.rs`

The atomic move: the wasm-only reactive layer lands in `component.rs`, both
consumers repoint at `crate::timeline` and read failures off `LoadStatus`, and
the old file is deleted in the same commit (moving the last consumer makes it
dead code).

**Files:**

- Create: `web/src/timeline/component.rs`
- Modify: `web/src/timeline/mod.rs` (add the gated `component` decl + re-export)
- Modify: `web/src/pages/home.rs:5,49,52` (import path; `spawn_load_more` path;
  `read_error`)
- Modify: `web/src/pages/cockpit.rs:15,66,69` (same three)
- Delete: `web/src/pages/timeline.rs`
- Modify: `web/src/pages/mod.rs:10` (remove `pub(crate) mod timeline;`)
- Modify: `docs/web-style-guide.md` §8 (api.rs-omittable note)

**Interfaces:**

- Consumes: everything from Task 2
  (`crate::timeline::{TimelineCursor, LoadStatus, PageMode, apply_rows}`), plus
  `crate::posts::{PostCard, TimelinePage, TimelinePostSummary}`,
  `crate::pages::signal_read::read_signal`, `common::pagination::PageSize`,
  `crate::error::WebResult`.
- Produces (wasm-only, re-exported): `TimelineState`, `spawn_load_more`,
  `TimelineRows` — the same names/paths the consumers now import as
  `crate::timeline::…`.

- [x] **Step 1: Create `web/src/timeline/component.rs`**

The current `pages/timeline.rs` bodies, re-typed onto the Task-2 model:
`TimelineState` holds one `cursor: RwSignal<Option<TimelineCursor>>` +
`has_more` + one `status: RwSignal<LoadStatus>`; transitions delegate to
`apply_rows` / `TimelineCursor::from_page`. Zero cfgs inside the file.

```rust
//! Timeline pagination — the wasm-only reactive layer (ADR-0070): the
//! `TimelineState` signal bundle, the load-more task, and the shared
//! `TimelineRows` view. Its pure types + fold logic live in the ungated,
//! host-tested `state.rs`; this file carries no cfg gates of its own (its `mod`
//! declaration is `#[cfg(target_arch = "wasm32")]`).

use std::future::Future;

use leptos::prelude::*;
use leptos::task::spawn_local;

use common::ids::PostId;
use common::pagination::PageSize;
use common::time::UtcInstant;

use super::state::{apply_rows, LoadStatus, PageMode, TimelineCursor};
use crate::error::WebResult;
use crate::pages::signal_read::read_signal;
use crate::posts::{PostCard, TimelinePage, TimelinePostSummary};

/// The reactive state of a cursor-paginated timeline, shared by the public Local
/// timeline (`home.rs`) and the authed `/app` cockpit (`cockpit.rs`).
#[derive(Clone, Copy)]
pub struct TimelineState {
    pub rows: RwSignal<Vec<TimelinePostSummary>>,
    pub cursor: RwSignal<Option<TimelineCursor>>,
    pub has_more: RwSignal<bool>,
    pub status: RwSignal<LoadStatus>,
}

impl Default for TimelineState {
    fn default() -> Self {
        Self {
            rows: RwSignal::new(Vec::new()),
            cursor: RwSignal::new(None),
            has_more: RwSignal::new(false),
            status: RwSignal::new(LoadStatus::Idle),
        }
    }
}

impl TimelineState {
    /// Adopt a page's rows + cursor (a projector seed or a fresh fetch),
    /// replacing what's shown.
    pub fn adopt(&self, page: TimelinePage) {
        self.cursor.set(TimelineCursor::from_page(&page));
        self.has_more.set(page.has_more);
        self.rows.set(apply_rows(Vec::new(), page.posts, PageMode::Replace));
    }

    /// Resolve a re-fetch into the signals and settle to idle (clearing any prior
    /// failure). wasm-only: re-fetches resolve on the client, in the page's
    /// post-hydration `Effect`.
    pub fn resolve(&self, page: TimelinePage) {
        self.adopt(page);
        self.status.set(LoadStatus::Idle);
    }

    /// Record a fetch failure: empty the rows (don't show a stale page), clear
    /// the cursor + `has_more` so a failed timeline offers no "Load more", and
    /// mark the failure for display.
    pub fn fail(&self, message: String) {
        self.rows.set(Vec::new());
        self.cursor.set(None);
        self.has_more.set(false);
        self.status.set(LoadStatus::Failed(message));
    }
}

/// wasm-only load-more: fetch the next page with the current cursor and append
/// it. `fetch` is the page's list fn (`list_local_timeline` / `list_home_feed`).
pub fn spawn_load_more<F, Fut>(state: TimelineState, fetch: F)
where
    F: FnOnce(Option<UtcInstant>, Option<PostId>, Option<PageSize>) -> Fut + 'static,
    Fut: Future<Output = WebResult<TimelinePage>> + 'static,
{
    if state.status.get_untracked().is_in_flight() || !state.has_more.get_untracked() {
        return;
    }
    state.status.set(LoadStatus::InFlight);
    let (created_at, post_id) = match state.cursor.get_untracked() {
        Some(cursor) => (Some(cursor.created_at), Some(cursor.post_id)),
        None => (None, None),
    };
    spawn_local(async move {
        match fetch(created_at, post_id, Some(PageSize::default())).await {
            Ok(page) => {
                state.cursor.set(TimelineCursor::from_page(&page));
                state.has_more.set(page.has_more);
                state
                    .rows
                    .set(apply_rows(state.rows.get_untracked(), page.posts, PageMode::Append));
                state.status.set(LoadStatus::Idle);
            }
            Err(err) => state.status.set(LoadStatus::Failed(err.to_string())),
        }
    });
}

/// The scroll region shared by both timelines: the post list (or an empty
/// placeholder) followed by the load-more button.
#[component]
pub fn TimelineRows(
    state: TimelineState,
    on_mutate: Callback<()>,
    on_load_more: Callback<()>,
) -> impl IntoView {
    let read_rows = move || read_signal!(state.rows);
    let read_has_more = move || read_signal!(state.has_more);
    let read_in_flight = move || read_signal!(state.status).is_in_flight();
    view! {
        <div class="j-scroll">
            {move || {
                let rows = read_rows();
                if rows.is_empty() {
                    view! { <p>"No posts yet."</p> }.into_any()
                } else {
                    rows.into_iter()
                        .map(|p| view! { <PostCard post=p banner=None on_mutate=on_mutate /> })
                        .collect::<Vec<_>>()
                        .into_any()
                }
            }}
            {move || {
                read_has_more()
                    .then(|| {
                        view! {
                            <button on:click=move |_| on_load_more.run(()) disabled=read_in_flight>
                                {move || {
                                    if read_in_flight() { "Loading\u{2026}" } else { "Load more" }
                                }}
                            </button>
                        }
                    })
            }}
        </div>
    }
}
```

- [x] **Step 2: Wire `component` into `mod.rs`**

Append to `web/src/timeline/mod.rs`:

```rust
#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::{spawn_load_more, TimelineRows, TimelineState};
```

- [x] **Step 3: Rewire `web/src/pages/home.rs`**

- Line 5: `use crate::pages::timeline::{TimelineRows, TimelineState};` →
  `use crate::timeline::{LoadStatus, TimelineRows, TimelineState};`
- Line 49:
  `crate::pages::timeline::spawn_load_more(state, list_local_timeline);` →
  `crate::timeline::spawn_load_more(state, list_local_timeline);`
- Line 52: `let read_error = move || read_signal!(state.error);` →

```rust
    let read_error = move || match read_signal!(state.status) {
        LoadStatus::Failed(message) => Some(message),
        _ => None,
    };
```

(The `if let Some(err) = read_error()` render block at lines 56-59 is unchanged
— `read_error()` is still `Option<String>`.)

- [x] **Step 4: Rewire `web/src/pages/cockpit.rs`** (identical shape)

- Line 15: `use crate::pages::timeline::{TimelineRows, TimelineState};` →
  `use crate::timeline::{LoadStatus, TimelineRows, TimelineState};`
- Line 66: `crate::pages::timeline::spawn_load_more(state, list_home_feed);` →
  `crate::timeline::spawn_load_more(state, list_home_feed);`
- Line 69: `let read_error = move || read_signal!(state.error);` → the same
  `match read_signal!(state.status) { LoadStatus::Failed(message) => Some(message), _ => None }`
  closure as home.rs Step 3.

- [x] **Step 5: Delete the old module**

```bash
git rm web/src/pages/timeline.rs
```

Then remove line 10 (`pub(crate) mod timeline;`) from `web/src/pages/mod.rs`.

- [x] **Step 6: Add the `web-style-guide.md` §8 note**

In §8's per-file layout description, add one line noting that `api.rs` (like
`server.rs`/`component.rs`) is omitted for a **server-less** vertical — one with
no `#[server]` fns or wire types of its own (timeline is the first such case,
#329).

- [x] **Step 7: Verify no stale references remain**

Run: `rg -n 'pages::timeline|state\.error|loading_more' web/src` Expected: no
hits for `pages::timeline`; no `state.error` / `loading_more` on `TimelineState`
(matches only unrelated code, if any — inspect each hit).

- [x] **Step 8: Run the gate**

Run: `cargo xtask check` Expected: green — crucially **wasm-clippy** (the sole
clippy gate for `pages` + `timeline/component.rs`, both wasm-only) compiles the
rewired consumers and the new component clean.

- [x] **Step 9: Commit**

```bash
git add web/src/timeline/component.rs web/src/timeline/mod.rs web/src/pages/home.rs web/src/pages/cockpit.rs web/src/pages/mod.rs docs/web-style-guide.md
git commit -m "refactor(web/timeline): move reactive machinery into the vertical

Relocate TimelineState/spawn_load_more/TimelineRows out of pages/timeline.rs
into the wasm-only web/src/timeline/component.rs, re-typed onto the #329
TimelineCursor + LoadStatus model. home.rs/cockpit.rs import from
crate::timeline and read failures off LoadStatus::Failed; pages/timeline.rs is
deleted. Note api.rs as omittable for a server-less vertical in web-style-guide
§8. No behaviour change."
```

(The `git rm` from Step 5 is already staged.) Run `cargo xtask check` first so
the hook passes clean.

- [x] **Step 10: Full-gate verification**

Run: `cargo xtask validate --no-e2e` Expected: green (static + wasm-clippy +
coverage). Local e2e is reaped on this host, so the timeline behavioral flows
(`end2end/tests/{posts,authed-flash,feeds,visibility}.spec.ts`) are gated by
CI's e2e matrix on the PR (jaunder-ship). Confirm a clean tree afterward
(`git status --porcelain` — `cargo xtask check` auto-fixes fmt and may leave
staged edits).
