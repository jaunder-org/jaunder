# Spec — #671: converge the five timeline pages on a host-tested paint fold

**Issue:** [#671](https://github.com/jaunder-org/jaunder/issues/671) —
`web(timeline): thin the three timeline pages onto a shared TimelineGate; host-cover TimelineState transitions`
**Branch:** `worktree-issue-671-timeline-gate` · **Fork anchor:**
`wt-base-issue-671`

## 1. Problem

`SiteTagPage`, `UserTagPage`, and `UserTimelinePage`
(`web/src/posts/component.rs`) are near-identical wiring around the
`crate::timeline` bundle, and all three carry
`#[expect(clippy::too_many_lines)]`. `home` and `cockpit` inline the same
machine a fourth and fifth time in slightly different shapes.

The load-bearing defect is not length, it is **what the wasm gate hides**.
`#329` split the pure value model (`TimelineCursor`, `LoadStatus`) into
host-tested `timeline/state.rs`, but left the reactive `TimelineState` signal
bundle in wasm-only `timeline/component.rs`. Since #520 there is no
`#[component]` coverage exemption and a wasm-only file never host-compiles at
all, so every one of `TimelineState`'s transitions — and the render decision
each page re-derives in a view closure — is invisible to the coverage gate and
asserted only by e2e.

Note for anyone extending this: `Effect` **does not run in a host test**
(`web/src/reactive/mod.rs:89`). Reactive _wiring_ is therefore permanently
e2e-only; what can be host-tested is the state transitions the wiring dispatches
to, and the fold that decides what to paint. That boundary is why the design
below puts every decision in `state.rs` and leaves only `Effect::new` /
`spawn_local` in `component.rs`.

## 2. Design decisions (resolved in interview)

**D1 — One host-compiled `timeline/state.rs`.** The reactive `TimelineState`
bundle moves out of `component.rs` and joins the pure value model in ungated,
coverage-measured `state.rs`. Precedent: `forms/field.rs` holds pure
`field_error<T>` beside the reactive `Field<T>` in one host file, with
`Owner::new()` tests; `tags/input_state.rs` likewise. Rejected: splitting
pure→`logic.rs` + reactive→`state.rs` to mirror `tags/input_logic.rs` +
`tags/input_state.rs` — naming symmetry not worth renaming a file nobody asked
to move, and `adopt()` calls `TimelineCursor::from_page` two lines away.

**D2 — "Never loaded" is a `LoadStatus` variant, not a parallel flag.**
`LoadStatus` gains `NeverLoaded` as its `#[default]`. Because `adopt()` now
settles the status to `Idle`, `resolve()` becomes identical to `adopt()` and
**is deleted**. The three pages' duplicated `let loaded = RwSignal::new(false)`
disappears. This extends the illegal-state-elimination `state.rs:47-50` already
applies to `loading && errored`: "idle but never loaded" becomes unrepresentable
too. (The comment at `state.rs:2-4` is about _where the bundle lives_, not this
gap — no existing comment names it; the three pages each re-derive it in prose
instead, e.g. `posts/component.rs:1056-1061`.)

**D3 — Errors stay typed, on `Result`'s error axis.** `LoadStatus::Failed`
carries `WebError`, not a pre-rendered `String` (`WebError` derives
`Clone + PartialEq + Eq` — `web/src/error.rs:16`). `into_failure()` returns
`Option<WebError>`; `.to_string()` happens once, at the render. No page threads
an anonymous `Option<String>` around.

**D4 — The render decision is a host-tested fold.** `TimelineState::paint()`
returns `WebResult<TimelinePaint>` — failure on the error axis, the success
outcomes named by the enum. The component body reduces to a `Memo` plus a bare
`match`, one arm per variant. There is no standalone "failure memo": the memos
exist solely to dedupe re-paints (see D9), not to carry an error.

**D5 — Two sources of "no identity", one paint arm.** Absence is _route-derived_
for the user pages (a `Memo` over the URL segment, read synchronously) and
_fetch-derived_ for cockpit (`Ok(None)` off the session reconcile — a load
outcome, so it becomes `LoadStatus::Unidentified`). `paint()` folds both into
`TimelinePaint::Unidentified`. Note the route pages' `Invalid username` /
`Invalid tag` case is **not** this arm — their `Resource` already yields
`Err(WebError::validation(…))`, so it paints as an error banner.

**D6 — Per-page variation in a shared arm travels as data, not a closure.** The
two distinct renderings of `Unidentified` are retained, parameterized by a
`NoIdentity` data enum prop (`Blank` | `Redirect(&'static str)`), defaulted so
four of five pages never mention it. A `ViewFn`/closure prop would push the
choice back into uncovered per-page code; a data enum keeps the gate body a bare
`match`, which is what #306's thin-component guard wants by construction.

**D7 — All five pages converge on `TimelineGate`; nothing moves out of it.**
Page-specific chrome that today lives _inside_ the gated region is passed as
`children`: `home`'s masthead, and `cockpit`'s `Topbar` + `InlineComposer`.
Cockpit keeps its `match read_username()` shape so `InlineComposer` keeps its
plain `username: Username` prop (`posts/component.rs:758`) and needs no
reshaping — but **only the chrome moves**. Today that match's arms also carry
`<p class="j-loading">` (`cockpit/component.rs:95`) and `<TimelineRows/>`
(`:103`); both now belong to the **gate**, so moving the match verbatim would
emit each twice — a duplicated loading paragraph and a doubled feed (two
`.j-scroll`, two "Load more", which makes `posts.spec.ts:530`'s click a
strict-mode violation). Siblings that already sit **outside** the gated region
stay siblings, not children: `home`'s `FeedDiscovery` (`home/component.rs:72`)
and the three pages' `Topbar` / `FeedDiscovery` / `RsdDiscovery` /
`SubscribeButton` (`posts/component.rs:1110-1124`, `:1651-1659`, `:1778-1797`).
Folding those in would silently drop them from the error and `Unidentified` arms
— for `FeedDiscovery` that would break head-level feed autodiscovery. Chrome
renders in the **loading and rows** arms only, never in the error or
`Unidentified` arms. That is what both pages already do: cockpit's early returns
(`cockpit/component.rs:85-90`) paint _only_ the `<Redirect>` or the error
banner, with no chrome, and home's error branch paints only the banner.
Relocating cockpit's chrome outside the gate was considered and **rejected** —
it would newly paint the topbar and composer in the error arm and during the
anonymous `/login` bounce window, three visible changes for no gain.

**D8 — Chrome is a sibling reactive region, not per-arm markup.** The gate body
is two regions: `{move || show_chrome.get().then(|| children())}` followed by
`{move || match paint.get() { … } }`, where `show_chrome` is its own
`Memo<bool>`. Emitting `{children}` inside each arm instead would **tear down
and rebuild** the chrome subtree on every `Loading → Rows` transition. For
`home` that subtree is the `inner_html` masthead — projector-coincident markup
(ADR-0041 §2), and #653 was exactly a first-paint regression in that class. A
memo-gated sibling region dedupes `true → true`, so **for `home`** the chrome is
built once and survives the transition untouched (A6 asserts this). The claim is
scoped to `home` deliberately: `cockpit`'s children read `username`
(`cockpit/component.rs:91`), which flips `None → Some` at the same moment status
goes `NeverLoaded → Idle`, so its `Topbar`/`InlineComposer` subtree _is_ rebuilt
across that transition — identical to today, and not something D8 can or should
prevent.

**D9 — The memos are re-paint dedupes, and they are load-bearing.** `status` is
written on every refetch (`→ Idle`) and every load-more (`→ InFlight → Idle`). A
raw `status` read in the gate's view closure would re-run it on each of those
writes and **remount `TimelineRows`, rebuilding every `PostCard` on each
paginate**. Documented at the memo, not re-derived per page (cf.
`home/component.rs:56-59`, `cockpit/component.rs:71-79`).

**D10 — Asymmetric failure semantics are preserved.** `apply()`
(initial/refetch) clears rows, cursor, and `has_more` on failure so a failed
timeline offers no "Load more". `append()` (load-more) sets the status only,
keeping the successfully-fetched earlier pages. Rejected: unifying them — that
is a behavior change smuggled into a thinning issue.

**D11 — `cockpit` keeps its own resolve `Effect`.** It calls the shared,
host-tested transitions (`adopt` / `unidentified` / `fail`) but keeps the
`username` publish and its anti-remount guard inline. Rejected: making
`wire_timeline_resolve` generic over an identity payload (four callers would
carry a shape they never read), and splitting cockpit's `Resource` so it fits
the simple helper (reintroduces the second reactive hop #591 deliberately
removed). Cockpit's `bounce` signal is deleted — `status` carries it.

**D12 — The pattern is recorded as an ADR.** A numberless draft in
`docs/adr/drafts/`, numbered at ship by `cargo xtask adr promote`. ADR-0070 §6
("ADR-0055's retained principles carry forward unchanged") mandates extracting
pure logic into ungated, coverage-measured files but says nothing about
extracting a _render decision_; #306 flags thick components without prescribing
a remedy. This is that remedy, and the timeline is its first instance.

## 3. Target shape

### `web/src/timeline/state.rs` — host-compiled, ungated, coverage-measured

```rust
pub struct TimelineCursor { … }                     // unchanged

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum LoadStatus {
    #[default] NeverLoaded,
    Idle,
    InFlight,
    Failed(WebError),
    Unidentified,
}
impl LoadStatus {
    pub fn is_in_flight(&self) -> bool;
    pub fn into_failure(self) -> Option<WebError>;
}

/// What the gate should paint. Failure travels on `WebResult`'s error axis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimelinePaint { Loading, Rows(TagCtx), Unidentified }
impl TimelinePaint {
    /// Whether the page's own chrome accompanies this paint (D8).
    pub fn shows_chrome(&self) -> bool;   // Loading | Rows => true, Unidentified => false
}

/// How a page renders the `Unidentified` arm (D6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoIdentity { Blank, Redirect(&'static str) }

#[derive(Clone, Copy, Default)]
pub struct TimelineState { rows, cursor, has_more, status }

impl TimelineState {
    pub fn adopt(&self, page: TimelinePage);                  // + status = Idle
    pub fn adopt_seed(&self, page: Option<TimelinePage>);     // projector seed or not
    pub fn apply(&self, result: WebResult<TimelinePage>);     // Ok → adopt, Err → fail
    pub fn fail(&self, error: WebError);                      // clears rows/cursor/has_more
    pub fn unidentified(&self);                               // clears; status = Unidentified
    pub fn append(&self, result: WebResult<TimelinePage>);    // see below
    pub fn begin_load_more(&self) -> Option<(Option<UtcInstant>, Option<PostId>)>;
    pub fn paint(&self, context: Option<TagCtx>) -> WebResult<TimelinePaint>;
}
```

`#[derive(Default)]` on `TimelineState` replaces the current manual impl
(`timeline/component.rs:32-41`): `RwSignal<T>: Default where T: Default`
(`reactive_graph-0.2.14/src/signal/rw.rs:272`).

`append(Ok(page))` must do **all four** of: advance `cursor` from the new page,
overwrite `has_more`, **extend** `rows` (not replace), and settle `status` to
`Idle` — matching `timeline/component.rs:86-89`. `append(Err(e))` sets
`status = Failed(e)` and touches nothing else (D10).

`paint()`'s fold — 7 cases:

| `status`       | `context`   | result             |
| -------------- | ----------- | ------------------ |
| `Failed(err)`  | any         | `Err(err)`         |
| `NeverLoaded`  | any         | `Ok(Loading)`      |
| `Unidentified` | any         | `Ok(Unidentified)` |
| `Idle`         | `Some(ctx)` | `Ok(Rows(ctx))`    |
| `Idle`         | `None`      | `Ok(Unidentified)` |
| `InFlight`     | `Some(ctx)` | `Ok(Rows(ctx))`    |
| `InFlight`     | `None`      | `Ok(Unidentified)` |

### `web/src/timeline/component.rs` — wasm-only, no host-testable logic left

```rust
pub fn spawn_load_more<F, Fut>(state: TimelineState, fetch: F) where … {
    let Some((created_at, post_id)) = state.begin_load_more() else { return };
    spawn_local(async move {
        state.append(fetch(created_at, post_id, Some(PageSize::default())).await);
    });
}

pub fn wire_timeline_resolve(state: TimelineState, initial_page: Resource<WebResult<TimelinePage>>) {
    Effect::new(move |_| {
        if let Some(result) = initial_page.try_get().flatten() { state.apply(result); }
    });
}

#[component] pub fn TimelineRows(…) -> impl IntoView   // unchanged markup

#[component]
pub fn TimelineGate(
    state: TimelineState,
    on_mutate: Callback<()>,
    on_load_more: Callback<()>,
    #[prop(default = Signal::derive(|| Some(TagCtx::SiteWide)))] tag_context: Signal<Option<TagCtx>>,
    #[prop(default = "No posts yet.")] empty_text: &'static str,
    #[prop(default = NoIdentity::Blank)] no_identity: NoIdentity,
    #[prop(optional)] children: Option<ChildrenFn>,
) -> impl IntoView
```

Gate body (D8's two regions):

```rust
let paint = Memo::new(move |_| state.paint(tag_context.get()));
let show_chrome = Memo::new(move |_| paint.get().is_ok_and(|p| p.shows_chrome()));
view! {
    {move || show_chrome.get().then(|| children.clone().map(|c| c()))}
    {move || match paint.get() {
        Err(err)                        => view! { <p class="error">{err.to_string()}</p> }.into_any(),
        Ok(TimelinePaint::Loading)      => view! { <p class="j-loading">"Loading…"</p> }.into_any(),
        Ok(TimelinePaint::Rows(ctx))    => view! { <TimelineRows … tag_context=ctx … /> }.into_any(),
        Ok(TimelinePaint::Unidentified) => match no_identity {
            NoIdentity::Blank         => ().into_any(),
            NoIdentity::Redirect(path) => view! { <Redirect path=path /> }.into_any(),
        },
    }}
}
```

**Two implementation contingencies, both with a specified fallback.** Neither
pattern has an in-repo precedent, so if either fails to compile, take the
fallback rather than redesigning:

1. `#[prop(default = Signal::derive(…))]` — all 16 existing
   `#[prop(default = …)]` sites in `web/src` use a literal or unit variant.
   Fallback: `#[prop(optional)] tag_context: Option<Signal<Option<TagCtx>>>` and
   resolve to `SiteWide` in the body.
2. `children: Option<ChildrenFn>` — the only `children` props in `web/src` are
   `Option<Children>` (`FnOnce`, single-call: `posts/component.rs:130`,
   `topbar/component.rs:9`). `ChildrenFn` is
   `Arc<dyn Fn() -> AnyView + Send + Sync>` and is re-callable, which D8 needs;
   `impl ToChildren for ChildrenFn` exists
   (`leptos-0.8.19/src/children.rs:118`), so this is very unlikely to fire.
   Fallback: **`BoxedChildrenFn`** (`children.rs:160`) — **not** `ViewFn`, which
   has no `ToChildren` impl and so cannot occupy `children` position at all (it
   would have to become a named `chrome=` prop, changing every call site).
   `BoxedChildrenFn` is not `Clone`, so on the fallback path
   `children.clone().map(|c| c())` becomes `children.as_ref().map(|c| c())`.

### Per-page result

| Page               | Uses                                                                                                                                 | `#[expect(too_many_lines)]` |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------ | --------------------------- |
| `SiteTagPage`      | `adopt_seed`, `wire_timeline_resolve`, `TimelineGate` (+`empty_text`)                                                                | **dropped**                 |
| `UserTagPage`      | same + `tag_context`                                                                                                                 | **dropped**                 |
| `UserTimelinePage` | same + `tag_context`                                                                                                                 | **dropped**                 |
| `HomePage`         | same; masthead `<div inner_html=…>` as `children`                                                                                    | n/a (never had one)         |
| `CockpitPage`      | own `Effect` (D11); `NoIdentity::Redirect("/login")`; its existing `match read_username()` (Topbar + `InlineComposer`) as `children` | n/a (never had one)         |

## 4. Acceptance criteria

**A1.** `rg 'too_many_lines' web/src/posts/component.rs` matches neither
`SiteTagPage`, `UserTagPage`, nor `UserTimelinePage` (`PostCreateForm` and
`EditPostPage` keep theirs — out of scope).
`cargo clippy --all-features --all-targets -- -D warnings` is clean, which also
proves each page is under the 100-line threshold: `#[expect]` (not `allow`)
makes an unfulfilled expectation a hard failure, so the attribute cannot be
silently left behind.

**A2.** `web/src/timeline/mod.rs` declares `mod state;` with **no**
`target_arch` cfg, and `pub use state::{…}` exports `TimelineState`,
`LoadStatus`, `TimelineCursor`, `TimelinePaint`, and `NoIdentity` ungated.
`TimelineState` is **removed** from the gated `pub use component::{…}`
(currently `mod.rs:18`) — leaving it in both collides. The
`target-arch-placement` xtask check passes: no item-level `target_arch` cfg is
introduced, `state.rs` stays ungated, `component.rs` stays gated at its `mod`
line.

**A3.** `state.rs` has `#[test]`s running under a fresh `Owner` (the
`web::reactive`/`forms::Field`/`tags::input_state` convention) covering:

- `adopt` sets rows/cursor/`has_more` and settles to `Idle`;
- `adopt_seed(None)` is a no-op; `adopt_seed(Some(page))` adopts;
- `apply(Ok)` adopts; `apply(Err)` clears rows, cursor, and `has_more`;
- `append(Ok)` **extends** rows _and_ advances `cursor` _and_ overwrites
  `has_more` _and_ settles to `Idle` — all four asserted, so a load-more that
  forgets the cursor and refetches page 1 forever cannot pass;
- `append(Err)` sets `Failed` and **retains** the existing rows, cursor, and
  `has_more` (D10);
- `unidentified()` clears and sets `Unidentified`;
- `begin_load_more()` → `None` when in flight, `None` when `!has_more`, else
  `Some(query)` after setting `InFlight`;
- `paint()` for **all 7 cases** of the §3 table;
- `shows_chrome()` for each `TimelinePaint` variant;
- `NoIdentity`'s derives are exercised on the host (it is otherwise only matched
  in the wasm gate body, so its `#[derive]` line would be an uncovered host line
  — and A4 forbids a marker). Same for any `TimelinePaint`/`LoadStatus` derive
  the tests above don't reach.

**A4.** `cargo xtask validate` is green, including the coverage gate. The
relocated code is now measured, so A3's tests are what keep it green — **no**
new `cov:ignore` or `crap:allow` marker is added under `web/src/timeline/`.

**A5.** Behavior unchanged on every existing e2e path. Specifically these pass
**unmodified**: `posts.spec.ts`'s `"No posts with this tag yet."` assertion
(currently line 854), its `"user tag page lists that user's tagged posts"` test
(the smoke test #671's own text cites as originating in #653), its
`"cockpit /app shows the authenticated home feed with pagination"` (line 502),
`media.spec.ts:90` (which depends on the cockpit composer being present), and
`authed-flash.spec.ts`'s `"anonymous: /app bounces to /login"` (line 83) and
`"owner: /app cockpit boots straight into the personalized feed"` (line 53).

`posts.spec.ts:502` is load-bearing and easy to overlook: it is the **only** e2e
that exercises cockpit's rows arm and load-more (`toHaveCount` at `:524`/`:532`,
the "Load more" click at `:530`), making it the behavioral backstop for
`append`/D10 and the test that catches a duplicated feed if D7's children are
over-broad.

**A6.** Exactly **one** visible change ships, and it is asserted rather than
incidental: an **unseeded client-side** navigation to `/` paints `.j-loading`
where it previously painted `"No posts yet."`. Cockpit's four paint outcomes are
unchanged (D7), so there is no second or third visible change. The new e2e must:

- **register the stall before navigating.** Stall — not fail — the fetch, via
  `page.route('**/api/list_local_timeline', …)` held open (endpoint name from
  `posts/api/listing.rs:143`), so observing `.j-loading` is deterministic rather
  than a race. The route interception **must** be installed before the click, or
  the fetch escapes it. `failServerFn` (`helpers.ts:105`) is the existing
  `page.route` precedent; this needs a releasable variant;
- enter the document on a **non-`/`** URL, then client-side navigate by clicking
  `.j-brand`. This is load-bearing twice over. First, home reads its seed from
  the _initial document_ at mount (`csr/src/lib.rs:30-44`) and the context
  persists, so entering on `/` leaves it seeded and it never paints `Loading`; a
  non-`/` SPA-shell entry gives `seed = None` (`csr/src/lib.rs:28`). Second, `/`
  has **no** projector shell fallback (`server/src/projector/mod.rs:200-228`,
  unlike `profile`/`site_tag` at `:273`/`:244`), so a _full load_ of `/` is
  always seeded — a client-side nav is the only way in. `.j-brand` is a plain
  `<a href="/">` present in both the anonymous `inner_html` sidebar
  (`sidebar/markup.rs:47`) and the authed one (`sidebar/component.rs:89`);
  `leptos_router`'s document-level click handler makes a raw anchor a
  client-side navigation, which `auth.spec.ts:111-128` already proves for
  `a[href="/logout"]` (`selectors.ts:22`, `sidebar/component.rs:151`) by showing
  a `window` stash survives the click;
- while stalled, assert `.j-loading` is visible **and** the masthead is present
  — the masthead, not `.j-scroll`, is the anchor, because `.j-scroll` is emitted
  only by `TimelineRows` (`timeline/component.rs:117`) and so does not exist in
  the Loading arm;
- stamp the masthead node (set a `data-*` attribute via `page.evaluate`), then
  release the fetch, wait for `.j-scroll`, and assert the stamp **survives** —
  proving D8 kept the chrome subtree alive across `Loading → Rows` instead of
  rebuilding it. Also assert the masthead still precedes `.j-scroll` in document
  order, which needs an explicit mechanism (`page.evaluate` +
  `Node.compareDocumentPosition`) — Playwright locators do not express document
  order;
- **if `.j-loading` never appears, do not weaken the assertion.** Distinguish
  the two causes: a full document reload (the click was not intercepted — check
  a `window` stash survived, per `auth.spec.ts:84-108`) versus a seeded `/` (the
  entry URL was wrong). Only after both are excluded is D8 in question.

**A7.** `cockpit/component.rs` contains no `bounce` signal; its redirect is
driven by `LoadStatus::Unidentified` via `NoIdentity::Redirect("/login")`. Its
`username` publish retains the `get_untracked()` inequality guard with the #591
comment explaining why, and `InlineComposer` keeps its `username: Username` prop
type.

**A8.** The three module doc comments that this change falsifies are corrected:
`timeline/mod.rs:1-9` ("only the pure … `state` and `render` leaves and the
wasm-only reactive `component`"), `timeline/state.rs:1-4` ("The reactive
`TimelineState` … lives in the wasm-only `component.rs`"), and
`timeline/component.rs:1-5` ("the `TimelineState` signal bundle").

**A9.** A numberless ADR draft in `docs/adr/drafts/` records D4/D6/D8 as a
reusable pattern (host-tested paint fold; component body as `Memo` + bare
`match`; per-page variation as a data enum; chrome as a memo-gated sibling
region), citing #306, #520, ADR-0070 §6, ADR-0041 §2, and this issue.

**A10 — projector coincidence is verified empirically, not argued.** The
projector's own render fns are untouched and `adopt_seed` runs synchronously
before first render, so a seeded page yields `Ok(Rows(_))` on its first
`paint()` and never enters the Loading arm. That is an argument, not coverage:
A5's e2e assert _content_ after hydration settles and would pass straight
through a visible first-paint flash — as #653 demonstrated on these very pages.
So both levels are asserted:

- **Unit:** a test proves `paint()` after `adopt_seed(Some(page))` is
  `Ok(TimelinePaint::Rows(_))`, never `Loading`.
- **Browser:** `end2end/tests/timeline-cls.spec.ts` carries an
  `expectNoShiftAcrossMount` probe (`end2end/tests/layout-shift.ts:47`,
  delivered by the closed #202) for each of the **four** projector-painted
  timeline routes — `/`, `/tags/:tag`, `/~:username`, `/~:username/tags/:tag` —
  at the default `tolerancePx: 0` (exact), each with an `afterMount` assertion
  so a zero-shift result cannot be a frozen-projector no-op. `/app` is excluded:
  it is `no-store` and never projector-painted, so it has nothing to coincide
  with.

These probes must be **written before** the page sweeps and pass on the
pre-sweep tree. A regression test authored after the change only documents the
end state; green before _and_ after is what proves preservation. Tolerances are
never loosened to get green.

## 5. Out of scope

- `PostCreateForm` / `EditPostPage` `#[expect(clippy::too_many_lines)]` —
  unrelated pages.
- #306's guard itself. This change moves logic in the direction that guard will
  demand (in flight on `worktree-issue-306-thin-component-guard`), but neither
  implements nor depends on it.
- Unifying the `Invalid username` / `Invalid tag` error-banner path with the
  `Unidentified` arm — deliberately different axes (D5).
- Changing what `TimelineRows` renders. Its markup is projector-coincident with
  `render_timeline_page` (`posts/render.rs:228-245`) and is untouched.
- `#[client_only]`: nothing to assert — the macro is fully retired repo-wide
  (ADR-0070 §7), so it cannot be reintroduced.
