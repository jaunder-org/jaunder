# web module-level host/wasm boundary (issue #300) — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating individual tasks to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Spec:**
[`docs/superpowers/specs/2026-07-06-issue-300-web-wasm-boundary.md`](../specs/2026-07-06-issue-300-web-wasm-boundary.md)
— the plan is "how"; the spec is "what/why". Reference it by section rather than
re-narrating.

**Goal:** Replace the 20 line-level `target_arch` gates in `web/src/pages/` with
a single module-level `#[cfg(target_arch = "wasm32")]` on `pages`, after
relocating the pure logic it strands into host-compiled homes.

**Architecture:** Direction A (spec §Approach) — one `web` crate. Relocate pure
helpers + their tests out of `pages/` (→ `web::render`, `web::tags`) so they
stay host-tested and coverage-measured, then gate `pages` wasm-only at `lib.rs`,
which deletes every per-line gate at once. Server fns and `render` are
untouched.

**Tech Stack:** Rust, Leptos (csr/ssr features), `cargo nextest`, `cargo xtask`
(check/validate), Nix gate.

## Review header

**Scope — in:** relocate pure logic out of `pages/`; delete the fake-value host
stub; gate `pages` wasm-only; strip the now-dead per-line gates; add a
wasm-target clippy pass; ADR; tracker reconciliation. **Scope — out:** the crate
split (Direction B — filed in Task 1); the `#[server]` arg-struct / 2
`too_many_arguments` (→ #299); `must_use_candidate` (already resolved by #94).
No behavior change to the shipping client or server.

**Tasks:**

1. File Direction B as a follow-on issue; reconcile #300/#299 acceptance
   (tracker).
2. Draft the ADR recording the module-level boundary + B deferral.
3. Relocate `is_valid_tag_slug` + `normalize_tag_token` (+tests) → `web::tags`;
   repoint `TagInput`.
4. Relocate `format_bytes` (+tests) → `web::render`; repoint `MediaPage`.
5. Relocate the `avatar_parts` / `format_post_time` / `DEFAULT_THEME` tests →
   `web::render`.
6. Measured-line audit of `pages/`; relocate any remaining pure non-glue
   helpers; write the disposition list.
7. Gate `pages` wasm-only at `lib.rs`; strip all now-dead/redundant per-line
   gates; delete the orphaned fake/marker host tests.
8. Add the wasm-target clippy pass to `cargo xtask` + the Nix gate.
9. Final `cargo xtask validate` + acceptance sweep.

**Key risks/decisions:**

- Deleting the `local_datetime_to_utc_rfc3339` host arm can only land **with or
  after** the `pages` gate (Task 7) — the fn has no host body afterward, so an
  earlier deletion breaks the host build. Kept atomic in Task 7.
- The measured-line audit (Task 6) must run **before** gating (Task 7) so no
  pure non-glue line silently leaves the coverage set. Backs spec AC#5.
- The wasm clippy pass (Task 8) may surface lints host clippy never saw in the
  now-wasm-only `pages/`; the task includes fixing them (no new suppressions).

## Global Constraints

_Every task's requirements implicitly include these (spec §Acceptance / project
`CONTRIBUTING.md`):_

- **No new lint suppressions.** The 2 `#[allow(clippy::too_many_arguments)]`
  stay in place (their removal is #299's).
- **Coverage-measured set must not shrink** except for genuinely wasm-only UI
  glue; relocated pure logic stays measured.
- **No behavior change** to the shipping wasm client or the server —
  behavior-preserving relocation, verified by the surviving host tests + the e2e
  matrix.
- **Commit gate:** the pre-commit hook runs `cargo xtask check`; run it first so
  it passes clean (**jaunder-commit**). **No `Co-Authored-By` trailer.** One
  clean commit per task.
- Relocations are **moves**: the test count in a relocated helper's new home
  must match what left `pages/`; nothing duplicated.

---

### Task 1: File Direction B; reconcile the tracker

**Files:** none (GitHub tracker only).

- [x] **Step 1:** File the Direction B (crate split) follow-on issue via
      **jaunder-issues** in `jaunder-org/jaunder`. Body: summarize spec
      §Approach Direction B — split `web` into a shared pure crate (`render` +
      helpers, both targets, host-tested/measured), a server-fn API crate
      (dual-target via the `#[server]` macro, no manual `target_arch` gates),
      and a wasm-only client crate (`pages`). Note it subsumes #299's
      `#[server]` arg-list restructuring, and relates ADR-0040/0044/0050/0051 +
      this issue's ADR (Task 2). Label `tooling`; milestone "Code quality
      improvement".
- [x] **Step 2:** Edit issue #300's body — amend the Acceptance section: remove
      the `too_many_arguments` criterion and add a line "The 2
      `too_many_arguments` suppressions are out of scope; owned by #299." (per
      spec §Corrections).
