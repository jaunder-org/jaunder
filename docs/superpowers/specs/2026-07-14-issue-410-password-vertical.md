# Spec — Issue #410: Password vertical (StrNewtype secret + thread `Password` through auth flows)

**Issue:** jaunder-org/jaunder#410 · **Milestone:** Domain-value type safety
(newtypes) · **Umbrella:** #404 **Blocked by:** #403 (StrNewtype/IdNewtype
derives — merged), #414 (client-side validation pattern + ADR-0065 — merged).
Both closed; unblocked.

## Problem

`common::password::Password` is the last of the #404 verticals. It carries ~40
lines of hand-written trailer surface (a redacting `Debug`, an inherent
`as_str()`) instead of the standard derive, and — although prior work (#407
username threading, #414's `LoginPage` worked example) already threaded
`Password` through the server/host layers — two web forms still submit passwords
through raw `<input>` elements with no client-side validation. This vertical
closes both gaps so `Password` fully conforms to ADR-0063 §2 (the **secret**
variant) and ADR-0065 (typed wire args + client pre-validation, with the secret
exception).

## Current state (already correct — do NOT change)

- **Host/storage layer already takes `&Password`** everywhere:
  `UserStorage::create_user`, `AtomicOps::create_user_with_invite`,
  `UserStorage::authenticate`, `AtomicOps::confirm_password_reset`,
  `UserStorage::set_password`. The hash/verify sinks (`storage/src/helpers.rs`
  `hash_password`/`verify_password`) take `Password` by value.
- **All three `#[server]` entry points already keep the wire arg as `String` and
  parse on the first body line**, threading `&Password` inward — the ADR-0065
  secret-exception shape:
  - `register` — `web/src/auth/mod.rs:69` `password: String` → `:81`
    `.parse::<Password>()?`
  - `login` — `web/src/auth/mod.rs:150` `password: String` → `:160`
    `.parse::<Password>()?`
  - `confirm_password_reset` — `web/src/password_reset/mod.rs:74`
    `new_password: String` → `:78` `.parse::<Password>()?`
- **`LoginPage` already uses `<ValidatedInput<Password>>`** with
  disable-until-valid (`web/src/pages/auth.rs:132-146`) — #414's worked example.

**Consequence:** no storage-crate signature changes and no `#[server]`-signature
changes are in scope. A secret has **no serde bridge** (by design), so the wire
arg **must stay `String`** and be parsed on entry — adding
`Serialize`/`Deserialize` to `Password` is explicitly _wrong_.

## Scope — the delta

### 1. Adopt the derive on `common/src/password.rs`

- Change `#[derive(Clone)]` → `#[derive(Clone, StrNewtype)]` and add
  `#[str_newtype(secret)]` on `struct Password(String)`.
- **Delete** the hand-written `impl fmt::Debug for Password`
  (`password.rs:102-106`) — the secret variant generates
  `f.write_str(concat!(stringify!(Password), "([redacted]))"))`, byte-identical
  to `Password([redacted])`. Do **not** add `#[derive(Debug)]`.
- Remove the now-unused `use std::fmt` (keep `std::str::FromStr`).
- **Drop the inherent `as_str()`** (`password.rs:41-43`) in favour of the
  generated `AsRef<str>` (secret surface = `AsRef<str>` only, ADR-0063 §2).
  Migrate the only callers:
  - `common/src/password.rs` tests (`:138`, `:160`) → `p.as_ref()`.
  - `storage/src/helpers.rs:350`, `:400` (`force-hash-error` /
    `force-verify-error` test hooks) → `password.as_ref()` (disambiguated by the
    `== "…"` `&str` comparison).
- Keep hand-written `FromStr`, `hash()`, `verify()`, `PasswordError` unchanged.
- The derive also adds `TryFrom<String>` (routes through `FromStr`); unused for
  now but part of the standard secret surface.

### 2. Adopt the #414 client-validation component on the two remaining forms

Mirror `LoginPage` exactly (already-shipped reference).

- **`RegisterPage`** (`web/src/pages/auth.rs`): imports already present. Add
  `let password = Field::<Password>::new();`. Replace the raw password `<input>`
  (`:51-59`) with
  `<ValidatedInput<Password> label="Password" name="password" input_type="password" autocomplete="new-password" field=password />`.
  Gate the submit button (`:78`) on
  `!(username.is_valid() && password.is_valid())`.
- **`ResetPasswordPage`** (`web/src/pages/password_reset.rs`): add
  `use common::password::Password;` and
  `let new_password = Field::<Password>::new();`. Replace the raw
  `<label>"New password" <input name="new_password"/></label>` (`:82`) with
  `<ValidatedInput<Password> label="New password" name="new_password" input_type="password" autocomplete="new-password" field=new_password />`.
  Gate the submit button (`:83`) on `!new_password.is_valid()`. (Keep the hidden
  `token` input untouched.)

The `name="password"` / `name="new_password"` strings must stay verbatim — they
bind the form field to the `#[server]` arg and to the e2e selectors.

### 3. e2e — exercise the newly-adopted component

Per ADR-0065, the component's rendering/interaction is proven via e2e. Add one
focused assertion per migrated form:

- **Register** (`end2end/tests/auth.spec.ts`): typing a <8-char password shows
  the inline `.error` ("at least 8 characters") and the Register button is
  `disabled`; a valid password clears the error and enables submit.
- **Reset** (`end2end/tests/password_reset.spec.ts`): same on the `new_password`
  field of `/reset-password` — too-short → inline error + disabled "Set new
  password"; valid → enabled.

Existing flow e2e (which fill valid ≥8-char passwords) must continue to pass
unchanged, since disable-until-valid enables the button once a valid value is
entered.

## Out of scope

- `common/src/auth.rs:24` HTTP-Basic parsing — the `String` there is an
  app-password/session token verified via `storage::auth::hash_token`, not
  `Password::verify`. Different vertical.
- `#17` (hashed-password / `TokenHash` newtype) — the _stored_ value is out of
  this vertical.
- Any storage/`#[server]` signature change; any serde bridge on `Password`.

## Design decision considered and rejected — the InviteCode two-type split

We considered mirroring #400's `ProfferedInviteCode` (inbound, `common`, serde)
/ `InviteCode` (host-only, serde-free domain) split for `Password`, motivated by
"never let the stored value be exfiltrated." Rejected, because the analogy
conflates two different values:

- For an invite code the **inbound** secret and the **at-rest** secret are the
  _same_ string, so one Proffered/host-only split governs one value's direction.
- For a password the inbound value (**plaintext `Password`**) and the at-rest
  value (**the Argon2 hash**) are _different_. The plaintext is never persisted
  and never returned; the hash is the only stored/returnable thing.

Two consequences pin the decision:

1. **`Password` has no serde by design**, so it is _already_ structurally
   impossible to place in a `#[server]` return payload (won't compile).
   InviteCode needed a host-only type + an xtask placement gate only _because_
   `ProfferedInviteCode` carries serde (to deserialize inbound), which serde
   alone can't confine to parameter position. `Password` gets that guarantee for
   free — a split would add machinery to enforce what the surface already
   guarantees, and would force the client-side `ValidatedInput<Password>` type
   to be re-plumbed (the client must name the plaintext type for FromStr
   pre-validation, so it cannot go host-only).
2. **The exfiltration concern is real but applies to the hash, not the
   plaintext** — and its proper vehicle is a `host`-only, serde-free hash
   newtype (`PasswordHash`), which is #17's declared territory ("session
   token/hash … `RawToken` vs `TokenHash`"). The stored hash is already
   un-exposed by convention today (`storage/src/users.rs:18`: "Does not expose
   `password_hash`"); #17 makes that a compile-time fact.

Decision: **#410 stays the plaintext `Password` secret vertical (single type, no
split); the stored-hash guarantee is left to #17.**

## Acceptance

- `Password` derives `StrNewtype` with `#[str_newtype(secret)]`; hand-written
  redacting `Debug` gone (generated); `FromStr`/`hash`/`verify`/`PasswordError`
  unchanged; `format!("{p:?}") == "Password([redacted])"` preserved.
- `as_str()` removed; all reads go through `AsRef<str>`.
- `Password` cannot `Display`, (de)serialize, `Deref`-coerce, extract an owned
  `String`, or value-compare (guaranteed by the secret derive's `compile_fail`
  doctests in `macros`).
- Register and reset-password forms validate the password client-side via
  `<ValidatedInput<Password>>` with disable-until-valid; login's existing
  behaviour unchanged.
- No bare `String`/`&str` plaintext-password param remains **past** the
  `#[server]` boundary (already true; re-verified). The three wire args stay
  `String` by the secret exception.
- `cargo xtask validate` clean (the two new e2e assertions run in the e2e
  matrix).

## Risks / notes

- Removing `as_str()` touches the `storage` crate → a storage rebuild in the
  coverage gate (~2 min). Expected, not a failure.
- `ResetPasswordPage`'s raw input currently lacks the `j-form-*` classes;
  `ValidatedInput` adds them — a minor, intended visual normalization consistent
  with the other forms.
- No ADR needed — ADR-0063 (§2 secret) and ADR-0065 (secret exception) already
  govern this; this vertical is their first `Password` adoption, not a new
  decision.
