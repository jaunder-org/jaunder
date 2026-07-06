# Plan — Issue #94: Lint suppression audit → zero `#[allow]`

**Spec:**
[`docs/superpowers/specs/2026-07-05-issue-94-lint-suppression-audit.md`](../specs/2026-07-05-issue-94-lint-suppression-audit.md)
**For agentic workers:** drive with **`jaunder-iterate`**; delegate an
individual task via **`jaunder-dispatch`** when useful. Tick checkboxes in real
time. Commit per task (**`jaunder-commit`**); no `Co-Authored-By` trailer.

## Review header

**Goal.** Remove every in-source `#[allow(...)]` (192 sites / 309+ lint
occurrences), fixing the code by default; leave only the three approved
`#[expect]` keepers (K1–K3), each with an inline rationale.
`clippy::pedantic = warn` and `unwrap_used`/`expect_used = deny` stay on.

**Scope.**

- _In:_ all `*.rs` suppressions (clippy + rustc), production and test; the one
  `web/Cargo.toml` lint-disable line (spec D-config-1); converting the three
  keepers to `#[expect]`.
- _Out:_ the anti-regrowth guardrail (Task 1 files it as a follow-up issue); any
  `clippy.toml` change; runtime skips / `#[ignore]` / `cov:ignore`; weakening
  any lint level.

**Tasks (one line each).**

1. File the anti-regrowth guardrail as a follow-up issue (separable concern).
2. `web` `must_use_candidate`: crate lint-disable + delete all ~48 allows +
   `#[must_use]` on the 1 helper.
3. Production `expect`/`unwrap`: fix `media_manager` panic + the 3 feature-gated
   `src/` expects.
4. `cast_*` (14): checked conversions or proof-carrying `#[expect]`.
5. `needless_pass_by_value` (9): borrow instead of move.
6. `too_many_arguments` (9): parameter structs.
7. `too_many_lines` — production (~12): split functions.
8. One-off prod/rustc lints: `struct_field_names`, `similar_names` (storage),
   `duration_suboptimal_units`, `format_collect`, `unused_variables`,
   `unused_imports`, `dead_code`.
9. Test-code `items_after_statements` (26): hoist items above statements.
10. Test-code `unused_async` (26): drop needless `async`.
11. Test-code `too_many_lines` (~33): split long test functions.
12. Test-code `similar_names` (~26): rename.
13. Delete redundant test `unwrap_used`/`expect_used` + inert `unused_macros`
    allows; collapse emptied blocks.
14. Keepers → `#[expect]`: K1 `Resource::new`, K2 `test_support` scaffolding, K3
    `rstest_reuse` (fix-or-expect).
15. Final gate: `cargo xtask validate` green + survey reports **0** `#[allow]`.

**Key risks / decisions.**

- Test-code churn (Tasks 9–12) is broad but behavior-preserving; each is its own
  commit.
- `cast_*` (Task 4): a wrong "lossless" claim hides a truncation bug — prefer
  `TryFrom`.
- `unused_macros` (Task 13) deletion is safe (no `macro_rules!` in the test
  tree); the gate re-flags any real use.
- K3 `rstest_reuse` may resist a fix; bounded fallback is a consolidated
  justified `#[expect]`.

## Global constraints

- **Survey is the ratchet.** After each task,
  `rg -n -e '#!?\[allow\(' -e '#!?\[expect\(' --glob '*.rs' --glob '!target/**'`
  must show the touched lint's `#[allow]` count strictly lower and never
  re-introduce one. Final state: 0 `#[allow]`.
- **Never weaken a lint level.** No edits to `clippy.toml`; no new
  crate/workspace `allow` except the single approved `web` `must_use_candidate`
  line. `pedantic = warn` and the `deny`s stay.
- **Every surviving `#[expect]` carries an inline rationale** and is one of
  K1–K3 (or a Task-4 proof-carrying cast exception, called out in its commit
  message).
- **Behavior-preserving.** Refactors (splits, param structs, borrows) must not
  change behavior; rely on the existing suite + coverage gate. Storage prod-code
  touches don't add tests, so no backend-parity obligation arises; do **not**
  edit ADR-0019 dialect files.
- **Gate before commit.** Run `cargo xtask check` clean before each commit
  (**`jaunder-commit`**); the pre-commit hook runs the full check.
- **Grouping.** One commit per task, message `fix(lint): <category> (#94)` (or
  `chore`/`refactor` as apt). Tasks 9–13 each edit the shared test blocks for a
  single lint only, leaving the block valid until Task 13 empties it.

