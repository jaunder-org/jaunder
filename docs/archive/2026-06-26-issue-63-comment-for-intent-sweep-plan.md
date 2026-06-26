# Comment-for-Intent Sweep Implementation Plan (issue #63)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Dispatch each area's subagent with `jaunder-dispatch` content folded into the **Subagent Brief** below.

**Goal:** Bring existing code in line with the comment-for-intent convention (#62) — add *why* comments to non-obvious code, prune redundant *what*-narration, add module purpose headers — without burying the code in comments.

**Architecture:** Behavior-frozen sweep. The codebase is partitioned into **12 disjoint file-set areas**. Each area is a parallel Opus subagent pass (**≤2 concurrent**); the controller reviews each returned diff against the restraint bar, spot-fixes, and **commits that area** (`docs(<area>): …`). After all areas, the controller runs one compile sanity check, then **HALTS for the user's local review of the committed history — before push and before the PR**. The heavy `validate --no-e2e` gate runs only when the user approves the push.

**Tech Stack:** Rust (5 product crates + `xtask` + `tools`), TypeScript/Playwright (`end2end/`). Targeting signal: `crap-manifest.json`.

## Global Constraints

Every task's subagent inherits these verbatim (the **restraint bar**):

- **Behavior-frozen:** comments and doc-comments only. ZERO changes to code, signatures, formatting, or imports. Do not run `cargo fmt`/`leptosfmt`.
- **Add** an intent/why comment *only* where a competent reader cannot readily infer why the code exists, or where the code takes a path that is not, at first glance, the obvious one — there, the comment's job is to return the surprising-looking code to a state of being **obviously correct**.
- **Never** comment self-evident code (getters, plain mappings, obvious control flow, well-named functions with plain bodies). **When in doubt, leave it out.**
- **Prune** a comment that only restates adjacent code. **Keep** any comment carrying rationale, an ADR/issue reference, or a non-obvious-constraint warning — even if terse.
- **Tests:** comment only a non-obvious case (what invariant/edge it guards), never Arrange/Act/Assert mechanics.
- **Module `//!`:** add a purpose header only where absent; do not pad existing ones.
- **Match** surrounding comment style and density. One comment per non-obvious decision.
- **Scope:** the 5 product crates + `xtask/` + `tools/` + `end2end/`. (Generated files, migrations SQL, and build scripts are out of scope.)

---

## Subagent Brief (template — every area task supplies AREA, FILES, CRAP SLICE)

Paste this into each dispatched subagent, filling the three variables. Dispatch with **model: Opus** and **mode: acceptEdits** (so edits to existing files persist without interactive approval, which background agents cannot grant).

> You are doing a **comment-for-intent** pass over a bounded set of files in the jaunder repo. **Read `CONTRIBUTING.md` → `## Code conventions` → the "Comment for intent" bullet first** — that convention is your specification.
>
> **Behavior-frozen:** you may ONLY add/edit/remove comments and doc-comments. Do not change code, signatures, imports, or formatting. Do not run any formatter or gate; the controller gates afterward.
>
> **Your files (edit only these, in place, via the Edit tool at the absolute paths given):** `<FILES>`
>
> **The bar:**
> - Add a *why* comment only where a reader can't readily infer why the code exists, or where the code takes a path that isn't, at first glance, the obvious one — there the comment must make the surprising code obviously correct.
> - Never comment self-evident code. **When in doubt, leave it out.**
> - Prune comments that merely restate adjacent code. Keep any comment with rationale, an ADR/issue ref, or a constraint warning.
> - Tests: comment only non-obvious cases (the invariant/edge a case guards), never mechanics.
> - Add a module `//!` purpose header only where absent.
> - Match surrounding style/density.
>
> **Look here first (highest CRAP/complexity = most likely obscured intent):** `<CRAP SLICE>`. These functions are complex or under-covered; if a path in them is non-obvious, that's exactly where a why-comment earns its place. Several relate to documented decisions — where relevant, cite the ADR/issue rather than re-explaining (e.g. read-then-write transaction retries → ADR-0021 / issues #51–#53; password-reset hashing order → ADR-0022 / issue #60). Don't force a comment if the code is already obviously correct.
>
> **Worked examples:**
> ```rust
> // ❌ mechanical — restates the code
> // Loop over the sessions and remove the expired ones.
>
> // ✅ intent + why the non-obvious path
> // Reap expired sessions on read rather than via a background sweep:
> // logins are rare enough that a periodic job isn't worth the moving part,
> // and reaping here keeps the auth check the single source of truth.
> ```
> ```rust
> // ❌ redundant — prune this
> let lower = username.to_lowercase(); // lowercase the username
>
> // ✅ why it must happen *here* (returns surprising code to obviously-correct)
> // Lowercase at the boundary, before storage sees it: usernames are
> // case-insensitive identifiers, and every lookup assumes the stored form
> // is already normalized.
> let lower = username.to_lowercase();
> ```
>
> **Return a concise change report**, grouped by file: each comment ADDED (`path:line` + the intent in a few words) and each comment PRUNED (`path:line` + why redundant). Do not paste full diffs. Do not claim a file is done if you did not open it.

---

## Per-area cycle (identical for every task below)

Each task supplies only **FILES**, **CRAP SLICE**, and any **NOTES**. The controller runs this 3-step cycle for it:

1. **Dispatch** the Subagent Brief (Opus, acceptEdits) with the task's FILES + CRAP SLICE + NOTES.
2. **Review** the returned report and `git diff -- <task paths>`: reject/spot-fix any comment on self-evident code (over-commenting) and any pruned comment that carried rationale (over-pruning); confirm the diff touches comment lines only (`git diff --stat` + scan — no code/signature/format changes).
3. **Commit the area**: `git add <task paths> && git commit -m "docs(<area>): comment for intent" -m "Refs: #63"` (no `Co-Authored-By`). Tick the task's checkbox.

Areas have **disjoint file sets**, so the ≤2 concurrent agents never touch the same file; each commits independently.

---

## Tasks

- [ ] **Task 1 — `common`.** FILES: `common/src/**/*.rs` (26). CRAP: `atompub/entry.rs` `read_xhtml_content` L235 (21), `Parser::start` L128 (14), `entry_from_xml` L199 (14), `resolve_ref` L332 (12), `Parser::text` L168 (9), `write_entry` L424 (9); `feed/feed_path.rs` `parse` L51 (17); `render.rs` `extract_org_title` L149 (9).

- [ ] **Task 2 — `storage` core.** FILES: `storage/src/*.rs` (top-level only; exclude `sqlite/`+`postgres/`; ~23). CRAP: `backup.rs` `export_directory_backup` L158 (13), `restore_media_entries` L425 (12), `mirror_media_entries` L467 (10), `order_by_clause` L235 (10), `previous_directory_backup` L543 (8), `copy_or_link_media_file` L502 (8), `restore_backup` L109 (8); `post_service.rs` `perform_post_creation` L262 (13); `invites.rs` `use_invite` L101 (8); `smtp.rs` `load_smtp_config` L109 (8).

- [ ] **Task 3 — `storage` backends (sqlite + postgres).** FILES: `storage/src/sqlite/**/*.rs` (15) + `storage/src/postgres/**/*.rs` (16) — one agent sees BOTH. CRAP: `postgres/mod.rs` `confirm_password_reset` L139 (19, cov66% — ADR-0022/#60), `create_user_with_invite` L76 (17, cov66% — ADR-0021/#51); `postgres/posts.rs` `update_post` L26 (11 — #52), `tag_post` L99 (11 — #53); `postgres/backup.rs` `export_database` L17 (12), `restore_database` L58 (11), `export_table` L219 (8); `sqlite/backup.rs` `restore_database` L58 (13), `export_database` L17 (12), `bind_json_value` L166 (8), `export_table` L217 (8); `sqlite/mod.rs` `create_user_with_invite` L149 (12), `confirm_password_reset` L213 (12); `sqlite/posts.rs` `update_post` L25 (11), `tag_post` L97 (11); `sqlite/feed_events.rs` `claim_pending_batch` L27 (10). **NOTES:** where the backends deliberately diverge (PG `ON CONFLICT` vs SQLite retry, isolation handling), the why-comment must name the divergence and its reason — prime "Backend parity rules" territory; keep the two sides' comments consistent.

- [ ] **Task 4 — `server/src` core.** FILES: `server/src/*.rs` (top-level only; exclude subdirs; 28). CRAP: `main.rs` `run` L33 (26, cc23 — highest in repo); `commands.rs` `prepare_server` L329 (10), `directory_has_entries` L286 (9), `cmd_user_create` L104 (8); `backup.rs` `prune_backups` L119 (11); `media.rs` `serve_response` L136 (9).

- [ ] **Task 5 — `server/src` subsystems.** FILES: `server/src/{atompub,feed,mailer,websub}/**/*.rs` (17). CRAP: `atompub/mod.rs` `atompub_op` L70 (13), `atompub/posts.rs` `member_put` L339 (13), `collection_post` L260 (11), `collection_get` L52 (8), `atompub/media.rs` `collection_post` L53 (8); `feed/handlers.rs` `serve` L24 (14), `feed/regenerate.rs` `regenerate_feed` L28 (14), `feed/worker.rs` `tick` L54 (13); `mailer/smtp.rs` `send_email` L77 (14, cov37% — under-tested, intent likely obscure), `from_config` L36 (9).

- [ ] **Task 6 — `server/tests` web + helpers.** FILES: `server/tests/web/**/*.rs` (13) + `server/tests/helpers/**/*.rs` (2). CRAP: none (test fns). **NOTES:** tests rule — comment only non-obvious cases / what invariant each shared helper guards; test bodies are the biggest over-comment risk, guard hard.

- [ ] **Task 7 — `server/tests` storage.** FILES: `server/tests/storage/**/*.rs` (2 files, ~6637 lines — large; page through, don't assume). CRAP: none. **NOTES:** tests rule; these large files likely encode non-obvious backend-parity/edge expectations — comment the *case*, not the assertions.

- [ ] **Task 8 — `server/tests` rest.** FILES: `server/tests/{atompub,feed,misc}/**/*.rs` (17). CRAP: none. **NOTES:** tests rule.

- [ ] **Task 9 — `web/src` pages.** FILES: `web/src/pages/**/*.rs` (16). CRAP: `pages/backup.rs` `backup_settings_form` L47 (12, **cov0%**), `pages/upload.rs` `upload_file` L143 (10). **NOTES:** pages mostly *render* (CONTRIBUTING: "components should only render data") — expect FEW additions; flag only non-obvious behavior (hydration quirks, conditional render tied to auth/visibility). Strong over-comment guard.

- [ ] **Task 10 — `web/src` server-fns + `hydrate`.** FILES: `web/src/*.rs` (top-level, 4) + `web/src/{auth,posts,audiences,backup,email,invites,media,password_reset,profile,sessions,site,subscriptions,tags}/**/*.rs` (exclude `pages/`) + `hydrate/src/**/*.rs` (1). CRAP: `web/src/error.rs` `as_metric_str` L94 (8). **NOTES:** server-fn boundaries are prime intent territory — `require_auth`, username lowercasing, storage-error→`ServerFnError` mapping; comment *why* at these boundaries.

- [ ] **Task 11 — `end2end` (Playwright TS).** FILES: `end2end/tests/**/*.ts` + `end2end/playwright.config.ts` (20). CRAP: N/A. **NOTES:** targets are the HELPERS — `hydration.ts`, `perf.ts`, `otel.ts`, `actions.ts`, `helpers.ts`, `websub.ts`, `mail.ts`, `fixtures.ts` (environment-aware timeouts/warmup/hydration budgets/OTel capture — ADR-0012, flakiness #18/#49). `*.spec.ts` bodies are declarative — skip unless a step is genuinely surprising. Use `//` comments, match TS style.

- [ ] **Task 12 — `xtask` + `tools` (dev tooling).** FILES: `xtask/src/**/*.rs` (13) + `tools/coverage/src/**/*.rs` (3) + `tools/devtool/src/**/*.rs` (3). CRAP: none (not instrumented). **NOTES:** `xtask/src/coverage/**` (classify.rs, diffmap.rs, crap.rs, report.rs, baseline.rs) is the logic-dense part — the coverage classification/diff-attribution rules are non-obvious and tie to issues #3/#7/#37; comment the *why* of the classification choices. The step/runner glue is mostly self-evident — skip it.

---

## Landing phase (controller only — after all 12 area commits)

- [ ] **L0: Convention + planning-doc commit.** Ensure the `CONTRIBUTING.md` wording sharpening and the spec/plan docs are committed (e.g. a `docs:` commit `Refs: #63`). (May be done first or last; keep it a separate commit from the code-area commits.)
- [ ] **L1: Whole-diff consistency pass.** `git log --oneline` + skim each area commit for uniform tone/density and any stray code change. Fix inline (fixup commits ok).
- [ ] **L2: Compile sanity (fast, not the full gate).** `cargo check --workspace` in the worktree — catches a malformed doc-comment. Fix + amend if needed. (This is the only build run before the user looks — NOT `validate`.)
- [ ] **L3: HALT — user reviews committed history locally** (`git log`/`git diff main...HEAD`). No push, no PR yet. Incorporate requested changes as follow-up commits.
- [ ] **L4: On user approval — gate then ship.** `cargo xtask validate --no-e2e` (comments don't affect coverage/e2e, but it's the pre-push gate), then `jaunder-ship` (archive spec/plan, push, open PR referencing #62/#63). **Merge is a separate halt point.**

---

## Self-review (plan vs spec)

- **Coverage:** scope (5 product crates prod+test + `xtask` + `tools` + e2e) → Tasks 1–12 cover `common`(1), `storage`(2,3), `server/src`(4,5), `server/tests`(6,7,8), `web`(9,10), `hydrate`(10), `end2end`(11), `xtask`+`tools`(12). ✓ Restraint bar → Global Constraints + Brief. ✓ CRAP signal → per-task slices (xtask/tools/tests have none — noted, not a gap). ✓ ≤2 concurrent → architecture. ✓ Commit-per-area + halt-before-push/PR (revised per user) → per-area cycle Step 3 + Landing L3. ✓ Compile-check-not-validate before review → L2. ✓ Prune + module headers + tests-only-non-obvious → Global Constraints. ✓
- **Placeholders:** none — exact paths + real CRAP slices per task.
- **Consistency:** area file-sets are disjoint (pages excluded from Task 10; backend subdirs excluded from Task 2; `server/src` subdirs excluded from Task 4) — required for safe parallel per-area commits.
