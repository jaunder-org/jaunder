# Timeline Gate Convergence Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
[`docs/superpowers/specs/2026-07-29-issue-671-timeline-gate.md`](../specs/2026-07-29-issue-671-timeline-gate.md)
— referenced by decision (D1–D12) and acceptance criterion (A1–A9). **Read it
first.** This plan is "how"; the spec is "what/why" and is not restated here.

**Goal:** Relocate the reactive `TimelineState` into host-compiled
`web/src/timeline/state.rs` where its transitions and its render decision become
`Owner`-tested, then converge all five timeline pages onto one `TimelineGate`,
dropping three `#[expect(clippy::too_many_lines)]`.

**Architecture:** Every decision moves into host-compiled `state.rs` — the
transitions (`adopt`/`apply`/`fail`/`unidentified`/`append`/`begin_load_more`)
and the render fold (`paint() -> WebResult<TimelinePaint>`). Wasm-only
`component.rs` keeps just the two things that cannot host-run: `Effect::new` and
`spawn_local`. `TimelineGate` becomes two memo-gated sibling regions plus a bare
four-arm `match`; per-page variation travels as a data enum (`NoIdentity`) or
`children`.

**Tech Stack:** Rust, leptos 0.8.2 (CSR), `cargo nextest`, `cargo xtask`,
Playwright.

## Review header

**Scope — in:** `web/src/timeline/{mod,state,component}.rs`; the five pages
(`web/src/posts/component.rs` ×3, `web/src/home/component.rs`,
`web/src/cockpit/component.rs`); one e2e in `end2end/tests/posts.spec.ts` plus a
helper in `end2end/tests/helpers.ts`; one ADR draft; a one-word visibility
change in `web/src/posts/render.rs`.

**Scope — out:** spec §5 — `PostCreateForm`/`EditPostPage` expects, #306's guard
itself, the `Invalid username`/`Unidentified` axis merge, `TimelineRows` markup,
`#[client_only]`. No separable concerns surfaced during the interview, so there
is **no** issue-filing task.

| Task | Deliverable                                                                                                                                                                                               |
| ---- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1    | Relocate `TimelineState` verbatim to host `state.rs`; `Owner` tests for `adopt`/`resolve`/`fail`; fix 3 falsified module docs; move the export (A2, A8)                                                   |
| 2    | `LoadStatus::Failed(WebError)`; `into_failure() -> Option<WebError>`; `.to_string()` at the 5 render sites (D3)                                                                                           |
| 3    | Complete the state machine: `NeverLoaded`, `Unidentified`, `adopt` settles `Idle`, delete `resolve`, add `adopt_seed`/`apply`/`unidentified`/`append`/`begin_load_more`; thin `spawn_load_more` (D2, D10) |
| 4    | The paint fold: `TimelinePaint`, `shows_chrome()`, `NoIdentity`, `paint()` — host-tested (D4–D6)                                                                                                          |
| 5    | `TimelineGate` + `wire_timeline_resolve` (D7, D8)                                                                                                                                                         |
| 6    | Layout-shift probes for the 4 projector-painted timelines — **green before the sweeps**, guarding Tasks 7–9 (A10)                                                                                         |
| 7    | Sweep the three pages; drop all three `#[expect]` (A1)                                                                                                                                                    |
| 8    | `home` onto the gate, masthead as `children` — new e2e written first as the failing test (A6)                                                                                                             |
| 9    | `cockpit` onto the gate; delete `bounce` (D11, A7)                                                                                                                                                        |
| 10   | ADR draft (A9)                                                                                                                                                                                            |

**Key risks / decisions:**

- **T8 is the risk concentration.** home's masthead is projector-coincident
  `inner_html` (ADR-0041 §2); #653 was a first-paint regression in exactly that
  class. D8's two-region gate exists to keep that subtree alive across
  `Loading → Rows`, and T8's e2e asserts it via a stamp-survives check. If the
  stamp does not survive, D8 is wrong — stop and revise the spec, do not weaken
  the assertion.