---

## Task 1 — File the anti-regrowth guardrail follow-up issue

Separable concern per spec "Out of scope". Use **`jaunder-issues`**.

- [x] Create issue: **#294** (Task, `tooling`, milestone "Code quality
      improvement"), native blocked-by #94, added to Backlog project #1, linked
      from #94 via comment.

_Verify:_ `gh issue view <new#>` shows it open and linked from #94. No code
change.

## Task 2 — `web` `must_use_candidate`: disable + delete allows + fix the helper

Spec D-config-1. Files: `web/Cargo.toml`; all `web/src/pages/*.rs` +
`web/src/**` carrying the allow (`ui.rs`, `posts.rs`, `auth.rs`, `profile.rs`,
`password_reset.rs`, `audiences.rs`, `upload.rs`, `media.rs`,
`feed_discovery.rs`, `mod.rs`, `home.rs`, `email.rs`, `cockpit.rs`, `backup.rs`,
`invites.rs`, `sessions.rs`, `site.rs`, `timeline.rs`).

- [x] `web/Cargo.toml`: add under `[lints.clippy]` (user chose web-only: dropped
      `workspace = true` and re-declared the workspace lints locally + the
      exception, since Cargo forbids augmenting `workspace = true`)
      ``toml     # Leptos view fns return `impl IntoView` consumed by the framework; a caller can't     # ignore the return, so must_use_candidate is noise crate-wide here. (#94)     must_use_candidate = "allow"     ``
- [x] Delete every `#[allow(clippy::must_use_candidate)]` in `web/` (~48
      occurrences — all but the one helper sit on Leptos view fns).
- [x] Add `#[must_use]` to `web/src/pages/ui.rs::local_datetime_to_utc_rfc3339`
      with a comment noting it's a deliberate manual keep (the crate disable
      means clippy no longer flags it).

_Verify:_ `rg 'allow\(clippy::must_use_candidate' web/` → no matches;
`cargo xtask check` green (`-p web` clippy passes with the disable in place).

## Task 3 — Production & feature-gated `expect`/`unwrap`

Spec P2 + F-misc. Files: `server/src/media_manager.rs`, `common/src/mailer.rs`,
`common/src/password.rs`.

- [x] `server/src/media_manager.rs:~265`: replace
      `.parent().expect("target path has a parent")` with
      `.parent().ok_or_else(|| anyhow::anyhow!("media target path {} has no parent",     target_path.display()))?`
      (fn returns `anyhow::Result`). Remove the `#[allow]` + the "provably
      unreachable" comment.
- [x] `common/src/mailer.rs` (`test_utils` `CapturingMailSender`, ×2): replace
      `.lock().expect("mutex poisoned")` with
      `.lock().unwrap_or_else(|e| e.into_inner())` (recover the poisoned guard).
      Remove both `#[allow]`s. Update the `/// Panics…` doc line.
- [x] `common/src/password.rs` (`cheap-kdf` branch): make `hasher()` return
      `Result<argon2::Argon2<'static>, PasswordError>` (or construct params via
      `?`) and propagate through `hash()` (already `-> Result`). Remove the
      `#[allow]`. _If propagation proves disproportionate,_ fall back to a
      justified `#[expect(clippy::expect_used)]` + proof comment and note the
      fallback in the commit message.

_Verify:_ `rg 'allow\(clippy::(unwrap|expect)_used' common/src server/src` → no
matches; `cargo nextest run -p common -p server` green; `cargo xtask check`
green.

## Task 4 — `cast_*` (14)

Files: `web/src/render/mod.rs`, `web/src/pages/ui.rs`, `web/src/pages/media.rs`,
`server/src/observability.rs`, `common/src/feed/window.rs`.

- [x] For each `cast_possible_truncation`/`cast_precision_loss`/`cast_sign_loss`
      allow: replace the `as` cast with a checked `TryFrom`/`try_into` (handling
      the error path), or — where the value is provably in range — keep a single
      `#[expect(clippy::cast_…)]` with an **in-comment proof** of the bound.
      Prefer the checked conversion.

_Verify:_ `rg 'allow\(clippy::cast_' --glob '*.rs' .` → no matches (only
proof-carrying `#[expect]` if any); `cargo xtask check` green.

## Task 5 — `needless_pass_by_value` (9)

