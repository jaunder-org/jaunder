# Issue #791 — Seeded e2e auth Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace UI-driven register/login as e2e test _setup_ with out-of-band
`test-support` seeding, cutting ~35 % of in-span suite time.

**Architecture:** Two new `test-support` subcommands (`seed-user`,
`create-session`) emit a JSON seed record (session cookie + localStorage marker,
both built by Rust from the server's own primitives). TypeScript helpers inject
the cookie and a companion seed cookie into the browser context; one tombstoned
`addInitScript` per context plants the marker pre-paint. Specs convert to
`signInAs*` helpers; thirteen named holdouts keep the real UI flows.

**Spec:** `docs/superpowers/specs/2026-08-02-issue-791-seeded-e2e-auth.md`
(D1–D11, AC1–AC8). This plan is the "how"; read the spec for the "why."

**Tech Stack:** Rust (test-support/common/web/host crates),
TypeScript/Playwright 1.58.2 (end2end/), cargo nextest, `cargo xtask` gates.

## Review header

**Scope — in:** common marker-codec move; test-support seed subcommands; end2end
seed/helpers/fixtures plumbing; all call-site conversions; the AC verification
runs. **Out:** the Playwright ≥ 1.59 bump (Task 1 files it); #792/#793/#801
(siblings); any production behavior change.

**Tasks:**

1. File the D11 Playwright-bump issue; commit spec + plan.
2. `common::session_user` — move the marker codec; `web` re-exports.
3. test-support lib: `SeedRecord`, `seed_user`, `create_session_for_user` + unit
   tests (AC1).
4. test-support CLI: `seed-user` / `create-session` subcommands + dispatch test.
5. end2end `seed.ts`: `SeedRecord`, `seedUserViaTool`, `createSessionViaTool`,
   `applySeededSession`; all tool spawns self-time (D7).
6. end2end `helpers.ts`: add `signInAsNewUser` / `signInAsNewUserKnown` /
   `signInAs` / `registerViaUi`, `generateUsername`, `TEST_PASSWORD` (old
   helpers not yet deleted).
7. end2end `fixtures.ts`: `TestUser` gains seed fields; `user` / `verifiedUser`
   / `registeredPage` reshape (D8).
8. `auth.spec.ts` + `authed-flash.spec.ts`: conversions, holdout comments, AC5
   seeded-pre-paint tests.
9. Setup-login conversions: admin-site, backup, invite, email, visibility
   (`loginAs`).
10. Register conversions A: atompub, audiences, authed-cls, timeline-cls.
11. Register conversions B: feeds (incl. D9), media (incl. D5 gotos), posts,
    visibility (`registerKnown`).
12. Cutover: delete `register` / `registerKnown` / `registerAndLogin`;
    AC2/AC3/AC4 greps; full `e2e-local` green.
13. Measurements: AC6 server-fn coverage byte-identical; AC7 traces
    before/after; AC8 `cargo xtask validate`.

**Key risks/decisions:**

- The tombstoned init script is the subtlest piece (D3). Task 5 pins its exact
  source; Task 8 adds a test for the logout row (seed → logout → full navigation
  stays anonymous), beyond AC5's minimum, because the pushState logout tests
  never re-run an init script.
- Additive-then-delete sequencing (Tasks 6 → 12): `register` stays until every
  call site is converted, so each commit's gate run is green; AC3's grep gates
  the deletion.
- The marker codec's tests move _with_ it to `common` (their natural home,
  in-crate coverage). AC2's "existing tests still pass unchanged" is satisfied
  by the same test bodies passing unchanged at the new location; `marker.rs`
  becomes a pure re-export. Disclosed at plan approval.
- D6 gained a 13th row (`password_reset.spec.ts` post-reset form logins) and D7
  now uses `withTimedAction(null, …)` instead of a new `timedToolCall` — both
  spec amendments made 2026-08-03 after the rebase re-census; disclosed at plan
  approval.

## Global Constraints

- Worktree:
  `/home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api`.
  All gates run with cwd pinned there.
- Per-commit gate:
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask check`
  must pass before every commit (jaunder-commit). No `Co-Authored-By` trailer.
- AC2: `rg -n 'HttpOnly|SameSite|jaunder_auth' end2end/ test-support/` returns
  no hits outside comments. TypeScript never spells the marker key, cookie
  attributes, or `Set-Cookie` shape — they come from the Rust seed record.
  (Pinned TS constants `jaunder_seed_marker` / `jaunder_seed_applied`
  deliberately do **not** contain `jaunder_auth`.)
- `registerViaUi` keeps emitting the timed action under the name `flow.register`
  — the name is not renamed (AC7).
- No test-only route on the shipped server; no HTTP round-trip for seeding (D1).
  cheap-kdf stays OFF; seed-user pays real Argon2 once per seeded user.
- Playwright 1.58.2: `addInitScript` cannot be removed — hence one tombstoned
  script per context (D3). No `removeInitScript` exists even in 1.59 (D11).
- Username generation and the fixed password stay in TypeScript (D4):
  `generateUsername()` and `TEST_PASSWORD = "testpassword123"`.
- Storage-side tests in test-support use the crate's existing temp-SQLite style
  (`main.rs` tests), calling `lib.rs` functions directly — no stdout capture
  (AC1).

---

### Task 1: File the D11 Playwright-bump issue; commit spec + plan

**Files:**

- Create: GitHub issue (no repo file)
- Reference: `docs/superpowers/specs/2026-08-02-issue-791-seeded-e2e-auth.md`
  (D11)

**Interfaces:**

- Produces: issue URL recorded in the plan's checkbox note; milestone #6
  membership.

- [x] **Step 1: File the issue** (per jaunder-issues; type required, topic
      label, project, milestone) — filed as #815 (type Task, label test-infra,
      milestone set, Jaunder Backlog #1)

```bash
gh issue create --repo jaunder-org/jaunder --type Task --label test-infra \
  --milestone "Test infrastructure & E2E" \
  --title "deps: bump Playwright to ≥ 1.59 for disposable init scripts" \
  --body-file /tmp/issue-791-d11.md
