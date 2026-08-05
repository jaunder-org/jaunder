# Plan — issue #826: move the whole workspace to Rust edition 2024

Spec:
[`docs/superpowers/specs/2026-08-05-issue-826-edition-2024.md`](../specs/2026-08-05-issue-826-edition-2024.md)
Issue: https://github.com/jaunder-org/jaunder/issues/826 Branch:
`worktree-issue-826-edition-2024` · fork anchor: tag `wt-base-issue-826`

**For agentic workers:** drive with **`jaunder-iterate`**; delegate an
individual task with **`jaunder-dispatch`** where useful. Tick checkboxes in
real time.

---

## Review header

### Goal

Move all 13 crates to `edition = "2024"`, discharging the two things the edition
breaks: 79 test-side `unsafe` env calls (replaced by one audited `with_env`
helper, which also absorbs ~26 reader-only lock sites) and 5 wasm view-lifetime
errors (fixed by `+ use<>`).

### Scope

**In:** 13 manifest edition flips · `resolver = "3"` in all three workspace
roots · doctest-harness fixture edition · `common::test_support::with_env` +
conversion of all env _access_ sites (mutating and reader-only) · `+ use<>` on 4
`web` helpers · 3 let-chains · 2 mis-gated test structs · one ADR, promoted.

**Out:** #301's three contingent suppressions · any behaviour change.

### The sequencing decision

The tree currently has the edition already flipped and `use<>` applied — that
was **measurement**, not implementation, and it is all **staged**. T2 restores a
green 2021 baseline, because:

- On edition 2021 `std::env::set_var` is safe **and** wrapping it in `unsafe` is
  _also_ warning-free (std marks these `#[rustc_deprecated_safe_2024]`, which
  suppresses `unused_unsafe` — measured clean under `-D warnings` on rustc
  1.95.0). So `with_env` can be written in its **final** form, `unsafe` and all,
  on 2021. The bulky conversion lands in small green commits a reviewer can
  read.
- `use<>` and let-chains are 2024-only, so they _cannot_ land before the flip.
- The edition flip then becomes a small atomic commit (T9) containing only what
  the edition actually forces.

### Tasks

| #   | Task                                                                 | Gate                   |
| --- | -------------------------------------------------------------------- | ---------------------- |
| T1  | ~~File resolver-3 follow-up~~ — measured as a no-op, folded into T8  | —                      |
| T2  | Restore a green edition-2021 baseline                                | `cargo xtask check`    |
| T3  | Add `common::test_support::with_env` + unit tests                    | `cargo xtask check`    |
| T4  | Convert `host/src/capture.rs`                                        | `cargo xtask check`    |
| T5  | Convert `storage/` (2 files)                                         | `cargo xtask check`    |
| T6  | Convert `server/src/cli.rs` (32 lock sites)                          | `cargo xtask check`    |
| T7  | Convert `server/src/observability.rs` (21 lock sites)                | `cargo xtask check`    |
| T8  | Pre-flip housekeeping: resolver 3, `dead_code` gating, stale comment | `cargo xtask check`    |
| T9  | **The edition commit** — 13 manifests, harness, `use<>`, let-chains  | `cargo xtask validate` |
| T10 | Write + promote the ADR                                              | `cargo xtask check`    |
| T11 | Conformance sweep against AC1–AC16                                   | `cargo xtask validate` |

### Key risks / decisions

- **Reader-only lock holders.** `server/src/cli.rs` takes `ENV_LOCK` 32 times
  but mutates in only 10 tests; `observability.rs` 21 vs 16. ~26 tests hold the
  lock purely to read a stable env. AC5 deletes that lock, so **every one of
  them must be wrapped in `with_env(|_env| …)`** or their serialization is
  silently lost. This is the single largest correctness trap in the plan and the
  reason T6/T7 are scoped by _lock acquisition_, not by mutation site.
- **Silent semantics.** Edition 2024 changes tail-expr and `if let` temporary
  drop scopes and match ergonomics with no diagnostic. Mitigation is T9's full
  `validate` incl. all four e2e combos, plus T11's `nextest list` diff.