Files: `web/src/pages/ui.rs` (×4), `web/src/feed_discovery.rs` (×2),
`web/src/posts/server.rs`, `web/src/pages/media.rs`,
`server/src/media_manager.rs`.

- [x] Fixed the 3 plain-fn sites by borrowing (`render_media_row` →
      `&MediaItem`, `private_post_not_found_error` → `&InternalError`,
      `validate_filename` → `Option<&str>`); the 6 `#[component]` sites →
      justified `#[expect]` (Leptos props must be owned — framework false
      positive, like `must_use_candidate`).

_Verify:_ `rg 'allow\(clippy::needless_pass_by_value' --glob '*.rs' .` → none;
`cargo xtask check` green.

## Task 6 — `too_many_arguments` (9)

Files: `storage/src/post_service.rs` (×3), `storage/src/posts.rs` (×2),
`web/src/posts/mod.rs` (×2), `server/src/atompub/posts.rs`,
`server/src/atompub/mapping.rs`.

- [x] Param structs for 6 internal fns (`RenderedPostContent`,
      `RenderedPostUpdate`, `PermalinkDate`, `MakePost`); `member_put` axum
      handler → `#[expect]`; the 2 Leptos `#[server]` fns → justified `#[allow]`
      (macro-emitted lint; `#[expect]` impossible — the only 2 plain `#[allow]`s
      left in the audit).

_Verify:_ `rg 'allow\(clippy::too_many_arguments' --glob '*.rs' .` → none;
`cargo nextest run -p storage -p web -p server` green; `cargo xtask check`
green.

## Task 7 — `too_many_lines` — production (~12)

Files: `web/src/pages/posts.rs` (×4), `web/src/pages/ui.rs` (×3),
`web/src/pages/media.rs`, `web/src/pages/audiences.rs`, `storage/src/posts.rs`,
`server/src/media.rs`, `server/src/feed/worker.rs`.

- [ ] Extract cohesive sub-functions from each long function until under
      threshold. Behavior- preserving; keep names descriptive. Remove each
      item-level `#[allow(clippy::too_many_lines)]` (leave any test-crate
      blanket instances for Task 11).

_Verify:_ item-level prod `too_many_lines` allows gone; `cargo xtask check`
green; coverage gate green (`cargo xtask check` includes it).

## Task 8 — One-off prod & rustc lints

- [ ] `storage/src/helpers.rs` `struct_field_names`: rename fields to drop the
      redundant prefix.
- [ ] `storage/src/backup.rs` `similar_names`: rename the two similar bindings.
- [ ] `server/src/feed/worker.rs` `duration_suboptimal_units`: use the suggested
      unit constructor.
- [ ] `common/src/feed/metadata.rs` `format_collect`: replace with the suggested
      `fold`/`write!` form.
- [ ] `web/src/pages/upload.rs` `unused_variables`: use or drop the variable
      (prefix `_` only if semantically a placeholder; prefer removing).
- [ ] `server/tests/helpers/mod.rs` `unused_imports` (×2) + `dead_code`: delete
      the dead re-exports/items, or — if a re-export is legitimately used only
      under some backend/feature — convert to a justified `#[expect]` with
      rationale (note in commit).

_Verify:_ `rg` for each of these lints → none (or justified `#[expect]` for the
conditional re-export); `cargo xtask check` green.

## Task 9 — Test-code `items_after_statements` (26)

Files: the ~27 `server/tests/**` module files carrying the shared block.

- [ ] In each flagged function, move `fn`/`struct`/`const` item definitions
      above the first statement. Remove `clippy::items_after_statements` from
      that file's `#![allow(...)]` block (leave the block's other entries).

_Verify:_ `rg 'items_after_statements' --glob '*.rs' .` → none;
`cargo nextest run -p server` green; `cargo xtask check` green.

## Task 10 — Test-code `unused_async` (26)

Files: same test modules (+ `server/tests/helpers/mod.rs`).

- [ ] Remove `async` from helpers/fixtures that never `.await`; update call
      sites (drop `.await`). Where a trait/signature forces `async`, satisfy the
      lint another way (e.g. it does await, or restructure). Remove
      `clippy::unused_async` from each block.

_Verify:_ `rg 'unused_async' --glob '*.rs' .` → none;
`cargo nextest run -p server` green; `cargo xtask check` green.

## Task 11 — Test-code `too_many_lines` (~33)

Files: the test modules + each test crate root `main.rs` that lists it.