- **Projector coincidence is verified empirically, not by inference.** The
  projector's own render fns are untouched, and `adopt_seed` runs synchronously
  before first render so a seeded page never enters the Loading arm — but that
  is an argument, not coverage, and the existing e2e assert content _after_
  hydration settles, so they pass straight through a flash (#653 proved it).
  T6's probes, written first and green on the pre-sweep tree, are what turn the
  argument into a gate.
- **Two leptos patterns have no in-repo precedent** (spec §3):
  `#[prop(default = Signal::derive(…))]` and `children: Option<ChildrenFn>`.
  Each has a specified fallback. Take the fallback; do not redesign.
- **Coverage flips from exempt to measured.** Everything moved into `state.rs`
  is now gated on. A4 forbids a new `cov:ignore`/`crap:allow` under
  `web/src/timeline/`, so each task's tests must land with its code, not after.
  Note derives count as executable lines (spec A3, final bullet) — `NoIdentity`
  in particular is only _matched_ in wasm.
- **Task order keeps every commit green.** T3 adds `NeverLoaded` before the
  pages are swept; that is safe because the pages read `into_failure()` (`None`
  for `NeverLoaded`) and their own `loaded` signal, neither of which changes
  meaning. Verified against all four current `adopt()` call sites.

## Global Constraints

Copied from the spec and `CONTRIBUTING.md`; every task's requirements include
these.

- **No new `cov:ignore` or `crap:allow` marker under `web/src/timeline/`** (A4).
- **No `target_arch` cfg on any item** — only on a `mod`/`use` line in a
  `mod.rs` (ADR-0070 §2; enforced by
  `xtask/src/steps/target_arch_placement_check.rs`).
- **`state.rs` stays ungated; `component.rs` stays gated at its `mod` line**
  (A2).
- **No `#[client_only]`** — the macro is retired repo-wide; there is nothing to
  add.
- **Do not add a `TimelinePostSummary` fixture to `common::test_support`.** It
  would need `RenderedHtml::from_trusted`, and the #398 guard
  (`xtask/src/steps/rendered_html_from_trusted_check.rs:87-98`) parses each file
  standalone and flags every non-test mention; `common/src/test_support.rs` is a
  flat feature-gated file with no inline `#[cfg(test)] mod`, so it would be
  flagged. Moot anyway — the fixture already exists as
  `crate::posts::render::test_fixtures::sample_summary`
  (`web/src/posts/render.rs:283`), already `pub(crate)`, already used
  cross-module at `web/src/app/render.rs:192`. **Reuse it; add nothing and
  change no visibility.**
- **Owner-test convention:** every test touching an `RwSignal` runs inside
  `Owner::new()` + `.set()` + `drop`, per `web/src/tags/input_state.rs:150-157`.
- **`TagCtx` naming.** The type is `crate::taglist::TagCtx`. Both existing
  consumers import it aliased — `use crate::taglist::TagCtx as TagContext;`
  (`timeline/component.rs:20`, `posts/component.rs:28`) — so code in those two
  files says `TagContext`, while new code in `state.rs` says `TagCtx`. That is
  the existing convention, not an inconsistency; the `TimelinePaint::Rows(_)`
  binding is positional, so the alias never has to agree.
- **Commit messages:** conventional prefix, **no `Co-Authored-By` trailer**.
- **Per-commit gate:** run `devtool run -- cargo xtask check` before every
  commit and let it pass clean (`jaunder-commit`). It auto-fixes formatting, so
  re-check `git status --porcelain` afterwards and stage what it rewrote.
- **Fast gate for the inner loop:**
  `devtool run -- cargo xtask check --no-test`. It runs the whole static ladder
  including `wasm-clippy`, so it **does** lint the wasm target — there is no
  separate wasm step to remember. **Do not hand-roll
  `cargo clippy -p web --target wasm32-unknown-unknown`:** it omits
  `--features csr` and the sibling `-p client -p csr`, so it fails with ~9
  unrelated `cannot find reactive in client` / `cannot find upload in client`
  errors that have nothing to do with your change. The real invocation, with two
  temporary `-A` flags, is `xtask/src/steps/static_checks.rs:76-98`.

---

### Task 1: Relocate `TimelineState` to host-compiled `state.rs`

Pure move plus tests — **no semantic change**. Reviewable as a no-op.

**Files:**

- Modify: `web/src/timeline/state.rs` (add the bundle + tests; fix the module
  doc at `:1-4`)
- Modify: `web/src/timeline/component.rs` (remove the bundle at `:22-69`; fix
  the doc at `:1-5`)
- Modify: `web/src/timeline/mod.rs` (move the export; fix the doc at `:1-9`)
- Test: in-file `#[cfg(test)] mod tests` in `web/src/timeline/state.rs`

**Interfaces:**

- Consumes:
  `crate::posts::render::test_fixtures::{sample_summary, one_post_page}` — an
  existing `#[cfg(test)] pub(crate) mod test_fixtures`
  (`web/src/posts/render.rs:252`; fns at `:283` and `:300`), **already**
  `pub(crate)` and already imported cross-module at `web/src/app/render.rs:192`.
  No visibility change is needed anywhere; note the module is `test_fixtures`,
  **not** `tests`. `one_post_page()` is exactly
  `page_with(vec![sample_summary()], None, None, false)` — use it instead of
  rebuilding that page by hand.
- Produces: `web::timeline::TimelineState { rows, cursor, has_more, status }` —
  ungated, host-compiled, `#[derive(Clone, Copy, Default)]`, with `adopt`,
  `resolve`, `fail` moved verbatim. `spawn_load_more`, `TimelineRows` stay in
  `component.rs`.

- [x] **Step 1: Move the bundle into `state.rs`**

Cut `TimelineState`, its `impl Default`, and its `impl` block
(`component.rs:22-69`) into `state.rs`, below `LoadStatus`. Replace the
hand-written `Default` with `#[derive(Clone, Copy, Default)]` on the struct —
`RwSignal<T>: Default where T: Default`
(`reactive_graph-0.2.14/src/signal/rw.rs:272`), so the four fields default
correctly. Add `use leptos::prelude::*;` to `state.rs`. In `component.rs`, add
`TimelineState` to the `use super::state::{…}` list. **Keep `LoadStatus` and
`TimelineCursor` in that import** — `spawn_load_more` still uses both
(`component.rs:81`, `:82`, `:86`, `:89`, `:91`); neither becomes unused until T3
Step 4 thins it, and clippy's `-D warnings` will force the prune there.

- [x] **Step 2: Move the export**

In `web/src/timeline/mod.rs`, extend the ungated re-export and shrink the gated
one:

```rust
pub(crate) mod render;
mod state;
pub use state::{LoadStatus, TimelineCursor, TimelineState};

#[cfg(target_arch = "wasm32")]
mod component;
#[cfg(target_arch = "wasm32")]
pub use component::{spawn_load_more, TimelineRows};
```

`TimelineState` **must** leave the gated list — exporting it from both collides
(A2).

- [x] **Step 3: Correct the three falsified module docs** (A8)

Each currently asserts the bundle is wasm-only:

- `mod.rs:1-9` — "only the pure host-tested `state` and `render` leaves and the
  wasm-only reactive `component`" → say `state` now also holds the reactive
  signal bundle, host-tested under an `Owner`, and `component` keeps only
  `Effect`/`spawn_local`-bound code.
- `state.rs:1-4` — delete "The reactive `TimelineState` that wraps these in
  signals lives in the wasm-only `component.rs`"; say it lives here and is
  `Owner`-tested, citing #671.
- `component.rs:1-5` — drop "the `TimelineState` signal bundle" from the
  inventory.

- [x] **Step 4: Write the failing tests**

Append to `state.rs`'s existing `#[cfg(test)] mod tests`. Extend the existing
`page()` helper (currently `state.rs:87-98`, always empty `posts`) to take rows,
and add the `Owner` harness:

```rust
use crate::posts::render::test_fixtures::sample_summary;
use common::seed::TimelinePostSummary;

fn page_with(
    posts: Vec<TimelinePostSummary>,
    next_cursor_created_at: Option<UtcInstant>,
    next_cursor_post_id: Option<PostId>,
    has_more: bool,
) -> TimelinePage {
    TimelinePage { posts, next_cursor_created_at, next_cursor_post_id, has_more }
}

/// Run `body` under a fresh reactive `Owner` (the `web::reactive` / `forms::Field` /
/// `tags::input_state` convention), so `RwSignal`s work host-side without a browser.
fn with_owner(body: impl FnOnce()) {
    let owner = Owner::new();
    owner.set();
    body();
    drop(owner);
}

#[test]
fn default_state_is_empty_and_idle() {
    with_owner(|| {
        let state = TimelineState::default();
        assert!(state.rows.get().is_empty());
        assert_eq!(state.cursor.get(), None);
        assert!(!state.has_more.get());
        assert_eq!(state.status.get(), LoadStatus::Idle);
    });
}

#[test]
fn adopt_replaces_rows_cursor_and_has_more() {
    with_owner(|| {
        let state = TimelineState::default();
        state.adopt(page_with(
            vec![sample_summary()],
            Some(instant()),
            Some(PostId::from(7)),
            true,
        ));
        assert_eq!(state.rows.get().len(), 1);
        assert_eq!(
            state.cursor.get(),
            Some(TimelineCursor { created_at: instant(), post_id: PostId::from(7) })
        );
        assert!(state.has_more.get());
    });
}

#[test]
fn resolve_adopts_and_clears_a_prior_failure() {
    with_owner(|| {
        let state = TimelineState::default();
        state.fail("boom".to_owned());
        state.resolve(page_with(vec![sample_summary()], None, None, false));
        assert_eq!(state.rows.get().len(), 1);
        assert_eq!(state.status.get(), LoadStatus::Idle, "failure cleared");
    });
}

#[test]
fn fail_empties_the_timeline_and_records_the_message() {
    with_owner(|| {
        let state = TimelineState::default();
        state.adopt(page_with(
            vec![sample_summary()],
            Some(instant()),
            Some(PostId::from(7)),
            true,
        ));
        state.fail("boom".to_owned());
        assert!(state.rows.get().is_empty(), "no stale page");
        assert_eq!(state.cursor.get(), None);
        assert!(!state.has_more.get(), "a failed timeline offers no Load more");
        assert_eq!(state.status.get(), LoadStatus::Failed("boom".to_owned()));
    });
}
```

- [ ] **Step 5: Run the tests, verify they fail** — **NOT DONE.** The code and
      tests were written together, so the first `nextest` run was already green.
      For a verbatim relocation the red state is a compile error
      (`TimelineState` absent from `crate::timeline::state`), which is
      guaranteed rather than informative — but the step was still skipped, not
      satisfied. Later tasks add real behavior and must run red first.

```
cargo nextest run -p web timeline::state
```

Expected: FAIL — `TimelineState` not found in `crate::timeline::state` before
Step 1 lands.

- [x] **Step 6: Run the tests, verify they pass**

```
cargo nextest run -p web timeline::state
```

Expected: PASS — 7 tests (3 pre-existing at `state.rs:101`, `:121`, `:134`, plus
4 new). _Observed: 7 passed, 121 skipped._

- [x] **Step 7: Verify nothing downstream moved**

```
cargo clippy -p web --all-features --all-targets -- -D warnings
devtool run -- cargo xtask check --no-test
```

Expected: both clean. The five pages are untouched in this task, so any error
here means the export move in Step 2 is wrong.

- [ ] **Step 8: Commit**

```bash
devtool run -- cargo xtask check
git status --porcelain
git add web/src/timeline/state.rs web/src/timeline/component.rs web/src/timeline/mod.rs
git commit -m "refactor(timeline): host-compile the TimelineState signal bundle (#671)"
```

---

### Task 2: Carry the typed `WebError` in `LoadStatus::Failed`

**Files:**

- Modify: `web/src/timeline/state.rs` (`LoadStatus::Failed`, `into_failure`,
  `fail`, tests)
- Modify: `web/src/timeline/component.rs` (`spawn_load_more`'s `Err` arm, `:91`)
- Modify: `web/src/posts/component.rs` (`:1087`, `:1104`, `:1127`, `:1628`,
  `:1645`, `:1662`, `:1749`, `:1769`, `:1800`)
- Modify: `web/src/home/component.rs` (`:47`, `:60`, `:75`)
- Modify: `web/src/cockpit/component.rs` (`:62`, `:79`, `:89`)

**Interfaces:**

- Consumes: T1's host-compiled `TimelineState`.
- Produces: `LoadStatus::Failed(WebError)`;
  `LoadStatus::into_failure(self) -> Option<WebError>`;
  `TimelineState::fail(&self, error: WebError)`. Render sites stringify with
  `{err.to_string()}` instead of `{err}`.

- [ ] **Step 1: Update the tests first**

In `state.rs`'s tests, replace **every** `String` failure payload with
`WebError::validation("boom")` — the two pre-existing `Failed("boom".into())`
mentions (`state.rs:137`, `:142`) plus T1's two `state.fail("boom".to_owned())`
calls and its `Failed("boom".to_owned())` assertion — and add an assertion that
the type survives the round trip:

```rust
#[test]
fn into_failure_returns_the_typed_error() {
    assert_eq!(LoadStatus::Idle.into_failure(), None);
    assert_eq!(LoadStatus::InFlight.into_failure(), None);
    assert_eq!(
        LoadStatus::Failed(WebError::validation("boom")).into_failure(),
        Some(WebError::validation("boom")),
        "the error kind survives, not just its message"
    );
}
```

Delete the old `load_status_accessors_cover_each_arm`'s `into_failure`
assertions that compared against `Some("boom".to_owned())`; keep its
`is_in_flight` assertions and update the `Failed` payload there too.

- [ ] **Step 2: Run the tests, verify they fail**

```
cargo nextest run -p web timeline::state
```

Expected: FAIL — mismatched types, `String` vs `WebError`.

- [ ] **Step 3: Change the payload and the five call sites**

`state.rs`: `Failed(WebError)`; `into_failure(self) -> Option<WebError>`;
`fail(&self, error: WebError)` storing `LoadStatus::Failed(error)`. Add
`use crate::error::WebError;`.

Then at every producer drop the eager stringify — `state.fail(err.to_string())`
becomes `state.fail(err)` (`posts/component.rs:1087`, `:1628`, `:1749`;
`home/component.rs:47`; `cockpit/component.rs:62`), and `spawn_load_more`'s
`Err` arm (`component.rs:91`) becomes
`state.status.set(LoadStatus::Failed(err))`.

At every consumer, stringify at the render instead: the five `read_error` memos
are unchanged in shape but now yield `Option<WebError>`, so each
`view! { <p class="error">{err}</p> }` becomes `{err.to_string()}`
(`posts/component.rs:1127`, `:1662`, `:1800`; `home/component.rs:75`;
`cockpit/component.rs:89`).

- [ ] **Step 4: Run the tests, verify they pass**

```
cargo nextest run -p web timeline::state
cargo clippy -p web --all-features --all-targets -- -D warnings
devtool run -- cargo xtask check --no-test
```

Expected: tests PASS, both clippy runs clean.

- [ ] **Step 5: Commit**

```bash
devtool run -- cargo xtask check
git status --porcelain
git add web/src/timeline web/src/posts/component.rs web/src/home/component.rs web/src/cockpit/component.rs
git commit -m "refactor(timeline): carry the typed WebError in LoadStatus::Failed (#671)"
```

---

### Task 3: Complete the state machine

Adds the two new statuses and every remaining transition, and thins
`spawn_load_more`. The five pages change only where the compiler forces them
(`resolve` → `adopt`).

**Files:**

- Modify: `web/src/timeline/state.rs`
- Modify: `web/src/timeline/component.rs` (`spawn_load_more`, `:73-94`)
- Modify: `web/src/posts/component.rs` (`:1086`, `:1627`, `:1748`),
  `web/src/home/component.rs` (`:46`), `web/src/cockpit/component.rs` (`:59`)

**Interfaces:**

- Consumes: T2's `LoadStatus::Failed(WebError)`.
- Produces, all on `TimelineState`: `adopt(&self, page: TimelinePage)` (now
  settles `status = Idle`); `adopt_seed(&self, page: Option<TimelinePage>)`;
  `apply(&self, result: WebResult<TimelinePage>)`;
  `fail(&self, error: WebError)`; `unidentified(&self)`;
  `append(&self, result: WebResult<TimelinePage>)`;
  `begin_load_more(&self) -> Option<(Option<UtcInstant>, Option<PostId>)>`. Plus
  `LoadStatus::{NeverLoaded, Unidentified}`, `NeverLoaded` being `#[default]`.
  `resolve()` is **deleted**.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn default_status_is_never_loaded() {
    with_owner(|| {
        assert_eq!(TimelineState::default().status.get(), LoadStatus::NeverLoaded);
    });
}

#[test]
fn adopt_settles_to_idle_and_clears_a_prior_failure() {
    with_owner(|| {
        let state = TimelineState::default();
        state.fail(WebError::validation("boom"));
        state.adopt(page_with(vec![sample_summary()], None, None, false));
        assert_eq!(state.rows.get().len(), 1);
        assert_eq!(state.status.get(), LoadStatus::Idle, "adopt IS the old resolve");
    });
}

#[test]
fn adopt_seed_adopts_only_when_seeded() {
    with_owner(|| {
        let state = TimelineState::default();
        state.adopt_seed(None);
        assert!(state.rows.get().is_empty());
        assert_eq!(state.status.get(), LoadStatus::NeverLoaded, "not seeded, not loaded");

        state.adopt_seed(Some(page_with(vec![sample_summary()], None, None, false)));
        assert_eq!(state.rows.get().len(), 1);
        assert_eq!(state.status.get(), LoadStatus::Idle);
    });
}

#[test]
fn apply_ok_adopts_and_apply_err_empties() {
    with_owner(|| {
        let state = TimelineState::default();
        state.apply(Ok(page_with(
            vec![sample_summary()],
            Some(instant()),
            Some(PostId::from(7)),
            true,
        )));
        assert_eq!(state.rows.get().len(), 1);
        assert!(state.has_more.get());

        state.apply(Err(WebError::validation("boom")));
        assert!(state.rows.get().is_empty(), "no stale page on a refetch failure");
        assert_eq!(state.cursor.get(), None);
        assert!(!state.has_more.get());
        assert_eq!(state.status.get(), LoadStatus::Failed(WebError::validation("boom")));
    });
}

#[test]
fn unidentified_empties_the_timeline_and_marks_the_status() {
    with_owner(|| {
        let state = TimelineState::default();
        state.adopt(page_with(vec![sample_summary()], None, None, true));
        state.unidentified();
        assert!(state.rows.get().is_empty());
        assert_eq!(state.cursor.get(), None);
        assert!(!state.has_more.get());
        assert_eq!(state.status.get(), LoadStatus::Unidentified);
    });
}

// A4/B4: all four effects asserted, so an `append` that forgets the cursor — and
// therefore refetches page 1 forever — cannot pass.
#[test]
fn append_ok_extends_rows_and_advances_the_cursor() {
    with_owner(|| {
        let state = TimelineState::default();
        state.adopt(page_with(
            vec![sample_summary()],
            Some(instant()),
            Some(PostId::from(7)),
            true,
        ));
        state.status.set(LoadStatus::InFlight);

        let later: UtcInstant = "2026-07-20T10:30:00Z".parse().unwrap();
        state.append(Ok(page_with(
            vec![sample_summary(), sample_summary()],
            Some(later),
            Some(PostId::from(9)),
            false,
        )));

        assert_eq!(state.rows.get().len(), 3, "extends, does not replace");
        assert_eq!(
            state.cursor.get(),
            Some(TimelineCursor { created_at: later, post_id: PostId::from(9) }),
            "cursor advances to the new page"
        );
        assert!(!state.has_more.get(), "has_more is overwritten");
        assert_eq!(state.status.get(), LoadStatus::Idle);
    });
}

// D10: load-more failure keeps the pages already fetched.
#[test]
fn append_err_marks_the_status_and_retains_the_rows() {
    with_owner(|| {
        let state = TimelineState::default();
        state.adopt(page_with(
            vec![sample_summary()],
            Some(instant()),
            Some(PostId::from(7)),
            true,
        ));
        state.append(Err(WebError::validation("boom")));

        assert_eq!(state.rows.get().len(), 1, "page 1 survives a page-2 failure");
        assert_eq!(
            state.cursor.get(),
            Some(TimelineCursor { created_at: instant(), post_id: PostId::from(7) }),
            "cursor untouched"
        );
        assert!(state.has_more.get(), "has_more untouched");
        assert_eq!(state.status.get(), LoadStatus::Failed(WebError::validation("boom")));
    });
}

#[test]
fn begin_load_more_guards_then_marks_in_flight() {
    with_owner(|| {
        let state = TimelineState::default();

        state.has_more.set(false);
        assert_eq!(state.begin_load_more(), None, "nothing more to fetch");

        state.has_more.set(true);
        state.status.set(LoadStatus::InFlight);
        assert_eq!(state.begin_load_more(), None, "already in flight");

        state.status.set(LoadStatus::Idle);
        state.cursor.set(Some(TimelineCursor {
            created_at: instant(),
            post_id: PostId::from(7),
        }));
        assert_eq!(
            state.begin_load_more(),
            Some((Some(instant()), Some(PostId::from(7)))),
            "hands back the cursor as a query pair"
        );
        assert_eq!(state.status.get(), LoadStatus::InFlight, "and marks it in flight");
    });
}

#[test]
fn begin_load_more_without_a_cursor_yields_an_empty_query() {
    with_owner(|| {
        let state = TimelineState::default();
        state.has_more.set(true);
        assert_eq!(state.begin_load_more(), Some((None, None)));
    });
}

#[test]
fn is_in_flight_covers_every_status() {
    assert!(!LoadStatus::NeverLoaded.is_in_flight());
    assert!(!LoadStatus::Idle.is_in_flight());
    assert!(LoadStatus::InFlight.is_in_flight());
    assert!(!LoadStatus::Failed(WebError::validation("boom")).is_in_flight());
    assert!(!LoadStatus::Unidentified.is_in_flight());
}

#[test]
fn into_failure_covers_every_status() {
    assert_eq!(LoadStatus::NeverLoaded.into_failure(), None);
    assert_eq!(LoadStatus::Idle.into_failure(), None);
    assert_eq!(LoadStatus::InFlight.into_failure(), None);
    assert_eq!(LoadStatus::Unidentified.into_failure(), None);
    assert_eq!(
        LoadStatus::Failed(WebError::validation("boom")).into_failure(),
        Some(WebError::validation("boom"))
    );
}
```

Delete `default_state_is_empty_and_idle` from T1 (superseded by
`default_status_is_never_loaded`), `resolve_adopts_and_clears_a_prior_failure`
(superseded by `adopt_settles_to_idle_and_clears_a_prior_failure`), and the
older `load_status_accessors_cover_each_arm` (superseded by the two exhaustive
tests above).

- [ ] **Step 2: Run the tests, verify they fail**

```
cargo nextest run -p web timeline::state
```

Expected: FAIL — `NeverLoaded`, `Unidentified`, `adopt_seed`, `apply`,
`unidentified`, `append`, `begin_load_more` all undefined.

- [ ] **Step 3: Implement against the tests**

Add the two `LoadStatus` variants (`NeverLoaded` carrying `#[default]`, moved
off `Idle`) and the seven `TimelineState` methods to the signatures in
**Interfaces** above. Every branch is pinned by a Step 1 test — the guard's two
`None` paths and its `Some` path, `append`'s four effects and its rows-retaining
`Err` arm, `apply`'s two arms, `adopt_seed`'s two arms, and each new
`is_in_flight`/`into_failure` arm — so the tests determine the bodies. Delete
`resolve()`.

One invariant the tests cannot express, so state it in a doc comment on `adopt`:
settling to `Idle` is what makes `adopt` serve **both** the projector-seed path
and the fetch-resolve path, which is why `resolve` no longer exists (D2).

- [ ] **Step 4: Thin `spawn_load_more`**

Replace `component.rs:73-94`'s body with the six-line shell from spec §3,
delegating to `begin_load_more` and `append`. Its `where` clause and generics
are unchanged. This is where `LoadStatus` and `TimelineCursor` finally leave
`component.rs`'s `use super::state::{…}` — clippy's `-D warnings` will name
them; prune exactly what it names.

- [ ] **Step 5: Point the five `resolve` callers at `adopt`**

`state.resolve(page)` → `state.adopt(page)` at `posts/component.rs:1086`,
`:1627`, `:1748`, `home/component.rs:46`, `cockpit/component.rs:59`. Nothing
else on those pages changes yet — each keeps its own `loaded` signal and its own
view shape until T7–T9.

- [ ] **Step 6: Run the tests, verify they pass**

```
cargo nextest run -p web timeline::state
cargo clippy -p web --all-features --all-targets -- -D warnings
devtool run -- cargo xtask check --no-test
```

Expected: tests PASS, both clippy runs clean. `unidentified()` has no caller yet
— that is fine, it is a `pub` re-exported item and covered by its own test.

- [ ] **Step 7: Commit**

```bash
devtool run -- cargo xtask check
git status --porcelain
git add web/src/timeline web/src/posts/component.rs web/src/home/component.rs web/src/cockpit/component.rs
git commit -m "refactor(timeline): complete the host-tested TimelineState machine (#671)"
```

---

### Task 4: The paint fold

**Files:**

- Modify: `web/src/timeline/state.rs`
- Modify: `web/src/timeline/mod.rs` (export the two new types)

**Interfaces:**

- Consumes: T3's `LoadStatus`.
- Produces: `TimelinePaint { Loading, Rows(TagCtx), Unidentified }` with
  `shows_chrome(&self) -> bool`; `NoIdentity { Blank, Redirect(&'static str) }`;
  `TimelineState::paint(&self, context: Option<TagCtx>) -> WebResult<TimelinePaint>`.
  All three re-exported ungated from `timeline/mod.rs`.

- [ ] **Step 1: Write the failing tests**

One case per row of spec §3's 7-row table, plus `shows_chrome` per variant, plus
the derive-exercise A3 requires so `NoIdentity`'s `#[derive]` is not an
uncovered host line.

```rust
use crate::taglist::TagCtx;
use common::test_support::parse_username;

fn for_user() -> TagCtx {
    TagCtx::ForUser(parse_username("bob"))
}

#[test]
fn paint_reports_failure_on_the_error_axis() {
    with_owner(|| {
        let state = TimelineState::default();
        state.fail(WebError::validation("boom"));
        assert_eq!(
            state.paint(Some(TagCtx::SiteWide)),
            Err(WebError::validation("boom")),
            "failure outranks context"
        );
        assert_eq!(state.paint(None), Err(WebError::validation("boom")));
    });
}

#[test]
fn paint_reports_loading_before_the_first_load() {
    with_owner(|| {
        let state = TimelineState::default();
        assert_eq!(state.paint(Some(TagCtx::SiteWide)), Ok(TimelinePaint::Loading));
        assert_eq!(state.paint(None), Ok(TimelinePaint::Loading));
    });
}

#[test]
fn paint_reports_unidentified_when_the_fetch_found_nobody() {
    with_owner(|| {
        let state = TimelineState::default();
        state.unidentified();
        assert_eq!(
            state.paint(Some(TagCtx::SiteWide)),
            Ok(TimelinePaint::Unidentified),
            "a fetch-determined absence outranks a present context"
        );
    });
}

#[test]
fn paint_reports_rows_once_loaded_with_a_context() {
    with_owner(|| {
        let state = TimelineState::default();
        state.adopt(page_with(vec![sample_summary()], None, None, false));
        assert_eq!(
            state.paint(Some(TagCtx::SiteWide)),
            Ok(TimelinePaint::Rows(TagCtx::SiteWide))
        );
        assert_eq!(state.paint(Some(for_user())), Ok(TimelinePaint::Rows(for_user())));

        state.status.set(LoadStatus::InFlight);
        assert_eq!(
            state.paint(Some(TagCtx::SiteWide)),
            Ok(TimelinePaint::Rows(TagCtx::SiteWide)),
            "a load-more in flight keeps painting rows"
        );
    });
}

#[test]
fn paint_reports_unidentified_when_the_route_context_is_absent() {
    with_owner(|| {
        let state = TimelineState::default();
        state.adopt(page_with(vec![sample_summary()], None, None, false));
        assert_eq!(state.paint(None), Ok(TimelinePaint::Unidentified));

        state.status.set(LoadStatus::InFlight);
        assert_eq!(state.paint(None), Ok(TimelinePaint::Unidentified));
    });
}

// Projector coincidence, pinned at unit level: a seeded page must paint ROWS on its
// very first `paint()`, never `Loading`. `adopt_seed` runs synchronously in the
// component body before first render, so if this ever returned `Loading` a
// projector-painted page would flash its loading placeholder over server-rendered
// content — the #653 class. Fails in milliseconds; T6's CLS probes are the
// browser-level backstop.
#[test]
fn a_seeded_timeline_paints_rows_immediately_never_loading() {
    with_owner(|| {
        let state = TimelineState::default();
        state.adopt_seed(Some(page_with(vec![sample_summary()], None, None, false)));
        assert_eq!(
            state.paint(Some(TagCtx::SiteWide)),
            Ok(TimelinePaint::Rows(TagCtx::SiteWide)),
            "a projector-seeded page must never paint Loading"
        );
    });
}

#[test]
fn shows_chrome_for_every_paint() {
    assert!(TimelinePaint::Loading.shows_chrome());
    assert!(TimelinePaint::Rows(TagCtx::SiteWide).shows_chrome());
    assert!(!TimelinePaint::Unidentified.shows_chrome());
}

// A3's final bullet: `NoIdentity` is only *matched* in the wasm gate body, so without
// this its derive line is an uncovered host line and A4 forbids a marker.
#[test]
fn no_identity_variants_are_distinct_and_copyable() {
    let blank = NoIdentity::Blank;
    let redirect = NoIdentity::Redirect("/login");
    assert_ne!(blank, redirect);
    assert_eq!(redirect, redirect, "Copy + PartialEq");
    assert_eq!(format!("{blank:?}"), "Blank");
}

// Same reason, for `TimelinePaint`: `assert_eq!` only formats on FAILURE, so the tests
// above never invoke its `Debug`. Invoke it explicitly or the derive is uncovered.
#[test]
fn timeline_paint_is_debug_printable() {
    assert_eq!(format!("{:?}", TimelinePaint::Loading), "Loading");
    assert_eq!(format!("{:?}", TimelinePaint::Unidentified), "Unidentified");
    assert!(format!("{:?}", TimelinePaint::Rows(TagCtx::SiteWide)).contains("Rows"));
}
```

- [ ] **Step 2: Run the tests, verify they fail**

```
cargo nextest run -p web timeline::state
```

Expected: FAIL — `TimelinePaint`, `NoIdentity`, `paint`, `shows_chrome`
undefined.

- [ ] **Step 3: Implement against the tests**

Add the two enums with the derives from spec §3 (`TimelinePaint`:
`Clone, Debug, PartialEq, Eq`; `NoIdentity`:
`Clone, Copy, Debug, PartialEq, Eq`), `shows_chrome()`, and `paint()` to the §3
fold table. All 7 rows plus both `shows_chrome` outcomes are pinned above, so
the tests determine the bodies. Add `use crate::taglist::TagCtx;` to
`state.rs`'s **production** imports (the test block above imports it too).
Export all three from `timeline/mod.rs`'s ungated `pub use state::{…}`.

- [ ] **Step 4: Run the tests, verify they pass**

```
cargo nextest run -p web timeline::state
cargo clippy -p web --all-features --all-targets -- -D warnings
```

Expected: PASS, clippy clean.

- [ ] **Step 5: Commit**

```bash
devtool run -- cargo xtask check
git status --porcelain
git add web/src/timeline
git commit -m "feat(timeline): host-tested paint fold for the timeline gate (#671)"
```

---

### Task 5: `TimelineGate` and `wire_timeline_resolve`

Wasm-only, so no host test is possible (`Effect` does not run host-side). The
contract is the signature plus wasm-clippy; T7–T9's e2e are what exercise it.

**Files:**

- Modify: `web/src/timeline/component.rs`
- Modify: `web/src/timeline/mod.rs` (gated re-export)

**Interfaces:**

- Consumes: T4's `TimelinePaint`, `NoIdentity`, `paint`, `shows_chrome`; T3's
  `apply`.
- Produces:
  `wire_timeline_resolve(state: TimelineState, initial_page: Resource<WebResult<TimelinePage>>)`
  and the `TimelineGate` component to spec §3's exact prop list. Both added to
  the `#[cfg(target_arch = "wasm32")] pub use component::{…}` list.

- [ ] **Step 1: Write both items**

Copy spec §3's `wire_timeline_resolve` and `TimelineGate` verbatim, including
the two-region body. Add `use leptos_router::components::Redirect;` and
`use super::state::{NoIdentity, TimelinePaint};`.

Two doc comments carry invariants no test can express — write them out:

- On the `paint` memo: D9. `status` is written on every refetch (`→ Idle`) and
  every load-more (`→ InFlight → Idle`); reading `status` raw here would re-run
  the closure on each write and **remount `TimelineRows`, rebuilding every
  `PostCard` on each paginate**. The memo dedupes, so only a real transition
  re-paints.
- On the `show_chrome` memo and its sibling region: D8. Chrome is a **separate**
  region rather than `{children}` inside each arm, because emitting it per-arm
  tears the subtree down and rebuilds it on every `Loading → Rows`. For `home`
  that subtree is the `inner_html` masthead — projector-coincident markup
  (ADR-0041 §2), the class of bug #653 was. The memo dedupes `true → true`, so
  **for home** it is built once and survives the transition. Scope the claim to
  home: `cockpit`'s children read `username`, which flips `None → Some` at the
  same moment the status settles, so its subtree _is_ rebuilt across that
  transition — identical to today, and not something this region can prevent.

- [ ] **Step 2: Verify it compiles for both targets**

```
devtool run -- cargo xtask check --no-test
cargo clippy -p web --all-features --all-targets -- -D warnings
```

Expected: both clean.

**If either fails on one of the two unprecedented patterns, take the spec §3
fallback — do not redesign:**

- `#[prop(default = Signal::derive(…))]` →
  `#[prop(optional)] tag_context: Option<Signal<Option<TagCtx>>>`, resolved to
  `SiteWide` in the body. `#[prop(optional)]` on an `Option<T>` field
  auto-strips the `Option` in the setter
  (`leptos_macro-0.8.17/src/component.rs:1033`), so call sites still pass
  `Signal<Option<TagCtx>>` with no `Some(…)` wrapper.
- `children: Option<ChildrenFn>` → **`Option<BoxedChildrenFn>`**
  (`leptos-0.8.19/src/children.rs:160`), **not** `ViewFn` — `ViewFn` has no
  `ToChildren` impl (`children.rs:249-265`) so it cannot occupy `children`
  position at all; it would have to become a named `chrome=` prop, changing
  every call site. `BoxedChildrenFn` is not `Clone`, so on this path
  `children.clone().map(|c| c())` becomes `children.as_ref().map(|c| c())`.

Both are unlikely to fire: `impl ToChildren for ChildrenFn` exists
(`children.rs:118`), and the `#[prop(default = …)]` expression is evaluated in
the caller's owner at `build()`.

- [ ] **Step 3: Commit**

```bash
devtool run -- cargo xtask check
git status --porcelain
git add web/src/timeline
git commit -m "feat(timeline): add TimelineGate and wire_timeline_resolve (#671)"
```

---

### Task 6: Layout-shift probes across the projector → mount transition

**Written before the sweeps, deliberately.** These probes must pass on the
**pre-sweep** code and keep passing through Tasks 7–9. A regression test
authored after the change only documents the end state; one that is green before
and after is what actually proves preservation.

This closes a real gap. Tasks 7–9 restructure the CSR side of four
projector-painted routes, and the existing e2e assert _content_ after hydration
settles — they would pass through a visible first-paint flash. That is not
hypothetical: **#653 was exactly such a flash on the tag pages and the suite did
not catch it.** `expectNoShiftAcrossMount` exists for this
(`end2end/tests/layout-shift.ts:47`, delivered by the now-closed #202) and
`authed-cls.spec.ts:42` is still its only caller.

**Files:**

- Create: `end2end/tests/timeline-cls.spec.ts`

**Interfaces:**

- Consumes: `expectNoShiftAcrossMount(page, probe)` and `MountShiftProbe`
  (`end2end/tests/layout-shift.ts:47`, `:16`). It holds the
  `**/pkg/jaunder*.wasm` request so the projector's first paint stays frozen,
  samples each target's `boundingBox()`, releases, waits for
  `body[data-hydrated]` + `document.fonts.ready`, re-samples, and asserts
  `|Δx|, |Δy| <= tolerancePx`. No timers — parallel-safe under `workers>1`.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Write the four probes**

One `test` per projector-painted timeline route. `/app` is **excluded** — it is
`no-store` and never projector-painted (`cockpit/component.rs:3-6`), so there is
nothing to coincide with.

Keep `tolerancePx` at its default `0` (exact). Every probe supplies an
`afterMount` assertion, because the helper's docstring warns a green result must
not be able to be a no-op — prove the mount actually happened by asserting
something only the reactive tree emits (`.j-scroll`, which `TimelineRows` alone
renders, `timeline/component.rs:117`).

Targets per route: the chrome element that must not move, plus the first post
row (so a shift in the rows region is caught too). Use the seeded content the
harness already creates — reuse `posts.spec.ts`'s fixture pattern for a user
with a tagged, published post, and resolve the concrete username/tag before
writing the URLs rather than hardcoding.

```ts
test("/ : masthead and first row do not shift across mount", async ({
  page,
}) => {
  await expectNoShiftAcrossMount(page, {
    url: "/",
    targets: (p) => [
      { name: "masthead hero", locator: p.locator(".j-hero") },
      { name: "first post", locator: p.locator("article.j-post").first() },
    ],
    afterMount: async (p) => {
      // Proves the reactive tree really mounted, so a zero-shift result cannot be
      // a frozen-projector no-op.
      await expect(p.locator(".j-scroll")).toHaveCount(1);
    },
  });
});
```

Repeat for `/tags/<tag>`, `/~<user>`, and `/~<user>/tags/<tag>`, each with
`.j-topbar` in place of `.j-hero` (those three paint a `Topbar`, not the
masthead) and the same `first post` + `afterMount` shape.

- [ ] **Step 2: Run them against the pre-sweep tree, verify they PASS**

```
cargo xtask e2e-local timeline-cls.spec.ts
```

Expected: **PASS**. This is the baseline — the projector and the current CSR
tree already coincide, and that is what Tasks 7–9 must not break. A failure here
means the probe is wrong (bad selector, unseeded route, content the harness did
not create), **not** that the codebase has a flash — diagnose the probe before
touching anything else.

- [ ] **Step 3: Commit**

```bash
devtool run -- cargo xtask check
git status --porcelain
git add end2end/tests/timeline-cls.spec.ts
git commit -m "test(e2e): layout-shift probes for the four projector-painted timelines (#671)"
```

---

### Task 7: Sweep the three timeline pages

**Files:**

- Modify: `web/src/posts/component.rs` — `UserTimelinePage` (`:1025-1148`),
  `SiteTagPage` (`:1573-1678`), `UserTagPage` (`:1681-1822`)

**Interfaces:**

- Consumes: T5's `TimelineGate` / `wire_timeline_resolve`, T3's `adopt_seed`.
- Produces: nothing new. Removes three `#[expect(clippy::too_many_lines)]` and
  three `loaded` signals; `TimelineRows`/`Memo`/`Effect` no longer appear in
  these three components.

Wasm-only view code, so the contract is clippy plus the unmodified e2e (A5).
Each page's target body is written out below because no test can pin it.

**These three pages pass no `children`.** `Topbar`, `FeedDiscovery`,
`RsdDiscovery`, and `SubscribeButton` already sit _outside_ the gated region
today and stay **siblings** of `<TimelineGate/>`, not children — folding them in
would drop them from the error and `Unidentified` arms, and for `FeedDiscovery`
that would break head-level feed autodiscovery.

- [ ] **Step 1: Rewrite `SiteTagPage`** (simplest — no `tag_context`)

Delete its `#[expect(clippy::too_many_lines)]`, its `loaded` signal, its
`Effect`, and its `read_error` memo. Keep the `params`/`tag` memo,
`mutate_version`/`on_mutate`, `initial_page`, and `on_load_more` exactly as they
are. The seed block and the view become:

```rust
    let state = TimelineState::default();
    // Public projector seed (#178/#179): adopt the seeded posts for a matching tag so
    // first paint shows content — guarded so a client-side nav to a different tag
    // ignores the initial URL's seed; the reactive fetch still runs.
    state.adopt_seed(match use_context::<Option<PageSeed>>().flatten() {
        Some(PageSeed::SiteTag { tag: seed_tag, page })
            if tag.get_untracked().as_ref() == Some(&seed_tag) => Some(page),
        _ => None,
    });

    wire_timeline_resolve(state, initial_page);

    // The canonical tag for the heading (a newtype is not `IntoRender`), or empty for an
    // unparseable segment — the page renders a validation error anyway.
    let read_tag = move || tag.get().map(|t| t.to_string()).unwrap_or_default();

    view! {
        {move || {
            tag.get().map(|tag| view! { <FeedDiscovery surface=FeedSurface::SiteTag { tag } /> })
        }}
        <Topbar title=move || format!("#{}", read_tag()) sub="Posts on this instance" />
        <TimelineGate
            state=state
            on_mutate=on_mutate
            on_load_more=on_load_more
            empty_text="No posts with this tag yet."
        />
    }
```

Note the outer `{move || view! { … }}` wrapper around the old `FeedDiscovery`
fragment (`:1652-1658`) is redundant and collapses to one closure.

- [ ] **Step 2: Rewrite `UserTimelinePage`**

Same deletions. Its `username` memo, `initial_page`, `on_load_more`,
`FeedDiscovery`/ `RsdDiscovery`/`Topbar`/`SubscribeButton` chrome and
`display_username` are unchanged.

```rust
    let state = TimelineState::default();
    // Public projector seed (#178/#179): if the server painted this profile, adopt its
    // posts so first paint shows content. Guarded on the username so a client-side
    // navigation to a *different* profile ignores the initial URL's seed.
    state.adopt_seed(match use_context::<Option<PageSeed>>().flatten() {
        Some(PageSeed::Profile { username: seed_user, page })
            if username.get_untracked().as_ref() == Some(&seed_user) => Some(page),
        _ => None,
    });

    wire_timeline_resolve(state, initial_page);
```

and the third view fragment (`:1125-1146`) becomes:

```rust
        <TimelineGate
            state=state
            on_mutate=on_mutate
            on_load_more=on_load_more
            tag_context=Signal::derive(move || username.get().map(TagContext::ForUser))
        />
```

The `Signal::derive` reproduces the old
`match username.get() { Some(user) => rows with ForUser, None => () }` exactly:
`None` folds to `TimelinePaint::Unidentified`, which the default
`NoIdentity::Blank` renders as nothing.

- [ ] **Step 3: Rewrite `UserTagPage`**

Same deletions; `username`/`tag` memos, `initial_page`, `on_load_more`, chrome,
`read_username` and `read_tag` unchanged.

```rust
    let state = TimelineState::default();
    // Public projector seed (#178/#179): adopt the seeded posts for a matching
    // username+tag so first paint shows content; the reactive fetch still runs.
    state.adopt_seed(match use_context::<Option<PageSeed>>().flatten() {
        Some(PageSeed::UserTag { username: seed_user, tag: seed_tag, page })
            if username.get_untracked().as_ref() == Some(&seed_user)
                && tag.get_untracked().as_ref() == Some(&seed_tag) => Some(page),
        _ => None,
    });

    wire_timeline_resolve(state, initial_page);
```

and the third view fragment (`:1798-1820`):

```rust
        <TimelineGate
            state=state
            on_mutate=on_mutate
            on_load_more=on_load_more
            tag_context=Signal::derive(move || username.get().map(TagContext::ForUser))
            empty_text="No posts with this tag yet."
        />
```

- [ ] **Step 4: Prune the imports**

`posts/component.rs:30` drops `TimelineRows` and gains `TimelineGate`,
`wire_timeline_resolve`. If no other component in the file still uses
`Memo`/`Effect`, let clippy tell you — do not guess.

- [ ] **Step 5: Verify**

```
devtool run -- cargo xtask check --no-test
cargo clippy -p web --all-features --all-targets -- -D warnings
```

Expected: clean. A clean run **proves A1**: with the `#[expect]` gone, a page
still over 100 lines would fail `-D warnings`; and had a page dropped under
threshold with the attribute still present, the unfulfilled expectation would
fail too.

- [ ] **Step 6: Run the affected e2e, verify behavior is unchanged** (A5)

```
cargo xtask e2e-local posts.spec.ts
cargo xtask e2e-local profile.spec.ts
cargo xtask e2e-local timeline-cls.spec.ts
```

Expected: PASS, **with no edits to any spec file**. If a spec needs editing,
behavior changed — stop and diagnose. `timeline-cls.spec.ts` was green before
this task (T6); if one of its probes now fails, this sweep moved
projector-coincident markup on that route — the #653 class. Fix the sweep, never
the tolerance.

- [ ] **Step 7: Commit**

```bash
devtool run -- cargo xtask check
git status --porcelain
git add web/src/posts/component.rs
git commit -m "refactor(posts): sweep the three timeline pages onto TimelineGate (#671)"
```

---

### Task 8: `home` onto the gate

The risk concentration. The e2e is written **first**, as the failing test.

**Files:**

- Modify: `end2end/tests/helpers.ts` (add a releasable stall helper)
- Modify: `end2end/tests/posts.spec.ts` (add the new test)
- Modify: `web/src/home/component.rs`

**Interfaces:**

- Consumes: T5's `TimelineGate`/`wire_timeline_resolve`, T3's `adopt_seed`.
- Produces: `stallServerFn(page, endpoint) -> () => Promise<void>` in
  `helpers.ts` — routes `**/api/${endpoint}`, holds each request until the
  returned release fn is called, then continues it.

- [ ] **Step 1: Add the stall helper**

`failServerFn` (`helpers.ts:105-112`) is the existing `page.route` precedent;
this is its releasable sibling. It must hold the request rather than failing it,
so observing the loading arm is deterministic rather than a race (spec A6,
second bullet).

```ts
/**
 * Hold every call to `**​/api/${endpoint}` open until the returned release fn runs, then
 * let it continue normally.  Unlike `failServerFn` (which fulfills a 500 immediately),
 * this makes a *loading* state deterministically observable instead of racing the fetch.
 */
export async function stallServerFn(
  page: Page,
  endpoint: string,
): Promise<() => Promise<void>> {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route(`**/api/${endpoint}`, async (route) => {
    await gate;
    await route.continue();
  });
  return async () => {
    release();
  };
}
```

Deliberately **no** `page.unroute` in the release fn: unrouting immediately
after `release()` races the still-suspended handler's `route.continue()`, and
nothing in the test needs the route torn down. (If an assertion throws before
`release()` the handler stays parked until teardown — acceptable for a single
test, and the alternative would swallow the real failure.)

- [ ] **Step 2: Write the failing e2e**

Add `stallServerFn` to the existing `./helpers` import at
`end2end/tests/posts.spec.ts:8` (`expect` already comes from `./fixtures`), then
append the test. A bare `{ page }` fixture is valid in this file (`:317`,
`:413`, `:451`, `:502`).

Every load-bearing choice from spec A6 is encoded. The entry URL is **not** `/`:
home reads its seed from the initial document at mount (`csr/src/lib.rs:30-44`)
and `/login` is SPA-shell, so `seed = None` (`csr/src/lib.rs:28`) — and a _full
load_ of `/` is always seeded, since `site_timeline` has no shell fallback
(`server/src/projector/mod.rs:200-228`). The nav is a client-side click on
`.j-brand`, a plain `<a href="/">` in the anonymous `inner_html` sidebar
(`sidebar/markup.rs:47`; the authed twin is `sidebar/component.rs:89`) —
`leptos_router`'s document-level click handler makes a raw anchor a client-side
navigation, which `auth.spec.ts:111-128` already proves for `a[href="/logout"]`.
The stall is registered **before** the click, or the fetch escapes the route.
The anchor for the chrome assertion is the masthead, not `.j-scroll`, which
`TimelineRows` alone emits (`timeline/component.rs:117`).

```ts
test("unseeded client-nav to / paints Loading with the masthead intact", async ({
  page,
}) => {
  // Enter on a NON-`/` URL. Home reads its seed from the initial document, and `/login`
  // is the SPA shell (no seed) — a full load of `/` is ALWAYS projector-seeded, so a
  // client-side nav is the only way to reach the unseeded Loading arm.
  await goto(page, "/login");

  // A full document load would wipe this; a client-side nav preserves it (auth.spec.ts:84).
  await page.evaluate(() => {
    (window as Window & { __jaunderNoReload?: boolean }).__jaunderNoReload =
      true;
  });

  // Register the stall BEFORE the click or the fetch escapes the route.
  const release = await stallServerFn(page, "list_local_timeline");
  await click(page, ".j-brand");

  // Loading arm: the gate paints `.j-loading` (it painted "No posts yet." before #671),
  // and D8's sibling chrome region keeps the masthead up alongside it.
  await waitForSelector(page, ".j-loading");
  const masthead = page.locator(".j-hero");
  await expect(masthead).toBeVisible();
  await expect(page.locator(".j-scroll")).toHaveCount(0);

  // The nav really was client-side — otherwise the Loading arm above was a full-load
  // artifact and this test proves nothing about the gate.
  const sameDocument = await page.evaluate(
    () =>
      (window as Window & { __jaunderNoReload?: boolean }).__jaunderNoReload ===
      true,
  );
  expect(sameDocument).toBe(true);

  // Stamp the live node, then let the fetch through.
  await masthead.evaluate((el) => {
    el.setAttribute("data-j-probe", "1");
  });
  await release();

  // Rows arm: the SAME masthead node survives the transition — a per-arm `{children}`
  // would have torn it down and rebuilt it, losing the stamp (the #653 hazard class).
  await waitForSelector(page, ".j-scroll");
  await expect(page.locator(".j-hero[data-j-probe='1']")).toHaveCount(1);
  await expect(page.locator(".j-loading")).toHaveCount(0);

  // …and still precedes the rows. Playwright locators do not express document order, so
  // ask the DOM directly.
  const mastheadFirst = await page.evaluate(() => {
    const hero = document.querySelector(".j-hero");
    const scroll = document.querySelector(".j-scroll");
    if (!hero || !scroll) return false;
    // DOCUMENT_POSITION_FOLLOWING === 4
    return (hero.compareDocumentPosition(scroll) & 4) !== 0;
  });
  expect(mastheadFirst).toBe(true);
});
```

`.j-hero` is the hero half of the masthead (`web/src/home/render.rs:11`); it
sits inside the `inner_html` wrapper, so stamping it proves the whole wrapped
subtree survived — a torn-down wrapper takes `.j-hero` with it.

- [ ] **Step 3: Run the e2e, verify it fails**

```
cargo xtask e2e-local posts.spec.ts
```

Expected: FAIL — home still renders the empty-rows placeholder, so `.j-loading`
never appears and `waitForSelector` times out.

- [ ] **Step 4: Rewrite `HomePage`**

Change the import at `:10` from
`use crate::timeline::{TimelineRows, TimelineState};` to
`use crate::timeline::{TimelineGate, TimelineState};` — `spawn_load_more` stays
fully-qualified at its call site, as today.

Delete the `Effect` (`:43-50`) and the `read_error` memo (`:60`). Keep
`FeedDiscovery`, `refresh_version`/`on_mutate`, `initial_page`, `on_load_more`,
and `masthead` unchanged — in particular do **not** move the `masthead` binding
or alter its `<div>`, only relocate it into the gate's `children` slot.

```rust
    let state = TimelineState::default();
    // Public projector seed (#178/#179): `/` is the anonymous site (Local) timeline for
    // EVERYONE, including the authenticated owner (#181, ADR-0044 D10) — adopt the seed
    // as the initial state so first paint shows content, no swap. The seed variant itself
    // identifies the page, so no URL guard is needed here.
    state.adopt_seed(match leptos::prelude::use_context::<Option<PageSeed>>().flatten() {
        Some(PageSeed::SiteTimeline(page)) => Some(page),
        _ => None,
    });

    // …refresh_version / on_mutate / initial_page unchanged…

    wire_timeline_resolve(state, initial_page);

    let on_load_more = Callback::new(move |()| {
        crate::timeline::spawn_load_more(state, list_local_timeline);
    });

    let masthead = super::render::render_masthead();

    view! {
        <FeedDiscovery surface=FeedSurface::Site />
        <TimelineGate state=state on_mutate=on_mutate on_load_more=on_load_more>
            <div style="display:contents" inner_html=masthead.clone()></div>
        </TimelineGate>
    }
```

- [ ] **Step 5: Run the e2e, verify it passes**

```
devtool run -- cargo xtask check --no-test
cargo xtask e2e-local posts.spec.ts
```

Expected: clippy clean, e2e PASS — including the pre-existing tests in that
file, unmodified.

**Diagnose a failure by which assertion broke — the causes are different and
only one of them implicates the design.** Do not weaken any assertion.

- `waitForSelector(".j-loading")` **times out** → the Loading arm was never
  reached. Two causes, both test bugs, not design bugs: the click was a full
  document reload (the `sameDocument` probe would also be false — re-check that
  `.j-brand` resolves to a real anchor), or `/` arrived seeded (the entry URL
  was `/`, or `/login` unexpectedly carried a seed). Fix the test.
- `sameDocument` is **false** → the nav was not client-side, so everything after
  it is a full-load artifact. Fix the test; the gate is not implicated.
- `.j-hero[data-j-probe='1']` **is absent** while the two above passed → the
  chrome subtree _was_ torn down and rebuilt across `Loading → Rows`. **This one
  means D8 is wrong.** Stop and revise the spec's D8 with what you observed; do
  not drop the stamp check to get green.

- [ ] **Step 6: Confirm the neighbours still pass**

```
cargo xtask e2e-local authed-flash.spec.ts
cargo xtask e2e-local authed-cls.spec.ts
cargo xtask e2e-local timeline-cls.spec.ts
```

Expected: PASS unmodified — these own `/`'s first-paint and layout-shift
behavior (A5, A10).

`timeline-cls.spec.ts`'s `/` probe is the important one here: this is the only
task where the masthead's `inner_html` div actually relocates (into the gate's
`children`), so its zero-shift assertion is the empirical check that the
relocation is invisible on a **seeded** load. Step 2's stamp check covers the
_unseeded_ path; together they cover both.

- [ ] **Step 7: Commit**

```bash
devtool run -- cargo xtask check
git status --porcelain
git add web/src/home/component.rs end2end/tests/helpers.ts end2end/tests/posts.spec.ts
git commit -m "refactor(home): put the local timeline on TimelineGate (#671)"
```

---

### Task 9: `cockpit` onto the gate

**Files:**

- Modify: `web/src/cockpit/component.rs`

**Interfaces:**

- Consumes: T5's `TimelineGate`, T3's `adopt`/`unidentified`/`fail`, T4's
  `NoIdentity`.
- Produces: nothing new. Deletes the `bounce` signal and the `read_error` memo.

D11: cockpit keeps its own `Effect` — its `Resource` payload is
`WebResult<Option<(Username, TimelinePage)>>` and it must publish `username`
behind the #591 anti-remount guard, neither of which `wire_timeline_resolve` can
do. D7: its `Topbar` + `InlineComposer` become the gate's `children`, moved
**wholesale** as the existing `match read_username()` so `InlineComposer` keeps
its `username: Username` prop.

- [ ] **Step 1: Rewrite the Effect and the view**

Keep `state`, `username`, `refresh_version`/`on_mutate`, `session`,
`initial_page`, and `on_load_more` exactly as they are. Delete `bounce` (`:21`),
`read_bounce` (`:80`) and `read_error` (`:79`).

**Move only the chrome into `children`.** Today's `match read_username()` arms
also carry `<p class="j-loading">` (`:95`) and `<TimelineRows/>` (`:103`); both
now belong to the **gate**, so moving the match verbatim would emit each twice —
a duplicated loading paragraph and a doubled feed. That would make
`posts.spec.ts:530`'s `'button:has-text("Load more")'` click a strict-mode
violation and break the `toHaveCount` at `:524`. The block below is already
correct; do not "restore" the two dropped lines.

```rust
    // Copy the resolved Resource into the timeline signals once it loads. This Effect stays
    // page-specific (#671 D11): the payload carries the session-confirmed identity, which
    // no shared helper can publish.
    Effect::new(move |_| {
        if let Some(result) = initial_page.try_get().flatten() {
            match result {
                Ok(Some((user, page))) => {
                    // Only set `username` when it actually changes: a spurious set would
                    // re-run the children closure and REMOUNT InlineComposer, wiping its
                    // publish/draft flash (a re-fetch fires on every publish via
                    // `refresh_version`) — #591.
                    if username.get_untracked().as_ref() != Some(&user) {
                        username.set(Some(user));
                    }
                    state.adopt(page);
                }
                // Anonymous / expired (D6): `status` carries the bounce, and the gate's
                // `no_identity` prop turns it into the `/login` redirect.
                Ok(None) => state.unidentified(),
                Err(err) => state.fail(err),
            }
        }
    });

    let read_username = move || username.get();

    view! {
        <TimelineGate
            state=state
            on_mutate=on_mutate
            on_load_more=on_load_more
            no_identity=NoIdentity::Redirect("/login")
        >
            {move || match read_username() {
                None => view! { <Topbar title="Home" /> }.into_any(),
                Some(user) => {
                    view! {
                        <Topbar title="Home" sub="Your home feed" />
                        <InlineComposer username=user on_publish=refresh_version.write_only() />
                    }
                        .into_any()
                }
            }}
        </TimelineGate>
    }
```

This reproduces all four current outcomes: the redirect and the error banner
paint alone (chrome is suppressed in those arms by `shows_chrome()`), loading
paints `Topbar` + `.j-loading`, and loaded paints `Topbar` + composer + rows.

- [ ] **Step 2: Prune the imports**

Drop `leptos_router::components::Redirect` (`:11`) and `TimelineRows` (`:14`);
add `TimelineGate` and `NoIdentity`. Let clippy confirm.

- [ ] **Step 3: Verify** (A7)

```
devtool run -- cargo xtask check --no-test
cargo clippy -p web --all-features --all-targets -- -D warnings
rg -n 'bounce' web/src/cockpit/component.rs
```

Expected: both clippy runs clean; `rg` finds **nothing** (A7).

- [ ] **Step 4: Run the cockpit e2e** (A5)

```
cargo xtask e2e-local authed-flash.spec.ts
cargo xtask e2e-local auth.spec.ts
cargo xtask e2e-local posts.spec.ts
cargo xtask e2e-local media.spec.ts
```

Expected: PASS unmodified — in particular `"anonymous: /app bounces to /login"`
(`authed-flash.spec.ts:83`) and
`"owner: /app cockpit boots straight into the personalized feed"` (`:53`).

`posts.spec.ts` and `media.spec.ts` are **not optional here.**
`posts.spec.ts:502` ("cockpit /app shows the authenticated home feed with
pagination") is the only e2e that exercises cockpit's rows arm and load-more
(`toHaveCount` at `:524`/`:532`, the click at `:530`) — it is the behavioral
backstop for `append`/D10 and the test that catches an over-broad `children`.
`media.spec.ts:90` depends on the cockpit composer being present. The four tests
named above would all still pass with a doubled feed, so they cannot catch that
mistake alone.

- [ ] **Step 5: Commit**

```bash
devtool run -- cargo xtask check
git status --porcelain
git add web/src/cockpit/component.rs
git commit -m "refactor(cockpit): put the home feed on TimelineGate (#671)"
```

---

### Task 10: Record the pattern as an ADR draft

**Files:**

- Create: `docs/adr/drafts/reactive-paint-fold.md`

**Interfaces:** none — documentation only. Numberless by design;
`cargo xtask adr promote` assigns the number and updates `docs/README.md` at
ship (`jaunder-ship`).

- [ ] **Step 1: Write the draft** (A9)

Follow the house ADR shape (Context / Decision / Consequences) used by
`docs/adr/0070-web-vertical-wasm-only-component-files.md`. Record D4, D6, and D8
as one reusable pattern, and cite #306, #520, ADR-0070 §6, ADR-0041 §2, and
#671:

- **Context.** ADR-0070 §6 requires pure, host-testable logic to live in
  ungated, coverage-measured files, and #520 retired the `#[component]` coverage
  exemption — a wasm-only component never host-compiles, so its branching is
  invisible to the gate. #306 will _fail_ the gate on over-complex component
  bodies but prescribes no remedy. Neither document says how to extract a
  **render decision**, as opposed to logic, so each page re-derived its own in
  an uncovered view closure.
- **Decision.** (1) A reactive state bundle exposes a host-tested fold returning
  a typed render decision, failure on `Result`'s error axis. (2) The component
  body is a `Memo` plus a bare `match`, one arm per variant — thin by
  construction, so #306's guard passes without special-casing. (3) Per-page
  variation within a shared arm travels as a **data enum**, not a closure prop,
  so the choice stays host-testable. (4) Chrome that must survive a paint
  transition lives in its own memo-gated sibling region, never as `{children}`
  repeated per arm — repeating it tears the subtree down and rebuilds it, which
  on projector-coincident markup is the ADR-0041 §2 / #653 flash hazard.
- **Consequences.** State transitions and the render decision become
  coverage-measured; adding a paint outcome is a compile error at every gate;
  the pattern's cost is one enum and one fold per widget. First instance:
  `web/src/timeline` (#671).

- [ ] **Step 2: Format and verify the gate sees it**

```
prettier -w docs/adr/drafts/reactive-paint-fold.md
devtool run -- cargo xtask check
```

Expected: clean. (The pre-commit hook runs prettier and restages prose, so
formatting first avoids a dirty tree after the commit.)

- [ ] **Step 3: Commit**

```bash
git add docs/adr/drafts/reactive-paint-fold.md
git commit -m "docs(adr): draft the reactive paint-fold pattern (#671)"
```

---

## Final verification

- [ ] **Full local gate**

```
devtool run -- cargo xtask validate
```

Expected: green, including coverage and all four
`{sqlite,postgres}×{chromium,firefox}` e2e combos (A4) — which is where
`timeline-cls.spec.ts`'s four probes run against every backend/browser pair
(A10). Run it **foreground with a long timeout** — background runs get killed.
If a single firefox combo exits 124, re-confirm that one combo alone before
believing it (host-contention flake, not a defect).

A geometry probe is the one check here that could fail for a browser-specific
reason rather than a code reason. If a probe fails on **firefox only** while
chromium passes, sample the observed Δ before doing anything else — and if it
turns out to be sub-pixel rounding, loosen that probe's `tolerancePx` with a
comment citing the value and the browser, per the helper's "start exact, loosen
on evidence" rule (`layout-shift.ts:36-40`). A probe failing on **both**
browsers is a real shift; fix the code.

- [ ] **Spot-check the acceptance criteria that no command proves**

```
rg -n 'too_many_lines' web/src/posts/component.rs
rg -n 'cov:ignore|crap:allow' web/src/timeline
rg -n 'target_arch' web/src/timeline
```

Expected: the first matches only `PostCreateForm` and `EditPostPage` (A1); the
second matches nothing (A4); the third matches only the two `mod`/`use` lines in
`mod.rs` (A2).

- [ ] **Archive check.** The plan and spec are archived by `jaunder-ship`; do
      not move them by hand.