- **Unmeasured test targets.** `server`/`storage`/`host` test targets have never
  compiled under 2024. T9 is where that lands.
- **Mutex poisoning.** `with_env` must ignore poisoning
  (`unwrap_or_else(PoisonError::into_inner)`) — otherwise the first panicking
  test poisons the global lock for every later one. There is an existing test
  for this behaviour (`observability.rs:982`) whose content moves to `common`
  (spec D1a).

---

## Global constraints

- **No `Co-Authored-By` trailer** on any commit.
- **Stage, then commit** — never `git commit -- <paths>`. The pre-commit hook
  runs the full `cargo xtask check`; run it first (**`jaunder-commit`**).
- Conventional-commit subjects ending `(#826)`.
- Pin the gate cwd:
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-826-edition-2024 -- cargo xtask check`
- **Touch no #301 file:** `web/src/avatar/component.rs`,
  `web/src/feed_discovery/component.rs`.
- Behaviour-preserving. Exactly one test may be removed in this branch — the one
  named in spec D1a — and nothing may be renamed, weakened, or `#[ignore]`d.

---

## T1 — ~~File the resolver-3 follow-up issue~~ → superseded

- [x] Issue filed: **#835** — https://github.com/jaunder-org/jaunder/issues/835
- [x] **#835 closed as folded into this branch.** The user challenged the
      deferral and was right. The issue's premise — and this plan's — was that
      resolver 3 changes feature unification. It does not: that is resolver
      v1→v2. **v2→v3 is MSRV-aware version selection**, and no manifest in this
      workspace declares `rust-version`, so it has nothing to act on.

      Measured v2 vs v3, all **byte-identical**: `cargo tree -e features
      --workspace` (host), the same `--target wasm32-unknown-unknown`, and
      `Cargo.lock`. A genuine no-op, so it lands in T8 rather than a separate PR.
      #835 carries the correction for future readers. See spec D5.

---

## T2 — Restore a green edition-2021 baseline

**The measurement edits are STAGED with a clean worktree.**
`git checkout -- <path>` restores the worktree _from the index_ and would
therefore do **nothing**. Use the `HEAD`-sourced form.

- [x] `git restore --source=HEAD --staged --worktree -- client/Cargo.toml common/Cargo.toml csr/Cargo.toml host/Cargo.toml macros/Cargo.toml server/Cargo.toml storage/Cargo.toml test-support/Cargo.toml web/Cargo.toml xtask/Cargo.toml tools/coverage/Cargo.toml tools/devtool/Cargo.toml tools/doctests/Cargo.toml web/src/invites/component.rs web/src/media/component.rs`
- [x] Keep the spec and plan docs staged — they are the only intended additions.
- [x] Verify: `rg -n 'edition' -g 'Cargo.toml' -g '!target'` → 13 × `"2021"`.
- [x] Verify: `git status --short` shows only the two doc files.
- [x] Gate: `cargo xtask check --no-test` → PASS (exit 0).
- [x] No commit.

---

## T3 — Add `common::test_support::with_env` (TDD)

**Files**

- Create `common/src/test_support/env.rs`
- Convert `common/src/test_support.rs` (flat, 473 lines of `parse_*` helpers) →
  `common/src/test_support/mod.rs`; add `mod env; pub use env::with_env;`
- `server/Cargo.toml` — dev-dep `common` gains `"test-support"` (AC14)

**Interface** — closure receives a handle, per spec D1.

```rust
use std::ffi::{OsStr, OsString};

/// Runs `f` with exclusive, serialized access to the process environment,
/// restoring every variable `f` touched to its prior value on the way out —
/// including if `f` panics.
///
/// Holds one process-global lock for the whole closure. Tests that only *read*
/// the environment should still wrap in `with_env(|_env| …)`: the lock's job is
/// to serialize readers against writers, not merely writers against each other.
///
/// **Not reentrant** — nesting deadlocks. Apply everything through the one handle.
pub fn with_env<R>(f: impl FnOnce(&mut Env) -> R) -> R;

pub struct Env { /* prior values, for restoration */ }

impl Env {
    pub fn set(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>);
    pub fn remove(&mut self, key: impl AsRef<OsStr>);
}
```