- [ ] Split long test functions into focused helper steps (behavior/assertions
      unchanged). Remove `clippy::too_many_lines` from every test block/root
      that lists it.

_Verify:_ `rg 'too_many_lines' --glob '*.rs' .` → none (Task 7 cleared prod);
`cargo nextest run -p server` green; `cargo xtask check` green.

## Task 12 — Test-code `similar_names` (~26)

Files: the test modules.

- [ ] Rename similarly-named bindings to distinct, descriptive names. Remove
      `clippy::similar_names` from each block (and `storage/src/backup.rs` was
      handled in Task 8).

_Verify:_ `rg 'similar_names' --glob '*.rs' .` → none;
`cargo nextest run -p server` green; `cargo xtask check` green.

## Task 13 — Delete redundant test `unwrap`/`expect` + inert `unused_macros` allows

By now each test block has only `clippy::unwrap_used`, `clippy::expect_used`,
and `unused_macros` left (all redundant/inert). The 27
`single_component_path_imports` allows on `use rstest_reuse;` are **not**
touched here — they are handled in Task 14 (K3), so this task's verify is scoped
to the lints it actually removes.

- [ ] Delete `clippy::unwrap_used`/`clippy::expect_used` from every
      `server/tests/**` block and crate-root `main.rs` (the pre-existing
      `clippy.toml` `allow-*-in-tests` covers these integration-test target
      crates).
- [ ] Delete the **2 standalone** `#[allow(clippy::expect_used)]` in
      `server/tests/helpers/websub_capturing.rs` (~lines 25, 33) — item-level,
      not in a block.
- [ ] Delete every `#![allow(unused_macros)]` (no `macro_rules!` exists in the
      test tree — inert).
- [ ] Remove now-empty `#![allow(...)]` attributes entirely (blocks reduced to
      only `single_component_path_imports` remain until Task 14).

_Verify:_
`rg -n 'allow\(clippy::(unwrap|expect)_used\)|allow\(unused_macros\)' --glob 'server/tests/**/*.rs'`
→ none; `cargo nextest run -p server` green; `cargo xtask check` green. (The
blanket "0 `#[allow]`" assertion lives at Task 15, after K3 clears the last test
allows.)

## Task 14 — Convert the three keepers to `#[expect]`

Spec K1–K3.

- [ ] **K1** `web/src/error.rs:~397`: `#[allow(clippy::disallowed_methods)]` →
      `#[expect(clippy::disallowed_methods, reason = "the one sanctioned Resource::new; all other     call sites must use web::server_resource (#124)")]`
      (keep/merge the existing comment).
- [ ] **K2** `storage/src/test_support.rs:~14`:
      `#![allow(clippy::unwrap_used, clippy::expect_used)]` →
      `#![expect(clippy::unwrap_used, clippy::expect_used, reason = "deliberately unwrap/expect-     heavy both-backend test scaffolding; test-support feature, ADR-0033")]`.
      Keep the existing rationale comment.
- [ ] **K3** `single_component_path_imports` (27×, `use rstest_reuse;`): first
      try a fix that satisfies the lint (e.g. `use rstest_reuse::*;`, or hoist a
      single import to each test crate root). If none works, replace each
      `#[allow]` with
      `#[expect(clippy::single_component_path_imports,     reason = "rstest_reuse 0.7 requires this bare import for its #[template]/#[apply] macros")]`,
      consolidated to the fewest sites (ideally one per test crate root).

_Verify:_ `rg -n '#!?\[expect\(' --glob '*.rs' .` shows only K1/K2/K3 (+ any
Task-4 proof cast); `rg -n '#!?\[allow\(' --glob '*.rs' --glob '!target/**' .` →
**0**; `cargo xtask check` green.

## Task 15 — Final gate & survey

- [ ] Run the survey:
      `rg -n -e '#!?\[allow\(' -e '#!?\[expect\(' --glob '*.rs' --glob '!target/**'`
      → zero `#[allow]`; every `#[expect]` is K1/K2/K3 or a Task-4 proof cast,
      each with rationale.
- [ ] `cargo xtask validate` green (static + clippy `-D warnings` + coverage +
      all four e2e combos).
- [ ] Confirm AC-config: `web/Cargo.toml` has the one lint line; `clippy.toml`
      unchanged; `pedantic = warn` and the `deny`s intact.

_Verify:_ all acceptance criteria in the spec (AC-zero-allow, AC-keepers,
AC-no-prod-panic, AC-config, AC-gate, AC-behavior, AC-followup) hold.
