# Spec — issue #212: replace `seed-e2e-fixtures.sh` with `test-support` subcommands

- Issue: [#212](https://github.com/jaunder-org/jaunder/issues/212)
- Milestone: E2E test suite
- Follows: #210 (ADR-0046, the `test-support` binary + `seed-posts`)
- Date: 2026-07-03

## Goal

Retire `scripts/seed-e2e-fixtures.sh` **in its entirety** and the two inline
raw-SQL `site_config` INSERTs in `flake.nix`, moving every e2e fixture-setup
step behind `test-support` subcommands that link the real `storage` crate. This
realizes the immediate follow-up named in ADR-0046 §Consequences.

## What exists today

The e2e fixture setup is split across two places:

- **`scripts/seed-e2e-fixtures.sh`** (39 lines), invoked from `flake.nix`'s
  `mkE2eSqliteCheck` (line ~834) and `mkE2ePostgresCheck` (line ~991):
  1. `jaunder user-create --username testlogin  --password testpassword123`
  2. `jaunder user-create --username testnoemail --password testpassword123`
  3. `jaunder user-create --username testoperator --password testpassword123 --operator`
  4. `rm -f "$JAUNDER_MAIL_CAPTURE_FILE"` (mail-capture reset)
  - Header (lines 17-18) flags `site.registration_policy` as having "no CLI for
    that yet", pushing that step out to raw SQL in the caller.
- **Inline raw SQL in `flake.nix`**, per backend, immediately before the script:
  - `INSERT ... site_config ('site.registration_policy', 'open')`
  - `INSERT ... site_config ('feeds.websub_hub_url', 'https://hub.test.local/')`
  - SQLite via `sqlite3 …db "…"` (line ~830-831); Postgres via
    `sudo -u postgres psql -d jaunder -c "…"` (line ~989-990).

## Design

Three new generic, parameterized subcommands on the existing `test-support`
binary (decision: **generic primitives**, not a single monolithic fixture
command — matches `seed-posts`' reusable style, keeps the "3 fixture users" list
co-located in `flake.nix` next to the `site_config` setup that is already
there). Each new lib helper mirrors the `seed_posts_for_user` pattern: a
documented `pub` fn in `test-support/src/lib.rs` with a `#[cfg(test)]`
SQLite-only smoke test; `main.rs` stays a thin clap dispatch.

### `test-support create-user`

```
create-user --db <DbConnectOptions>       # --db / JAUNDER_DB, like seed-posts
            --username <name>
            --password <pw>
            [--display-name <name>]
            [--operator]
```

- lib: `create_user(state, username, password, display_name, operator) -> Result<i64>`.
  Parses `username: &str -> common::username::Username` and
  `password: &str -> common::password::Password`, then calls
  `state.users.create_user(&uname, &pw, display_name, operator)` — the **same
  storage path** `jaunder user-create` uses (`server::commands::cmd_user_create`).
- **Omits** the `common::metrics::registration(… CliBypass …)` call that
  `cmd_user_create` makes (decision: skip the metric — this is out-of-process
  test seeding, and the metric would pollute observability/registration-policy
  assertions the e2e suite may make).
- Assumes a freshly-`jaunder init`'d DB (the script's existing precondition); no
  upsert / idempotency.

### `test-support set-site-config`

```
set-site-config --db <DbConnectOptions>
                --key <key>
                --value <value>
```

- lib: `set_site_config(state, key, value) -> Result<()>` → `state.site_config.set(key, value)`
  (`SiteConfigStorage::set`, an upsert). Generic key/value — one command absorbs
  **both** raw-SQL INSERTs (`site.registration_policy`, `feeds.websub_hub_url`)
  and any future site-config the e2e suite needs.

### `test-support reset-mail`

```
reset-mail --path <file>                   # --path / JAUNDER_MAIL_CAPTURE_FILE
```

- lib: `reset_mail(path) -> Result<()>` — `std::fs::remove_file` with `rm -f`
  semantics (a `NotFound` error is success; anything else propagates). This is
  the one **non-storage** step; folding it into `test-support` (decision) lets
  the shell script die completely, so all fixture setup is the one tool.
- No `--db`.

## Deduplication — the shared seed recipe

`storage::test_support::seed_posts` and the binary's `seed_posts_for_user`
today each hand-inline the same "a timeline-visible seeded post is
`create_rendered_post(… Markdown, published?.now, vec![Public])`" recipe (a
deliberate ~12-line duplication — the binary re-implements it to avoid linking
storage's heavy `test-support` scaffolding `tempfile`/`rstest_reuse`; see
`test-support/Cargo.toml` lines 23-28). Collapse the recipe (not the loops, not
the differing slug/body schemes) into one helper:

- **New lightweight `storage` feature `seed-posts`** gating a single helper — it
  pulls **no new deps** (only `create_rendered_post` + `chrono` + `common`,
  already core):

  ```rust
  // storage, #[cfg(any(test, feature = "seed-posts"))]
  pub async fn seed_rendered_post(
      posts: &dyn PostStorage, user_id: i64,
      slug: Slug, body: String, published: bool,
  ) -> Result<i64, CreatePostError> {
      create_rendered_post(posts, user_id, None, slug, body,
          PostFormat::Markdown, published.then(Utc::now),
          None, vec![AudienceTarget::Public]).await
  }
  ```
