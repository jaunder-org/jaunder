# Spec — #301: eliminate the web view-component lint suppressions

Issue: [#301](https://github.com/jaunder-org/jaunder/issues/301) Branch:
`worktree-issue-301-web-lint-suppressions` Fork-point tag: `wt-base-issue-301`

## The inventory has drifted from the issue

#301 was written in July against a tree since restructured (#527 dissolved
`ui/`, #657/#658 relocated leaves). Its lists are stale. The **measured**
inventory at this branch's fork point:

| Issue claims                                                                                                                                       | Actual today                                                                                                                                                                                |
| -------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `too_many_lines` — 8 view fns (`UserTimelinePage`, `SiteTagPage`, `UserTagPage`, `MediaPage`, `AudiencesPage`, `Sidebar`, `ui.rs`, `EditPostPage`) | **2** — `PostCreateForm` (`web/src/posts/component.rs:475`), `EditPostPage` (`:1075`). The other six no longer carry one                                                                    |
| `needless_pass_by_value` — 6 components (`TagList`, `Avatar`, `Dot`, `AudiencePicker`, `FeedDiscovery`, `RsdDiscovery`)                            | **7 params across 5 components**. `Dot` no longer exists; `AudiencePicker` is alive (`posts/mod.rs:73`) but its **suppression** is gone; `PostDisplay` (3 params) is not named by the issue |
| `cast_*` — 3 suppressions                                                                                                                          | **3 suppressions**, covering **4 casts** — `server/src/observability.rs:287` covers two. One site is outside the "web view-component" framing                                               |
| crate-wide `must_use_candidate = "allow"`                                                                                                          | still at `web/Cargo.toml:76`                                                                                                                                                                |

The seven `needless_pass_by_value` params: `Avatar::name`
(`avatar/component.rs:8`, sig `:14`), `FeedDiscovery::surface` (`:11`, sig
`:16`) and `RsdDiscovery::username` (`:45`, sig `:50`) in
`feed_discovery/component.rs`, `PostDisplay::{post, banner, tag_context}`
(`posts/component.rs:149`, sig `:155-162`), `TagList::context`
(`taglist/component.rs:10`, sig `:16`). All five carry the identical blanket
reason quoted in D1. `Avatar::size: u32` is `Copy`; `TagList::tags` is consumed
via `into_iter` (`taglist/component.rs:21`).

## Decisions

### D1 — The shared reason is half true, and is **edition-scoped**

All five components carry:

> Leptos `#[component]` props are stored by the framework and must be owned; the
> borrow clippy suggests isn't expressible in a component signature

**True under edition 2021.** `leptos_macro`'s `#[component]` builds a props
struct from the fn's generics verbatim; a `&'a T` prop needs a lifetime the
generated `-> impl IntoView` will not capture under `edition = "2021"`
(`web/Cargo.toml:4`) — E0700.

**False in general, and false under edition 2024.** "Props must be owned" is
refuted by this codebase: `&'static str` (`icon/component.rs:6`,
`banner/component.rs:11`), `Resource<_>` (`media/component.rs:175`),
`RwSignal<_>` (`posts/component.rs:83`, `tags/component.rs:22`), `TextProp`
(`topbar/component.rs:7`).

**Measured, not assumed:** with `web` flipped to `edition = "2024"`, both a
lifetime-prop component and a call site compile clean against Leptos 0.8.19
(forced rebuild after `cargo clean -p web`):

```rust
#[component] pub fn AvatarProbe<'a>(name: &'a Username) -> impl IntoView { … }
#[component] pub fn AvatarProbeCaller(name: Username) -> impl IntoView {
    view! { <AvatarProbe name=&name /> }
}
```

**But the payoff collapses on inspection, and this was measured too.** Editions
are per-crate, so `web` alone can move — that is not the obstacle. The obstacle
is that under edition 2024 the **wasm** build (where these components actually
compile) fails with five errors, all one class — a view borrowing a function
parameter:

```
web/src/invites/component.rs:75  E0515: cannot return value referencing function parameter `i`
web/src/media/component.rs:258   E0515: … referencing function parameter `item`
web/src/media/component.rs:310   E0515: … referencing function parameter `item`
web/src/media/component.rs:336   E0521: borrowed data escapes; `'1` must outlive `'static`
web/src/media/component.rs:397   E0521: borrowed data escapes; `'1` must outlive `'static`
```

