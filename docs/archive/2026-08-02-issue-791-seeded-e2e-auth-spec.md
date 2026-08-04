# Spec — issue #791: seed e2e users/sessions out-of-band

Provenance: [#791](https://github.com/jaunder-org/jaunder/issues/791), child of
[#788](https://github.com/jaunder-org/jaunder/issues/788) (e2e client-side
cost).

## Problem

Driving `/register` through the browser is the single largest cost in the e2e
suite. Measured from CI run 30714621799: **103 `flow.register` runs per combo**
(84 tests ×1, 8 ×2, 1 ×3), averaging 2.46 s chromium / ~2.9 s firefox — 210 s
per chromium combo, 306 s per firefox combo, **~35 % of all in-`e2e.test`-span
time**. Each run pays a cold CSR page load + WASM mount, a form fill, a submit,
and a session redirect, none of which is the subject of the test that pays it.

The same waste appears in **18 of the 19** `login()` call sites, which want an
authenticated context rather than the login flow (10 of them are
`login(page, "testoperator", …)`).

## Decisions

### D1 — Mechanism: a `test-support` subcommand, not HTTP

Provisioning happens through the existing out-of-process `test-support` binary
(ADR-0046), doing in-process storage writes — the seam `seedPostsViaTool`
already uses (`end2end/tests/seed.ts:30`). No test-only route is added to the
shipped server and no HTTP round-trip is paid. (`seedConfigViaTool`, by
contrast, spawns the shipped `jaunder` binary; it is a sibling precedent for
out-of-process seeding, not the same tool.)

Two subcommands, both taking `--db` (`env = "JAUNDER_DB"`) like the existing
ones:

| Subcommand                                               | Does                                                                |
| -------------------------------------------------------- | ------------------------------------------------------------------- |
| `create-session --db … --username U [--label L]`         | Creates a session for an **existing** user; prints the seed record. |
| `seed-user --db … --username U --password P [--label L]` | `create-user` + `create-session` in one DB open; prints the record. |

Both print one line of JSON on stdout:

```json
{
  "username": "user1754…",
  "user_id": 42,
  "is_operator": false,
  "token": "kQ8…",
  "set_cookie": "session=kQ8…; HttpOnly; SameSite=Lax; Path=/",
  "marker_key": "jaunder_auth",
  "marker": "{\"username\":\"user1754…\",\"is_operator\":false}"
}
```

`--label` defaults to `"E2E seed"`. No test asserts on pre-existing session
labels (`atompub.spec.ts:73-92` asserts only the app password it mints itself),
and a distinct label makes seeded sessions obvious on `/sessions` and in
debugging.

`seed-user` creates the account through the real `UserStorage::create_user` path
(`test-support/src/lib.rs:106`), so the password is genuinely argon2-hashed and
the account stays loginable — `auth.spec.ts:56` still logs in as the `user`
fixture's account through the form.

**Structure.** The record-producing functions live in `test-support/src/lib.rs`
and **return** a `SeedRecord`; `main.rs` only serialises it to stdout. This is
what makes AC1 testable without capturing stdout, and matches the existing split
(`lib.rs` does the work, `main.rs` is a thin shell — `main.rs:76-77`).

### D2 — Both client-visible artifacts come from Rust, not from TypeScript

A seeded session has **two** artifacts, and neither is re-spelled in TypeScript:

- **The cookie.** `set_cookie` is emitted by
  `host::auth::session_cookie_header(&token, false)` (`host/src/auth.rs:81`) —
  the same function the server uses.
- **The marker.** `MARKER_KEY`, `SessionUser`, `encode_marker` and
  `decode_marker` move from `web/src/auth/marker.rs` to a new
  `common::session_user` module, verbatim with their tests. `SessionUser`'s only
  dependency is `common::username::Username`, so the move carries no leptos and
  respects `test-support`'s deliberately lean dep list
  (`test-support/Cargo.toml:24-28`). `web/src/auth/marker.rs` becomes a
  re-export, so no other call site changes.

**Cookie mapping.** `session_cookie_header` emits `Path=/` and no `Domain`;
Playwright's `addCookies` rejects `url` together with `domain`/`path`. So
`seed.ts` parses the header and passes `domain` (from
`new URL(BASE_URL).hostname`) plus the parsed `path`, never `url`. `BASE_URL`
already varies by harness (`helpers.ts:51-52`), and this is the only place the
origin is derived.

### D3 — The marker is seeded by one tombstoned init script per context

The CSR's flash-free boot (#181, #591, ADR-0044) reads a **localStorage marker**
`jaunder_auth`, written client-side by the real register/login flows and read by
the pre-paint `<head>` script. A cookie-only seed would boot every converted
test anonymous-then-reconcile — which breaks `authed-cls.spec.ts` and
`timeline-cls.spec.ts` outright, since the chrome appearing post-mount _is_ a
layout shift.

The naive mechanism — `context.addInitScript` with a fixed payload — is unsound.
**Playwright 1.58.2 has no way to remove an init script** (`addInitScript`
gained a `Disposable` return only in
[1.59](https://playwright.dev/docs/release-notes); there is still no
`removeInitScript` —
[microsoft/playwright#29499](https://github.com/microsoft/playwright/issues/29499)).
A fixed script re-injects on every later document load, so after a UI logout the
page boots `html.authed data-user=<stale>`, and a second seed stacks a second
script instead of replacing the first.

So: **one** init script is registered per context (guarded by a `WeakSet`), and
its payload rides in a readable companion cookie the helper can rewrite. The
script applies the payload only when it differs from what it last applied:

```js
// registered once per context; payload comes from the companion cookie
const want = readCookie(COMPANION); // the `marker` field, URI-encoded
if (want === null) return;
if (localStorage.getItem(APPLIED) === want) return; // already applied
localStorage.setItem(markerKey, want);
localStorage.setItem(APPLIED, want);
```

The tombstone is what makes it correct without any call-site cooperation:

| Event                        | State                      | Effect                                 |
| ---------------------------- | -------------------------- | -------------------------------------- |
| seed Alice, first navigation | `applied` unset ≠ cookie   | writes marker + tombstone              |
| later navigations            | `applied` == cookie        | no-op; the app owns the marker         |
| **UI logout**, then navigate | `applied` == cookie        | no-op — the app's removal is respected |
| **re-seed as Bob**           | `applied` (A) ≠ cookie (B) | writes Bob's marker + tombstone        |

`addInitScript` runs before the document's own inline `<head>` script, so the
pre-paint script sees the marker. (Verified against the 1.58.2 API surface.)

### D4 — The swap lands inside the existing helpers; the real flows get explicit names

- `signInAsNewUser(page)` replaces `register(page, firstNavMs)`.
- `signInAsNewUserKnown(page)` replaces `registerKnown(page, firstNavMs)`.
- `signInAs(page, username)` replaces setup-only
  `login(page, username, password)`.
- `registerViaUi(page, firstNavMs)` holds today's `register` body.
- `login(page, username, password, firstNavMs?)` keeps today's body, unchanged,
  for the one test whose subject is that logging in writes a marker.

`register` / `registerKnown` are **removed**, not kept as aliases: a `register`
that never visits `/register` misdescribes itself, and every call site is being
touched anyway.

**Username generation stays in TypeScript**, unchanged from `helpers.ts:197`
(`user${Date.now()}${Math.random().toString(36).slice(2, 8)}`), and is passed to
the tool as `--username`. The caller needs to know the name before the call
returns, and the existing per-user-unique scheme is what `seedPostsViaTool`'s
per-user slug uniqueness already relies on. The fixed password
`"testpassword123"` likewise stays a TypeScript constant.

### D5 — The seeded helpers do not navigate

`signInAsNewUser` / `signInAs` seed the context and return; they leave the page
where it was. The test's own first `goto` becomes the cold navigation, so each
call saves a whole cold page load on top of the form and submit.

Call sites that act on the current page immediately afterwards therefore need an
explicit `goto` added. The complete list, from a survey of all ~49
`register`/`registerKnown` sites and all 19 `login()` sites:

| Site                | Why it needs a `goto`                          |
| ------------------- | ---------------------------------------------- |
| `media.spec.ts:138` | `waitForSelector(page, "a[href='/media']")`    |
| `media.spec.ts:145` | `click(page, "a[href='/media']")`              |
| `auth.spec.ts:115`  | `page.evaluate` then `click(SEL.logoutLink)`   |
| `auth.spec.ts:147`  | `click(page, SEL.logoutLink)`                  |
| `auth.spec.ts:164`  | `expect(".j-sb-foot").toContainText(username)` |

`authed-flash.spec.ts:72` has the same shape (`page.evaluate` writing
`jaunder_home_redirect` on what would be `about:blank`) but is resolved by D6
instead: it becomes a holdout.

The `firstNavMs` parameter becomes dead on the seeded helpers and is dropped.

### D6 — Named holdouts keep the real flows honest

The complete set of surviving UI-auth call sites, each carrying a comment saying
what it proves:

| Site                                                                           | Flow            | Proves                                        |
| ------------------------------------------------------------------------------ | --------------- | --------------------------------------------- |
| `auth.spec.ts:13` "register page shows form"                                   | inline form     | `/register` renders                           |
| `auth.spec.ts:21` "register rejects a too-short password client-side"          | inline form     | client-side validation                        |
| `auth.spec.ts:36` "register with open policy succeeds"                         | inline form     | **`registration::register` coverage**         |
| `auth.spec.ts:48` "login page shows form"                                      | inline form     | `/login` renders                              |
| `auth.spec.ts:56` "login with valid credentials succeeds"                      | inline form     | **`auth::login` coverage**                    |
| `auth.spec.ts:87` "login navigates client-side without a full document reload" | inline form     | login is a pushState, not a reload (#591)     |
| `auth.spec.ts:133` "login with wrong password shows error"                     | `fillLoginForm` | the login error path                          |
| `invite.spec.ts:22` "invite link registration completes end-to-end"            | inline form     | invite-gated registration (#433)              |
| `invite.spec.ts:86` "invite-only /register with no code shows guidance"        | inline form     | the invite-only guidance branch               |
| `authed-flash.spec.ts:17` "pre-paint auth marks html.authed"                   | `registerViaUi` | **registering leaves a correct marker**       |
| `authed-flash.spec.ts:64` "`jaunder_home_redirect='app'` … redirect / → /app"  | `registerViaUi` | the pre-paint redirect path, on a real marker |
| `authed-flash.spec.ts:104` "operator: admin chrome … from the marker"          | `login`         | **logging in leaves a correct marker**        |
| `password_reset.spec.ts:36-46` "login with new password after reset"           | `fillLoginForm` | a reset password logs in; the old one fails   |

`password_reset.spec.ts` was missed by the original census (it calls
`fillLoginForm`, not `login`); its post-reset form logins are the test's
subject, so it is a holdout by nature. Added 2026-08-03 after the rebase
re-census.

Deliberately **not** holdouts: the three `login()` sites in `auth.spec.ts`
(`:115`, `:147`, `:164`) are logout tests using login as setup, and
`invite.spec.ts:113` is an operator-setup login. All four convert to `signInAs`
(with an added `goto` where D5 requires one).

### D7 — Seeding is a timed action

Seeds record through the existing `withTimedAction(page | null, name, fn)`
(`actions.ts`) with a **null page** — page-less actions became first-class when
#794 landed it for the capture-file polls (`ActionRecord.pageUrl` is optional),
so no new timing API is added. Seeds record as `tool.users.seed` and
`tool.sessions.create`; a throwing seed records `ok: false` with the error and
rethrows, which `withTimedAction` already does. (The pre-#794 draft of this
decision added a synchronous `timedToolCall`; the rebase made it redundant.)

The two existing untimed tool spawns are wrapped at the same time
(`tool.posts.seed`, `tool.config.set`) so every out-of-process spawn is
attributed — which is what #788's next round of analysis needs.

`api.*` naming is rejected: these spawns never touch HTTP.

### D8 — Fixture reshaping

`TestUser` (`fixtures.ts:294`) gains the seed record's fields — `token`,
`setCookie`, `marker`, `isOperator` — keeping `username` / `password` / `email`
so existing destructuring is unaffected. (#791's "fixtures keep their
signatures" is honoured for consumers; the value gains fields, loses none.)

- `user` — becomes a pure `seedUserViaTool()` call. No throwaway context, no
  page, no navigation at all.
- `verifiedUser` — applies `user`'s seed record to its throwaway context via
  `applySeededSession(context, user)` instead of logging in, then drives
  set-email/verify through the UI as today (that flow is
  `email::request_verification` / `email::verify` coverage).
- `registeredPage` — seeds and then navigates to `/` once. Unlike the bare
  helper it must still yield a mounted page, because its consumers assume one.

### D9 — Identity switching replaces a workaround

`feeds.spec.ts:223-229` logs Alice out before registering Bob, and its comment
says why: `register()`'s success-wait (`a[href='/logout']`) resolves instantly
against Alice's still-present link, so Bob's session might not be active when
the next post is published. Seeding has no success-wait, so that hazard is gone:
the logout dance is **deleted**, and `signInAsNewUser(page)` replaces the cookie
and the companion cookie in place (D3 handles the marker). The following
`createPostViaApi` uses `page.request`, which shares the context cookie jar, so
it is authored by Bob.

### D10 — ADR

A new ADR is drafted at `docs/adr/0098-e2e-seeded-auth.md` (numberless;
promoted at ship by `cargo xtask adr promote`). ADR-0046 is about the binary;
this is a distinct decision about which flows e2e may fake, and the holdout
table in D6 is exactly the thing a future reader would delete without knowing
why.

### D11 — Separable concern: the Playwright bump

Playwright ≥ 1.59's disposable init scripts would simplify D3 for the
identity-switch case — re-seed could dispose-and-recreate the script with the
payload baked in, dropping the companion-cookie indirection and the per-context
`WeakSet`. **The bump does not retire the tombstone.** The tombstone exists for
the UI-logout case: logout is a page-side event Playwright never surfaces to
Node, so an undisposed fixed-payload script would re-inject the stale marker on
the post-logout navigation and boot `html.authed` after logout. Only an in-page
guard respects logout without call-site cooperation, and init scripts re-run on
every document load in every version (there is still no `removeInitScript` —
[microsoft/playwright#29499](https://github.com/microsoft/playwright/issues/29499)).
The bump is also a `flake.lock` nixpkgs change (`flake.nix:1279`, `:1305` pin
`playwright-test` and `playwright-driver.browsers` in lockstep with
`end2end/package.json:11`) that moves every other Nix-built dependency with it.
It is filed as its own issue under milestone #6 rather than folded in here.

## Acceptance criteria

**AC1 — the subcommands exist and are honest.** `test-support seed-user` and
`test-support create-session` each print a single JSON object with the D1
fields. Unit tests in `test-support`'s existing style (temp SQLite DB, per
`main.rs:159-164`) call the `lib.rs` functions directly — no stdout capture —
and assert that the returned record's `set_cookie` parses to a token that
`SessionStorage::authenticate` accepts and resolves to the seeded `user_id`, and
that `marker` round-trips through `common::session_user::decode_marker` to the
seeded username and operator flag.

**AC2 — neither artifact is duplicated.**
`rg -n 'HttpOnly|SameSite|jaunder_auth' end2end/ test-support/` returns no hits
outside a comment. The cookie string comes from
`host::auth::session_cookie_header` and the marker from
`common::session_user::encode_marker`; `web/src/auth/marker.rs` is a re-export
whose existing tests still pass unchanged.

**AC3 — `register` / `registerKnown` are gone.**
`rg -n '\bregister(Known)?\s*\(' end2end/tests` returns **no matches**.
`rg -n 'registerViaUi\(' end2end/tests` returns exactly the two
`authed-flash.spec.ts` holdout sites plus its definition in `helpers.ts`.

**AC4 — the holdouts are exactly D6's table.** The thirteen rows of D6 are the
complete set of surviving UI-auth call sites, verified by file:line, each with a
comment naming what it proves. No other spec navigates to `/register` or
`/login`, or calls `registerViaUi` / `login` / `fillLoginForm`.

**AC5 — a seeded session boots authed pre-paint.** A new test in
`authed-flash.spec.ts` — sibling to the `registerViaUi` holdout at `:17` and
named to say so — calls `signInAsNewUser(page)` (which returns the username),
then `goto(page, "/")`, and asserts `html` has class `authed` and
`data-user=<username>`. This is what proves D3's tombstone actually satisfies
the pre-paint script.

**AC6 — server-fn coverage is unchanged.** `cargo xtask e2e sqlite chromium`
(which produces the capture `regenerate` reads from
`.xtask/diagnostics/e2e-sqlite-chromium/`), then
`cargo xtask server-fn-coverage regenerate`. `docs/coverage/server-fns.json`
must be **byte-identical** to its value at `wt-base-issue-791` — the whole file,
`covered` and `orphans` alike, since that is what the gate compares
(`xtask/src/steps/server_fn_coverage_check.rs:143-151`, `CONTRIBUTING.md:543`).
Churn in `server-fns-evidence.json`'s per-test titles is expected. No new entry
in `server-fns-allowlist.json`.

**AC7 — the win is measured like-for-like.** `cargo xtask traces run --top 25`
is run **twice locally**: once at `wt-base-issue-791` and once at the branch
head, so both numbers come from the same harness (the issue's 103 × 2.46 s is a
CI figure and is not the baseline). Both outputs go in the PR body. Required:
`flow.register` count drops to **≤ 4** _and_ `registerViaUi` still emits the
action under the name `flow.register` — the name is not changed, so the count
means what it says. Because `--top 25` slices by `max_ms` and the per-test
action list is a top-30 slice, the cheap `tool.*` rows may not appear in the
default view; their cost is reported from the same run with an explicit
`--project`/full listing rather than inferred from absence.

**AC8 — the suite is green on all four combos.** `cargo xtask validate` passes
(static + coverage + `{sqlite,postgres}×{chromium,firefox}` e2e).

## Explicitly out of scope

- The Playwright ≥ 1.59 bump (D11) — filed separately.
- The per-test warmup fixture (#792) and the WebSub polling idle (#793) —
  sibling children of #788.
- CSR mount cost (#801), which depends on #794's boot breakdown.
- Whether the evidence file should carry per-test titles at all (#757); this
  change churns those titles but does not decide the question.
