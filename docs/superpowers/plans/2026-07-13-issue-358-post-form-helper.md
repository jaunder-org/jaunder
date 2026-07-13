# Plan — issue #358: consolidate the `post_form` test helpers

Spec:
[`docs/superpowers/specs/2026-07-13-issue-358-post-form-helper.md`](../specs/2026-07-13-issue-358-post-form-helper.md).
The spec is "what/why"; this plan is "how". Read the spec's **Decision** and
**Per-file conversion** sections — they are not repeated here.

## Review header

- **Goal:** Delete the 13 copy-pasted `post_form` definitions across
  `server/tests/web/*.rs`, replacing them with one private core + five thin
  wrappers in `server/tests/helpers/mod.rs`. Pure refactor; no test assertion or
  product-code change.
- **Scope — in:** `server/tests/helpers/mod.rs` (add core + wrappers + 3 media
  fns to the registrar) and the 11 web test files that carry a copy. **Out:**
  `web_tags.rs` (no copy), `server/src/lib.rs`'s separate registrar, the static
  drift guard — both in #426 (filed, blocked-by #358).
- **Follow-up already filed:** #426. No issue-filing task remains.
- **Tasks:**
  1. Add `post_form_inner` + the 5 wrappers + the 3 media fns to
     `helpers/mod.rs`.
  2. Convert the 6 canonical-case files → `post_form`.
  3. Convert `web_media.rs` → `post_form` and drop its inline registration.
  4. Convert `web_account.rs` → `post_form`, collapsing the dead `Set-Cookie`
     slot.
  5. Convert `web_email.rs` + `web_password_reset.rs` → `post_form_with_mailer`.
  6. Convert `web_auth.rs` → the `secure` / `_with_ua` / `_with_bearer`
     wrappers.
  7. Full-suite gate + acceptance-criteria sweep.
- **Key risks/decisions:**
  - The helper is `#[path = "../helpers/mod.rs"]`-included into **six**
    integration harnesses of the `jaunder` package — `web`, `misc`, `feed`,
    `atompub`, `storage`, `projector` — so it recompiles into six binaries. All
    six are test targets of the _same_ package and share one dev-dependency set
    (`web`, `tower`, `common`, `server_fn`, `axum` are all present), so there is
    **no** risk of the new imports resolving in some includers but not others —
    the concern is only that Task 1 build the whole package's tests, not just
    `--test web`. `cargo build --tests -p jaunder` compiles all six.
    Unused-in-5-of-6 wrappers are absorbed by the crate-level
    `#![allow(dead_code)]`.
  - After each file's local `post_form` is deleted, imports it used _only_ for
    the inline body (`axum::http`, `axum::body::Body`, `tower::ServiceExt`,
    `crate::helpers::{noop_mailer, tmp_storage_path}`, `Arc`, `create_router`)
    may go unused → clippy `unused_imports` will fail the gate. Each conversion
    task removes the now-dead imports in the same step.
  - `secure_cookies` is a real behavioral knob in `web_auth` (insecure-cookie
    tests) — do not let the rename to `post_form_with_secure_flag` drop the
    `false` call sites.

## Global constraints