The same RPIT-capture rule that makes a reference prop expressible is what makes
these illegal: once the lifetime is captured, Leptos's requirement that a
**stored** view be `'static` bites. The probe above compiled only because
`avatar_parts(name)` produced an owned `String` and the reference never entered
the view.

So a `&'a T` prop is usable only when the prop is fully consumed into owned data
**before** the view — precisely the case where taking it by value and consuming
it already satisfies the lint. Reference props do not let ownership be pushed
above the component: **the view must own.** Edition 2024 therefore unlocks
nothing for these seven params.

The migration is filed with its measured cost as
[#826](https://github.com/jaunder-org/jaunder/issues/826) and is out of scope
here.

**Consequence for this branch:** any surviving reason must be site-specific and
must not assert the blanket claim. Where the edition-2021 limit is the operative
fact, the reason must say so explicitly rather than implying a permanent
language constraint.

### D2 — Genuine fixes preferred; two named anti-patterns barred

The issue's acceptance ("every case fixed in code") is the target and each site
gets a real attempt, but an _earned_ suppression beats a contortion:

- **Barred:** pessimizing a helper that correctly takes `&str` (`avatar_parts`,
  `canonicalize`, `rsd_href`) into taking an owned value purely to quiet the
  caller.
- **Barred:** an artificial rebinding (`let post = post;`) that works only
  because clippy's shadowing analysis is imprecise. (Evidence it is imprecise:
  `AudienceHeader` (`audiences/component.rs:226`) has a borrow-only use of a
  non-`Copy` newtype outside any closure at `:233` and does not fire; the
  visible difference is a self-shadowing `let name = …(&name)`. It takes a
  second, likely `Copy`, prop, so it is not perfectly analogous — but the
  inference holds.)

**Expected per-site outcome** (the implementer may beat this; they may not
silently fall short of it):

| Site                                       | Expected                                                                                                   |
| ------------------------------------------ | ---------------------------------------------------------------------------------------------------------- |
| `TagList::context`                         | Removed with the component (D4)                                                                            |
| `RsdDiscovery::username`                   | **Fix** — single use; a consuming form is reachable without harming `rsd_href`                             |
| `FeedDiscovery::surface`                   | Fix if reachable; else re-justified (4 borrows across `Link` attrs, no closure)                            |
| `Avatar::name`                             | Attempt a genuine consuming use (an accessible `title`/`aria-label` is a real markup gap); else re-justify |
| `PostDisplay::{post, banner, tag_context}` | **Re-justified** per D3                                                                                    |

### D3 — `PostDisplay` keeps a suppression, with a corrected reason

`PostDisplay` is the **terminal owner**: its sole caller `PostCard`
(`posts/component.rs:361`) moves all three values in, and `PostView<'a>`
(`posts/render.rs:140-150`) borrows from them at `component.rs:169-179` across
`render_post_content`. They must be owned _here_ to outlive the borrow-view.
Clippy cannot see "owns in order to lend."

