# Plan — Issue #245: burn down migrated `cov:ignore` debt

**Cycle:** full host-testable sweep (user-chosen), lightweight plan + one
approval gate. **Branch:** `worktree-issue-245-burn-cov-ignore` · fork tag
`wt-base-issue-245`.

## Progress (live — for resume after compaction)

- **#292** (the `unreachable!("msg")` gate exemption) shipped as a SEPARATE PR
  (#295) and is **MERGED** to main. PR2 (this branch, #245) rides on top.
- **DONE — Task 1** (`ui.rs` cfg-split helpers
  `local_datetime_to_utc_rfc3339`/`marker_matches`/`marker_username_on_boot`):
  markers removed, 4 host tests, verified green.
- **DONE — mockall harness**: `#[automock]` on all 8 storage traits behind
  `storage/test-utils`; PostStorage + a few others got explicit `'a` on
  `Option<&T>` args (commented). Probe confirmed mocks add **zero** uncovered
  production lines. `web`+`jaunder` dev-deps already enable
  `storage/test-utils`.
- **DONE — Task 2 (web crate)**: covered listing `?`-regions ×2,
  `map_audience_error` ×2 (pure), `list_my_subscribers` non-numeric fallback,
  `require_operator` user-absent, subscriptions self-target, `publish_post`
  update-error arms ×2. All markers deleted, 155 web tests green. NB: subagent
  had named modules `cov_tests`; **normalized to `mod tests`** (and
  `mod server_tests` in posts/mod.rs where a plain `mod tests` already existed)
  — do NOT reintroduce `cov_tests`. A reusable `web/src/test_support.rs`
  `auth_parts()` helper + a hand-written `SessionStorage` stub were added
  (SessionStorage is not automock'd; `web` has no async-trait dep).
- **IN FLIGHT — Task 3 (server=`jaunder`)**: feed handlers/worker, commands,
  media, media_manager via mocks.
- **TODO — Task 4 (storage)**: import_table/rollback/bootstrap via real
  fixtures + mocks (dual-backend, needs pg).
- **TODO — rebase** onto merged `origin/main` (defer to after Tasks 1–4; needs a
  clean tree → commit first with `SKIP_PRE_COMMIT=1`).
- **TODO — Task 5**: convert provably-dead lines to `unreachable!("msg")`,
  delete markers (uses the merged exemption).
- **TODO — Task 6**: reconciliation table + KEEP audit → **one**
  `cargo xtask validate --no-e2e` → push → open PR2 → HALT before merge.

## Goal

Shrink the set of accepted-uncovered lines "as small as possible" by (a)
covering genuinely host-testable `cov:ignore`'d logic and deleting those
markers, and (b) where lines are provably dead, expressing them as
`unreachable!("msg")` — which, per the new Task 0 exemption, needs **no marker
at all**. Retain markers only for lines that are reachable-in-process but
untestable host-side (wasm arms, process/serve/network glue, fs-error unwinds).

Scope covers the real production/test source only. **Out of scope / not real
markers:** `xtask/src/coverage/*` (the coverage engine's own test fixtures and
doc-comments — verified, not suppressions), all `docs/**` (prose), and
`CONTRIBUTING.md` (documentation of the feature).

## Decisions (resolved)

1. **Gate exemption scope** — DECIDED: _fold `unreachable!("msg")` exemption
   into this cycle as Task 0_ (first commit). Every provably-dead line becomes
   marker-free instead of `unreachable!()`
   - a retained `cov:ignore`.
2. **Macro scope** — DECIDED: `unreachable!` **only**, non-empty message
   required. `panic!` is often a real reachable error path;
   `todo!`/`unimplemented!` are unfinished-work reminders that _should_ fail
   coverage.

**Open challenge from the cold review (for user):** the reviewer recommends
landing Task 0 as a _separate, focused gate-mechanics PR first_, then the
burn-down on top — rather than folding an engine change into a multi-file sweep
validated as one monolith. Mitigated here by (a) dropping the reason-tagging
churn (Task 0 is now ~1 visitor + 1 wording tweak + tests) and (b) making Task 0
the first, self-contained, independently-valid commit. Folding-in stands unless
the user prefers the split.

## Safety rule for `unreachable!` conversions (load-bearing)

Converting a **graceful fallback** (`Ok(String::new())`, `return Ok(None)`, a
`.clone()` default) into `unreachable!()` replaces safe degradation with a
**panic**. Only convert a branch when it is _invariant-guaranteed_ dead (a
`let-else` after exhaustive logic; a match arm the type system/guards make
impossible; a test asserting the other branch). For
"defensive-but-currently-unreachable harmless fallback," prefer covering it with
a test if any input reaches it, else keep the `cov:ignore` marker. Each
conversion is verified against the surrounding code before editing.

---

## Task 0 — `unreachable!("msg")` structural coverage exemption (gate feature)

- [ ] **exempt.rs**: extend `ExemptVisitor` with `visit_macro` that matches
      `mac.path.is_ident("unreachable")` **and** `!mac.tokens.is_empty()`
      (message required), adding the macro invocation's span lines via
      `add_span`. `panic!`/`todo!`/`unimplemented!` and bare `unreachable!()`
      are deliberately NOT matched (stay measured → still fail if uncovered,
      forcing a message). Fail-closed is inherited (parse error → empty set).
- [ ] **exempt.rs — keep the `BTreeSet<u32>` signature** (no reason-tagging
      refactor). Add `unreachable!("msg")` span lines to the SAME returned set
      as `#[component]` bodies. Rationale: a _covered_ `unreachable!` is
      near-unobservable — reaching it panics ⇒ the test fails ⇒ `cargo llvm-cov`
      exits non-zero ⇒ the Nix check produces no report ⇒ the gate never runs —
      so threading a reason map through `gate::evaluate` + its four tests + the
      `mod.rs` closure is churn for a dead branch. Only a literal `unreachable!`
      invocation with a **non-empty** token stream is matched;
      `std::unreachable!`/aliases/macro-generated forms won't match and stay
      measured (fail-closed).
- [ ] **gate.rs / mod.rs**: no signature change. Only generalize the A1-guard
      _wording_ so it names both exemption kinds ("a covered line sits inside a
      coverage-exempt span — a `#[component]` body or an `unreachable!`
      assertion …"), preserving the existing test assertions
      (`"revisit the     exemption"`). Add a `failure_report` unit test if the
      reword touches an untested branch, so Task 0's own new lines are covered.
- [ ] **Tests** (exempt.rs): `unreachable!("x")` exempt; bare `unreachable!()`
      NOT exempt; `panic!("x")`/`todo!()` NOT exempt; multi-line message span
      covered; `std::unreachable!("x")` NOT matched (documents the fail-closed
      boundary); parse-error fail-closed.
- [ ] **CONTRIBUTING.md** "Coverage and dependency policy": document the third
      exemption path (structural, message-required, fail-closed) alongside
      `#[component]` and `cov:ignore`.
- [ ] **ADR**: amend ADR-0050 (or a numberless draft via `jaunder-adr`)
      recording `unreachable!("msg")` as a structural exemption and the
      rationale (self-enforcing: reached ⇒ panic ⇒ test fails; message-required
      mirrors `crap:allow`). **Note the honest trade-off**: moving lines from
      text `cov:ignore` (immune to parse errors) to a structural syn exemption
      concentrates fail-closed risk — a single `syn` parse error in a file drops
      _all_ its `unreachable!` exemptions at once (e.g. 29 in `cli.rs` → 29 loud
      failures). Loud and safe, but a robustness downgrade vs. the pure-text
      path, per the exemption's design.

**This commit must pass coverage on its own new code** — the exempt.rs/gate.rs
tests provide it.

---

## Task 1 — web cfg-split host helpers (P1 + siblings) · no dependency on Task 0

Three non-`#[component]` helpers in `web/src/pages/ui.rs` whose `cov:ignore`
wrongly brackets the fn signature + the `#[cfg(not(wasm32))]` host arm (the wasm
arm is already cfg-excluded from the host build and needs no marker). Delete the
misplaced markers; add host unit tests to the existing `mod tests`.

- [ ] `local_datetime_to_utc_rfc3339` (**P1**): delete markers at
      L278-start/L284-stop/L297/L299; test `""`→None, `"   "`→None, and the host
      arm passes the trimmed input through.
- [ ] `marker_matches(author)`: tighten to the wasm arm only; test host arm
      returns `false`.
- [ ] `marker_username_on_boot()`: tighten to the wasm arm only; test host arm
      returns `None`.

## Test seam — add `mockall` (decided; user proposed adding a library)

The codebase has **no** general fake/erroring storage and no mock library; the
storage traits are 20–63 methods, so a hand-fake to reach one `Err` arm would
add more uncovered surface than it removes. Resolution: add **`mockall`**
(MIT/Apache-2.0, `cargo deny`-clean, `#[async_trait]`-compatible) as an
**optional dep of `storage`, folded into the existing `test-utils` feature** —
so it is never in a shipped binary, and `web`/`server` dev-deps (already
`storage = { features = ["test-utils"] }`) get `MockPostStorage` etc.
`#[cfg_attr(feature = "test-utils", automock)]` sits above each trait's
`#[async_trait]`. A fault-injection test becomes ~3 lines
(`expect_*().returning(|| Err(...))`).

**Pilot finding (2026-07-05):** `#[automock]` on `PostStorage` fails to compile
— `E0106/E0637` lifetime errors on `cursor: Option<&PostCursor>` (a reference
_nested_ in a generic; bare `&T` args are fine). `Option<&PostCursor>` appears
**only in `PostStorage`** (its ~4 cursor-paginated list methods), so the other
**7 traits** (Media/FeedEvent/FeedCache/User/Subscription/Audience/SiteConfig)
should `#[automock]` cleanly. PostStorage needs one of: **(a)** explicit
lifetimes on its ~4 cursor methods (trait + 2 impls; semantically identical to
elision, but touches production sigs for test tooling), **(b)** a verbose
`mock!` block (no prod change, ~30 sigs re-declared), or **(c)** KEEP the 4
PostStorage-dependent arms (listing `?`-regions ×2, posts/mod TOCTOU ×2) and
mock only the 7 clean traits. **DECIDED: option (a)** — explicit `'a` on
PostStorage's **6** cursor methods (`list_published_by_user`, `list_published`,
`list_drafts_by_user`, `list_collection_by_user` [uses
`Option<&CollectionCursor>`], `list_posts_by_tag`, `list_user_posts_by_tag`) in
the trait AND the `impl<DB> PostStorage for PostStore<DB>`, **each with a
comment** explaining the explicit lifetime is for `mockall::automock` (can't
synthesize a lifetime for a reference nested in a generic; identical to
elision).

- [ ] **Probe attribution**: once one trait mocks cleanly, run a targeted
      `cargo llvm-cov --text` to confirm mockall's generated code attributes
      uncovered mock methods to the single `#[automock]` line (worst case: one
      marker per mocked trait) — NOT to N production lines.
- [ ] Mock-based tests touch no live pool → annotate `#[tokio::test]` with
      `// guard:no-backend — mock store`.

## Task 2 — web `#[server]` / boundary logic via `mockall` + the Owner/`provide_context` harness

The reactive-context test pattern already exists (`Owner::new()` +
`provide_context` in `error.rs`/`auth/server.rs` tests). Combine it with
`mockall` mocks provided into context.

- [ ] `web/src/posts/mod.rs` L634 (`NotFound|Unauthorized`→not_found) and
      L636-639 (`Internal`→storage): mock `update_post` returning each error
      after a valid `get_post`.
- [ ] `web/src/audiences/mod.rs` `map_audience_error` `NotFound`/`Storage` arms
      (add a `#[cfg(server)]` test mod) + L179 non-numeric `subscriber_ref`
      fallback.
- [ ] `web/src/backup/server.rs` L15 `require_operator` user-absent →
      unauthorized (mock `UserStorage`→None).
- [ ] `web/src/subscriptions/mod.rs` L113 self-target → `Ok(false)` (auth
      context username == author).
- [ ] `web/src/posts/listing.rs` L259 & L305 `?` Err region —
      `fetch_posts_by_tag` takes `&dyn PostStorage` directly (clean seam): mock
      whose `list_posts_by_tag` returns `Err`.

## Task 3 — server feed handlers, worker, commands, media (DI fakes)

HTTP handlers take `Extension<Arc<dyn …Storage>>`; the worker takes trait-object
deps — all injectable per ADR-0016. (CONTRIBUTING: every HTTP endpoint must have
an integration test, so these are owed anyway.)

- [ ] `server/src/feed/handlers.rs` (8 markers): 500-on-regen-error,
      500-on-cache-error, `NOT_MODIFIED` etag + If-Modified-Since, `feed_site`
      404/delegate, `feed_site_tag`/`feed_user_tag` bad-ext 404, `feed_user_tag`
      happy path.
- [ ] `server/src/feed/regenerate.rs` L64 `FeedSurface::Site` canonical-url arm.
- [ ] `server/src/feed/worker.rs` (~5): claim-error log+return, empty-batch
      return, go-live error log, regen-success info fields, `mark_exhausted` on
      attempts-exceeded.
- [ ] `server/src/commands.rs` L62 `describe_bootstrap_error` `Sqlx` arm;
      L388-399 dev auto-init branch of `prepare_server` (temp storage,
      `prod=false`, `127.0.0.1:0`).
- [ ] `server/src/media.rs` L152 etag `NOT_MODIFIED`; L161 content-type fallback
      (find_by_hash→None, file present).
- [ ] `server/src/media_manager.rs` L228-232 `CreateMediaError::Internal` map
      (fake `create_media`→Err).

## Task 4 — storage backend logic (storage harness, dual-backend)

Respect backend parity: cover both SQLite and Postgres where the harness allows
(`#[apply(backends)]`), or state the single-backend reason. **Every**
DB-touching async test added under `storage/src` (not just `post_service.rs`)
must carry `#[apply(backends)]`/`#[apply(backends_matrix)]`, a
`#[apply(sqlite_only)]`/`postgres_only` + `// reason:`, or
`// guard:no-backend — <reason>` to satisfy the `test-backend-pattern` guard
(CONTRIBUTING §guard). Pure _synchronous_ `#[test]` unit tests are never flagged
— verify each new test's shape against the guard before running.

- [ ] `storage/src/post_service.rs` L419-423 `Internal`→`Storage` map (fake
      `PostStorage`, `guard:no-backend`).
- [ ] `storage/src/sqlite/backup.rs` L148-151 & `storage/src/postgres/backup.rs`
      L139-145 `import_table` missing-column `InvalidBackup` (ragged-ndjson
      fixture) — dual-backend.
- [ ] `storage/src/postgres/backup.rs` L89-93 restore rollback — dual-backend
      the existing sqlite-only test (currently deferred to #136; covering it
      here removes the marker).
- [ ] `storage/src/postgres/bootstrap.rs` L68 `DatabaseExists` (pre-create db) &
      L89 `Err(other)` passthrough (invalid statement) — pg tests, mod exists.
- [ ] `storage/src/backup.rs` L563 `previous_directory_backup` no-parent arm
      (call with root path).

## Task 5 — convert provably-dead lines to `unreachable!("msg")`, delete markers (depends on Task 0)

Only invariant-guaranteed-dead lines (verified per the safety rule). Each
becomes a message-carrying `unreachable!` and loses its `cov:ignore`.

- [ ] `server/src/cli.rs` (29×):
      `let Commands::X{..} = parse(..) else { panic!("wrong variant") }` →
      `unreachable!("parse deterministically yields Commands::X")`. Mechanical
      batch.
- [ ] `server/src/main.rs` L38-41: existing bare `unreachable!()` → add a
      message; drop marker.
- [ ] `storage/src/test_support.rs` L200/L211 (already `unreachable!()`, add
      messages), L385/L453/L508/L551 (`panic!` let-else →
      `unreachable!("<invariant>")`).
- [ ] `server/src/observability.rs` L265 (Registry guarantees a live span during
      `on_close`) & L1014 (the test's route unconditionally inserts the
      extension, so the `else` 500 is dead — the test _constructs_ the
      invariant, not "proves the branch").
- [ ] `storage/src/sqlite/backup.rs` L190 (any i64-range number is already
      returned by the `as_i64` arm, so the `as_u64`+`try_from` branch is empty —
      or simply delete the dead `else if`), L436 (test invariant).
- [ ] `web/src/posts/server.rs` L225 (`next_cursor` always `Some` after the
      non-empty guard at L209).

**NOT converted — moved to KEEP (Task 6) per the safety rule (cold-review
S2/S3):**

- `server/src/websub/http.rs` L151 — _timing_-dead (client timeout beats the 30s
  sleep), not invariant-dead; converting risks a real panic in a spawned server
  task if timing shifts.
- `common/src/atompub/entry.rs` L377 — graceful fallback on **external XML**
  input (a quick-xml contract, not a local invariant); a panic here is a DoS
  vector. **Unconditional KEEP.**
- `server/src/mailer/smtp.rs` L43-46 & `server/src/mailer/file.rs` L32 —
  graceful startup/serialize error paths; a panic-on-boot / panic-on-send is
  strictly worse than the current `Err`/`?`. KEEP.
- `storage/src/backup.rs` L456/L523 — the marker is a **block-form on a closing
  brace** (can't carry `unreachable!`), and it is the _same_ `.parent()` pattern
  classified reachable-and-testable at L563 (Task 4). Fs-defensive region → KEEP
  as a marker.

## Task 6 — retain + sharpen KEEP-host-untestable markers

No coverage change; ensure each retained `cov:ignore` states _why_ (and
implicitly why it can't be `unreachable!`). Most already carry reasons; this is
a light audit, likely a no-op or tiny.

- Retained (exhaustive — cross-checked against `rg -n 'cov:ignore'`):
  `csr/src/lib.rs`, `web/src/lib.rs` (wasm entry/mount); `web/src/error.rs`
  `server_resource` (SSR reactive); every `#[component]` prop-list block in
  `ui.rs`/`upload.rs`/`timeline.rs` and every non-component view-builder
  (`backup.rs`/`media.rs`/`posts.rs`/`site.rs`/`ui.rs` builders, timeline signal
  state); `server/src/main.rs` bootstrap/serve/spawn glue;
  `server/src/mailer/smtp.rs` **L43-46, L86, L88, L97, L104, L109**
  (address-parse + real SMTP send — all no-host-seam), `mailer/file.rs` **L32
  (serialize) + L47 (JoinError)**; `server/src/assets.rs` RustEmbed;
  `server/src/websub/http.rs` **L99, L151, L158** (serve/spawn/timing glue);
  `server/src/commands.rs` L118-125 (TTY prompt), L484 (post-serve return);
  `common/src/atompub/entry.rs` **L377** (external-XML fallback);
  `storage/src/backup.rs` fs-error unwinds (**L194, L456, L500, L507-510,
  L523**, and the test multi-line-`?` artifacts L708/L736/L849/L950),
  `storage/src/postgres/backup.rs` L56-61 export rollback,
  `storage/src/postgres/mod.rs` argon2 error, `test_support.rs` runtime/timeout
  glue (#232); `web/src/posts/server.rs` L230 (loop-exhaustion — reachable only
  via 10k-row mock; impractical, keep with reason).
- [ ] **Final marker reconciliation (mandatory before ship):** produce an
      inventory table mapping _every_ production `cov:ignore` line (from
      `rg -n 'cov:ignore'`, excluding `xtask/coverage/*`, `docs/**`,
      `CONTRIBUTING.md`) to exactly one disposition: COVERED-&-removed (Tasks
      1-4), CONVERTED-to-`unreachable!` (Task 5), or KEPT (this task, with
      reason). No marker may be un-dispositioned. `test-support/src/main.rs` is
      out of scope (tracked in #232) — state it.

---

## Execution discipline (pay the testing tax once)

Per user direction, pay the instrumented coverage/test tax **once**. Correct
mechanism (cold-review S1 — my earlier "commits don't run coverage" was FALSE:
`.githooks/pre-commit` runs `cargo xtask check`, incl. the Nix coverage build,
on _every_ commit):

1. Make **all** edits across Tasks 0–6 in the working tree.
2. Run `cargo xtask check` once (Fix mode — applies fmt/leptosfmt/prettier so
   later commits don't trip the format-restage hook), then
   `cargo xtask validate --no-e2e` once to get the green verdict on the final
   tree.
3. Split into the separate commits with **`SKIP_PRE_COMMIT=1`** (hook L14) on
   each intermediate commit — the whole tree is already validated, and the
   pre-commit hook would otherwise re-run against the _working tree_ (not the
   staged subset), redundantly. What makes this safe is the content-addressed
   Nix cache: all commits share one working-tree content → one coverage build.
4. **Ordering matters for bisectability** (S1): the hook validates the working
   tree, not each commit's subset, so intermediate commits are only
   _individually_ valid if ordered — **Task 0 (exemption) must be the first
   commit**, and Task 5 (`unreachable!` conversions, which depend on it) must
   come after. Keep each covered-line's test and its marker-removal in the
   _same_ commit.
5. Push once → the pre-push `validate` runs against the full tree and hits the
   warm cache.

HALT before merge (open PR referencing #245; release issue to Done at ship).

## Verification

- `cargo xtask validate --no-e2e` green on the final tree (coverage floor risen;
  marker count dropped).
- `rg -c 'cov:ignore'` on production source shows the reduced set; every
  remaining marker is KEEP-host-untestable with a reason.
- No new `unreachable!` sits on a reachable branch (each verified against
  surrounding invariants).
