# Plan — #298: collapse the six server integration-test crates into one binary

Spec:
[`2026-07-13-issue-298-server-test-support.md`](../specs/2026-07-13-issue-298-server-test-support.md).
The spec is "what/why"; this plan is "how." Sections referenced by name below.

## Review header

**Goal.** Retire the `#[path]`-clone of `server/tests/helpers/mod.rs` by making
the six integration-test crates one binary, so the 3 `#[allow]`s drop and the 6
crate-level `#![expect]`s fold to 1 — zero net-new suppressions,
`cargo xtask validate` green.

**Scope.**

- _In:_ the collapse (single `[[test]]` target), dropping the
  `storage::test_support` compat re-export + converting import sites, fixing 2
  `misc` cross-refs, removing the unread `CapturedPing.sent_at` field, an ADR,
  and a CONTRIBUTING prose fix.
- _Out:_ consolidating the remaining duplicated local helpers — already filed as
  **#429** / **#430** (both `blocked-by #298`); no behavior change to any test.

**Tasks.**

1. Drop the `storage::test_support` compat re-export; convert all import sites
   to direct imports. (−1 `#[allow]`)
2. Remove the unread `CapturedPing.sent_at` field. (unblocks removing the
   `dead_code` allow)
3. Collapse to one `integration` test binary; drop the remaining 2 allows + fold
   6 `#![expect]` → 1; fix the 2 `misc` cross-refs. (the structural core)
4. Record the decision as an ADR draft.
5. Fix the stale `CONTRIBUTING.md` references to `helpers/mod.rs`.

**Key risks / decisions.**

- Tasks 1 and 3 are each **atomic** — the tree does not compile partway through
  the import sweep or the crate merge — so each is verified as a whole by a
  green suite, not sub-stepped. Task 1 is a mechanical sweep → good
  `jaunder-dispatch` candidate.
- Collapse renames `tests/<x>/main.rs` → `mod.rs`; `autotests = false` + one
  explicit `[[test]]` prevents any subdir being auto-discovered as a stray
  target.
- Verify the coverage classifier still works when the binary is `integration`
  and test paths carry a `<subsystem>::` prefix (Task 3) — no external refs to
  the old target names were found, but confirm.

**For agentic workers.** Execute with **`jaunder-iterate`**, delegating a task
to **`jaunder-dispatch`** where noted. Tick checkboxes in real time.

## Global constraints

- **No behavior change.** This is a pure refactor; the existing integration
  suite is the regression net. Every task ends with
  `cargo nextest run -p jaunder` green **and** `cargo xtask check` clean (fmt +
  clippy + Nix coverage/tests) before committing (**`jaunder-commit`**; the
  pre-commit hook runs the full `cargo xtask check`).
- **`validate` (incl. e2e) runs once at ship.** Per-task verify uses `check` (no
  e2e): no task touches product code or the browser-driven e2e surface, so e2e
  is unaffected; the full `cargo xtask validate` gate (all four
  `{sqlite,postgres}×{chromium,firefox}` combos) is proven at
  **`jaunder-ship`**, which is where the spec's "`validate` green" criterion is
  formally met. The `jaunder` `--lib`/`--bin` `#[cfg(test)]` unit tests are
  untouched by the target-layout change (`autotests` governs only
  integration-test discovery) and run unchanged — the "same total nextest count"
  check in Task 3 covers them.
- **No new suppressions** of any kind (`#[allow]`/`#[expect]`, clippy or rustc).
  The only suppression that survives is the single crate-level
  `#![expect(clippy::unwrap_used, clippy::expect_used)]` in `tests/main.rs`.
- **No `Co-Authored-By` trailer** on commits.
- Commits: one per task; message references #298.

## Name mapping (used by Tasks 1 & 3)

Split each `use crate::helpers::{…}`. Import from **`storage::test_support`**
(the harness re-export being dropped):

> `Backend`, `TestEnv`, `TestBase`, `CloseablePool`, `PostgresDbGuard`,
> `PG_URL_FILE`, `backends`, `backends_matrix`, `sqlite_only`, `postgres_only`,
> `noop_mailer`, `seed_posts`, `nonexistent_postgres_url`,
> `postgres_bootstrap_url`, `postgres_test_authority`, `recorded_postgres_url`,
> `sqlite_url`, `template_postgres_url`, `unique_postgres_url`

