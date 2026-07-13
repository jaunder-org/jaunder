# Spec — #418: enforce ADR-0053 single-backend homing + correct the drift

**Status:** design resolved (expanded from the original "no dual-backend in a
dialect file" framing after a full audit + investigation of every single-backend
test). **Decision record:** no new ADR — this mechanizes ADR-0053 §1 (home by
what it proves) and §2 (presume a coverage gap), both accepted. If the new
`guard:low-level-db` marker / convention warrants a note, add it to ADR-0053's
Consequences. **Sibling:** #419 (reason requirement) folds into the same guard,
same branch.

## The guarantee

Taking ADR-0053 §2 at its word, every DB-touching test resolves to exactly one
honest shape. There is **no legitimate slot for a single-backend template in a
generic file**, and **no test may wear a backend template it does not use**:

| Needs a DB? | Nature                                                                                                                                   | Correct form                                                                  | Home                                                             |
| ----------- | ---------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| Yes         | backend-common                                                                                                                           | `#[apply(backends)]` (uses `backend`)                                         | generic module                                                   |
| Yes         | dialect-specific, via the standard fixture                                                                                               | `sqlite_only` / `postgres_only` (uses `backend`)                              | matching dialect dir                                             |
| Yes         | low-level DB work that **can't** use the `backend` fixture (self-fixtures: `unique_postgres_url`, bootstrap admin, both engines at once) | bare `#[tokio::test]` + `// guard:low-level-db — <reason>`                    | by judgment: one-database → dialect dir; cross-backend → generic |
| No          | backend-independent                                                                                                                      | bare `#[tokio::test]` + `// guard:no-backend — <reason>` (or plain `#[test]`) | anywhere                                                         |

Two consequences drive the correction sweep and the guard:

1. **`*_only` ⟺ a dir named for its backend.** A `*_only` in a generic file is a
   defect (wrong crate, bogus §2 reason, or misfiled). Established via ADR-0053
   §2: the only decisive reason to be single-backend is backend-_exclusive_
   (dialect-specific), which §1 already homes to the dialect dir.
   ("Backend-independent, one suffices" is §2's _non-decisive_ reason, not a
   keep.)
2. **A template must be used.** The `#[apply(...)]` templates exist to inject
   and parametrize over `#[case] backend`. A test that discards it
   (`let _ = backend;`) is dishonest: either it can use the injected backend (→
   use it) or it can't (→ it's doing low-level work → bare `#[tokio::test]` +
   `guard:low-level-db`).

## The guard (folded into `test-backend-pattern`, roots `storage/src` + `server/tests`)

Three composed rules over each `#[tokio::test]` / `#[apply(...)]` cluster, all
line-scannable (pure `(path, source)` functions, mirroring the existing
`violations()`):

1. **Homing** — keyed on a `/sqlite/` or `/postgres/` path component: `sqlite/`
   may hold only `sqlite_only`; `postgres/` only `postgres_only`; a generic file
   only `backends` / `backends_matrix`. The opposite `*_only`, or any
   `backends*` in a dialect dir, or any `*_only` in a generic file → flagged.
2. **Template-or-marker** — every `#[tokio::test]` must carry a backend
   template, or `// guard:no-backend`, or `// guard:low-level-db`. The two
   markers are allowed anywhere (a `guard:low-level-db` test's placement is a
   homing judgment the lint can't make — a bare test has no backend to key on).
3. **Param-honesty** — a `*_only` / `backends*` cluster whose body contains
   `let _ = backend;` (or whose param is `#[case] _backend`) → flagged: "uses a
   backend template but ignores the injected backend — use it or drop the
   template." After the sweep nothing carries the discard, so this locks the
   honesty in.

Plus **#419**: every single-backend template keep **and** every `guard:*` marker
must carry a non-empty reason (mirrors `crap:allow`). A _presence_ check can't
catch a _false_ reason — that stays a review concern (the audit found several
copy-pasted ones).

## The correction sweep (audited; the ①②③ split)

Home each relocation by **subject** — storage-layer property → `storage` crate;
server/CLI → `server/tests` — then single-backend → the matching dialect dir,
**split out of any file mixing dual and single-backend tests**.

- **① Discards the param but _could_ use it → use it, keep the template:**
  `claim_pending_batch_no_lock_contention` (`sqlite_only`;
  `Backend::Sqlite.setup()` → `backend.setup()`),
  `pg_teardown::per_test_database_is_dropped_on_teardown` (`postgres_only`;
  `Backend::Postgres.setup()` → `backend.setup()`).
- **② Self-fixtures (can't use the param) → bare `#[tokio::test]` +
  `guard:low-level-db`:** the migration trio (**collapsed to 1** — see below),
  `pg_teardown::unique_postgres_database_is_dropped_on_guard_drop`, the two
  `cmd_create_pg_db` provisioning tests, the three `backup_interop` tests.
- **③ Already uses the param, just misfiled → relocate:**
  `sqlite_pool_enforces_foreign_keys`, `every_foreign_key_is_deferrable`.
- **Backend-common → dual:** `export_propagates_media_mirror_failure`,
  `pending_subscription_is_not_admitted`, `feed_events_marks_run` →
  `#[apply(backends)]`.
- **No DB:** `cmd_create_pg_db_rejects_non_postgres_urls` → bare +
  `guard:no-backend`.

**The migration trio.** The three `open_*` tests are the _only_ coverage of the
public `open_database` entry point running the real migration sequence against a
from-scratch Postgres DB — `setup()` uses a pre-migrated template clone, and
`ensure_template_db()` runs `migrate!` directly, both bypassing the public path.
But 2 of 3 are redundant with each other (test 1 ⊂ test 2; test 3 ≡ test 2 on
PG) and none tests the upgrade path its name implies. This is legitimately
Postgres-only (the template optimization is PG-only; SQLite has no template, so
every SQLite test already covers its from-scratch path). → **collapse to one
bare `guard:low-level-db` test** in `storage/src/postgres/`, honest reason. Not
dual (the SQLite half is redundant), not `postgres_only` (it self-fixtures via
`unique_postgres_url`, ignoring the param).

## Ordering

Corrections (all preserve the _existing_ guard — they keep a template/marker or
become a `guard:*` bare test) land first, one commit each; the strengthened
guard lands only after the tree conforms. Detail → the plan.

## Out of scope

- Semantic "does it touch the DB / is the reason true / is this bare low-level
  test in the right dir" — needs type info / judgment; delegated to review (the
  guard enforces template-based homing + param-honesty + marker presence).
- The 99 `#[apply(backends)]` tests (already correct); any churn of the
  `server/tests` per-directory-binary layout.

## Verification

`cargo xtask validate` green after the sweep — static + clippy + coverage
(confirm no regression from moving tests into coverage-measured `storage/src`) +
the full e2e matrix, with the strengthened guard passing (proving both the guard
and the conformance).
