# Plan — Issue #410: Password vertical (StrNewtype secret + client-validated auth forms)

**Spec:**
[`docs/superpowers/specs/2026-07-14-issue-410-password-vertical.md`](../specs/2026-07-14-issue-410-password-vertical.md)
— read it for the _what/why_, the current-state analysis (server/host threading
already done), and the rejected InviteCode-split alternative. This plan is the
_how_.

## Review header

**Goal.** Make `common::password::Password` conform to the ADR-0063 §2
**secret** surface via `#[derive(StrNewtype)] #[str_newtype(secret)]`, and
finish the ADR-0065 client-validation adoption on the two auth forms still using
raw password `<input>`s.

**Scope.**

- **In:** the `Password` derive adoption (drop hand-written `Debug`, drop
  inherent `as_str()` → generated `AsRef<str>`); migrate `RegisterPage` +
  `ResetPasswordPage` password inputs to `<ValidatedInput<Password>>` with
  disable-until-valid; two focused e2e assertions.
- **Out:** any `#[server]`/storage signature change; any serde bridge on
  `Password`; the stored-hash newtype (#17); HTTP-Basic app-password
  (`common/src/auth.rs`). Server/host threading is already complete on `main`.

**Tasks (one line each).**

1. Adopt `#[str_newtype(secret)]` on `Password`; delete hand-written `Debug` +
   inherent `as_str()`; migrate the `.as_str()` callers to `.as_ref()`.
2. `RegisterPage`: raw password `<input>` → `<ValidatedInput<Password>>` + gate
   submit on `password.is_valid()`; add register e2e (too-short → inline error +
   disabled).
3. `ResetPasswordPage`: raw `new_password` `<input>` →
   `<ValidatedInput<Password>>` + disable-until-valid; add reset e2e (too-short
   → inline error + disabled).

**Key risks / decisions.**

- The generated redacting `Debug` is
  `concat!(stringify!(Password), "([redacted])")` = byte-identical
  `Password([redacted])`; the existing `debug_does_not_expose_value` test is the
  regression guard — it must pass unchanged.
- Dropping `as_str()` touches `storage/src/helpers.rs` (two test-coverage hooks)
  → a `storage` rebuild in the coverage gate (~2 min). Expected, not a failure.
- `<ValidatedInput<…>>` uses a generic component tag; **leptosfmt mangles
  these** (#420) — write it, then let `cargo xtask check` / leptosfmt format;
  don't hand-fight the formatting.
- The inline error is gated on `touched` (set on blur); the **button** disables
  on input immediately. So each e2e must `blur()` the field to assert the
  _message_, but can assert the _disabled button_ right after typing.

**For agentic workers.** Execute with **`jaunder-iterate`** (per-task implement
→ check → commit → review), delegating a task to **`jaunder-dispatch`** if
useful. Tick the checkboxes below in real time. Run the gate via
**`jaunder-commit`** before each commit.

## Global constraints

- Rust edition/workspace conventions per `CONTRIBUTING.md`. No `Co-Authored-By`
  trailer.
- `Password` must **not** gain `#[derive(Debug)]`, `Display`, serde, `Deref`,
  `Borrow`, `From<Self> for String`, or `PartialEq` — the secret variant
  deliberately omits them (enforced by the macros crate's `compile_fail`
  doctests).
- Form field `name=` strings stay verbatim (`"password"`, `"new_password"`) —
  they bind the wire arg and the e2e selectors.
- Per-task check ladder: targeted `cargo nextest run -p <crate>` →
  `cargo xtask check` (full fmt+clippy+coverage) clean before commit. e2e
  verified via `cargo xtask e2e sqlite chromium` (one combo) locally; the full
  `{sqlite,postgres}×{chromium,firefox}` matrix runs in `validate`/CI.

---

## Task 1 — Adopt `#[str_newtype(secret)]` on `Password`

**Files**

- `common/src/password.rs` — the derive adoption.
- `storage/src/helpers.rs` — migrate two `.as_str()` test-hook callers to
  `.as_ref()`.

**Interface change.** `Password`'s borrowed-read path moves from the inherent
`as_str()` to the derived `AsRef<str>`.
`FromStr`/`hash`/`verify`/`PasswordError` are untouched. New surface additions
from the derive: `AsRef<str>`, `TryFrom<String>` (routes through `FromStr`),
redacting `Debug`.

**Test (regression-first).** The contract test already exists —
`debug_does_not_expose_value` (`common/src/password.rs`) asserts
`format!("{p:?}") == "Password([redacted])"`. It passes today (hand-written
`Debug`) and must still pass after (generated `Debug`) — that _is_ the red/green
guard for the swap. Run it before and after.

**Steps.**

1. Edit the module head of `common/src/password.rs`:

   ```rust
   use std::str::FromStr;

   use macros::StrNewtype;
   use thiserror::Error;

   const MIN_LENGTH: usize = 8;
   ```

   (Drop `fmt` from the `use std::{...}`; add `use macros::StrNewtype;`,
   mirroring `common/src/username.rs:3`.)

2. Change the struct attributes (keep the doc comment; it already documents the
   no-`Display` intent):

   ```rust
   #[derive(Clone, StrNewtype)]
   #[str_newtype(secret)]
   pub struct Password(String);
   ```

3. **Delete** the hand-written `Debug` impl (the whole
   `impl fmt::Debug for Password { … }` block).

4. **Delete** the inherent `as_str()` method (the
   `#[must_use] pub fn as_str(&self) -> &str { &self.0 }` block).
   `hash()`/`verify()` are unaffected — they already read `self.0` directly.

5. Migrate the callers of `Password::as_str()` to `AsRef<str>`:
   - `common/src/password.rs` tests: `p.as_str()` → `p.as_ref()` at the two
     sites; rename the test `as_str_returns_original_value` →
     `as_ref_returns_original_value`. The
     `production_params_roundtrip_regardless_of_feature` test's
     `p.as_str().as_bytes()` → `p.as_ref().as_bytes()`.
   - `storage/src/helpers.rs:350` and `:400`:
     `password.as_str() == "force-…-for-test-coverage"` →
     `password.as_ref() == "force-…-for-test-coverage"`. The `== "…"` `&str`
     comparison disambiguates the `AsRef` target. _Fallback if inference
     complains:_ `AsRef::<str>::as_ref(&password)`.

**Run.**

```
cargo nextest run -p common password
```

Expected: **PASS** (all `Password` unit tests, incl. the redaction contract,
`as_ref` roundtrip, hash/verify).

```
cargo nextest run -p storage helpers
```

Expected: **PASS** (the `force-hash-error` / `force-verify-error` coverage hooks
still trip via `as_ref`).

```
cargo xtask check
```

Expected: **PASS** clean (fmt + clippy + Nix coverage — includes the `storage`
rebuild).

**Commit** (via `jaunder-commit`):
`types(common): adopt StrNewtype secret on Password; drop hand-written Debug + as_str (#410)`

---

## Task 2 — `RegisterPage`: client-validated password field

**Files**

- `web/src/pages/auth.rs` — `RegisterPage` component (`Password`, `Field`,
  `ValidatedInput` already imported).
- `end2end/tests/auth.spec.ts` — new too-short assertion.

**Test (TDD, e2e-first).** Add to `end2end/tests/auth.spec.ts` (mirrors the
existing register tests; `SEL.password` = `input[name="password"]`,
`SEL.submit`, `SEL.error`):

```ts
test("register rejects a too-short password client-side", async ({ page }) => {
  await goto(page, "/register");
  await page.fill(SEL.username, "validusername");
  await page.fill(SEL.password, "short"); // < 8 chars
  await page.locator(SEL.password).blur(); // touched → message shows
  await expect(page.locator(SEL.error)).toBeVisible();
  await expect(page.locator(SEL.submit)).toBeDisabled();
  // A valid password clears the error and enables submit.
  await page.fill(SEL.password, "longenough123");
  await expect(page.locator(SEL.submit)).toBeEnabled();
});
```

Run against the current (raw-input) page → **FAIL** (no inline error, button not
gated on password):

```
cargo xtask e2e sqlite chromium
```

(Or the targeted playwright invocation the suite documents; the failing test
proves the gap.)

**Implement.** In `RegisterPage` (`web/src/pages/auth.rs`):

1. Add the field alongside `let username = Field::<Username>::new();`:

   ```rust
   let password = Field::<Password>::new();
   ```

2. Replace the raw password `<label>…<input …/></label>` block with the
   validated component:

   ```rust
   <ValidatedInput<Password>
       label="Password"
       name="password"
       input_type="password"
       autocomplete="new-password"
       field=password
   />
   ```

   (leptosfmt will re-split the `<ValidatedInput<Password>` tag as it does for
   the sibling `<ValidatedInput<Username>` — don't pre-format it.)

3. Gate the submit button on both fields:
   ```rust
   prop:disabled=move || !(username.is_valid() && password.is_valid())
   ```

**Run.**

```
cargo xtask check
```

Expected: **PASS** (web compiles host + wasm; clippy/fmt clean).

```
cargo xtask e2e sqlite chromium
```

Expected: **PASS** — the new test plus the existing
`register with open policy succeeds` (which fills a valid `newpassword123`, so
the button enables) stay green.

**Commit** (via `jaunder-commit`):
`web(auth): client-validate the register password via ValidatedInput<Password> (#410)`

---

## Task 3 — `ResetPasswordPage`: client-validated new-password field

**Files**

- `web/src/pages/password_reset.rs` — `ResetPasswordPage` component (needs the
  `Password` import added).
- `end2end/tests/password_reset.spec.ts` — new too-short assertion.

**Test (TDD, e2e-first).** Add to `end2end/tests/password_reset.spec.ts` (the
field is `input[name="new_password"]`, no `SEL` constant — inline it; submit is
`SEL.submit`, error `SEL.error`):

```ts
test("reset-password rejects a too-short password client-side", async ({
  page,
}) => {
  await goto(page, "/reset-password?token=any_token"); // token value irrelevant; never submitted
  const pw = page.locator('input[name="new_password"]');
  await pw.fill("short"); // < 8 chars
  await pw.blur(); // touched → message shows
  await expect(page.locator(SEL.error)).toBeVisible();
  await expect(page.locator(SEL.submit)).toBeDisabled();
  await pw.fill("longenough123");
  await expect(page.locator(SEL.submit)).toBeEnabled();
});
```

(Import `SEL` in the spec if not already imported.) Run against the current
raw-input page → **FAIL** (button never disabled, no inline error).

**Implement.** In `web/src/pages/password_reset.rs`:

1. Add the import (the module currently imports only `Username`;
   `Field`/`ValidatedInput` are already imported at line 2):

   ```rust
   use common::password::Password;
   ```

2. In `ResetPasswordPage`, add after
   `let confirm_action = ServerAction::<ConfirmPasswordReset>::new();`:

   ```rust
   let new_password = Field::<Password>::new();
   ```

3. Replace the raw new-password label/input
   (`<label>"New password" <input type="password" name="new_password" /></label>`)
   with:

   ```rust
   <ValidatedInput<Password>
       label="New password"
       name="new_password"
       input_type="password"
       autocomplete="new-password"
       field=new_password
   />
   ```

   (Keep the hidden `token` input directly above it untouched.)

4. Gate the submit button:
   ```rust
   prop:disabled=move || !new_password.is_valid()
   ```

**Run.**

```
cargo xtask check
```

Expected: **PASS**.

```
cargo xtask e2e sqlite chromium
```

Expected: **PASS** — the new test plus the existing
`password reset flow completes successfully` and
`visiting reset-password with invalid token shows error` (both fill valid
≥8-char passwords → button enables) stay green.

**Commit** (via `jaunder-commit`):
`web(password-reset): client-validate the new-password field via ValidatedInput<Password> (#410)`

---

## Final verification (before ship)

```
cargo xtask validate
```

Expected: **PASS** — full gate incl. the four e2e combos. This is the "green →
may ship" signal handed to `jaunder-ship`.

## Self-review checklist

- [ ] `Password` derives `StrNewtype` + `#[str_newtype(secret)]`; no
      `#[derive(Debug)]`; hand-written `Debug` and `as_str()` gone;
      `FromStr`/`hash`/`verify`/`PasswordError` unchanged.
- [ ] `format!("{p:?}") == "Password([redacted])"` still holds (existing test
      green).
- [ ] No `Password::as_str` caller remains; all reads via `.as_ref()`.
- [ ] Register + reset forms use `<ValidatedInput<Password>>` with
      disable-until-valid; login unchanged; `name=` strings verbatim.
- [ ] Two new e2e tests pass; all pre-existing auth/reset e2e still pass.
- [ ] No `#[server]`/storage signature changed; no serde on `Password`.
- [ ] `cargo xtask validate` clean.