**Implementation notes**

- One `static ENV_LOCK: Mutex<()>` in this module — the _only_ env lock in the
  workspace (AC9).
- **Ignore poisoning:**
  `ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner)`.
- `Env` records the **first** prior value seen per key (so set-then-change-then-
  remove still restores the original), in a `Vec<(OsString, Option<OsString>)>`
  or an `IndexMap`-like structure; restore in `Drop` so unwinding restores.
- Write the `unsafe { … }` blocks and their safety comment **now** — verified
  warning-free on edition 2021 under `-D warnings`, so no rework at T9.
- `AsRef<OsStr>` on both key and value is required: real sites pass `&Path` /
  `&PathBuf` (`host/src/capture.rs:109`, `server/src/observability.rs:849,871`,
  `storage/src/postgres/mod.rs:333`).

**Tests** — in-file `#[cfg(test)]`, using variable names unique per test
(`JAUNDER_TEST_WITH_ENV_<CASE>`) so they cannot collide with real config vars.

- [ ] `restores_prior_value_after_closure` **(AC6 part 1)**
- [ ] `restores_prior_value_when_closure_panics` — `catch_unwind` around the
      `with_env` call **(AC6)**
- [ ] `removes_variable_that_was_previously_unset` — set inside, `var_os` is
      `None` after **(AC7)**
- [ ] `supports_interleaved_states_in_one_acquisition` — set A, assert; change
      A, assert; remove pre-set B, assert; verify all restored after. Completing
      at all proves no self-deadlock. **(AC8)** This is the shape
      `host/src/capture.rs:117-126` needs.