- **For agentic workers:** execute with **`jaunder-iterate`**; delegate a task
  to a subagent via **`jaunder-dispatch`** where it helps (the bulk call-site
  edits in tasks 2/6 are good candidates — keep the file churn out of the
  driver's context).
- **No product code changes.** Only `server/tests/**`. If a test assertion would
  have to change to make it compile, stop — that means a behavior drift the spec
  forbids.
- **Verify command (fast loop):** `cargo nextest run -p jaunder --test web` runs
  the whole web integration binary. Per-file:
  `cargo nextest run -p jaunder --test web <module>::` (e.g. `web_auth::`).
- **Gate before commit:** `cargo xtask check` (fmt + clippy + Nix coverage/tests
  over sqlite + postgres). Run via `devtool run -- cargo xtask check`. Per
  **`jaunder-commit`**. **No `Co-Authored-By` trailer.**
- **Commit granularity:** one commit per task (tasks 2–6 are independent file
  sets). Task 1 + task 2 may share a commit if task 1 alone leaves unused-fn
  warnings that only clear once a caller exists — decide at implementation time;
  prefer separate commits, fall back to combined only if the gate forces it.
- Confirm exact symbol paths at implementation time by reading the current
  copies (`web::media::ListMyMedia` etc., `common::mailer::MailSender`,
  `storage::AppState`) — the spec's signatures are the intended shape, not
  copy-paste-final.

---

## Task 1 — Add the consolidated surface to `helpers/mod.rs`

**Files:**

- Edit `server/tests/helpers/mod.rs`.

**Interfaces:** add one private `post_form_inner` and five `pub async fn`
wrappers exactly as shaped in the spec's Decision section. `post_form_inner`
builds the router (`jaunder::create_router` with `test_options()`, the passed
`mailer`, and `secure_cookies`), sets the `cookie` / `bearer` / `user_agent`
headers when `Some`, drives it with `tower::ServiceExt::oneshot`, and returns
`(StatusCode, Option<String>, String)` where the middle is the response's first
`set-cookie` header (matching how `web_auth`/`web_account`'s copies extract it
today).

Model the body on an existing copy — lift the richest one (`web_auth`'s
`post_form`/`_with_ua`/`_with_bearer`) so the header-setting + `Set-Cookie`
extraction logic is preserved verbatim; generalize the mailer and the two extra
header slots.

Add the 3 media fns to `ensure_server_fns_registered()`:

```rust
server_fn::axum::register_explicit::<web::media::ListMyMedia>();
server_fn::axum::register_explicit::<web::media::MediaUsage>();
server_fn::axum::register_explicit::<web::media::DeleteMedia>();
```

(Confirm the exact type paths against `web_media.rs:28-30`.)

**Import hygiene:** ensure the file `use`s everything the new bodies need
(`std::sync::Arc`, `axum::body::Body`,
`axum::http::{Request, StatusCode, header}`, `tower::ServiceExt`,
`common::mailer::MailSender`, `jaunder::create_router`). The crate-level
`#![allow(dead_code)]` covers wrappers unused by a given test crate.

**Test:** no new test. Verification is compile + downstream build:

- `cargo build --tests -p jaunder` → **PASS** (the web test binary compiles the
  new helpers even before any caller switches over).
- `cargo build --tests -p jaunder` → **PASS** — compiles all six harnesses that
  include `helpers/mod.rs` (`web`, `misc`, `feed`, `atompub`, `storage`,
  `projector`), confirming the new imports resolve in every includer (they share
  one dev-dep set, so this is a formality, but run it — the file recompiles six
  times).

**Done when:** both builds pass; `helpers/mod.rs` exposes the 5 wrappers +
updated registrar; no web file changed yet.

---

## Task 2 — Convert the 6 canonical-case files → `post_form`

**Files (edit each):** `audiences.rs`, `web_backup.rs`, `web_posts.rs`,
`web_sessions.rs`, `web_site.rs`, `web_subscriptions.rs`.

Per file:

1. Delete the local `async fn post_form`.
2. Add `post_form` to the file's `use crate::helpers::{…}` named-import block.
3. Remove imports left unused by the deletion (compiler/clippy will name them).
4. Call sites are unchanged — same name, same `(StatusCode, String)` return.

**Test / verify:**

- `cargo nextest run -p jaunder --test web audiences:: web_backup:: web_posts:: web_sessions:: web_site:: web_subscriptions::`
  → **PASS** (same count as before).
- `cargo clippy -p jaunder --tests` → clean (no `unused_imports`).

**Done when:**
`rg 'fn post_form' server/tests/web/{audiences,web_backup,web_posts,web_sessions,web_site,web_subscriptions}.rs`
→ 0 matches; those modules' tests pass unchanged.

_(Good `jaunder-dispatch` candidate — mechanical, six files, high call-site
volume in web_posts.)_

---

## Task 3 — Convert `web_media.rs` → `post_form`, drop inline registration

**Files:** edit `server/tests/web/web_media.rs`.

1. Delete the local `async fn post_form` (including its inline
   `register_explicit::<web::media::…>()` for
   `ListMyMedia`/`MediaUsage`/`DeleteMedia` — those now live in the shared
   registrar from task 1).
2. Import `post_form` from `crate::helpers`; drop now-unused imports.
3. Call sites unchanged.

**Test / verify:**

- `cargo nextest run -p jaunder --test web web_media::` → **PASS** — this is the
  load-bearing check that the registrar fold works: if the 3 fns weren't
  correctly homed in task 1, media routes 404 and these tests fail.

**Done when:**
`rg 'fn post_form|ListMyMedia|MediaUsage|DeleteMedia' server/tests/web/web_media.rs`
→ 0 matches; `web_media::` tests pass.

---

## Task 4 — Convert `web_account.rs` → `post_form`, collapse the dead cookie slot

**Files:** edit `server/tests/web/web_account.rs`.

1. Delete the local `async fn post_form` (the 3-tuple copy).
2. Import `post_form` from `crate::helpers`; drop now-unused imports.
3. Rewrite all 17 call-site destructures: `(status, _, body)` →
   `(status, body)`, `(status, _, _)` → `status`, `(status1, _, _)` → `status1`,
   `(_status, _, _)` → `_status`, etc. (The middle element was always `_`; drop
   it.)

**Test / verify:**

- `cargo nextest run -p jaunder --test web web_account::` → **PASS** (same
  count).
- `cargo clippy -p jaunder --tests` → clean.

**Done when:** `rg 'fn post_form' server/tests/web/web_account.rs` → 0; no
`(_, _, _)`-style ignored-cookie destructure of a `post_form` result remains;
tests pass.

---

## Task 5 — Convert `web_email.rs` + `web_password_reset.rs` → `post_form_with_mailer`

**Files:** edit `server/tests/web/web_email.rs`,
`server/tests/web/web_password_reset.rs`.

Per file:

1. Delete the local `async fn post_form` (the mailer-taking copy).
2. Import `post_form_with_mailer`; drop now-unused imports.
3. Call sites already pass the mailer as an argument — rename `post_form(` →
   `post_form_with_mailer(`; argument order (`state, mailer, uri, body, cookie`)
   is preserved by the wrapper signature, so the calls are otherwise unchanged.

**Test / verify:**

- `cargo nextest run -p jaunder --test web web_email:: web_password_reset::` →
  **PASS** — exercises the injected `CapturingMailSender` path (the reason these
  two files diverged).

**Done when:**
`rg 'fn post_form' server/tests/web/{web_email,web_password_reset}.rs` → 0; both
modules' tests pass.

---

## Task 6 — Convert `web_auth.rs` → `secure` / `_with_ua` / `_with_bearer` wrappers

**Files:** edit `server/tests/web/web_auth.rs`.

1. Delete the three local defs: `post_form`, `post_form_with_ua`,
   `post_form_with_bearer`.
2. Import `post_form_with_secure_flag`, `post_form_with_ua`,
   `post_form_with_bearer` from `crate::helpers`; drop now-unused imports.
3. Rename the base call sites `post_form(` → `post_form_with_secure_flag(`
   (arity, the trailing `secure_cookies` arg, and the
   `(StatusCode, Option<String>, String)` destructure all unchanged — including
   the `false`-secure insecure-cookie tests and the `set_cookie`-reading
   login/session sites). `_with_ua` / `_with_bearer` call sites keep their names
   (now resolving to the imported wrappers).

**Test / verify:**

- `cargo nextest run -p jaunder --test web web_auth::` → **PASS** — the largest
  and most behaviorally varied file; confirm the insecure-cookie
  (`secure_cookies=false`) and `Set-Cookie`-asserting tests still pass.

**Done when:** `rg 'fn post_form' server/tests/web/web_auth.rs` → 0;
`web_auth::` tests pass.

_(Good `jaunder-dispatch` candidate — ~29 call sites across three variants.)_

---

## Task 7 — Full gate + acceptance-criteria sweep

**Verify (no code change unless a check fails):**

1. `rg 'fn post_form' server/tests/web/` → **0 matches** (AC#1).
2. `rg 'post_form_inner' server/tests/helpers/mod.rs` → 1 def; wrappers delegate
   (AC#2 — manual read confirms no duplicated router/request-building).
3. `rg 'web::media::(ListMyMedia|MediaUsage|DeleteMedia)'` → present in
   `helpers/mod.rs`, absent from `web_media.rs` (AC#4).
4. `git diff wt-base-issue-358 -- server/tests/web/*.rs` shows only helper-call
   / destructure / import changes — **no assertion edits** (AC#5,
   AC#6-no-behavior).
5. `devtool run -- cargo xtask check` → **green** (AC#6 — host tests
   sqlite+postgres + clippy + coverage). Test count unchanged vs baseline.

**Done when:** all five checks pass; the branch is ready for `jaunder-ship`.

---

## Self-review

- Every spec acceptance criterion maps to a task check: AC1→T2–T6+T7.1,
  AC2→T1+T7.2, AC3→T1 (surface) exercised by T5/T6, AC4→T1+T3+T7.3, AC5→T4+T7.4,
  AC6→T7.5.
- Tasks are independently verifiable (each ends in a scoped `nextest` run) and
  ordered so the shared surface (T1) lands before any caller depends on it, and
  the registrar fold (T1) is validated by the first media-dependent tests (T3).
- No task smuggles product-code or new-feature work; the separable guard concern
  is #426, already filed — no filing task needed.