Keep via **`crate::helpers`** (names `helpers` itself defines):

> `ensure_server_fns_registered`, `test_options`, `tmp_storage_path`,
> `CapturingWebSubClient`, and the `post_form*` family (`post_form`,
> `post_form_with_mailer`, `post_form_with_secure_flag`, `post_form_with_ua`,
> `post_form_with_bearer`)

---

## Task 1 — Drop the compat re-export; convert import sites

Removes the `unused_imports` allow at `helpers/mod.rs:24` (still 6 crates after
this). **Atomic** (won't compile partway). Good `jaunder-dispatch` candidate — a
mechanical sweep.

**Files / interfaces:**

- `server/tests/helpers/mod.rs`: delete the `#[allow(unused_imports)]` +
  `pub use storage::test_support::{…}` block (lines 24–30). `helpers` still uses
  some of those names internally (e.g. `post_form_inner` calls `noop_mailer()`),
  so add a plain
  `use storage::test_support::{noop_mailer /* + any other names helpers' own bodies use */};`
  at the top — determined by what fails to compile, not guessed.
- All ~27 files under `server/tests/*/` that do `use crate::helpers::{…}` (and
  inline `crate::helpers::<name>()` calls): apply the **Name mapping** above —
  harness names → `use storage::test_support::{…}`; local names stay
  `crate::helpers::{…}`. Rewrite inline calls the same way
  (`crate::helpers::noop_mailer()` → `storage::test_support::noop_mailer()`;
  `crate::helpers::seed_posts(…)` → `storage::test_support::seed_posts(…)`;
  `crate::helpers::tmp_storage_path()` stays).
- Do **not** touch the `websub_capturing` re-export (`helpers/mod.rs:32–36`) or
  the `#![allow(dead_code)]` — those are removed in Task 3.

**Verify:**

- `cargo nextest run -p jaunder` — all tests PASS (unchanged count).
- `cargo xtask check` — clean.
- `rg -n 'pub use storage::test_support' server/tests/helpers/mod.rs` — no
  match.
- `rg -n '#\[allow\(unused_imports\)\]' server/tests/helpers/mod.rs` — exactly
  **one** match remains (the `websub_capturing` one at ~line 35).

**Commit:**
`test(server): import harness from storage::test_support directly (#298)`

## Task 2 — Remove the unread `CapturedPing.sent_at` field

Per spec step 7. Independent and small; must land **before** Task 3 removes the
`dead_code` allow (otherwise Task 3's clippy trips on the never-read field).

**Files / interfaces:**

- `server/tests/helpers/websub_capturing.rs`: remove the `sent_at` field from
  `CapturedPing`, its `sent_at: Utc::now()` initializer in the `WebSubClient`
  impl, and the now-unused `chrono::{DateTime, Utc}` import.

**Verify:**

- `cargo nextest run -p jaunder feed` — the feed suite (the only
  `CapturingWebSubClient` consumer, `feed_worker.rs`) PASSES; it reads only
  `.hub_url`/`.feed_url`.
- `cargo xtask check` — clean (no `field is never read` even though
  `#![allow(dead_code)]` still masks it here — confirms the field truly had no
  reader).

**Commit:** `test(server): drop unread CapturedPing.sent_at field (#298)`

## Task 3 — Collapse to one integration-test binary

The structural core. Removes the `dead_code` allow (`helpers/mod.rs:5`) and the
`websub_capturing` `unused_imports` allow (~line 35), and folds the six
`#![expect]` → one. **Atomic.** Delegate with a tight brief or do directly with
care on module paths.

**Files / interfaces:**

- `server/Cargo.toml`: add `autotests = false` to `[package]`, and a
  `[[test]] name = "integration"`, `path = "tests/main.rs"` section.
- **New** `server/tests/main.rs`:

  ```rust
  // unwrap/expect are permitted in test code (CONTRIBUTING); clippy's
  // allow-{unwrap,expect}-in-tests only exempts #[test] bodies, not the shared
  // test-helper fns, so this single crate-level expect covers them.
  #![expect(clippy::unwrap_used, clippy::expect_used)]

  mod helpers;

  mod atompub;
  mod feed;
  mod misc;
  mod projector;
  mod storage;
  mod web;
  ```

- Rename each `server/tests/<x>/main.rs` → `server/tests/<x>/mod.rs` (x ∈
  atompub, feed, misc, projector, storage, web); in each, delete the crate-level
  `#![expect(…)]` and the `#[path = "../helpers/mod.rs"] mod helpers;` line;
  keep the `mod <submodule>;` declarations (they resolve within the subdir —
  incl. `misc`'s `mod postgres;`).
- `server/tests/helpers/mod.rs`: delete `#![allow(dead_code)]` (line 5) and the
  `#[allow(unused_imports)]` above the
  `pub use websub_capturing::CapturingWebSubClient;` (~line 35). Keep the
  `pub use` itself (now used in-crate by the `feed` module).
- `server/tests/misc/backup_interop.rs:8` and
  `server/tests/misc/commands.rs:23`: `crate::backup_fixture::…` →
  `crate::misc::backup_fixture::…`.
- Update the three stale `helpers/mod.rs` path references in prose comments if
  any point at the old per-crate layout (leave real doc fixes to Task 5).

**Verify:**

- `cargo nextest run -p jaunder` — one `integration` binary now; **same total
  test count** as before the cycle (compare against a pre-change
  `cargo nextest list -p jaunder` count); all PASS. Test paths gain a
  `<subsystem>::` prefix (e.g. `web::web_auth::…`).
- `cargo xtask check` — clean.
- Suppression assertions:
  - `rg -n '#!\[allow' server/tests/helpers/mod.rs` — **no** match.
  - `rg -rn '#!\[expect\(clippy::unwrap_used' server/tests/` — **exactly one**
    match, in `tests/main.rs`.
  - `rg -rn '#\[path' server/tests/` — **no** match.
- Coverage classifier: run `cargo xtask check` (its Nix coverage step invokes
  `devtool coverage emit`) and confirm the run classifies as passing — i.e.
  `classify_nextest_output` (`tools/devtool/src/coverage/emit.rs`) still handles
  the `integration` binary name + `<subsystem>::` test paths. If it miscounts,
  fix the classifier in this task.

**Commit:**
`test(server): collapse six integration-test crates into one binary (#298)`

## Task 4 — ADR draft

Per spec "Decision record". Use **`jaunder-adr`** (numberless draft in
`docs/adr/drafts/`; `cargo xtask adr promote` numbers it at ship).

**Content:** _server integration tests are one binary_ — context (the
`#[path]`-clone that forced the suppressions), decision (one `[[test]]` target;
6 `#![expect]` → 1), the accepted tradeoff (lost per-subsystem build isolation;
a compile error in one subsystem's tests now fails the whole integration
binary), and its relationship to the in-process `storage::test_support` harness
(ADR-0033) and the out-of-process `test-support` seed binary (ADR-0046). Note
the measured time-neutrality + ~1.5 GB disk saving as rationale.

**Verify:** `prettier -w docs/adr/drafts/<file>.md`; `cargo xtask check` clean.

**Commit:** `docs(adr): record one-binary server integration tests (#298)`

## Task 5 — Fix stale CONTRIBUTING references

`CONTRIBUTING.md` (~lines 363, 366, 375) says the both-backend harness /
`Backend::setup` is "defined in `server/tests/helpers/mod.rs`" — it is
re-exported there (actually defined in `storage::test_support`, ADR-0033), and
after Task 1 test files import it directly.

**Files / interfaces:**

- `CONTRIBUTING.md`: correct the prose to say the harness lives in
  `storage::test_support` and is imported directly by the server integration
  tests (no `helpers/mod.rs` re-export). Keep it accurate to the post-collapse
  single-`integration`-binary layout.

**Verify:** `prettier -w CONTRIBUTING.md`; `cargo xtask check` clean (doc/link
checks).

**Commit:** `docs: correct CONTRIBUTING test-harness references (#298)`

---

## Self-review

- Every spec acceptance criterion maps to a task: 3 allows gone (T1 + T3), 6→1
  expect (T3), no `#[path]` (T3), no re-export (T1), `validate` green (all),
  single target + same test count (T3), ADR recorded (T4). The
  `sent_at`/`no-new-suppression` criterion is T2.
- Tasks are ordered so each ends green: T1 (imports) and T2 (field) are
  independent and precede the structural T3; T4/T5 are docs.
- Separable concerns (#429/#430) are filed, not folded in.
