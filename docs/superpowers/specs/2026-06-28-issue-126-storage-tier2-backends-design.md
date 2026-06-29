# Design — Issue #126: storage-crate contract tests → dual-backend, via `storage::test_support`

- Date: 2026-06-28
- Issue: [#126](https://github.com/jaunder-org/jaunder/issues/126)
- Status: approved (brainstorming)
- Supersedes the crate-shape decision of #125 / ADR-0033 (see "Why not db-test-harness").

## Context / Problem

`storage` has ~45 backend-common contract tests (in-file `#[cfg(test)]`) that assert
behavior both backends must satisfy but run on SQLite only. The goal is to run them on
both backends, like #54 does for the server suite.

#125 built a shared `db-test-harness` crate (`Backend::setup() -> Arc<AppState>` + the
`backends` rstest template) for this. But applying it to `storage`'s **own in-file
tests** is **impossible**: `storage` dev-depends on `db-test-harness`, which depends on
`storage` — a dev-dependency cycle. Building `storage`'s unit-test target then produces
**two distinct instances of `storage`** (the `#[cfg(test)]` instance the tests live in,
and the plain-lib instance `db-test-harness` links). `env.state.<handle>` is the
lib-instance's `Arc<dyn …>`; the test's crate-local functions/structs
(`load_registration_policy`, `perform_post_creation`, `PostCreation`, …) are the
cfg-test instance. Crossing them gives `E0308: mismatched types … multiple different
versions of crate storage` (reproduced directly). Only tests that stay entirely on the
lib side (trait-method calls + `common` types — e.g. `site_config`) compile, which is
why the #125 seed masked the problem.

## Decision

**Host the harness inside `storage` as a feature-gated `test_support` module**, delete
the standalone `db-test-harness` crate, and migrate `server` onto the feature. Then
`storage`'s in-file tests use `crate::test_support` — the *same* crate instance, no
cycle — and convert in place.

### Why not `db-test-harness` (supersede ADR-0033's crate shape)

The harness's whole job is to return `storage::AppState` (the DI composition root,
ADR-0016), so it is intrinsically *downstream* of `storage`. Any **separate** crate
that returns `AppState` must depend on `storage`, which forces the cycle for `storage`'s
own tests. There is no separate-crate restructuring that avoids it. Hosting the harness
*in* `storage` (same crate) is the only shape that lets `storage`'s in-file tests use
it. ADR-0033 is amended/superseded accordingly (the cycle is the recorded rationale).

The original objection to an in-`storage` harness — test scaffolding leaking into
release builds — is handled by the feature gate below: the module and its test-only
deps are excluded from any normal build.

## Components

### 1. `storage::test_support` (the relocated harness)

`storage/src/test_support.rs`, gated `#[cfg(any(test, feature = "test-support"))] pub
mod test_support;`. Contains everything currently in `db-test-harness/src/lib.rs`:
`Backend`, `TestEnv`/`TestBase`, per-test SQLite/Postgres provisioning, the
`backends`/`sqlite_only`/`postgres_only` rstest templates (still `#[export]`ed so
`server`'s separate test crate can `#[apply]` them), `recorded_postgres_url`,
`noop_mailer`, `test_sqlite_state_with_pool`, `seed_posts`, and a new `seed_user`. The
existing pure-fn unit tests (`bootstrap_url`/`splice_db_name`) and the `// cov:ignore`
markers move with it.

Now that it lives in `storage`, the module references `storage`'s own items as
`crate::…` (e.g. `crate::AppState`, `crate::open_database`) rather than `storage::…`.

### 2. `storage` feature + deps

```toml
[features]
test-support = ["dep:tempfile", "dep:rstest", "dep:rstest_reuse"]

[dependencies]
tempfile = { workspace = true, optional = true }
rstest = { workspace = true, optional = true }
rstest_reuse = { workspace = true, optional = true }

[dev-dependencies]            # so storage's OWN tests (cfg(test)) get them without the feature
tempfile.workspace = true
rstest.workspace = true
rstest_reuse.workspace = true
```

The `any(test, feature)` gate + dual (optional-regular + dev) dep declaration is the
standard test-support pattern: storage's own `cargo test` sees the module via `cfg(test)`
+ dev-deps; external consumers (server) get it via the feature + optional deps; a normal
release build excludes both. (`storage` already carries a vestigial empty `test-utils`
feature — reconcile/remove it.)

### 3. `server` migration

`server/tests/helpers` switches `db_test_harness::*` → `storage::test_support::*`;
`server/Cargo.toml` dev-deps gain `storage = { …, features = ["test-support"] }` and
drop `db-test-harness`. Behavior-preserving: the full server suite stays green.

### 4. Delete `db-test-harness`

Remove the crate directory, the workspace `members` entry, and every reference
(`Cargo.lock`). No trace remains in the working tree.

### 5. Convert the 45 storage contract tests in place

`site_config.rs` (24+1), `auth.rs` (4), `user_config.rs` (4), `post_service.rs` (12),
`posts.rs` (1) → `#[apply(backends)]` + `backend.setup()` + `&*env.state.<handle>`,
using `crate::test_support`. Tests needing a user use `seed_user`. (The `auth`/`post_service`/
`user_config`/`posts` cross-boundary failures vanish because everything is now one
`storage` instance.) The `backup.rs` (3) tests and the ADR-0019/annotation/guard hygiene
remain out of scope (follow-ups #136 and #135, already filed).

## ADR

Amend **ADR-0033**: change the decision from "shared standalone crate" to "feature-gated
`test_support` module in `storage`," recording the dev-dependency-cycle blocker as the
reason a separate crate is unworkable for `storage`'s own tests. Update the
`docs/README.md` table row title if needed.

## Verification

- Behavior-preserving relocation: full **server** suite green on both backends under the
  coverage pass (no committed server test-body changes beyond the `helpers` import swap).
- The 45 converted `storage` tests pass on both backends under the coverage pass.
- **No `db-test-harness` anywhere**: `rg db.test.harness` returns nothing; `cargo
  metadata` lists no such package.
- No coverage lowering; the `test_support` module's coverage is handled exactly as it
  was in the crate (pure-fn tests + `// cov:ignore` on the dead defensive arms).
- `cargo xtask validate` green.

## Acceptance

- `db-test-harness` is gone; the harness lives in `storage::test_support` (feature-gated).
- `server` builds on the feature; its suite is green on both backends.
- The 45 `storage` contract tests assert on both backends.
- ADR-0033 amended; follow-ups #135 / #136 filed.
- `cargo xtask validate` green, no coverage lowering.