- `storage`'s `test-support` feature **implies** `seed-posts`
  (`test-support = ["seed-posts", "dep:tempfile", "dep:rstest_reuse"]`), so the
  in-crate `test_support::seed_posts` (and storage's own `cfg(test)`) reach the
  helper; it keeps its `seed-{i}` / `# Post {i}` scheme and its panic-on-error
  loop, now a one-liner over `seed_rendered_post`.
- The **binary enables `storage/seed-posts`** in its *normal* deps (light — NOT
  `storage/test-support`, which stays dev-only). `seed_posts_for_user` keeps its
  username lookup, per-`prefix` `seed_slug`/`seed_body` scheme, slug-parse error,
  and `Result` loop — but the `create_rendered_post` call becomes
  `seed_rendered_post(&*state.posts, user.user_id, slug, body, published)`.

The recipe now lives once; the two callers still own their genuinely-different
schemes and error styles; and the binary still never links `tempfile`/
`rstest_reuse` (the Cargo comment's intent is preserved, updated to reference
the `seed-posts` feature). This is **not** the ADR-0046 "move `seed_posts` into
`test-support`" idea (still ruled out — see Out of scope): nothing leaves
`storage`; ADR-0033's same-instance constraint is untouched because
`seed_rendered_post` is a plain storage fn, not part of the
`AppState`-returning harness.

## `flake.nix` changes

For **each** of `mkE2eSqliteCheck` and `mkE2ePostgresCheck`, replace the
`site_config` raw-SQL lines **and** the `${./scripts/seed-e2e-fixtures.sh}`
invocation with `test-support` calls (`testSupportBin` is already on the VM
`environment.systemPackages`, flake lines ~793 / ~912):

```
test-support create-user     --db $DB --username testlogin    --password testpassword123
test-support create-user     --db $DB --username testnoemail  --password testpassword123
test-support create-user     --db $DB --username testoperator --password testpassword123 --operator
test-support set-site-config --db $DB --key site.registration_policy --value open
test-support set-site-config --db $DB --key feeds.websub_hub_url      --value https://hub.test.local/
test-support reset-mail      --path /var/lib/jaunder/mail.jsonl
```

- Postgres site: `$DB = postgres://jaunder:testpassword@127.0.0.1/jaunder`
  (already set as `JAUNDER_DB` there).
- **SQLite site:** the removed script relied on cwd `/var/lib/jaunder` + the
  default SQLite path with no explicit `--db`. `test-support` needs an explicit
  URL, so this site must now pass `--db sqlite:/var/lib/jaunder/data/jaunder.db`
  (or set `JAUNDER_DB` to it) — the DB whose raw INSERTs currently target
  `/var/lib/jaunder/data/jaunder.db`.
- Delete `scripts/seed-e2e-fixtures.sh`. Check `end2end/run-e2e.sh` (the local
  runner) for a second invocation site and migrate it the same way.

## Testing

- Per-helper SQLite-only `#[cfg(test)]` smoke tests in `lib.rs`, matching the
  `seed_tests` precedent (backend parity for the storage-linked helpers is
  proven end-to-end by the e2e matrix across `{sqlite,postgres}`):
  - `create_user`: creates a user, then `get_user_by_username` returns it;
    `--operator` sets the operator flag; a second create with the same username
    errors (uniqueness).
  - `set_site_config`: `set` then `get` round-trips; a second `set` on the same
    key overwrites (upsert).
  - `reset_mail`: removing an existing temp file deletes it; removing a
    missing file is `Ok` (rm -f semantics).
- `seed_rendered_post` (the shared recipe) is exercised transitively by both
  `storage::test_support::seed_posts`' existing tests and the binary's
  `seed_posts_for_user` smoke test — no new dedicated test needed, but confirm
  the coverage classifier counts it under the `seed-posts` feature build.
- Coverage: the new `pub` lib fns are covered by the smoke tests (line-based
  classifier — see repo coverage policy).
- Gate: `cargo xtask validate` (full, incl. the e2e matrix) is the real proof —
  it exercises the rewritten flake sites on both backends.

## Out of scope (audit conclusion)

No other testing-infrastructure code moves out of `storage/` (or elsewhere) into
`test-support` as part of #212:

- `storage::test_support` (`seed_posts`, `seed_user`, `TestEnv`, `Backend`,
  rstest templates) **cannot** move — **ADR-0033**: `storage`'s own
  `#[cfg(test)]` tests use it as the *same crate instance*; any crate returning
  `AppState` must depend on `storage`, so extracting it reintroduces the
  dev-dependency cycle → `E0308: multiple different versions of crate storage`.
  The `seed_posts`-**migration** note in ADR-0046 §Consequences is therefore
  no-op-at-best / cycle-at-worst and is **not** pursued: nothing leaves
  `storage`. (Distinct from the recipe **deduplication** above, which *shares* a
  plain storage fn without moving the harness — see "Deduplication".)
- `server::test_support` (`#[cfg(test)]`-only), `common::mailer::test_utils`
  (`CapturingMailSender`, feature-gated), and `server/tests/helpers` are all
  properly gated in-process test scaffolding that never reaches a production
  build. `test-support` (the binary) is for **out-of-process** e2e state
  manipulation — a different concern; nothing consolidates.

No new ADR: this executes an existing consequence of ADR-0046 and follows
established `test-support` conventions. (ADR-0046 remains `proposed`; whether to
flip it to `accepted` is noted but not forced by this issue.)