- [ ] `reader_only_section_restores_nothing` — `with_env(|_env| …)` compiles,
      runs, and leaves the environment untouched. **(AC9b's API precondition.)**
- [ ] `poisoned_lock_does_not_break_later_calls` — panicking `with_env` under
      `catch_unwind`, then a normal `with_env` succeeds. **(AC9; inherits the
      content of `observability.rs:982`.)**

**Run**

- [x] `cargo nextest run -p common test_support::env` — 7/7 PASS.
- [x] Gate: `cargo xtask check` → PASS. (First attempt failed: the doc example
      used a ` ```ignore ` fence, which the ADR-0095 doctest gate bans. Changed
      to ` ```text ` — it illustrates `server` types `common` cannot name.)
- [x] Commit `63b2a5fc`:
      `test(common): add with_env, the single audited env-access seam (#826)`

---

## T4 — Convert `host/src/capture.rs`

**Files:** `host/src/capture.rs` — `ENV_LOCK` at 84; mutations at 109, 113, 121,
123, 125.

- [x] Wrap **every** test that takes the lock — mutating or not — in `with_env`.
      Both lock-taking tests here mutate; the other two never took it.
- [x] `file_is_none_when_unset_or_blank` (117–126) is the interleaved case:
      remove → assert → set `"   "` → assert → remove. Expressed as **one**
      `with_env` with the assertions between the steps.
- [x] `set_var(DIR_ENV, &d)` is path-valued — relies on `AsRef<OsStr>`.
- [x] Delete `ENV_LOCK` and its stale comment.
- [x] `cargo nextest run -p host capture` → 5/5 PASS, same test names.
- [x] Gate: `cargo xtask check` → PASS.
- [x] Commit `1c2b1a9a`:
      `refactor(host): route capture env tests through with_env (#826)`

---

## T5 — Convert `storage/`

**Files:** `storage/src/db.rs` (`ENV_LOCK` at 302; mutations 340, 347, 349),
`storage/src/postgres/mod.rs` (`ENV_LOCK` at 324; 15 mutations 329–390)

- [x] Wrap every lock-taking test, mutating or not. All seven mutate.
- [x] Checked `postgres/mod.rs:328-338`: the assertions do **not** interleave —
      each test sets its delta, acts, then asserts. One `with_env` each.
- [x] `postgres/mod.rs:333` is path-valued.
- [x] Delete both `ENV_LOCK`s.
- [x] Existing tests only — no `#[tokio::test]` / `#[test]` churn.
- [x] `cargo nextest run -p storage` → 517/517 PASS. (A bare run first failed 6
      `case_2_postgres` tests with `ConnectionRefused`; that is the documented
      need for an ephemeral PG, not a regression. Re-ran under
      `devtool pg run --`.)
- [x] Gate: `cargo xtask check` → PASS.
- [x] Commit `5e951d6e`:
      `refactor(storage): route env tests through with_env (#826)`

---

## T6 — Convert `server/src/cli.rs`

**Scope is the 32 lock acquisitions, not the 17 mutation lines.** Only 10 tests
mutate; the other 22 hold the lock to read a stable env while clap parses.

**Files:** `server/src/cli.rs` — `ENV_LOCK` at 433 (comment 430–432); mutations
531–725; reader-only holders at 540, 587, 621, 631, 686, 696, 735, 759, 777,
787, 794, 805, 815, 827, 837, 846, 857, 867, 884, 894, 928, 984.

- [x] Convert all 32. Mutators get `env.set`/`env.remove`; readers become
      `with_env(|_env| { … })`.
- [x] The delta must wrap the `parse(...)` call, not just the assertion — clap
      reads env at parse time.
- [x] Delete `ENV_LOCK` at 433; its 430–432 comment is superseded by
      `with_env`'s docs. `use std::sync::Mutex` dropped too.
- [x] Confirm the converted count: `rg -c 'with_env' server/src/cli.rs` → 33 (32
      call sites + the import line).
- [x] `cargo nextest run -p jaunder cli` → 53 PASS. Independently verified the
      test-name set is unchanged: 48 test fns, `diff` of extracted names against
      `wt-base-issue-826` is empty.
- [x] Gate: `cargo xtask check` → PASS.
- [x] Commit `e66955aa`:
      `refactor(server): route cli env tests through with_env (#826)`

**Noted, not acted on:** a few "defaults" tests (e.g.
`environment_defaults_dev`) never cleared the variable whose default they
assert, so they depend on ambient env being unset. Preserved as-is — hardening
them changes what they assert and belongs in its own change.

---

## T7 — Convert `server/src/observability.rs`

**Scope is the 21 lock acquisitions.** 16 mutate; the rest (587, 664, 699, and
the poisoning test) are reader-only. Largest single conversion — prefer
**`jaunder-dispatch`**.

**Files:** `server/src/observability.rs` — `ENV_LOCK` at 540; mutations 627–930.

- [ ] Convert all 21, readers included.
- [ ] Tests setting two vars (e.g. 916–923) become **one** `with_env` with two
      `env.set` calls — never nested `with_env`, which deadlocks.
- [ ] `#[tokio::test]` at 806, 812, 817, 901, 913, 1012, 1066, 1090, 1121 —
      confirm which are in the lock-taking set. `with_env` takes a
      **synchronous** closure; that is sufficient because the env-sensitive
      region contains only `init_tracing_impl(...)`. Keep the closure body
      `.await`-free; do not make the closure `async`.
- [ ] `set_var(host::capture::DIR_ENV, dir.path())` at 849/871 — path-valued.
- [ ] **Delete `lock_env_recovers_from_poisoned_mutex` (982)** — spec D1a; its
      content already lives in `common` from T3. This is the one sanctioned
      removal; note it in the commit body.
- [ ] Delete `ENV_LOCK` at 540.
- [ ] `cargo nextest run -p jaunder observability` → PASS.
- [ ] Gate: `cargo xtask check` → PASS.
- [ ] Commit:
      `refactor(server): route observability env tests through with_env (#826)`

**Done when:** `rg -n 'ENV_LOCK|lock_env' server/src storage/src host/src` is
empty (AC5), and `rg -n '(set|remove)_var' -g '*.rs'` matches only
`common/src/test_support/env.rs` **plus** `test-support/tests/cli.rs:12` — which
T8 clears.

---

## T8 — Pre-flip housekeeping

Three edits that are **edition-independent** and therefore must not ride in the
edition commit.

- [x] **All three workspace roots** — `Cargo.toml`, `tools/Cargo.toml`,
      `xtask/Cargo.toml` — declare `resolver = "3"` (AC11). **Revised
      mid-execution after the user challenged the deferral, and they were
      right.** The original plan pinned `resolver = "2"` and filed #835, on the
      false premise that resolver 3 changes feature unification — that is v1→v2;
      v2→v3 is MSRV-aware version selection. No manifest declares
      `rust-version`, so v3 has nothing to act on. Measured v2 vs v3:
      `cargo tree -e features --workspace` (host), the same with
      `--target wasm32-unknown-unknown`, and `Cargo.lock` are all
      **byte-identical**. Adopted here; #835 closed with the correction. See
      spec D5.
- [x] `web/src/error.rs` — `#[cfg(feature = "server")]` on the `SourceError` and
      `OuterError` definitions **and their `impl` blocks** (the impls name the
      types, so they need the same gate). They are **not** dead: they are used
      throughout the `server`-gated tests. Not deleted, no `#[allow]`.
- [x] `test-support/tests/cli.rs:12` — stale doc comment reworded; it now says
      the tests pass the var to the child via `Command::env` and never mutate
      this process's environment (AC4).
- [ ] Gate: `cargo xtask check` → PASS.
- [x] Gate: `cargo xtask check` → PASS.
- [x] Commit `03c92031`:
      `build: adopt resolver 3, gate test-only structs, refresh a stale comment (#826)`

---

## T8b — The rustfmt style edition (unplanned; discovered during T9)

Flipping the crates to 2024 reformatted **209 files**, because `cargo fmt`
passes each crate's manifest edition to rustfmt and `style_edition` defaults to
it. The first T9 attempt measured 215 files; 209 of them were formatting.

- [x] Characterised the churn: 361 hunks — 161 `use` resorts, **192
      macro-argument re-wraps** (mostly `assert!` bodies), 8 whitespace. An
      initial claim that it was "almost entirely `use` declarations" was wrong
      and the user caught it.
- [x] User decision: **take the reformat**, as its own commit.
- [x] Pin `style_edition = "2024"` in `.rustfmt.toml` so the two decisions are
      decoupled from now on.
- [x] Landed on the **2021** tree, which is what proves it carries no edition
      semantics.
- [x] Commit `3535c873`:
      `style: pin and adopt the rustfmt 2024 style edition (#826)`

---

## T9 — The edition commit

Now contains **only** what the edition forces. This is the first time
`server`/`storage`/`host` test targets compile under 2024.

- [x] 13 × `Cargo.toml` → `edition = "2024"` (AC1)
- [x] `tools/doctests/src/harness.rs:35` → generated fixture `edition = "2024"`
      (AC13)
- [x] `web/src/media/component.rs` → `+ use<>` on `media_key_fields`,
      `render_media_row`, `force_delete_form`
- [x] `web/src/invites/component.rs` → `+ use<>` on `render_invite_row`
- [x] Let-chains — **19, not 3.** The plan's count came from per-crate
      `cargo clippy` runs judged by exit code, but warnings do not fail an exit
      code and the gate runs `-D warnings`. Applied via `cargo clippy --fix`
      across all three workspaces plus the wasm target; `fmt` fixed the
      indentation `--fix` leaves behind.
- [x] Final scope: **37 files** (13 manifests + harness + 4 `use<>` + 19
      let-chains), versus 215 when the reformat was tangled in.

**Run**

- [x] `cargo clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown`
      → exit 0 (AC3)
- [x] `cargo xtask check` → PASS
- [x] `cargo xtask validate` → **PASS**, 697 s, every step green including
      `nix-e2e`. (First attempt failed in 464 ms on the `clean-tree`
      precondition — `validate` is the pre-push gate and refuses a dirty tree,
      so it runs _after_ the commit, not before.)
- [x] Commit `d4b93c26`:
      `build: move the whole workspace to edition 2024 (#826)`
- [x] No unbudgeted breakage appeared in the newly-compiled test targets.

**If new breakage appears** in the newly-compiled test targets: fix it here if
it is edition-forced and mechanical; if it turns out to be a design question,
stop and raise it rather than improvising.

---

## T10 — ADR

Draft exists at `docs/adr/0100-edition-2024-unsafe-env-and-precise-capturing.md`
and is **gitignored**, so it does not land until promoted (AC12).

- [x] Re-read against what actually shipped: rewritten to the handle API with
      the reader-only and interleaved-state rationale, corrected call-site
      counts, a new decision 3 covering resolver 3 and `style_edition`, and the
      `-D warnings` measurement error recorded.
- [x] `cargo xtask adr promote` → **ADR-0100**, status proposed → accepted,
      README table synced (101 rows).
- [x] Fixed the `# ADR-XXXX:` heading the promote left behind — the `adr-format`
      gate catches it, which is how it was found.
- [x] Confirmed: the branch diff contains `docs/adr/0100-…md` and
      `docs/README.md` (the ADR index), and `adr-readme-parity` is green.
- [x] Gate: `cargo xtask check` → PASS.
- [x] Commit `f4095511`:
      `docs(adr): record the edition-2024 env and precise-capturing conventions (#826)`

---

## T11 — Conformance sweep

Walk every AC and record the result. Not a formality — three ACs in the first
draft of the spec were wrong precisely because nobody ran the commands.

- [x] **AC1** → 13 lines, all `"2024"`.
- [x] **AC2** `cargo xtask validate` → **PASS**, 697 s, every step green
      including `nix-e2e`.
- [x] **AC3** wasm clippy → exit 0 (also gate-covered).
- [x] **AC4** `rg -n '(set|remove)_var' -g '*.rs'` → only
      `common/src/test_support/env.rs` (5 matches: 1 doc line, 4 calls).
- [x] **AC5** → empty.
- [x] **AC6–AC9** seven `with_env` unit tests present and passing; exactly one
      env lock static in the workspace.
- [x] **AC9b** `with_env` counts vs former lock acquisitions (each count
      includes the import line): `cli.rs` 33 vs 32 locks · `observability.rs` 20
      vs 19 surviving · `capture.rs` 4 (2 sites + import + one comment
      reference) vs 2 · `db.rs` 3 vs 2 · `postgres/mod.rs` 6 vs 5. **No
      reader-only section was dropped.**
- [x] **AC10** gate warning-clean.
- [x] **AC11** all three roots `resolver = "3"`; #835 closed with the
      correction.
- [x] **AC12** ADR-0100 + `docs/README.md` in the branch diff.
- [x] **AC13** harness generates `edition = "2024"`; doctest gate green.
- [x] **AC14** `server/Cargo.toml` dev-deps `common` with `"test-support"`.
- [ ] **AC15 — VIOLATED IN LETTER, by one line.** `avatar/component.rs` is
      untouched; `feed_discovery/component.rs` has one import resorted by the
      repo-wide style-edition reformat. Unavoidable once that reformat was taken
      (exempting files fails `fmt`). No suppression or signature touched, so the
      criterion's purpose holds; #301 gets a trivial one-line rebase conflict.
      Recorded in the spec rather than ticked.
- [x] **AC16 — exact.** `cargo nextest list --workspace`, fork point vs HEAD:
      **2876 → 2882**. Added: exactly the 7 new `common::test_support::env`
      tests. Removed: exactly 1,
      `jaunder observability::tests::lock_env_recovers_from_poisoned_mutex`
      (spec D1a). **Every other one of 2,875 tests is set-equal.** `#[ignore]`
      count unchanged at 2, both pre-existing in
      `storage/src/sqlite/feed_events.rs`.

**15 of 16 clean; AC15 deviates by one formatting line, documented.**

Then hand off to **`jaunder-ship`**.