```

`/tmp/issue-791-d11.md`:

```markdown
## Problem

#791 (seeded e2e auth) plants the pre-paint auth marker with a tombstoned
`addInitScript` whose payload rides in a companion cookie, because Playwright
1.58.2 has **no way to remove an init script**. 1.59 made `addInitScript` return
a `Disposable` (there is still no `removeInitScript` —
[microsoft/playwright#29499](https://github.com/microsoft/playwright/issues/29499)).

## What the bump would simplify

Only the identity-switch path: re-seed could dispose-and-recreate the script
with the payload baked in, dropping the companion cookie and the per-context
`WeakSet`. **The tombstone itself stays** — UI logout is a page-side event
Playwright never surfaces to Node, so an in-page guard is required in every
version (see #791's spec D11).

## Cost

A `flake.lock` nixpkgs bump — `flake.nix` pins `pkgs.playwright-test` and
`pkgs.playwright-driver.browsers` in lockstep with `end2end/package.json` — so
it moves every other Nix-built dependency with it. Full `cargo xtask validate`
afterwards.

Provenance: #791 spec D11.
```

Then add it to the backlog project and record the URL:

```bash
gh project item-add 1 --owner jaunder-org --url <issue-url>
```

- [x] **Step 2: Commit the spec, plan, and cycle artifacts**

```bash
git add docs/superpowers/specs/2026-08-02-issue-791-seeded-e2e-auth.md \
        docs/superpowers/plans/2026-08-03-issue-791-seeded-e2e-auth.md
git commit -m "docs(spec): issue #791 seeded e2e auth — approved spec and plan"
```

(The ADR draft at `docs/adr/drafts/e2e-seeded-auth.md` is gitignored by design;
`HANDOFF-issue-791.md` stays untracked.)

---

### Task 2: `common::session_user` — move the marker codec

**Files:**

- Create: `common/src/session_user.rs`
- Modify: `common/src/lib.rs` (module list, alphabetical — between `seed` and
  `session_label`)
- Modify: `web/src/auth/marker.rs` (becomes a re-export)
- Modify: `web/src/auth/mod.rs:17-20` (doc comment repoints at
  `common::session_user`)

**Interfaces:**

- Consumes: nothing (first task with code).
- Produces:
  `common::session_user::{MARKER_KEY, SessionUser, encode_marker, decode_marker}`
  — consumed by Tasks 3 (test-support builds the marker) and unchanged web call
  sites via the `web::auth::marker` re-export.

_The move is verbatim: the file content below is the current
`web/src/auth/marker.rs` (module docs adjusted for the new home), so `web`'s
behavior cannot drift._

- [x] **Step 1: Create `common/src/session_user.rs`** with the full current
      content of `web/src/auth/marker.rs`:
  - Doc comment: keep the `#181, ADR-0044` advisory-marker explanation; drop the
    "wasm-only binding lives in `super::marker_storage`" line in favor of "the
    wasm-only `localStorage` binding lives in `web::auth::marker_storage`"; add
    one line: "Lives in `common` so the `test-support` binary can build markers
    without linking `web` (#791)."
  - Items, verbatim: `pub const MARKER_KEY: &str = "jaunder_auth";`,
    `pub struct SessionUser { pub username: Username, #[serde(default)] pub is_operator: bool }`
    (derives `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`),
    `encode_marker`, `decode_marker` — with their existing doc comments.
  - The `#[cfg(test)] mod tests` block, verbatim (all five tests:
    `round_trips_session_info`, `decode_defaults_is_operator_when_absent`,
    `round_trips_all_valid_username_chars`, `decode_rejects_malformed_json`,
    `decode_rejects_invalid_username`), with
    `use super::{decode_marker, encode_marker, SessionUser};` and
    `use common::test_support::parse_username;` adjusted to
    `crate::test_support::parse_username`.

- [x] **Step 2: Register the module** in `common/src/lib.rs`:

```rust
pub mod seed;
pub mod session_user;
pub mod session_label;
```

- [x] **Step 3: Run the moved tests, verify they pass at the new home**

Run: `cargo nextest run -p common session_user` Expected: PASS — 5 tests.

- [x] **Step 4: Reduce `web/src/auth/marker.rs` to a re-export**

```rust
//! The client-side **auth marker** (#181, ADR-0044): a JS-readable localStorage
//! value advertising "probably the owner" for pre-paint chrome adjustment. It is
//! ADVISORY, not a credential — the real session stays the HTTP-only cookie, and
//! the server authorizes every mutation.
//!
//! The pure codec + `MARKER_KEY` live in `common::session_user` (moved there so
//! `test-support` can build markers without linking `web`, #791) and are
//! re-exported here unchanged; the wasm-only `localStorage` binding lives in
//! [`super::marker_storage`] (#514).

pub use common::session_user::{decode_marker, encode_marker, SessionUser, MARKER_KEY};
```

Update `web/src/auth/mod.rs:17-19`'s doc comment: "pure `encode`/`decode` +
`MARKER_KEY`" now "re-exported from `common::session_user` (#791)".

- [x] **Step 5: Verify no web call site changed**

Run: `cargo nextest run -p web auth::marker` Expected: PASS — marker's tests are
gone from `web` (they moved), so this runs 0 tests; the real check is
compilation of every consumer:

Run: `cargo check -p web --target wasm32-unknown-unknown` and
`cargo nextest run -p web` Expected: PASS — `marker_storage.rs:13`,
`auth/mod.rs`, `session.rs`, and the pages all resolve through the re-export
untouched.

- [ ] **Step 6: Gate + commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask check
git add common/src/session_user.rs common/src/lib.rs web/src/auth/marker.rs web/src/auth/mod.rs
git commit -m "refactor(common): move auth marker codec to common::session_user (#791)"
```

(Note: new Rust files must be `git add`ed _before_ `cargo xtask check` — the Nix
source filter excludes untracked files, so the coverage build fails with "file
not found for module" otherwise.)

---

### Task 3: test-support lib — `SeedRecord`, `seed_user`, `create_session_for_user`

**Files:**

- Modify: `test-support/src/lib.rs`
- Modify: `test-support/Cargo.toml` (add `serde` + `serde_json` workspace deps)

**Interfaces:**

- Consumes: `common::session_user::{MARKER_KEY, SessionUser, encode_marker}`
  (Task 2); existing
  `test_support::create_user(state, username, password, display_name, operator) -> anyhow::Result<UserId>`;
  `storage::AppState` (`state.users: dyn UserStorage`,
  `state.sessions: dyn SessionStorage`);
  `host::auth::session_cookie_header(&RawToken, false)`.
- Produces (Task 4 serializes these; Task 5's TS mirrors the field names):

```rust
/// The JSON seed record printed by the `seed-user` / `create-session`
/// subcommands: everything a browser context needs to boot authenticated
/// pre-paint — the session cookie and the advisory marker, both built by the
/// server's own primitives so TypeScript never restates them (#791, AC2).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SeedRecord {
    pub username: String,
    pub user_id: i64,
    pub is_operator: bool,
    pub token: String,
    pub set_cookie: String,
    pub marker_key: String,
    pub marker: String,
}

/// Create a user (real `UserStorage::create_user` path — genuinely
/// argon2-hashed, stays loginable) and a session in one DB open.
/// `label` defaults to `"E2E seed"`.
pub async fn seed_user(
    state: &Arc<AppState>,
    username: &str,
    password: &str,
    label: Option<&str>,
) -> anyhow::Result<SeedRecord>

/// Create a session for an EXISTING user (e.g. the harness-seeded
/// `testoperator`); `is_operator` is read back from the user record so the
/// marker matches what a real login would write.
pub async fn create_session_for_user(
    state: &Arc<AppState>,
    username: &str,
    label: Option<&str>,
) -> anyhow::Result<SeedRecord>
```

- [x] **Step 1: Write the failing tests** (new
      `#[cfg(test)] mod seed_session_tests` in `test-support/src/lib.rs`,
      temp-SQLite style per `main.rs`'s `temp_db`; open state via
      `storage::open_existing_database`)

```rust
#[tokio::test]
async fn seed_user_returns_a_session_that_authenticates() // seed_user("alice","password123",None);
    // parse record.set_cookie: value of the first `session=` pair, up to `;`
    // → parse as RawToken → state.sessions.authenticate(&token) resolves
    //   user_id == UserId::from(record.user_id)
    // → decode_marker(&record.marker) == Some(SessionUser{username:"alice", is_operator:false})
    // → record.marker_key == "jaunder_auth" (asserted against MARKER_KEY, not a literal)
    // → list_sessions(user_id) has one session labelled "E2E seed"

#[tokio::test]
async fn seed_user_honours_an_explicit_label() // label Some("CI bot") → list_sessions shows "CI bot"

#[tokio::test]
async fn create_session_for_user_reflects_the_operator_flag() // create_user(…, operator=true) first;
    // create_session_for_user → decode_marker(.marker).is_operator == true

#[tokio::test]
async fn create_session_for_user_unknown_username_errors() // .is_err()

#[tokio::test]
async fn seed_user_duplicate_username_errors() // seed_user twice, same name → second .is_err()
```

- [x] **Step 2: Run, verify they fail**

Run: `cargo nextest run -p test-support seed_session` Expected: FAIL —
`seed_user` / `create_session_for_user` / `SeedRecord` undefined.

- [x] **Step 3: Implement** in `test-support/src/lib.rs`

Shared private core: look up/insert the user, then one session path — parse
`label.unwrap_or("E2E seed")` as `SessionLabel` (`common::session_label`,
`FromStr`), `state.sessions.create_session(user_id, &label) -> RawToken`, build
`set_cookie` via `host::auth::session_cookie_header(&token, false)`, build
`marker` via
`encode_marker(&SessionUser { username: username.parse()?, is_operator })`, and
return the record (`token` rendered via its `Display`). `seed_user` passes
`is_operator: false` to `create_user` and the record; `create_session_for_user`
reads the `UserRecord` via `state.users.get_user_by_username` (error
`unknown user {username}` when `None`) and takes `is_operator` from it. Every
branch is pinned by a Step 1 test.

- [x] **Step 4: Run, verify they pass**

Run: `cargo nextest run -p test-support` Expected: PASS — new 5 plus the
existing suites.

- [x] **Step 5: Gate + commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask check
git add test-support/src/lib.rs test-support/Cargo.toml
git commit -m "feat(test-support): SeedRecord + seed_user/create_session_for_user (#791)"
```

---

### Task 4: test-support CLI — `seed-user` / `create-session` subcommands

**Files:**

- Modify: `test-support/src/main.rs`

**Interfaces:**

- Consumes: Task 3's `seed_user` / `create_session_for_user`.
- Produces (Task 5 shells out to exactly these):

```text
test-support seed-user       --db $JAUNDER_DB --username U --password P [--label L]
test-support create-session  --db $JAUNDER_DB --username U [--label L]
```

Both print one line of JSON (the `SeedRecord`) on stdout; diagnostics stay on
stderr like the existing subcommands.

- [x] **Step 1: Extend the failing dispatch test** (`main.rs`'s
      `run_dispatches_db_commands_against_a_temp_db`): after the existing two
      dispatches,

```rust
run(cli(Commands::SeedUser {
    db: db.clone(),
    username: "bob".to_owned(),
    password: "password123".to_owned(),
    label: None,
}))
.await
.expect("seed-user should dispatch and succeed");

run(cli(Commands::CreateSession {
    db: db.clone(),
    username: "bob".to_owned(),
    label: Some("CI bot".to_owned()),
}))
.await
.expect("create-session should dispatch and succeed");

// Read-back proof of wiring: bob exists and holds two labelled sessions.
let state = storage::open_existing_database(&db).await.unwrap();
let bob = state
    .users
    .get_user_by_username(&"bob".parse().unwrap())
    .await
    .unwrap()
    .expect("bob created");
let sessions = state.sessions.list_sessions(bob.user_id).await.unwrap();
assert_eq!(sessions.len(), 2, "seed-user + create-session = two sessions");
```

- [x] **Step 2: Run, verify it fails**

Run: `cargo nextest run -p test-support run_dispatches` Expected: FAIL —
`Commands::SeedUser` / `Commands::CreateSession` undefined.

- [x] **Step 3: Implement** — two `Commands` variants (same `--db`/`--username`
      arg shapes as `CreateUser`; `seed-user` adds `--password`; both add
      `#[arg(long)] label: Option<String>`), two match arms in `run`, and two
      thin handlers:

```rust
async fn cmd_seed_user(
    db: &DbConnectOptions,
    username: &str,
    password: &str,
    label: Option<&str>,
) -> anyhow::Result<()> {
    let state = storage::open_existing_database(db).await?;
    let record = seed_user(&state, username, password, label).await?;
    println!("{}", serde_json::to_string(&record)?);
    Ok(())
}
```

`cmd_create_session` is the same shape over `create_session_for_user`.
(serde_json is already a dependency from Task 3.) `main.rs` only serializes —
the record-building stays in `lib.rs` (spec D1 "Structure").

- [x] **Step 4: Run, verify it passes**

Run: `cargo nextest run -p test-support` Expected: PASS.

- [x] **Step 5: Smoke the real binary**

Run:
`cargo run -p test-support -- seed-user --db "sqlite:$(mktemp -d)/t.db" --username smoke --password password123`
— after creating/migrating that DB (run `storage::open_database` once via the
existing dispatch test's approach, or point at any already-migrated temp DB).
Expected: one JSON line with all seven `SeedRecord` fields.

- [x] **Step 6: Gate + commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask check
git add test-support/src/main.rs
git commit -m "feat(test-support): seed-user and create-session subcommands (#791)"
```

---

### Task 5: end2end `seed.ts` — seed plumbing, tombstoned init script, timed tool calls

**Files:**

- Modify: `end2end/tests/seed.ts`
- Modify: `end2end/tests/posts.spec.ts:426-431, 463-473, 513-524` (await +
  unwrap `perf.timed` around now-self-timing calls — mechanical; the `register`
  calls inside are Task 11's)
- Modify: `end2end/tests/invite.spec.ts:13-17, 30-31, 89, 110` (await the
  `seedConfigViaTool` calls; `afterAll` becomes `async`)

**Interfaces:**

- Consumes: Task 4's subcommands; `withTimedAction` (`./actions`); `BASE_URL`
  (`./helpers` — call-time-only import cycle, safe in ESM).
- Produces (Tasks 6-7 consume):

```ts
/** The `test-support` seed record, mapped to camelCase at the JSON boundary. */
export type SeedRecord = {
  username: string;
  userId: number;
  isOperator: boolean;
  token: string;
  /** Full `Set-Cookie` value from the server's own `session_cookie_header`. */
  setCookie: string;
  /** The localStorage key the marker belongs under (Rust-owned). */
  markerKey: string;
  /** The advisory marker JSON, from `common::session_user::encode_marker`. */
  marker: string;
};

/** The subset `applySeededSession` needs — `TestUser` also satisfies it. */
export type SeededSession = Pick<
  SeedRecord,
  "setCookie" | "marker" | "markerKey"
>;

export async function seedUserViaTool(
  username: string,
  password: string,
): Promise<SeedRecord>;

export async function createSessionViaTool(
  username: string,
): Promise<SeedRecord>;

/** Inject the session + companion cookies and register the tombstoned init
 *  script (once per context). Does NOT navigate (spec D5). */
export async function applySeededSession(
  context: BrowserContext,
  session: SeededSession,
): Promise<void>;
```

- [x] **Step 1: Add the pinned constants and the tool runner**

```ts
/** Companion cookie carrying the marker payload to the init script. Named to
 *  stay clear of AC2's `jaunder_auth` rg check — the marker key itself is
 *  never spelled in TypeScript; it arrives in the seed record. */
const SEED_MARKER_COOKIE = "jaunder_seed_marker";
/** localStorage tombstone: the marker value the init script last applied. */
const SEED_APPLIED_KEY = "jaunder_seed_applied";

function runSeedTool(args: string[]): SeedRecord {
  const stdout = execFileSync("test-support", args, {
    stdio: "pipe",
    env: process.env,
    encoding: "utf8",
  });
  const raw = JSON.parse(stdout) as Record<string, unknown>;
  return {
    username: raw.username as string,
    userId: raw.user_id as number,
    isOperator: raw.is_operator as boolean,
    token: raw.token as string,
    setCookie: raw.set_cookie as string,
    markerKey: raw.marker_key as string,
    marker: raw.marker as string,
  };
}
```

- [x] **Step 2: Add `seedUserViaTool` / `createSessionViaTool`**, self-timing
      and page-less (D7):

```ts
export async function seedUserViaTool(
  username: string,
  password: string,
): Promise<SeedRecord> {
  return withTimedAction(null, "tool.users.seed", async () =>
    runSeedTool(["seed-user", "--username", username, "--password", password]),
  );
}

export async function createSessionViaTool(
  username: string,
): Promise<SeedRecord> {
  return withTimedAction(null, "tool.sessions.create", async () =>
    runSeedTool(["create-session", "--username", username]),
  );
}
```

(`--db` comes from `JAUNDER_DB` in the environment in both harnesses, like
`seedPostsViaTool`.)

- [x] **Step 3: Add `applySeededSession`** — cookie parse, companion cookie,
      once-per-context tombstoned init script:

```ts
const scriptedContexts = new WeakSet<BrowserContext>();

export async function applySeededSession(
  context: BrowserContext,
  session: SeededSession,
): Promise<void> {
  // Parse the server-emitted Set-Cookie value; only the pair and Path are
  // read — attribute names are never restated here (AC2).
  const [pair, ...attrs] = session.setCookie.split("; ");
  const eq = pair.indexOf("=");
  const name = pair.slice(0, eq);
  const value = pair.slice(eq + 1);
  const path =
    attrs
      .find((a) => a.toLowerCase().startsWith("path="))
      ?.slice("path=".length) ?? "/";
  const domain = new URL(BASE_URL).hostname;

  await context.addCookies([
    { name, value, domain, path, httpOnly: true, sameSite: "Lax" },
    {
      name: SEED_MARKER_COOKIE,
      value: encodeURIComponent(session.marker),
      domain,
      path: "/",
      httpOnly: false,
      sameSite: "Lax",
    },
  ]);

  if (!scriptedContexts.has(context)) {
    scriptedContexts.add(context);
    await context.addInitScript(`(() => {
  const prefix = ${JSON.stringify(SEED_MARKER_COOKIE)} + "=";
  let want = null;
  for (const part of document.cookie.split("; ")) {
    if (part.startsWith(prefix)) {
      want = decodeURIComponent(part.slice(prefix.length));
      break;
    }
  }
  if (want === null) return;
  if (localStorage.getItem(${JSON.stringify(SEED_APPLIED_KEY)}) === want) return;
  localStorage.setItem(${JSON.stringify(session.markerKey)}, want);
  localStorage.setItem(${JSON.stringify(SEED_APPLIED_KEY)}, want);
})();`);
  }
}
```

The tombstone table this implements is spec D3's: first nav applies, later navs
no-op, UI logout respected, re-seed as another user applies.

- [x] **Step 4: Self-time the two existing tool helpers** — `seedPostsViaTool`
      and `seedConfigViaTool` become `async`, wrapping their `execFileSync` in
      `withTimedAction(null, "tool.posts.seed", …)` /
      `withTimedAction(null, "tool.config.set", …)`. Bodies otherwise unchanged.

- [x] **Step 5: Update existing call sites to await**

- `posts.spec.ts:426-431` — drop the `perf.timed("seed_posts", …)` wrapper (the
  call self-times now); keep the arguments.
- `posts.spec.ts:463-466, 470-473, 514-517, 521-524` — drop the
  `perf.timed("seed_author_*" / "seed_self" / "seed_other")` wrappers; the
  `register` calls inside stay until Task 11.
- `invite.spec.ts:13-17` —
  `test.afterAll(async () => { await seedConfigViaTool(…); await seedConfigViaTool(…); })`;
  `:30-31`, `:89`, `:110` — add `await`.

- [x] **Step 6: Verify** — typecheck + the seed-heavy spec still passes end to
      end:

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask e2e-local tests/posts.spec.ts`
Expected: PASS (old `register` still present; only the tool-call timing
changed).

- [x] **Step 7: Gate + commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask check
git add end2end/tests/seed.ts end2end/tests/posts.spec.ts end2end/tests/invite.spec.ts
git commit -m "feat(e2e): seed sessions via test-support, time every tool spawn (#791)"
```

---

### Task 6: end2end `helpers.ts` — the `signInAs*` surface + `registerViaUi`

**Files:**

- Modify: `end2end/tests/helpers.ts`

**Interfaces:**

- Consumes: Task 5's `seedUserViaTool` / `createSessionViaTool` /
  `applySeededSession`.
- Produces (Tasks 7-11 consume; `register`/`registerKnown`/`registerAndLogin`
  remain until Task 12):

```ts
/** The fixed password every seeded account gets (spec D4). */
export const TEST_PASSWORD = "testpassword123";

/** `user1754…`-style unique names; `prefix` distinguishes invitees etc. */
export function generateUsername(prefix = "user"): string;

/** Seed a fresh account + session, inject into `page`'s context, return the
 *  username. Does NOT navigate (D5). */
export async function signInAsNewUser(page: Page): Promise<string>;

/** Same, returning the fixed password too — for tests that re-drive the
 *  account across contexts (`signInAs`) or through the login form. */
export async function signInAsNewUserKnown(
  page: Page,
): Promise<{ username: string; password: string }>;

/** Seed a session for an EXISTING account (e.g. the harness-seeded
 *  `testoperator`/`testlogin`), inject into `page`'s context. Does NOT
 *  navigate (D5). */
export async function signInAs(page: Page, username: string): Promise<void>;

/** The real UI registration flow — reserved for the D6 holdouts that prove
 *  it. Emits the timed action under the unchanged name `flow.register`. */
export async function registerViaUi(
  page: Page,
  firstNavigationTimeoutMs: number,
): Promise<string>;
```

- [x] **Step 1: Implement the five additions.** `registerViaUi` is today's
      `register` body verbatim (including the success/error race and the
      `flow.register` action name), with its username from `generateUsername()`.
      `signInAsNewUser` = `seedUserViaTool(generateUsername(), TEST_PASSWORD)` →
      `applySeededSession(page.context(), record)` → `record.username`.
      `signInAs` = `createSessionViaTool(username)` → `applySeededSession`. Each
      carries a doc comment stating the no-navigation postcondition (D5) and
      that the caller's first `goto` is the cold navigation.

- [x] **Step 2: Rewrite the module doc** (`helpers.ts:30-37`): seeded sign-in
      helpers are the default for setup; `registerViaUi` / `login` /
      `fillLoginForm` are for the D6 holdouts that exercise the real flows.

- [x] **Step 3: Verify** — existing suite still green (additive change):

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask e2e-local tests/auth.spec.ts`
Expected: PASS.

- [x] **Step 4: Gate + commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask check
git add end2end/tests/helpers.ts
git commit -m "feat(e2e): signInAs*/registerViaUi helpers backed by seeded sessions (#791)"
```

---

### Task 7: end2end `fixtures.ts` — `TestUser` gains seed fields; fixture reshape

**Files:**

- Modify: `end2end/tests/fixtures.ts` (`TestUser` type at :364; `registeredPage`
  :486-489; `user` :491-504; `verifiedUser` :545-555; imports :28-44)

**Interfaces:**

- Consumes: Task 5/6's helpers.
- Produces:

```ts
export type TestUser = {
  username: string;
  password: string;
  email: string;
  token: string;
  setCookie: string;
  marker: string;
  markerKey: string;
  isOperator: boolean;
};
```

- [x] **Step 1: Extend `TestUser`** to the shape above (gains five fields, loses
      none — D8).

- [x] **Step 2: `user` becomes a pure seed** (no throwaway context, no page, no
      navigation):

```ts
user: async ({}, use) => {
  const record = await seedUserViaTool(generateUsername(), TEST_PASSWORD);
  await use({
    username: record.username,
    password: TEST_PASSWORD,
    email: `${record.username}@example.com`,
    token: record.token,
    setCookie: record.setCookie,
    marker: record.marker,
    markerKey: record.markerKey,
    isOperator: record.isOperator,
  });
},
```

- [x] **Step 3: `verifiedUser` seeds instead of logging in**, then drives the
      set-email/verify UI as today:

```ts
verifiedUser: async ({ tracedContext, user, mailbox }, use) => {
  const context = await tracedContext();
  const page = await context.newPage();
  await applySeededSession(context, user);
  await setAndVerifyEmail(page, user.email, mailbox);
  await context.close();
  await use(user);
},
```

(`applySeededSession` accepts `TestUser` via the `SeededSession` pick;
`setAndVerifyEmail` navigates first, so the marker is planted before its `goto`.
The `testInfo`/`firstNav` locals that only fed `login` go away.)

- [x] **Step 4: `registeredPage` seeds then mounts `/` once**:

```ts
registeredPage: async ({ page, firstNav }, use) => {
  const record = await seedUserViaTool(generateUsername(), TEST_PASSWORD);
  await applySeededSession(page.context(), record);
  await goto(page, "/", { timeout: firstNav });
  await use(page);
},
```

It must still yield a mounted page — its consumers assume one (D8). Update its
doc comment ("Registers the DEFAULT page…" → seeds the default page's context
and mounts `/`).

- [x] **Step 5: Fix imports** — drop `register`/`login` from the `./helpers`
      import where now unused, add `generateUsername`, `TEST_PASSWORD`, `goto`;
      add `applySeededSession`, `seedUserViaTool` from `./seed`.

- [x] **Step 6: Verify** — the fixture-consuming specs pass unchanged (their
      call sites are unaffected):

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask e2e-local tests/email.spec.ts`
Expected: PASS — `user`/`mailbox`/`verifiedUser` consumers see the same
contract.

- [x] **Step 7: Gate + commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask check
git add end2end/tests/fixtures.ts
git commit -m "feat(e2e): seed the user/verifiedUser/registeredPage fixtures (#791)"
```

---

### Task 8: `auth.spec.ts` + `authed-flash.spec.ts` — conversions, holdouts, AC5 tests

**Files:**

- Modify: `end2end/tests/auth.spec.ts`
- Modify: `end2end/tests/authed-flash.spec.ts`

**Interfaces:**

- Consumes: Task 6/7's helpers and fixtures.

- [x] **Step 1: Convert the three setup-logins** (`auth.spec.ts:115`, `:147`,
      `:164`): `await login(page, user.username, user.password);` →
      `await signInAs(page, user.username);` **plus an added
      `await goto(page, "/");`** immediately after (D5 — all three act on the
      current page next: `page.evaluate` / `click(SEL.logoutLink)`). Update each
      test's comment: login-as-setup → seeded session; the logout subject is
      unchanged.

- [x] **Step 2: Holdout comments.** Every surviving UI-auth row of D6 carries a
      `// Holdout (spec D6): proves …` comment — `auth.spec.ts` :13, :21, :36,
      :48, :56, :87, :133 and the two `password_reset.spec.ts` fillLoginForm
      sites (no code change there, just the comment at the
      `goto(page, "/login")`).

- [x] **Step 3: `authed-flash.spec.ts` holdouts.** `:21` and `:72`
      `register(page, firstNav)` → `registerViaUi(page, firstNav)`, each with
      its D6 holdout comment ("registering leaves a correct marker" / "the
      pre-paint redirect path, on a real marker"). The `:108`
      `login(page, "testoperator", …)` stays, with its holdout comment ("logging
      in leaves a correct marker").

- [x] **Step 4: Add the AC5 test** as a sibling directly after the `:17` test:

```ts
// AC5 (#791): a seeded session — no UI flow — must satisfy the same pre-paint
// contract as the registerViaUi holdout above. This is what proves D3's
// tombstoned init script feeds the <head> script.
test("seeded: pre-paint auth marks html.authed and data-user", async ({
  page,
  firstNav,
}) => {
  const username = await signInAsNewUser(page);
  await goto(page, "/", { timeout: firstNav });

  await expect(page.locator("html")).toHaveClass(/\bauthed\b/);
  await expect(page.locator("html")).toHaveAttribute("data-user", username);
});
```

- [x] **Step 5: Add the tombstone's logout-row test** (D3's subtlest branch —
      the pushState logout tests never re-run an init script, so only a full
      post-logout navigation pins it):

```ts
// D3 (#791): after a UI logout the init script must NOT re-apply the seeded
// marker — the tombstone (applied == companion cookie) makes it a no-op.
test("seeded: logout survives a full navigation (tombstone respected)", async ({
  page,
  firstNav,
}) => {
  await signInAsNewUser(page);
  await goto(page, "/", { timeout: firstNav });
  await click(page, SEL.logoutLink);
  await page.waitForURL(`${BASE_URL}/`, { timeout: 10_000 });

  await goto(page, "/", { timeout: firstNav });

  await expect(page.locator("html")).not.toHaveClass(/\bauthed\b/);
  await expect(page.locator(SEL.logoutLink)).toHaveCount(0);
});
```

- [x] **Step 6: Fix imports** in both specs (`signInAs`, `registerViaUi`,
      `click` as needed; drop unused).

- [x] **Step 7: Verify**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask e2e-local tests/auth.spec.ts`
Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask e2e-local tests/authed-flash.spec.ts`
Expected: PASS both, including the two new tests.

- [x] **Step 8: Gate + commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask check
git add end2end/tests/auth.spec.ts end2end/tests/authed-flash.spec.ts end2end/tests/password_reset.spec.ts
git commit -m "test(e2e): convert auth specs to seeded sessions, pin seeded pre-paint (#791)"
```

---

### Task 9: Setup-login conversions — admin-site, backup, invite, email, visibility

**Files:**

- Modify: `end2end/tests/admin-site.spec.ts` (:10, :50, :92, :111)
- Modify: `end2end/tests/backup.spec.ts` (:12, :51, :74, :105)
- Modify: `end2end/tests/invite.spec.ts` (:36, :113)
- Modify: `end2end/tests/email.spec.ts` (:11, :29)
- Modify: `end2end/tests/visibility.spec.ts` (:65, :91 — the `loginAs` helper
  call sites)

**Interfaces:**

- Consumes: Task 6's `signInAs`.

- [x] **Step 1: Apply the conversion rule at every listed site:**
  - `await login(page, "testoperator", "testpassword123");` →
    `await signInAs(page, "testoperator");`
  - `await login(page, "testlogin", "testpassword123");` →
    `await signInAs(page, "testlogin");`
  - `await login(page, user.username, user.password);` →
    `await signInAs(page, user.username);`
  - `await login(page, loginAs.username, loginAs.password);` →
    `await signInAs(page, loginAs.username);`

  No added `goto` at any of these: each is followed by its own navigation
  (`goto(page, …)`, `setAndVerifyEmail`, or the visibility helper's permalink
  `goto`) — verify per site while editing. Where a converted line's comment
  references "Log in as", reword to "Sign in as … (seeded session)".

- [x] **Step 2: invite.spec holdout comments** at Test A's invitee form flow and
      Test B (D6 rows; code unchanged).

- [x] **Step 3: Fix imports** in all five files.

- [x] **Step 4: Verify**

Run (each):
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask e2e-local tests/<file>.spec.ts`
Expected: PASS all five. (`invite.spec.ts` runs in the serial `chromium-admin`
project — the host loop's filter still selects it.)

- [x] **Step 5: Gate + commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask check
git add end2end/tests/admin-site.spec.ts end2end/tests/backup.spec.ts end2end/tests/invite.spec.ts end2end/tests/email.spec.ts end2end/tests/visibility.spec.ts
git commit -m "test(e2e): signInAs for operator/setup logins (#791)"
```

---

### Task 10: Register conversions A — atompub, audiences, authed-cls, timeline-cls

**Files:**

- Modify: `end2end/tests/atompub.spec.ts` (:43, :76, :98)
- Modify: `end2end/tests/audiences.spec.ts` (:27, :32, :173, :189, :213, :218,
  :252, :274, :288, :313)
- Modify: `end2end/tests/authed-cls.spec.ts` (:30)
- Modify: `end2end/tests/timeline-cls.spec.ts` (:87)

**Interfaces:**

- Consumes: Task 6's `signInAsNewUser`.

- [x] **Step 1: Apply the conversion rule at every listed site:**
  - `const username = await register(page, firstNav);` →
    `const username = await signInAsNewUser(page);`
  - `await register(page, slowBrowserFirstNavigationTimeoutMs(info, 30_000));`
    (and `testInfo` variants) → `await signInAsNewUser(page);`
  - Extra-context pages (`xPage`, `viewerPage`, …) convert the same way.

  No added `goto`: every site either navigates next or acts purely via
  `page.request` (session cookie is already in the jar). Verify per site; if a
  site's next action touches the page DOM without an intervening `goto`, add
  `await goto(page, "/");` and note it — the census says none of these four
  files need one.

- [x] **Step 2: Clean dead budget plumbing.** Where `firstNav` /
      `slowBrowserFirstNavigationTimeoutMs` locals or destructured fixtures are
      now unused, remove them; keep them where a surviving
      `goto(..., { timeout: firstNav })` still consumes them. Update stale
      comments (e.g. `authed-cls.spec.ts:27-29`'s "register() (not the
      registeredPage fixture)…" — reword to `signInAsNewUser`, keeping the
      owner-scoping rationale).

- [x] **Step 3: Fix imports** in all four files.

- [x] **Step 4: Verify**

Run (each):
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask e2e-local tests/<file>.spec.ts`
Expected: PASS all four.

- [x] **Step 5: Gate + commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask check
git add end2end/tests/atompub.spec.ts end2end/tests/audiences.spec.ts end2end/tests/authed-cls.spec.ts end2end/tests/timeline-cls.spec.ts
git commit -m "test(e2e): signInAsNewUser for atompub/audiences/cls specs (#791)"
```

---

### Task 11: Register conversions B — feeds (D9), media (D5 gotos), posts, visibility

**Files:**

- Modify: `end2end/tests/feeds.spec.ts` (:47, :109, :153, :177, :198, :245,
  :302, :328; D9 at :190-196; budget comment :26-29)
- Modify: `end2end/tests/media.spec.ts` (:10, :41, :80, :138, :145, :153, :175,
  :232, :264, :281)
- Modify: `end2end/tests/posts.spec.ts` (:330, :424, :464, :471, :515, :522)
- Modify: `end2end/tests/visibility.spec.ts` (:114, :124, :162, :173, :217,
  :222, :228, :289)

**Interfaces:**

- Consumes: Task 6's `signInAsNewUser` / `signInAsNewUserKnown`.

- [x] **Step 1: feeds.spec.ts.** Task 10's rule at the eight sites, plus **D9**:
      delete the Alice logout dance (:190-196 — the `goto` to `/logout` / logout
      click and its explanatory comment) so Bob's seed simply replaces Alice's
      session in place; the comment at :190-196 is superseded by one line:
      "Bob's seed replaces Alice's cookie + companion cookie in place; the
      tombstoned init script swaps the marker (D9)." Update the budget comment
      at :26-29 (no more `register()` navigations — restate the remaining cost
      drivers). The Alice site becomes
      `const alice = await signInAsNewUser(page);`, Bob
      `const bob = await signInAsNewUser(page);`.

- [x] **Step 2: media.spec.ts.** Task 10's rule at the ten sites, **plus the two
      D5 `goto` additions**:
  - :138 — after `signInAsNewUser(page)`, add
    `await goto(page, "/", { timeout: slowBrowserFirstNavigationTimeoutMs(testInfo, 30_000) });`
    so `waitForSelector(page, "a[href='/media']")` has a mounted page.
  - :145 — same addition before `click(page, "a[href='/media']")`.

- [x] **Step 3: posts.spec.ts.** Task 10's rule at the six sites
      (:464/:471/:515/:522 are the `perf.timed`-unwrapped blocks from Task 5 —
      the register call inside each converts, and the block becomes two flat
      awaited statements).

- [x] **Step 4: visibility.spec.ts.** `registerKnown` sites →
      `signInAsNewUserKnown` (the return shape is identical:
      `{ username, password }`).

- [x] **Step 5: Fix imports** in all four files.

- [x] **Step 6: Verify**

Run (each):
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask e2e-local tests/<file>.spec.ts`
Expected: PASS all four.

- [x] **Step 7: Gate + commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask check
git add end2end/tests/feeds.spec.ts end2end/tests/media.spec.ts end2end/tests/posts.spec.ts end2end/tests/visibility.spec.ts
git commit -m "test(e2e): signInAsNewUser for feeds/media/posts/visibility (#791)"
```

---

### Task 12: Cutover — delete the old helpers; AC2/AC3/AC4 greps; full host suite

**Files:**

- Modify: `end2end/tests/helpers.ts` (delete `register`, `registerKnown`,
  `registerAndLogin`)

**Interfaces:**

- Consumes: every earlier task.

- [x] **Step 1: Delete** `register` (:197-234), `registerKnown` (:237-244), and
      `registerAndLogin` (:253-266) from `helpers.ts`.

- [x] **Step 2: AC3** — the old surface is gone:

Run: `rg -n '\bregister(Known)?\s*\(' end2end/tests` Expected: **no matches**
(scrub stray comment mentions too — they match the regex). Run:
`rg -n 'registerViaUi\(' end2end/tests` Expected: exactly 3 — the `helpers.ts`
definition + `authed-flash.spec.ts` ×2.

- [x] **Step 3: AC2** — no duplicated artifacts:

Run: `rg -n 'HttpOnly|SameSite|jaunder_auth' end2end/ test-support/` Expected:
no hits outside comments.

- [x] **Step 4: AC4** — holdouts exactly D6's thirteen rows:
      `rg -n 'registerViaUi\(|fillLoginForm\(|await login\(|goto\(page, "/(register|login)"' end2end/tests`
      and eyeball that every hit is a D6 row (or a helper definition), each with
      its holdout comment.

- [x] **Step 5: Full host suite green**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask e2e-local`
Expected: PASS — the whole suite (chromium + chromium-admin) on seeded auth.

- [x] **Step 6: Gate + commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask check
git add end2end/tests/helpers.ts
git commit -m "refactor(e2e): remove register/registerKnown/registerAndLogin (#791)"
```

---

### Task 13: Measurements — AC6 coverage, AC7 traces, AC8 validate

**Files:**

- Modify: `docs/coverage/server-fns-evidence.json` (regenerated; per-test title
  churn expected)

**Interfaces:**

- Consumes: everything. Produces: the PR-body numbers.

- [x] **Step 1: AC6 — server-fn coverage unchanged.**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask e2e sqlite chromium
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask server-fn-coverage regenerate
git diff wt-base-issue-791 -- docs/coverage/server-fns.json   # expect: empty
git diff --stat -- docs/coverage/server-fns-allowlist.json    # expect: no change
```

Expected: `server-fns.json` byte-identical to `wt-base-issue-791`; evidence-file
title churn only.

- [x] **Step 2: AC7 — like-for-like traces, before.** Create a scratch worktree
      at the fork tag and run the baseline there (the harness builds from the
      checkout, so it must be a separate tree):

```bash
git worktree add /tmp/jaunder-791-base wt-base-issue-791
devtool run --cwd /tmp/jaunder-791-base -- cargo xtask traces run --top 25
```

Record: `flow.register` count + total, warmup/fixture overhead, suite wall.

(May be started in the background as early as Task 3 to save wall-clock.)

- [x] **Step 3: AC7 — after.** Same command in the issue worktree at branch
      head; record the same rows, with an explicit full action listing (not just
      `--top 25`) for the `tool.users.seed` / `tool.sessions.create` /
      `tool.posts.seed` / `tool.config.set` rows.

Required outcomes: `flow.register` count ≤ 4 (the two `registerViaUi` holdouts
plus at most the auth.spec inline registrations, which record differently) — and
`registerViaUi` still emits under `flow.register`. Both outputs go in the PR
body.

- [x] **Step 4: AC8 — full local gate.**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-791-seed-users-via-api -- cargo xtask validate
```

Expected: PASS — static + coverage + all four
`{sqlite,postgres}×{chromium,firefox}` e2e combos.

- [x] **Step 5: Clean up the scratch worktree and commit any evidence
      regeneration.**

```bash
git worktree remove /tmp/jaunder-791-base
git add docs/coverage/server-fns-evidence.json
git commit -m "test(e2e): regenerate server-fn evidence for seeded auth (#791)"
```

---

## Self-review notes

- **Spec coverage:** D1 → Tasks 3-4; D2 → Tasks 2-3, 5; D3 → Task 5 (+ Task 8
  tests); D4 → Tasks 6, 12; D5 → Tasks 8, 11 (+ rule in 9-10); D6 → Tasks 8-9
  comments, Task 12 AC4; D7 → Task 5; D8 → Task 7; D9 → Task 11; D10 → ship
  (`adr promote`); D11 → Task 1. AC1 → Task 3; AC2/AC3/AC4 → Task 12; AC5 → Task
  8; AC6/AC7/AC8 → Task 13.
- **Type consistency:** `SeedRecord` field names match between Rust (`serde`
  snake_case) and TS (camelCase mapping in `runSeedTool`); `SeededSession` pick
  is satisfied by both `SeedRecord` and `TestUser`; `signInAsNewUserKnown`'s
  return matches the deleted `registerKnown`'s.
- **Known spec deviations (all disclosed at plan approval):** D6 13th row; D7
  via `withTimedAction(null,…)`; codec tests' home in `common`.
