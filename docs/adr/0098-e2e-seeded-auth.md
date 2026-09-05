# ADR-0098: e2e provisions auth by seeding, not by driving the UI

- Status: accepted
- Date: 2026-08-02
- Issue: [#791](https://github.com/jaunder-org/jaunder/issues/791)
- Amended: 2026-09-04
  ([#1233](https://github.com/jaunder-org/jaunder/issues/1233)) — the pinned
  Playwright 1.61 disposable init-script API replaces the original 1.58
  companion-cookie workaround; the original constraint remains historical
  context below.

## Context

Nearly every e2e test needs an authenticated account, and until now every one of
them obtained it the same way: by driving `/register` through the browser.
Measured from CI run 30714621799, that was **103 `flow.register` runs per
combo** at ~2.46 s chromium / ~2.9 s firefox — 210 s and 306 s per combo
respectively, **~35 % of all in-`e2e.test`-span time** (#788). Eighteen of the
nineteen `login()` call sites paid a comparable cost for the same reason: they
wanted a session, not a login.

The cost is a cold CSR page load plus WASM mount, a form fill, a submit, and a
redirect — none of which is the subject of the test paying for it.

Three constraints shape the alternative.

1. The credential is an HttpOnly `session` cookie whose exact attributes are
   built by `host::auth::session_cookie_header`.
2. The CSR's flash-free boot (#181, #591,
   [ADR-0044](0044-authenticated-owner-flash-free-enhancement.md)) depends on a
   **second**, client-visible artifact: the localStorage marker `jaunder_auth`,
   written by the register/login flows and read by the pre-paint `<head>`
   script. A session with the cookie but no marker boots visibly anonymous and
   then flips — which is a layout shift, so the CLS specs fail outright.
3. **Historical constraint.** At this decision's original acceptance, Playwright
   1.58.2 could not remove an init script. `addInitScript` gained a `Disposable`
   return in 1.59, although there was still no `removeInitScript`
   ([microsoft/playwright#29499](https://github.com/microsoft/playwright/issues/29499)).
   Consequently, a context-level script then lived on every subsequent document
   load for the context's lifetime. The pinned Playwright 1.61.1 API now lets
   the helper dispose its replacement script directly.

## Decision

The e2e suite provisions authentication **out of band**, through the
`test-support` binary ([ADR-0046](0046-test-support-seed-binary.md)):

- `test-support create-session --username U` mints a session for an existing
  user; `test-support seed-user --username U --password P` creates the account
  and its session in one database open. Both print one JSON line.
- **Both client-visible artifacts come from Rust.** The `Set-Cookie` value is
  produced by `host::auth::session_cookie_header`, and the marker by
  `common::session_user::encode_marker` — the codec having moved out of
  `web/src/auth/marker.rs` (which becomes a re-export) precisely so a non-leptos
  crate can reach it. Neither is re-spelled in TypeScript.
- The helper injects the cookie via `context.addCookies` and the marker via one
  **disposable, tombstoned init script per context**. A
  `WeakMap<BrowserContext, Disposable>` owns at most one seeded-auth script for
  each context. Before installing a replacement, the helper disposes the mapped
  script; a disposal failure leaves that entry intact, while successful disposal
  removes it before installation. An installation failure consequently leaves no
  entry. The helper immediately records the replacement after successful
  installation, before injecting the session cookie, so a cookie-injection
  failure still leaves the replacement owned for the next seed. The replacement
  bakes the Rust-produced marker key and value plus a fresh per-seed nonce into
  its payload. There is no readable companion cookie.
- The helpers are named for what they do: `signInAsNewUser(page)`,
  `signInAsNewUserKnown(page)`, and `signInAs(page, username)`. The old
  `register()` / `registerKnown()` are removed rather than aliased — a
  `register` that never visits `/register` misdescribes itself.
- The seeded helpers do **not** navigate. The test's own first `goto` becomes
  the cold navigation, so each call also saves a whole page load.
- Seeding is recorded in the trace as `tool.users.seed` / `tool.sessions.create`
  via the page-less form of the existing `withTimedAction`, so replacing UI time
  with tool time is visible rather than merely claimed.

### Why the tombstone, and not a fixed payload

The disposable replacement makes an identity switch deterministic: a re-seed
disposes the prior context script before installing the new one, so no prior
identity can remain registered. The tombstone remains necessary for logout,
without any call-site cooperation:

- After a UI logout, the app removes the marker — and a later full document load
  must not silently re-inject it, or the page boots
  `html.authed data-user=<stale>`.
- A same-user re-seed needs to reapply the marker after that logout even though
  the marker value itself is unchanged.

Each replacement compares its baked nonce to the tombstone. It writes its marker
and records its nonce only when they differ. Thus later loads leave marker
ownership with the app after logout, while every new seed — including one for
the same user — is distinguishable by nonce.

| Event                                  | State                            | Effect                                 |
| -------------------------------------- | -------------------------------- | -------------------------------------- |
| seed, first navigation                 | `applied` unset ≠ script nonce   | writes marker + tombstone              |
| later navigations                      | `applied` == script nonce        | no-op; the app owns the marker         |
| **UI logout**, then navigate           | `applied` == script nonce        | no-op — the app's removal is respected |
| **re-seed as another user**            | `applied` (A) ≠ script nonce (B) | writes B's marker + tombstone          |
| **logout, then re-seed the SAME user** | nonce differs                    | re-applies the marker — boots authed   |

Without the nonce, the last row would no-op (identical marker) and boot
anonymous pre-paint despite a fresh valid session.

### The holdouts

**The real flows survive at a named, deliberately small set of call sites**,
each carrying a comment saying what it proves:

| Site                                    | Flow            | Proves                                                                  |
| --------------------------------------- | --------------- | ----------------------------------------------------------------------- |
| `auth.spec.ts:13`, `:21`, `:36`         | inline form     | `/register`, its validation, and **`registration::register` coverage**  |
| `auth.spec.ts:48`, `:56`, `:87`, `:133` | inline form     | `/login`, its error path, pushState nav, and **`auth::login` coverage** |
| `invite.spec.ts:22`, `:86`              | inline form     | invite-gated registration (#433)                                        |
| `authed-flash.spec.ts:17`               | `registerViaUi` | **registering leaves a correct marker**                                 |
| `authed-flash.spec.ts:64`               | `registerViaUi` | the pre-paint redirect path, on a real marker                           |
| `authed-flash.spec.ts:104`              | `login`         | **logging in leaves a correct marker**                                  |
| `password_reset.spec.ts:36-46`          | `fillLoginForm` | a reset password logs in; the old one fails                             |

That table is the load-bearing part of this record. Seeding is safe **only**
because those rows still drive the genuine article; deleting one as "redundant"
silently removes the last evidence for a link in the chain. The bolded rows in
particular are the sole remaining coverage for `registration::register`,
`auth::login`, and the marker-writing behaviour of each.

## Consequences

- Server-fn flow coverage (#681,
  [ADR-0081](0081-empirical-server-fn-flow-coverage.md)) of
  `registration::register` and `auth::login` now rests entirely on the holdout
  rows above. `docs/coverage/server-fns.json` must stay byte-identical across
  this change; the per-test evidence titles churn heavily, which is expected.
- `docs/coverage/server-fns-evidence.json` will churn again on any future
  seeding change. That is a cost of the evidence file carrying titles at all
  (#757), not of this decision.
- `SessionUser` and the marker codec now live in `common`. That is a widening of
  `common`'s remit, justified by the codec being a wire format shared by the web
  client and out-of-process test tooling rather than web-only glue.
- Seeded users are created through the real `UserStorage::create_user` path, so
  passwords are genuinely argon2-hashed and seeded accounts remain loginable —
  `auth.spec.ts:56` still logs in as one through the form.
- Seeded sessions carry the label `"E2E seed"` (overridable), so they are
  distinguishable from real ones on `/sessions` and in debugging.
- There is no companion marker cookie. The marker payload exists only in the
  disposable init script, and the server receives no e2e-only marker artifact.
- Context closure remains Playwright's resource boundary. The helper adds no
  close wrapper or explicit context teardown hook.
- Anything that in future depends on a **third** client-visible auth artifact
  must either be seeded here too or be added to the holdout table. The marker
  was already such a surprise once.
