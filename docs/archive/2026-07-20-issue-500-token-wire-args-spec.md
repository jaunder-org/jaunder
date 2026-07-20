# Spec — type the reset/verification `token` wire args as `RawToken` (issue #500)

Status: proposed · Issue: jaunder-org/jaunder#500 · Family: newtypes (milestone
13), ADR-0063/0065. Concrete slice of the #91 boundary audit.

## Context — most of #500 is already shipped

The `ProfferedPassword` inbound-secret twin already exists
(`common/src/password.rs:63-86`: `#[str_newtype(secret, serde)]`, shared
`validate_password_shape`, `TryFrom<ProfferedPassword> for Password`), is
registered in the `proffered-secret` xtask gate
(`xtask/src/steps/proffered_secret_check.rs` `POLICED_TYPES`), and is already
the wire type for `confirm_password_reset`'s `new_password` and `register`'s
`password`. **No twin/gate/password work remains.**

The **only** outstanding #500 item: `confirm_password_reset`
(`web/src/password_reset/mod.rs:78`) and `verify_email`
(`web/src/email/mod.rs:60`) still take `token: String` and re-parse it in-body
via `RawToken::try_from(token)` — the last raw-`String`-then-parse secret/token
args, so #500's acceptance is not yet met.

## Solution

Type both `token` args as `RawToken` (which carries the full `StrNewtype` serde
trailer, so it is directly wire-capable — its only omission is the sqlx bridge).
The in-body `RawToken::try_from(token).map_err(...)` re-parse is deleted; the
typed `token` is used directly. The wire form is unchanged: the client still
submits a bare string (hidden input / query param), which the generated
`Deserialize` routes through `RawToken::from_str` on decode — moving
shape-rejection from the fn body to the wire boundary (ADR-0065 posture), with
the same observable non-OK outcome for a bad token.

## Acceptance criteria

- **AC1 — `confirm_password_reset`.** Its `token` arg is `RawToken`; the body no
  longer contains `RawToken::try_from(token)` (uses the typed `token` directly).
  `new_password` stays `ProfferedPassword` (already done).
- **AC2 — `verify_email`.** Its `token` arg is `RawToken`; the body no longer
  contains `RawToken::try_from(token)`.
- **AC3 — No raw-String secrets remain** in either fn (closes the issue's
  acceptance). `auth::register`/`login` password args stay `String` (documented
  intentional carve-out — unchanged).
- **AC4 — Behavior preserved + wire-rejection covered.** Existing wire tests
  (`web_password_reset.rs`, `web_email.rs`) still pass unchanged (their
  invalid-token bodies are valid base64url _shape_, so they still fail at DB
  lookup → non-OK). Add one test asserting a **shape-invalid** token (e.g.
  containing `!`, outside base64url) is rejected → non-OK, exercising the new
  `RawToken` wire-decode path (assert status, not message text, per ADR-0065).
- **AC5 — Gate.** `cargo xtask validate --no-e2e` clean (incl. the
  `proffered-secret` gate, already covering `ProfferedPassword`).

## Out of scope

- The `ProfferedPassword` twin, its gate registration, and the
  `new_password`/`register` password typing — all already shipped. Storage-layer
  `.confirm_password_reset`/ `.use_email_verification` signatures (already take
  `&RawToken`). No new newtype.

## Verification

`cargo xtask validate --no-e2e` (AC5). The client reset/verify flows still
submit a bare token string that decodes to `RawToken`.
