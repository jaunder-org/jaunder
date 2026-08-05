# Plan — #301: eliminate the web view-component lint suppressions

Spec:
[`docs/superpowers/specs/2026-08-04-issue-301-web-lint-suppressions.md`](../specs/2026-08-04-issue-301-web-lint-suppressions.md)
Issue: [#301](https://github.com/jaunder-org/jaunder/issues/301) Branch:
`worktree-issue-301-web-lint-suppressions` Fork-point tag: `wt-base-issue-301`

## Review header

**Goal.** Remove every `too_many_lines` and `cast_*` suppression in the changed
files, resolve the `needless_pass_by_value` set in code where a genuine fix
exists, restore the crate-wide `must_use_candidate` lint, and delete the
wasm-clippy escape hatch — without changing rendered markup.

**Scope — in:** `web/src/{avatar,feed_discovery,posts,taglist,media}/`,
`web/Cargo.toml`, the seven `#[must_use]` sites, `server/src/observability.rs`,
`xtask/src/steps/static_checks.rs` (arg + comment + unit test), `flake.nix`
(arg + comment), the `taglist` doc prose, `docs/web-style-guide.md`.

**Scope — out:** test-only `unwrap_used`/`expect_used` blocks; the edition-2024
migration (filed as [#826](https://github.com/jaunder-org/jaunder/issues/826));
the `web` client/server split (#300).

**Separable concerns:** already filed — **#826** (edition 2024, with its
measured 5-error cost). Nothing else surfaced, so there is no issue-filing task.

**Tasks:**

1. Restore `must_use_candidate`; annotate the seven helpers.
2. Fix the media casts — integer `format_bytes`, and extract the storage-usage
   percentage to a host-testable leaf.
3. Fix the two `server/src/observability.rs` casts (saturating).
4. Delete `TagList` and repoint the prose that names it.
5. Resolve `needless_pass_by_value` per site — **contains a user-approval
   halt**.
6. Decompose `PostCreateForm`.
7. Decompose `EditPostPage`.
8. Remove `-A unfulfilled_lint_expectations` from all five places.
9. Full gate + spec conformance walk.

**Key risks / decisions:**

- _The `thin-components` gate constrains every seam_ (`CONTRIBUTING.md:259-274`,
  ADR-0083): ≤2 control-flow units per surface, **no suppression exists**, and a
  plain `fn -> impl IntoView` explicitly does not count as decomposition. Tasks
  6 and 7 must check it on each extracted component, not just on the parent.
  Task 2 exists partly because `MediaUsagePanel` is already at 2 units and an
  in-place branch would break it.
- _`web/src/media/component.rs` is wasm-only_ (`media/mod.rs:15-16`), never
  host-compiled and not instrumentable by `cargo llvm-cov` (ADR-0055:21). Its
  logic **cannot** be unit tested where it lives — hence the D9 extraction in
  task 2. Attempting tests in place will fail silently by never running.
- _Task 5 cannot land unilaterally._ `CONTRIBUTING.md:112-116` requires explicit
  approval **before** a suppression lands, and a reworded reason is new text.
  Task 5 halts with the proposed reasons rather than committing them.
- _The xtask unit test asserts the exact clippy arg vector_
  (`static_checks.rs:220-250`, string at `:246`). Editing only the arg at
  `:96-97` breaks it — task 8 must touch all five places.
- _The two cast sites round differently, and only one can match today's output._
  `format_bytes` divides by powers of two, so integer math reproduces it exactly
  **below 2^53**; above that the `as f64` conversion is itself lossy and the new
  code is the more correct one. The storage percentage divides by `quota` (not a
  power of two) and then multiplies by 100 — two successive binary roundings —
  so **no** integer algorithm reproduces its displayed digit. Criterion 8 was
  amended to accept a ≤0.1pp shift there rather than demand the impossible.
- _`i64::MAX` is a trap as a test input._ It coincidentally agrees between the
  f64 and integer implementations, so a test suite whose only large value is
  `i64::MAX` passes while `format_bytes` is wrong across the whole petabyte
  band. Task 2 must test **at 2^53**.
- _Ordering hazard: tasks 6/7 can create new lint sites after task 5's approval
  gate has closed._ Extracted components take owned props lifted from
  `PostCreateForm`'s signature (`posts/component.rs:481-490`) and
  `EditPostPage`; any that only borrows a non-`Copy` prop fires
  `needless_pass_by_value` and would need a **new** suppression, which criterion
  9 says requires approval before landing. Tasks 6/7 carry an explicit step to
  return to task 5's halt if that happens.
- _Task 1 sets a tripwire that task 2 trips._ Task 1 removes the
  `must_use_candidate` disable and says "stop if a seventh site appears"; task 2
  then adds a new `pub` fn to the host-compiled `media/format.rs`, which is a
  seventh site by construction. Task 2 annotates it — that is expected, not a
  tripwire hit.
- _Measured starting state_ (do not re-derive): removing the
  `must_use_candidate` disable yields exactly 6 hits, none a view fn, identical
  on host and wasm. The wasm pass with `-A unfulfilled_lint_expectations`
  removed is already **zero warnings**, so both `too_many_lines` expects are
  genuinely fulfilled and task 8 is safe at any point after tasks 6–7.

**For agentic workers:** execute with **`jaunder-iterate`**, delegating via
**`jaunder-dispatch`** where useful. Tick checkboxes here in real time.

## Global constraints

- Rust. No `Co-Authored-By` trailer. Stage, then commit — never
  `git commit -- <paths>`.
- Before each commit:
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-301-web-lint-suppressions -- cargo xtask check`
  (**`jaunder-commit`**).
- **`let _ =` is not a fix.** Discarding a `#[must_use]` return to quiet the
  lint is the same move as `#[expect]` in a different costume, and this branch
  exists to stop doing that. If a value is genuinely meaningful, assert it (in a
  test) or use it (in production); if it genuinely is not, the `#[must_use]` is
  wrong and the attribute is what should change. Either outcome gets stated, not
  silently discarded. (Applied at `tags/input_state.rs`: the four discarded
  `handle_key` returns became assertions once `:120-128` confirmed arrows always
  return `true`.)
- **Three clippy configurations, not two** — `web` is feature-gated in three
  directions and a lint site can hide in any of them:
  - HOST-default: `cargo clippy -p web --all-targets`
  - HOST-server: `cargo clippy -p web --features server --all-targets` — the
    only one that compiles `web/src/posts/server.rs` (`posts/mod.rs:12`,
    `#[cfg(feature = "server")]`), and what the gate builds
  - WASM:
    `cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown`
    — the only one that compiles
    `web/src/{posts,media,invites,avatar,feed_discovery,taglist}/component.rs`

  **All three must be run before claiming a lint class is clear.** Task 1's
  original "exactly six `must_use_candidate` sites" was measured on the first
  and third only, and missed a seventh in the second; the gate caught it. Any
  task that verifies a lint set — notably task 5's `needless_pass_by_value` —
  must sweep all three.

- `xtask` is not a workspace member: use
  `cargo nextest run --manifest-path xtask/Cargo.toml`, not `-p xtask`.
- No new suppression without listing it for approval (criterion 9).

---

## Task 1 — restore `must_use_candidate`

**Files:** `web/Cargo.toml`; `web/src/email/status.rs`, `posts/parse.rs`,
`reactive/mod.rs`, `tags/input_state.rs` (×2), `timeline/state.rs`.

**Steps**

- [x] Delete `must_use_candidate = "allow"` and its two-line comment
      (`web/Cargo.toml:74-76`).
- [x] Add `#[must_use]` at `email/status.rs:9`, `posts/parse.rs:38`,
      `reactive/mod.rs:49`, `tags/input_state.rs:68`, `tags/input_state.rs:92`,
      `timeline/state.rs:200`. No other site should need one; if a seventh
      appears, stop and report rather than annotating it silently. — **Exactly
      six, as measured.**
- [x] **A seventh site, and the measurement that missed it.** The gate failed on
      `web/src/posts/server.rs:12` (`rendered_post`) — a pure host-only helper
      returning `Option<RenderedPost>`, the same class as the other six, now
      annotated. It hid behind `#[cfg(feature = "server")]` (`posts/mod.rs:12`),
      which neither of the two configurations originally used here compiles. The
      real total is **seven across three feature sets**; see the corrected
      three-configuration rule in Global constraints.
- [x] **Fallout: four discarded `handle_key` returns** in that module's own
      tests (`tags/input_state.rs`). First fixed with `let _ =` — which is the
      same dodge this branch exists to remove. Replaced with `assert!(...)` at
      each call after confirming `:120-128` that `ArrowDown`/`ArrowUp` return
      `true` unconditionally, which also pins prevent-default at both clamp
      boundaries where it was previously unchecked.

**Run:** HOST and WASM. Expected: **clean**, no `must_use_candidate`.

**Commit:**
`refactor(web): restore must_use_candidate; annotate the six helpers (#301)`

---

## Task 2 — the two media casts

Criterion 7 + 8, and D9's extraction.

**Files:** `web/src/media/format.rs` (edit + tests),
`web/src/media/component.rs` (edit).

**Steps**

- [ ] Rewrite `format_bytes` (`media/format.rs:12`) with integer math — no
      `f64`. Delete its `#[expect(clippy::cast_precision_loss)]` (`:7-11`).
      Output is identical to today below 2^53; above it the new code is the more
      correct one and divergence is accepted (criterion 8).
- [ ] Keep the **five existing tests** (`media/format.rs:38-58`) passing
      **unchanged** — they pin 0, 1023, 1024, 1536, 1 MB, 2 MB, 1 GB, all
      exactly reproducible. If any needs editing, stop: that means the rewrite
      changed behaviour in the range that is supposed to be identical.
- [ ] Add tests at each KB/MB/GB boundary ±1, a negative, and **2^53**. Do not
      rely on `i64::MAX` as the large-value case — it coincidentally agrees
      between implementations and would hide a wrong result across the petabyte
      band.
- [ ] Move the storage-usage percentage out of `media/component.rs:183-194` into
      a pure fn in `media/format.rs`, clamped, integer math. Delete the
      `#[expect]` at `:183-188`. `component.rs` calls the new fn.
- [ ] Add `#[must_use]` to the new pure fn — task 1 removed the crate-wide
      `must_use_candidate` disable, so a new host-compiled `pub` fn needs it.
      (This is the expected seventh site, not task 1's stop-and-report
      tripwire.)
- [ ] Test the percentage at quota 0, used 0, used == quota, used > quota
      (clamp). Pin the **new** values: the rendered `width:{pct:.1}%` may differ
      from today by up to 0.1pp, which criterion 8 accepts. Nothing else asserts
      on it — no other unit test, no e2e.
- [ ] Confirm `MediaUsagePanel`'s `Suspend` body has **not gained** a
      control-flow unit (it was already at 2 — `match` + `if`). Extraction
      should remove one.

**Run:** `cargo nextest run -p web` (host) — expected **PASS**; then WASM —
expected clean.

**Commit:**
`refactor(web/media): integer byte + storage-percentage formatting (#301)`

---

## Task 3 — the observability casts

**Files:** `server/src/observability.rs`.

**Steps**

- [ ] Replace both `as u64` casts (`:285-296`) with saturating conversions
      (`u64::try_from(...).unwrap_or(u64::MAX)`), per D7. Delete the
      `#[expect]`.
- [ ] A saturated maximum is the intended behaviour for an overflowing
      threshold; a wrapped value is not. If a test pins the old behaviour,
      update it deliberately and say so in the commit.

**Run:** `cargo nextest run -p server`. Expected **PASS**.

**Commit:**
`refactor(server): saturating millis conversions in observability (#301)`

---

## Task 4 — delete `TagList`

D4. **Only three code items go.**

**Files:** delete `web/src/taglist/component.rs`; edit `web/src/taglist/mod.rs`,
`web/src/taglist/markup.rs`, `docs/web-style-guide.md`.

**Steps**

- [ ] `git rm web/src/taglist/component.rs`.
- [ ] Remove the `mod component` declaration (`taglist/mod.rs:6-7`) and
      `pub use component::TagList` (`:12`).
- [ ] **Do not touch** `taglist/context.rs` (`TagCtx`) or `taglist/markup.rs`'s
      `render` — both are load-bearing on the ADR-0041 projector path and are
      used by `posts/render.rs`, `posts/component.rs:36`,
      `timeline/component.rs:20`, `timeline/state.rs`.
- [ ] Rewrite the "reactive twin" prose in `taglist/mod.rs:1-4` and
      `markup.rs:10`, which name `TagList`: the renderer's only consumer is now
      the pure path.
- [ ] `docs/web-style-guide.md` — three sites, and **two need a substitute
      example, not a repoint**: `:124` is a plain drop from the exported-widget
      list; `:147-151` cites `taglist/` as a canonical "`markup.rs` twin +
      wasm-only `component.rs`" pair, which it stops being (pick another leaf —
      `avatar/`, `icon/` and `topbar/` are cited alongside it); `:377-381` uses
      `taglist/component.rs` as the worked example of reading a newtype out at a
      view site, so it needs a replacement worked example.

**Run:** WASM — expected clean. Then
`rg -n '\bTagList\b' web/src/ docs/web-style-guide.md` → **nothing**.

**Do not** widen that `rg` to `docs/`: `docs/archive/` holds ~30 hits in nine
frozen planning documents, excluded from doc gates as a historical record
(`CONTRIBUTING.md:252-255`), and `docs/superpowers/` holds this plan and its
spec. A `docs/`-wide check can never go green.

**Commit:**
`refactor(web/taglist): delete the orphaned TagList component (#301)`

---

## Task 5 — `needless_pass_by_value`, per site

D2's table. **This task halts before committing any surviving suppression.**

**Files:** `web/src/feed_discovery/component.rs`, `web/src/avatar/component.rs`,
`web/src/posts/component.rs`.

**Steps**

- [ ] `RsdDiscovery::username` — attempt a genuine consuming form. **Barred:**
      changing `rsd_href` to take an owned value purely to quiet the caller.
- [ ] `FeedDiscovery::surface` — attempt a fix; four borrows across `Link` attrs
      with no closure makes this the hardest of the three.
- [ ] `Avatar::name` — **note the twin before touching this.**
      `avatar/component.rs:6-7` declares it the reactive half of a twin with
      `avatar/markup.rs::render` (`markup.rs:25`), and there are parity tests at
      `markup.rs:44-52`; `component.rs:18`'s integer `(size * 36 + 50) / 100`
      with its "must match the pure `render` twin" comment shows markup
      coincidence is load-bearing here. So the `title`/`aria-label` idea is only
      viable if the **same attribute goes into `markup.rs::render` and its
      parity test** — which expands into the projector paint. If that is not
      clearly worth it, re-justify instead. `avatar_parts` takes `&str`
      (`markup.rs:10`), so D2 bars the pessimizing route; re-justification is
      the likely outcome and that is acceptable.
- [ ] `PostDisplay::{post, banner, tag_context}` — keep the suppression, rewrite
      the reason per D3 (terminal owner; `PostView<'a>` borrows across
      `render_post_content`; ADR-0041 **§2**).
- [ ] For **every** site that ends up keeping a suppression, verify the D1
      blanket string appears nowhere in `web/src/` and that the replacement
      names a concrete site-specific mechanism.
- [ ] **HALT — present each surviving suppression with its proposed reason for
      explicit approval** (`CONTRIBUTING.md:112-116`; criterion 2b). Do not
      commit reworded reasons before that approval.
- [ ] Confirm no fix was achieved by artificial rebinding (`let x = x;`). If a
      site only goes quiet that way, it keeps a suppression instead — say so at
      the halt.

**Run:** WASM. Expected clean. Also re-run with
`-- --force-warn clippy::needless_pass_by_value` to confirm exactly the intended
set remains.

**Commit (post-approval):**
`refactor(web): resolve needless_pass_by_value; re-justify what remains (#301)`

---

## Task 6 — decompose `PostCreateForm`

**Files:** `web/src/posts/component.rs`; possibly `web/src/posts/page_state.rs`
/ `parse.rs` for extracted pure logic (ADR-0070 §6).

**Steps**

- [ ] Split at the `if compact` seam (`:532`): the two branches are independent
      views over shared signal setup (`:491-530`).
- [ ] Extract each branch as a **`#[component]`** (a plain `fn -> impl IntoView`
      does not count — `CONTRIBUTING.md:270-272`), private to the module per the
      `posts/mod.rs:68-70` convention unless an outside caller needs it.
- [ ] Move pure logic to a host-compiled leaf where it helps the line count
      (ADR-0070 §6; this is how #306 solved the same problem).
- [ ] Delete the `#[expect(clippy::too_many_lines)]` at `:475-479`.
- [ ] Check the `thin-components` budget on **each** new component, not only the
      parent. (Measured: both branches of the `if compact` seam currently
      contain **zero** control-flow units, so extraction leaves the parent at
      setup=1 and each child at 0. This task has headroom; task 7 does not.)
- [ ] Update `web/src/posts/mod.rs:66-75`, whose comment enumerates the private
      helpers and subcomponents by name.
- [ ] **If an extracted component fires `needless_pass_by_value`** — likely,
      since the lifted props include `Option<Username>` and callbacks — it needs
      a new suppression, which criterion 9 requires be approved **before**
      landing. Take it back to task 5's halt rather than committing it.
- [ ] No markup change — the rendered output must be identical.

**Run:** WASM (threshold is clippy's default 100). Expected: no
`too_many_lines`, no `thin-components` failure. Then `cargo xtask check`.

**Commit:**
`refactor(web/posts): decompose PostCreateForm at the compact seam (#301)`

---

## Task 7 — decompose `EditPostPage`

**Files:** `web/src/posts/component.rs`, `web/src/posts/mod.rs` (its `:66-75`
comment enumerates the private helpers and subcomponents by name, so it goes
stale when new ones are added).

**`EditPostPage` is already AT the thin-components budget** — 2 units:
`match post.await` (`:1137`) and `if let Ok(selection)` (`:1146`). Any
restructuring that adds a third branch fails the gate outright, so the seam must
**move control flow out**, not add it. Unlike task 6, no seam is named in
advance; find it against the unit counts before editing.

**Steps**

- [ ] Identify the seam by counting units per candidate region first. The two
      existing units are the natural boundaries — extracting the `match` arms or
      the `if let Ok(selection)` body as `#[component]`s moves a unit out of the
      parent rather than adding one.
- [ ] Continue the existing pattern: `EditSaveActions` (`:1287`) and
      `EditSaveOutcome` (`:1344`) carry "Split out of [`EditPostPage`] (#306)"
      doc comments (`:1282`, `:1342`) — direct precedent for this page.
      (`DraftList` at `:1444` is precedent for the technique but was split out
      of `DraftsPage`. `render_draft_row` at `:1470` is **not** precedent — a
      plain builder fn.)
- [ ] Delete the `#[expect(clippy::too_many_lines)]` at `:1075-1079`.
- [ ] Same `thin-components` check per extracted component; same
      no-markup-change rule.

**Run:** WASM. Expected: no `too_many_lines`.

**Commit:** `refactor(web/posts): decompose EditPostPage (#301)`

---

## Task 8 — remove the wasm-clippy escape hatch

D8. **Five places, not two.**

**Files:** `xtask/src/steps/static_checks.rs`, `flake.nix`.

**Steps**

- [ ] Remove `-A unfulfilled_lint_expectations` from the arg vector
      (`static_checks.rs:96-97`).
- [ ] Remove the explanatory comment at `static_checks.rs:68-75` — it describes
      the since-deleted `pages/AudiencesPage` and no longer describes anything.
- [ ] Update the unit test `wasm_clippy_lints_web_client_and_csr`
      (`static_checks.rs:220-250`, string at `:246`), which asserts the exact
      arg vector.
- [ ] Remove the flag from `flake.nix:1096` and its comment at `:1080`.

**Run:** `cargo nextest run --manifest-path xtask/Cargo.toml` — expected
**PASS**; then WASM — expected clean without the flag.

**Commit:**
`chore(xtask): drop the wasm unfulfilled_lint_expectations allow (#301)`

---

## Task 9 — full gate and conformance

- [ ] `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-301-web-lint-suppressions -- cargo xtask validate`
      — **with e2e**, since this branch changes rendered components. ~25 min;
      Bash background mode.
- [ ] Walk all ten spec criteria against `git diff wt-base-issue-301..HEAD`,
      checking each off explicitly. Criterion 1's `rg` is the authority, not the
      issue text.
- [ ] Confirm `rg -n '\bTagList\b' web/src/ docs/web-style-guide.md` returns
      nothing (criterion 4 — deliberately scoped; see task 4).
- [ ] **Write the PR body**, which is where criteria 2b and 9 are discharged:
      list every surviving and every newly-added suppression with its
      site-specific justification, and record that each was approved at task 5's
      halt. No other task produces this artifact.

## Self-review

- Criterion coverage: 1 → tasks 2/3/6/7; 2a → task 5; 2b → task 5's halt
  (approval)
  - task 9 (the PR-body listing); 3 → tasks 6/7; 4 → task 4; 5 → task 1; 6 →
    task 8; 7 → tasks 2/3; 8 → task 2; 9 → task 5's halt, tasks 6/7's
    return-to-halt step, and task 9's PR body; 10 → task 9.
- Tasks are ordered low-risk-first so the judgement-heavy work (5) and the
  largest diffs (6, 7) land against an already-green tree.
- Nothing smuggles unauthorized scope: the edition-2024 work is #826, and task 4
  is explicit about the three items that go and the two that must not.
- The one genuine halt inside the loop (task 5) is required by
  `CONTRIBUTING.md`, not discretionary.