- [x] **Step 3:** Add a comment to #299 noting it owns the
      `create_post`/`update_post` arg restructuring + the `input = Json`
      wire-shape constraint, referenced from #300.
- [x] **Step 4: Commit** — no code changed; this is a tracker-only task, nothing
      to commit. Record completion by ticking the boxes. (B filed as #303 with
      native `blocked-by #300`; #300/#299 reconciled via comments.)

---

### Task 2: Draft the ADR

**Files:**

- Create: `docs/adr/drafts/web-host-wasm-boundary-module-level.md` (numberless
  draft; **jaunder-adr** flow — `cargo xtask adr promote` numbers it at ship).

**Interfaces:**

- Produces: an ADR the spec (AC#9) and the Task 1 issues reference.

- [x] **Step 1:** Write the ADR via **jaunder-adr**. Content: the `web`
      host/wasm boundary is **module-level**, not line-level — `pages/` (the
      Leptos UI) compiles wasm-only; pure logic lives in host-compiled homes
      (`render`, `tags`, `auth::marker`); `#[server]` fn definitions and
      `render` stay host-compiled (ADR-0040 render coincidence; the projector
      never mounts the Leptos `App`). Rationale: line-level gates dragged
      `cov:ignore` noise and invited fake-value host stubs. A wasm-target clippy
      pass keeps wasm-only code linted. The crate split (Direction B) is
      deferred to its follow-on issue (Task 1). Status: Accepted. Relates
      ADR-0040, ADR-0044, ADR-0050, ADR-0051/#268.
- [x] **Step 2:** Verify the draft renders and follows the ADR template
      (`docs/adr/` conventions). No promotion now — that happens at ship.
- [x] **Step 3: No commit** — CORRECTION: `docs/adr/drafts/` is **gitignored**
      (except `README.md`) per ADR-0048/the drafts README; a draft carries no
      number and is never committed. It lives out-of-git until
      `cargo xtask adr promote` numbers + moves + stages it at ship
      (**jaunder-ship**). Nothing to commit here.

---

### Task 3: Relocate the tag helpers → `web::tags`

**Files:**

- Modify: `web/src/pages/ui.rs` (remove `is_valid_tag_slug` at :1193,
  `normalize_tag_token` at :1204, and their tests; repoint the two `TagInput`
  call sites at :1305,:1310 to `crate::tags::`)
- Modify: `web/src/tags/mod.rs` (add the two fns + a test module)
- Test: `web/src/tags/mod.rs` `#[cfg(test)] mod tests`

**Interfaces:**

- Produces: `web::tags::is_valid_tag_slug(s: &str) -> bool`,
  `web::tags::normalize_tag_token(raw: &str) -> String` (signatures +
  `#[must_use]` + doc-comments unchanged; the "mirrors
  `common::tag::Tag::from_str` without pulling `common` into WASM" doc stays
  accurate — `tags` is a `web` module, not `common`).

- [x] **Step 1:** Move `is_valid_tag_slug` and `normalize_tag_token` (with
      `#[must_use]` + doc-comments) verbatim from `ui.rs` into
      `web/src/tags/mod.rs`.
- [x] **Step 2:** Move their tests — the 9 `is_valid_tag_slug` cases
      (`ui.rs:1555-1602`) and the 4 `normalize_tag_token` cases
      (`ui.rs:1605-1627`, incl. `normalize_then_is_valid`) — into a
      `#[cfg(test)] mod tests` in `tags/mod.rs`, importing
      `use super::{is_valid_tag_slug, normalize_tag_token};`.
- [x] **Step 3:** In `ui.rs`, repoint the `TagInput` call sites
      (`normalize_tag_token(&input_text.get())` at :1305,
      `is_valid_tag_slug(&text)` at :1310) to `crate::tags::normalize_tag_token`
      / `crate::tags::is_valid_tag_slug`, and drop the moved fns/tests + the
      now-stale imports in the `ui.rs` test module.
- [x] **Step 4: Run tests, verify PASS in new home**

Run: `cargo nextest run -p web tags::` Expected: PASS — 13 relocated tests run
under `tags::tests::…`.

- [x] **Step 5: Verify no duplication / host build intact**

Run: `cargo build -p web` and
`rg 'fn is_valid_tag_slug|fn normalize_tag_token' web/src` → each defined
**once** (in `tags/mod.rs`). Expected: builds clean; single definition each.

- [x] **Step 6: Commit** (`cargo xtask check` first)

```bash
git add web/src/pages/ui.rs web/src/tags/mod.rs
git commit -m "refactor(web): relocate tag-slug helpers to web::tags (#300)"
```

---

### Task 4: Relocate `format_bytes` → `web::render`

**Files:**

- Modify: `web/src/pages/media.rs` (remove `format_bytes` at :14 + its
  `#[expect(clippy::cast_precision_loss, …)]` + the `mod tests` at :217; repoint
  `render_media_row`'s call site to `crate::render::format_bytes`)
- Modify: `web/src/render/mod.rs` (add `format_bytes` + its `#[expect]` + tests)

**Interfaces:**

- Produces: `web::render::format_bytes(bytes: i64) -> String` (signature + the
  `cast_precision_loss` `#[expect]` reason unchanged).

- [x] **Step 1:** Move `format_bytes` (with its
      `#[expect(clippy::cast_precision_loss, …)]` attribute) verbatim from
      `media.rs` into `render/mod.rs`. Make it `pub` (crate-internal use).
- [x] **Step 2:** Move the 4 `format_bytes_*` tests (`media.rs:220-240`) into
      `render/mod.rs`'s test module, importing `super::format_bytes`.
- [x] **Step 3:** In `media.rs`, repoint `render_media_row`'s
      `format_bytes(...)` call to `crate::render::format_bytes`; remove the
      moved fn + `mod tests`.
- [x] **Step 4: Run tests, verify PASS**

Run: `cargo nextest run -p web render::` Expected: PASS — the 4 `format_bytes_*`
tests run under `render::tests::…`.

- [x] **Step 5: Verify** `cargo build -p web` clean;
      `rg 'fn format_bytes' web/src` → single definition in `render/mod.rs`.

- [x] **Step 6: Commit** (`cargo xtask check` first) — NOTE: committed together
      with Task 3 in one `refactor(web): relocate pure helpers to host homes`
      commit (the git-add hook auto-stages all tracked edits, so Task 3 + Task 4
      landed as one reviewable relocation diff).

```bash
git add web/src/pages/media.rs web/src/render/mod.rs
git commit -m "refactor(web): relocate format_bytes to web::render (#300)"
```

---

### Task 5: Relocate the render/theme tests to `web::render`

**Files:**

- Modify: `web/src/pages/ui.rs` (remove the `avatar_parts_*` +
  `format_post_time_*` tests and the
  `use crate::render::{avatar_parts, format_post_time};` in its test module)
- Modify: `web/src/pages/mod.rs` (remove the `default_theme_is_nonempty` test at
  :198)
- Modify: `web/src/render/mod.rs` (add all three groups to its test module)

**Interfaces:**

- Consumes: `web::render::{avatar_parts, format_post_time, DEFAULT_THEME}`
  (already defined there).

- [x] **Step 1:** Move the 7 `avatar_parts_*` tests (`ui.rs:1484-1529`) and 4
      `format_post_time_*` tests (`ui.rs:1529-1551`) into `render/mod.rs`'s test
      module (they already reference `crate::render::…`; switch to `super::…`).
- [x] **Step 2:** Move `default_theme_is_nonempty` (`pages/mod.rs:198`) into
      `render/mod.rs`'s test module as `super::DEFAULT_THEME`; drop the
      now-empty `mod tests` from `pages/mod.rs`.
- [x] **Step 3: Run tests, verify PASS**

Run: `cargo nextest run -p web render::` Expected: PASS — avatar/time/theme
tests now under `render::tests::…`.

- [x] **Step 4: Verify** `cargo build -p web` clean. The only tests left in
      `pages/ui.rs`'s module are the `local_datetime_*` + `marker_*` stub tests
      (deleted in Task 7).

- [x] **Step 5: Commit** (`cargo xtask check` first)

```bash
git add web/src/pages/ui.rs web/src/pages/mod.rs web/src/render/mod.rs
git commit -m "test(web): relocate render/theme tests to web::render (#300)"
```

---

### Task 6: Measured-line audit of `pages/`

**Files:**

- Modify (if the audit finds relocatable pure logic): the relevant
  `pages/*.rs` + host home
- Doc: append the audit list to this plan (or the PR description)

**Interfaces:** — (audit; any relocation follows the Task 3/4 pattern).

- [x] **Step 1:** Enumerate every host-compiled (un-`target_arch`-gated)
      function/impl in `web/src/pages/*.rs` that is coverage-measured today.
      Command: `rg -n 'pub fn|fn |impl ' web/src/pages` cross-referenced against
      the current coverage report (`cargo xtask check`'s coverage output). Known
      candidates: `TimelineState::adopt` / `TimelineState::default`
      (`pages/timeline.rs:32,48`).
- [x] **Step 2:** Classify each: **pure non-glue logic** (relocate to a host
      home — `render`/`tags`/a new module — following Task 3's move-with-tests
      pattern, adding a host test if none exists) vs **wasm-only reactive/UI
      glue** (e.g. mutates Leptos `RwSignal`s — `TimelineState::adopt` sets
      signals — so it is glue, verified by the e2e matrix; record and let it
      leave the measured set).
- [x] **Step 3:** Write the disposition list: each measured `pages/` line
      leaving the set, with a one-line "relocated to X" or "wasm-only glue,
      e2e-verified" for each. This is the concrete evidence for spec AC#5. Paste
      it into the PR description at ship.
- [x] **Step 4:** Audit relocated **nothing** — all remaining host-compiled
      `pages/` code is glue or already-relocated pure logic. No code commit; the
      disposition list below is the deliverable.

#### Audit disposition (backs spec AC#5)

Every non-`#[component]` fn in `pages/` was enumerated (`rg` over
`web/src/pages/`, cross-checked against cov:ignore/`#[cfg]`/`#[component]`
markers). Full classification:

- **Already-relocated pure logic (Tasks 3–5), stays host-measured:**
  `is_valid_tag_slug` + `normalize_tag_token` → `web::tags`; `format_bytes` →
  `web::render`; the `avatar_parts` / `format_post_time` / `DEFAULT_THEME` tests
  → `web::render`. No coverage lost — moved with their subjects/tests.
- **`#[component]` bodies** (all the `*Page` / UI components): ADR-0050
  structural exemption — never counted; unaffected.
- **Already `cov:ignore`'d glue** (not measured today, so no shrink):
  `TimelineState::default`/`adopt` (`timeline.rs:32,48`),
  `permalink_first_paint` (`posts.rs:114`), `render_draft_row`/
  `render_delete_form`/`render_delete_result` (`posts.rs:966,1023,1044`),
  `audience_checkbox` (`ui.rs:561`), `authed_sidebar` (`ui.rs:1108`),
  `render_media_row` (`media.rs:156`), `site_settings_form` (`site.rs:57`),
  `backup_settings_form` (`backup.rs:47`).
- **Already wasm-only gated** (not host-measured today):
  `TimelineState::resolve`/ `fail`/`spawn_load_more` (`timeline.rs`),
  `upload_file` (`upload.rs:145`).
- **Wasm-boundary stubs — the ONLY measured lines leaving the set, all
  spec-acknowledged (Task 7 makes them wasm-only, verification → e2e):**
  `local_datetime_to_utc_rfc3339` (`ui.rs:278`; the fake host arm + its
  empty-guard test — the one genuine pure line lost, per spec §map(b), a
  one-line guard), `marker_matches` (`ui.rs:368`; host `false` arm),
  `marker_username_on_boot` (`ui.rs:1092`; host `None` arm).

**Conclusion:** no pure non-glue logic silently leaves the coverage-measured
set. The only shrink is the three wasm-boundary stubs' host arms (fake/None
values that never ship) plus the one acknowledged `local_datetime` empty-guard
test — exactly what the spec authorized.

---

### Task 7: Gate `pages` wasm-only; strip per-line gates; delete orphaned stub tests

**Files:**

- Modify: `web/src/lib.rs:30,44` (gate `pub mod pages;` + `pub use pages::App;`
  / other `pub use pages::{…}`)
- Modify: all of `web/src/pages/*.rs` (strip per-line `target_arch` gates;
  delete fake arm; delete orphaned tests)

**Interfaces:**

- Consumes: the relocations from Tasks 3–6 (nothing host-measured is left behind
  in `pages/`).
- Produces: `web::pages` compiles **only** under `target_arch = "wasm32"`.

- [x] **Step 1:** In `lib.rs`, gate the `pages` module and its single re-export
      wasm-only (the only two lib.rs-level references to `pages`; the
      `pub use ui::{…}` / `pub use upload::{…}` re-exports live inside
      `pages/mod.rs` and are gated automatically):

```rust
#[cfg(target_arch = "wasm32")]
pub mod pages;
#[cfg(target_arch = "wasm32")]
pub use pages::App;
```

Confirm no host code outside `pages/` consumes these (verified in spec: only
`mount_csr`'s wasm body uses `App`).

- [x] **Step 2:** Delete the now-dead / redundant per-line gates across
      `pages/*.rs` — the whole module is wasm-only, so:
  - remove every `#[cfg(not(target_arch = "wasm32"))]` arm (dead) — this deletes
    the `local_datetime_to_utc_rfc3339` fake arm (`ui.rs:293-296`), the
    `marker_matches` `false` arm (`ui.rs:373-377`), the delete-confirm
    `{ false }` (`ui.rs:450`), the `signal_read.rs:16` host arm
    (compile-appeasement per spec §Note on the projector), etc.;
  - remove every now-redundant `#[cfg(target_arch = "wasm32")]` inner gate
    (un-gate the wasm bodies);
  - remove every
    `#[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]` shim
    (the unused-var only arose on the deleted host arm).
- [x] **Step 3:** Delete the orphaned host-stub tests still in `pages/ui.rs`'s
      test module: `local_datetime_empty_or_blank_is_none`,
      `local_datetime_host_arm_returns_trimmed_input`,
      `marker_matches_is_false_off_wasm`,
      `marker_username_on_boot_is_none_off_wasm` (spec §map (b)). Remove the
      now-empty `mod tests` if nothing remains.
- [x] **Step 4: Verify both targets compile and AC#1 holds**

Run: `cargo build -p web` (default) and `cargo build -p server` → Expected: PASS
(pages excluded on host). Run:
`cargo build -p csr --target wasm32-unknown-unknown` (or the cargo-leptos wasm
build) → Expected: PASS (pages compiled). Run:
`rg 'cfg\(not\(target_arch = "wasm32"\)\)|cfg_attr\(not\(target_arch' web/src/pages`
→ Expected: **no matches** (AC#1, AC#2).

- [x] **Step 5: Commit** (`cargo xtask check` first) — verified: fast gate green
      (host clippy+compile),
      `cargo build -p web --features csr --target     wasm32-unknown-unknown`
      exit 0, both rg checks empty.

```bash
git add web/src/lib.rs web/src/pages
git commit -m "refactor(web): gate pages wasm-only, delete per-line target_arch gates (#300)"
```

---

### Task 8: Add the wasm-target clippy pass to the gate

**Files:**

- Modify: `xtask/src/steps/static_checks.rs` (add a wasm-target clippy
  invocation alongside the host `clippy --all-targets` at :54)
- Modify: the Nix gate check that mirrors static checks (find via
  `rg -l 'clippy' nix/ flake.nix` / the coverage/check derivation)

**Interfaces:**

- Produces: `cargo xtask check` and `cargo xtask validate` both run
  `cargo clippy --target wasm32-unknown-unknown` over the client feature and
  fail on warnings.

- [ ] **Step 1:** Confirm the wasm32 target is available in the gate toolchain
      (xtask's `build_csr` step already builds
      `--target wasm32-unknown-unknown`, so it is —
      `xtask/src/steps/build_csr.rs:26`).
- [ ] **Step 2:** Add a clippy step that **directly compiles `web::pages` under
      wasm32** so the pass actually lints it:
      `cargo clippy -p web --features csr --target wasm32-unknown-unknown -- -D warnings`.
      Target `-p web` (not `-p csr`): the `csr` _feature_ is declared on `web`
      (`web/Cargo.toml:67`), the `csr` _crate_ is a thin `mount_csr()` shim and
      may not define that feature, and `-p web --features csr --target wasm32`
      is the form that pulls `pages/` into the compile. Wire it into the same
      static-checks phase as the host clippy
      (`xtask/src/steps/static_checks.rs:54`) so `check` and `validate` both run
      it.
- [ ] **Step 3:** Mirror it in the Nix gate derivation so CI runs the same pass
      (ADR-0034 — the checks run raw tooling, not xtask; add it alongside the
      host `clippy = craneLib.cargoClippy` in `flake.nix`).
- [ ] **Step 4: Prove the pass genuinely lints `pages/`** — before trusting a
      clean run, temporarily inject a wasm-only lint into a `pages/` fn (e.g.
      `let _x = 1;` an unused binding, or a `#[allow]`-free
      `clippy::let_and_return`) and run the new step; confirm it **FAILS** on
      that lint. Then revert the injection. (A clean pass alone proves nothing —
      the failure is what confirms `pages/` is in the linted set.)
- [ ] **Step 5: Run the full check and fix any real wasm-only findings**

Run: `cargo xtask check` Expected: PASS. If the wasm clippy pass surfaces
genuine lints in the now-wasm-only `pages/` code, **fix the code** (no new
suppressions — Global Constraints).

- [ ] **Step 6: Commit**

```bash
git add xtask/ nix/ flake.nix
git commit -m "build(xtask): add wasm-target clippy pass to the gate (#300)"
```

---

### Task 9: Full validate + acceptance sweep

**Files:** none (verification).

- [ ] **Step 1:** Run the full local gate.

Run: `cargo xtask validate` (Bash background mode — long/cold; static + clippy +
coverage + e2e). Expected: green (spec AC#8).

- [ ] **Step 2:** Acceptance sweep against spec §Acceptance:
  - AC#1/#2:
    `rg 'cfg\(not\(target_arch = "wasm32"\)\)|cfg_attr\(not\(target_arch' web/src/pages`
    → empty; no fake host stub remains.
  - AC#4: relocated pure tests pass in their new homes
    (`cargo nextest run -p web tags:: render::`).
  - AC#5: coverage gate green + the Task 6 disposition list attached.
  - AC#6: wasm clippy pass present + green.
  - AC#7: `git diff wt-base-issue-300..HEAD` introduces no new
    `#[allow]`/`#[expect]`; the 2 `too_many_arguments` unchanged.
  - AC#9/#10: ADR draft present; #300/#299 reconciled; Direction B filed.
- [ ] **Step 2 note:** any gap → new task, don't paper over. This task closes
      only when the full sweep passes; hand off to **jaunder-ship**.