The governing principle is **ADR-0041 §2** ("share the pure fn, not the
component"). §4 is the anonymous-view/explicit-viewer seam and is _not_ the
citation for this claim — the code comment at `posts/component.rs:185` cites §4
correctly for a different point.

### D4 — `TagList` is deleted; the rest of the `taglist` leaf **must survive**

`TagList` has no call sites — definition plus the `pub use` at
`taglist/mod.rs:12`. Introduced with a caller by `8c1bb216` ("render tag chips
in PostDisplay footer"), orphaned by `386b25df` (#181), which moved tag
rendering into the pure render path so CSR and the projector emit identical
markup. It then survived #527 and #658, each relocating it mechanically without
noticing.

**Exactly three things are deleted:** `web/src/taglist/component.rs`, the
`mod component` declaration (`taglist/mod.rs:6-7`), and the
`pub use component::TagList` (`:12`) — both already
`#[cfg(target_arch = "wasm32")]`.

**Everything else in the leaf stays.** `TagCtx` (`taglist/context.rs:11`) is
load-bearing across `posts/render.rs` (`:18,53,64,75,90,102,119,149,230`),
`posts/component.rs:36` (aliased `TagContext`), `timeline/component.rs:20`,
`timeline/state.rs:17,64,225`, `taglist/markup.rs:6`; and `taglist::render`
(`markup.rs:13`) is the projector's tag renderer on the ADR-0041 path. Deleting
the leaf directory would break that path.

The "reactive twin" wording in `taglist/mod.rs:1-4` and `markup.rs:10` names
`TagList` and must be rewritten to describe a renderer whose only consumer is
now the pure path.

### D5 — `must_use_candidate` comes back on; #94's rationale has expired

#94 approved the disable (D-config-1) because "46 of 48 sites are Leptos view
fns". Measured with the disable off, **7 sites fire and none is a view fn**:

| Site                                                                                                                     | Feature set                  |
| ------------------------------------------------------------------------------------------------------------------------ | ---------------------------- |
| `email/status.rs:9`, `posts/parse.rs:38`, `reactive/mod.rs:49`, `tags/input_state.rs:68`, `:92`, `timeline/state.rs:200` | host-default + wasm          |
| `posts/server.rs:12` (`rendered_post`)                                                                                   | **`--features server` only** |

All seven are genuine helpers — precisely what the lint is for. The disable is
removed and all seven get `#[must_use]`. An approved decision **expiring on
changed facts**, not overruled on taste.

**How the seventh was nearly missed, and the rule it forces.** An earlier draft
of this decision claimed "exactly 6, identical on host and wasm". That was
measured on host-default and wasm/csr only. `web/src/posts/server.rs` sits
behind `#[cfg(feature = "server")]` (`posts/mod.rs:12`) and compiles under
**neither** — it needs `--features server`, which is what the gate builds, and
the gate is what caught it. `web` is feature-gated in three directions, so **a
claim that a lint class is clear must be measured on all three configurations**.
The same caution applies to D2's `needless_pass_by_value` set, which was also
originally measured on two.

The wasm run remains the one that answers #94's objection specifically, being
the only configuration that compiles the wasm-only `#[component]` modules — and
it yields zero view-fn hits.

**A corollary on how the annotations are satisfied.** Restoring the lint
surfaced four call sites discarding `TagInputState::handle_key`'s return
(`tags/input_state.rs`). Silencing those with `let _ =` would be the same
evasion as `#[expect]` in another costume, which is what this issue exists to
remove. They became assertions once `input_state.rs:120-128` confirmed
`ArrowDown`/`ArrowUp` return `true` unconditionally. **`let _ =` is not an
acceptable resolution anywhere in this branch:** either the value is meaningful
and gets asserted or used, or the `#[must_use]` is wrong and the attribute
changes — and either way it is stated.

### D6 — Decompose, under the `thin-components` gate

Both `too_many_lines` suppressions claim splitting "would fragment the page
without real benefit". The threshold is clippy's **default 100** (`clippy.toml`
sets only `allow-unwrap-in-tests` / `allow-expect-in-tests`).

**The `thin-components` gate constrains every seam** (`CONTRIBUTING.md:259-274`,
ADR-0083): a `#[component]` body may carry at most **2 control-flow units on
each of two surfaces** (setup, view), and **no suppression marker exists** for
it. Three consequences:

- Extracting into a plain `fn -> impl IntoView` **explicitly does not count as a
  fix** (`CONTRIBUTING.md:270-272`). So `render_draft_row`
  (`posts/component.rs:1470`) is _not_ precedent — it is a private builder fn,
  not a `#[component]`. The valid precedents are `EditSaveActions` (`:1287`),
  `EditSaveOutcome` (`:1344`) and `DraftList` (`:1444`), two of them split out
  of `EditPostPage`'s own page.
- Every extracted subcomponent is itself measured, so seam placement is not
  free.
- Pure logic should move to a host-compiled leaf (`page_state.rs` / `parse.rs`)
  per ADR-0070 §6 — how #306 solved this same problem, and the lever with the
  largest effect on line count.

Seams: `PostCreateForm` (278 lines, `:481-759`) has shared signal setup
(`:491-530`) then a top-level `if compact` at `:532` whose branches are
independent views. `EditPostPage` (196 lines, `:1081-1277`) continues its
existing extraction pattern.

New subcomponents stay **private** to `posts/component.rs` (per the
`posts/mod.rs:68-70` comment) unless a caller outside the module needs them.

### D7 — The `server/` casts are in scope

`server/src/observability.rs:285-296` — one `#[expect]` covering **two**
`cast_possible_truncation` casts (`elapsed` and `threshold` millis). It is item
3 of the issue's own list. Saturation on overflow is acceptable and preferred to
truncation (`u64::try_from(...).unwrap_or(u64::MAX)`), since the values are
observability thresholds where a saturated maximum is honest and a wrapped value
is not.

### D8 — Remove the wasm-clippy escape hatch (and it is already dead)

Per the issue's
[comment](https://github.com/jaunder-org/jaunder/issues/301#issuecomment-4898971022),
`-A unfulfilled_lint_expectations` was added as a temporary measure.

**The comment's stated mechanism no longer describes this tree.** It was written
for the since-deleted `pages/AudiencesPage`. `web/src/posts/component.rs` is
wasm-only (`posts/mod.rs:17-18`), so its two `#[expect(too_many_lines)]`s are
never evaluated on the host at all — they cannot be "host-fulfilled,
wasm-unfulfilled".

**Measured:** the wasm pass with the allow removed
(`cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -W warnings -A clippy::too_many_arguments`)
reports **zero** warnings. Two consequences: the flag is **already dead** and
could be removed independently of D6; and both `too_many_lines` expects **are**
fulfilled on wasm, so the decomposition is genuinely required rather than
assumed.

**Three occurrences, not two:** `xtask/src/steps/static_checks.rs:96-97` (the
arg), its explanatory comment at `:68-75`, and the unit test
`wasm_clippy_lints_web_client_and_csr` (`:220-250`, string at `:246`) which
asserts the exact arg vector — editing only the arg breaks the test. Plus
`flake.nix:1096` and its comment at `:1080`.

### D9 — The storage-usage percentage must move to a host-testable leaf

`web/src/media/component.rs` is wasm-only (`media/mod.rs:15-16`), never
host-compiled, and `cargo llvm-cov` cannot instrument wasm (ADR-0055:21). So
criterion 8's tests **cannot exist where that code lives**. The percentage
computation moves into `web/src/media/format.rs`, whose module doc (`:1-3`)
already describes exactly this pure-leaf pattern, and is tested there.

This also resolves a `thin-components` hazard: `MediaUsagePanel`'s `Suspend`
body (`media/component.rs:180-194`) already carries `match` + `if` = 2 units, so
adding a branch in place would break the gate. Extraction removes a branch
rather than adding one.

## Scope

**In scope** — `web/src/{avatar,feed_discovery,posts,taglist,media}/`,
`web/Cargo.toml`, the six `#[must_use]` sites in D5,
`server/src/observability.rs`, `xtask/src/steps/static_checks.rs` (arg, comment
**and** unit test), `flake.nix` (arg and comment),
`web/src/taglist/{mod.rs,markup.rs}` docs, and
`docs/web-style-guide.md:124,149,381` (which names `TagList` as an exported
widget example and points at `taglist/component.rs`).

**Out of scope** — the test-only
`#![allow(clippy::unwrap_used, clippy::expect_used)]` blocks
(`web/src/test_support.rs:11`, `timeline/server.rs:169`, `posts/api.rs:797`,
`subscriptions/server.rs:32`), which `CONTRIBUTING.md` names as policy-compliant
keepers; the edition-2024 migration (D1); the `web` client/server split (#300);
any rendered-markup change beyond what D2/D6/D9 require.

## Acceptance criteria

1. `rg -n 'clippy::(too_many_lines|cast_precision_loss|cast_possible_truncation)' web/src/ server/src/`
   returns **nothing**. (The suppressions are multi-line `#[expect(` with the
   lint on the following line, so a bare `rg 'expect\('` would match
   `.expect("…")` calls and never the lint names — it cannot test this.)
2. **(a) Mechanical:** the D1 blanket string ("props are stored by the framework
   and must be owned") appears nowhere in `web/src/`. **(b) Editorial:** every
   surviving `#[expect(clippy::needless_pass_by_value)]` names a concrete
   site-specific mechanism in the shape of D3's "owns in order to lend", and
   each is listed with its justification and **explicitly approved before
   landing** per `CONTRIBUTING.md:112-116` — a reworded reason counts as new
   text.
3. With both `#[expect(clippy::too_many_lines)]` deleted, the **wasm** clippy
   pass
   (`cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown`)
   reports no `too_many_lines` — the only target on which `posts/component.rs`
   is compiled at all. Threshold is clippy's default 100. Every extracted view
   is a `#[component]` in `web/src/posts/component.rs`, private unless an
   outside caller needs it; a plain `fn -> impl IntoView` does not count
   (`CONTRIBUTING.md:270-272`).
4. `rg -n '\bTagList\b' web/src/ docs/web-style-guide.md` returns nothing —
   including the doc prose in `taglist/mod.rs:1-4` and `taglist/markup.rs:10`.
   The scope is deliberate: `docs/archive/` holds ~30 hits across nine frozen
   planning documents and is excluded from doc gates as a historical record
   (`CONTRIBUTING.md:252-255`), and `docs/superpowers/` holds this spec and its
   plan, which name `TagList` throughout and are themselves archived at ship —
   so a `docs/`-wide `rg` can never go green and would push an implementer into
   editing the archive.

   `TagCtx`, `taglist/context.rs`, `taglist/markup.rs` and `taglist::render`
   **still exist and still compile**, and the projector tag path is unchanged.
   Two of the three `docs/web-style-guide.md` sites need a **substitute
   example**, not a repoint: `:147-151` cites `taglist/` as a canonical
   "`markup.rs` twin + wasm-only `component.rs`" pair, which it stops being; and
   `:377-381` uses `taglist/component.rs` as the worked example of reading a
   newtype out at a view site.

5. `must_use_candidate = "allow"` is gone from `web/Cargo.toml`; **all seven**
   D5 sites carry `#[must_use]`. Clippy clean on **all three** configurations —
   `cargo clippy -p web --all-targets`,
   `cargo clippy -p web --features server --all-targets`, and the wasm command
   in criterion 3 — since each compiles a different subset of the crate and a
   site can hide in any of them. No `#[must_use]` return is discarded with
   `let _ =`; call sites either assert the value or use it.
6. `-A unfulfilled_lint_expectations` is gone from
   `xtask/src/steps/static_checks.rs:96-97`, its comment at `:68-75`, the unit
   test `wasm_clippy_lints_web_client_and_csr` (`:220-250`), `flake.nix:1096`
   and its comment at `:1080`. `cargo nextest run` for xtask passes, and the
   wasm pass is green without the flag.
7. The four casts are resolved by integer formatting or checked conversion, not
   a wider suppression: `web/src/media/format.rs` (`format_bytes`), the
   storage-usage percentage (moved per D9), and both casts at
   `server/src/observability.rs:285-296` (saturating per D7).
8. **No regression in displayed output**, pinned by host-run unit tests in
   `web/src/media/format.rs`. This criterion deliberately does **not** demand
   byte-identical output. The issue asks for "behavior-preserving (e2e green)" —
   i.e. don't break the app — and an earlier draft over-read that as
   "byte-identical", which manufactured a false conflict with criterion 7: the
   storage percentage divides by `quota` (not a power of two) and then
   multiplies by 100, so its displayed digit is an artifact of two successive
   binary roundings and **no** integer algorithm reproduces it.

   What actually constrains the output, measured: the only assertions on these
   strings are `format_bytes`'s five existing tests (`media/format.rs:38-58`),
   pinning 0, 1023, 1024, 1536, 1 MB, 2 MB, 1 GB — all small powers of two that
   integer math reproduces exactly. The storage-usage percentage has **no** unit
   test and **no** e2e assertion.

   So: **(a)** those five tests pass **unchanged**; **(b)** new tests pin
   behaviour at each KB/MB/GB boundary ±1, a negative (`impl Into<i64>`, so
   negatives reach the `{bytes} B` arm), and **2^53** — the bound above which
   the `as f64` conversion is itself lossy and correct integer math legitimately
   diverges from today. `i64::MAX` must **not** be the only large-value test: it
   coincidentally agrees between the two implementations, so it would pass while
   the function was wrong across the entire petabyte band; **(c)** for the
   percentage, correctly-rounded integer math may shift the rendered
   `width:{pct:.1}%` (`media/component.rs:209`) by up to 0.1 percentage points —
   about 0.3 px on a 300 px decorative bar. That shift is **accepted**; the new
   values are pinned by tests on the extracted fn.

9. No new suppression of any kind is introduced without being listed in the PR
   body with its justification and explicitly approved before landing
   (`CONTRIBUTING.md:112-116`).
10. `cargo xtask validate` green **with e2e** — this branch changes rendered
    components.

## Notes

The issue's text is not a reliable inventory; criterion 1's `rg` is the
authority. That staleness is itself evidence for D8:
`-A unfulfilled_lint_expectations` has been hiding which expects still earn
their keep — measured now at zero stale, but only because someone looked.
