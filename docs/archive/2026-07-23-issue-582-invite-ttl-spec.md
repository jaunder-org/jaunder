# Spec — #582: bounded `InviteTtlHours` newtype for the invite TTL

Issue: [#582](https://github.com/jaunder-org/jaunder/issues/582) — milestone #13
(Domain-value type safety). Family: #464 (numeric newtype macro), #400
(`InviteCode`), #439 (invite form client validation), ADR-0063/0065.

## Problem

`create_invite(expires_in_hours: Option<u64>, recipient_email: Email)`
(`web/src/invites/api.rs:40`) carries the invite TTL as a bare `u64`. The
default (168) and the overflow bound
(`i64::try_from(hours).ok().and_then(Duration::try_hours)`, erroring
`"expires_in_hours too large"`) are enforced in-body — while the sibling arg is
a typed `Email`. The same bare-`u64` TTL + its own in-body bound recur in the
CLI (`cmd_user_invite`, `server/src/commands.rs:293`; `--expires-in`,
`server/src/cli.rs:191`).

## Decisions

1. **`InviteTtlHours` is a bounded `NumNewtype`** in `common::invite` (beside
   the invite-code types), **inner `i64`**, `min = 1`, `max = 336` (14 days),
   `default = 168` (7 days). The bound moves from two in-body checks to the type
   (parse/serde/clap all reject out-of-range).
2. **Inner `i64`, not `u64`.** `chrono::Duration::hours` takes `i64`, so an
   `i64` inner constructs the duration with no cast; the `max = 336` bound keeps
   `Duration::hours` far from overflow. It also cleanly rejects the existing
   `u64::MAX` test input (doesn't fit `i64`) and negatives (`min = 1`) at
   decode. The serialized wire form of a positive integer is unchanged.
3. **Bounds tighten deliberately.** Previously `expires_in_hours = 0` produced
   an immediately-expired invite and values up to ~`i64::MAX` hours were
   accepted; now `< 1` and `> 336` are rejected at the boundary. The 168 default
   is within `[1, 336]`.
4. **Keep the `<ActionForm>` — no dispatch conversion.** The TTL is
   optional-with-default and already lives as `Option<u64>` in that same
   `ActionForm` today (empty → `None` → default 168 is the current behavior), so
   typing it `Option<InviteTtlHours>` and rendering it as a
   `ValidatedInput<InviteTtlHours>` (`Field::optional`) keeps the ActionForm.
   (#581 converted its form to dispatch, but that rested on an "empty →
   `Some("")` → reject" premise the gate later falsified; an ActionForm carries
   an optional typed arg fine, empty decoding to `None`.) The omit→default path
   is verified by an integration test (the arbiter, per #581's lesson).
5. **CLI is in scope** (web + CLI). One coherent "adopt the TTL newtype" goal;
   typing the CLI arg removes the duplicated in-body bound and makes clap
   enforce it via `FromStr`.
6. **Storage is unaffected.**
   `InviteStorage::create_invite(expires_at: DateTime<Utc>)` takes the
   already-computed instant; the newtype lives only at the web/CLI construction
   layer.

## Design

### `common::invite` — the newtype

```rust
/// Hours until an invite code expires — bounded `1..=336` (14 days), default 168 (7 days).
/// The bound that `create_invite` used to enforce in-body now lives in the type. `i64` inner
/// so it feeds `chrono::Duration::hours` directly.
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(inner = i64, min = 1, max = 336, default = 168,
    error = "invite expiry must be between 1 and 336 hours")]
pub struct InviteTtlHours(i64);
```

Trailer: `value()` + `From<Self> for i64`, `TryFrom<i64>`, `FromStr`/serde
(transparent integer; rejects `< 1`, `> 336`, non-integer, and — for
serde/`FromStr` of `u64::MAX` — a value that doesn't fit `i64`), `Display`,
`Default` (168), and `InvalidInviteTtlHours`.

### `common::test_support`

- `parse_invite_ttl_hours(s: &str) -> InviteTtlHours` beside the other `parse_*`
  helpers.

### `web/src/invites/api.rs`

- `create_invite(expires_in_hours: Option<InviteTtlHours>, recipient_email: Email)`.
- Replace the
  `let hours = …unwrap_or(168); let duration = i64::try_from(hours)…try_hours… ok_or_else(…)?;`
  overflow block with a bound-free
  `let hours = expires_in_hours.unwrap_or_default().value();` (an `i64`), then
  `let expires_at = Utc::now() + chrono::Duration::hours(hours);`. **Keep the
  `hours` binding** — the email body later interpolates it (`api.rs:76`,
  "…expires in {hours} hours."); only the `try_from`/`try_hours`/`ok_or_else`
  overflow logic is deleted.

### `web/src/invites/component.rs`

- The plain `<input type="number" name="expires_in_hours" />` →
  `<ValidatedInput<InviteTtlHours> input_type="number" name="expires_in_hours" field=ttl … />`,
  where `ttl = Field::<InviteTtlHours>::optional()` (empty ⇒ valid ⇒ server
  default 168; a non-empty out-of-range value shows the newtype's message and
  gates submit). Keep the `<ActionForm>`.
- Submit `prop:disabled` becomes
  `move || !recipient.is_valid() || !ttl.is_valid()`.

### `server/src/cli.rs` + `server/src/commands.rs`

- `cli.rs:191`: `expires_in: Option<u64>` → `Option<InviteTtlHours>` (clap
  parses via the newtype's `FromStr`, applying the bound). Doc comment unchanged
  (defaults to 168).
- `commands.rs`: `cmd_user_invite(storage, expires_in: Option<InviteTtlHours>)`;
  delete the `i64::try_from(hours_u64)…"too large"` block;
  `let expires_at = Utc::now() + chrono::Duration::hours(expires_in.unwrap_or_default().value());`.

### Tests

- **`common::invite` unit test** for `InviteTtlHours` (mirrors the
  bounded-`NumNewtype` shape): `value()`/`From<Self>`, `FromStr` accepts
  `1`/`336`/`168` and rejects `0`/`337`/`-1`/`abc`/ the `u64::MAX` string with
  the domain message, `Default` = 168, serde round-trip + wire rejection of
  `0`/`337`, and the generated `TryFrom<i64>`. `parse_invite_ttl_hours` fixture.
- **`server/tests/web/web_account.rs`**:
  `create_invite_large_hours_returns_error` (posts `u64::MAX`) — change
  `assert_eq!(status, INTERNAL_SERVER_ERROR)` to
  `assert_ne!(status, StatusCode::OK)` (the rejection now happens at typed-arg
  decode, the framework's status, before the handler runs — the no-email
  assertion still holds). **Add two default-path tests** (both must decode
  `Option<InviteTtlHours>` to `None` → default 168 → OK
  - invite created), because the browser's `Field::optional` submits an
    _empty-present_ field, a different decode path from omission — and this is a
    numeric `Option`, where #581 only proved the _string_ case:
  * `create_invite_omits_hours_uses_default`: POST `recipient_email=…` with
    **no** `expires_in_hours` key.
  * `create_invite_empty_hours_uses_default`: POST
    `expires_in_hours=&recipient_email=…` (empty-present, what the form actually
    submits) — mirrors #581's `clears_via_empty_destination`. **If this decodes
    to a rejection instead of `None`, decision 4 (keep the ActionForm) is
    falsified and the form must convert to `.dispatch` (dispatching
    `ttl.parsed()`) — the TDD arbiter.**
- **`server/tests/misc/commands.rs`**: `cmd_user_invite_default_expires_in`
  (passes `None`) unchanged.
  `cmd_user_invite_too_large_expires_in_returns_error` can no longer pass
  `u64::MAX` to a typed arg — convert it to
  `cmd_user_invite_with_explicit_hours` passing
  `Some(parse_invite_ttl_hours("48"))` and asserting success (covers the `Some`
  arm); the out-of-range rejection is covered by the newtype unit test's
  `FromStr`/serde cases.
- **`server/src/cli.rs` unit tests** (~lines 600-616):
  `user_invite_parses_expires_in` asserts `Some(48)` — once the arg is
  `Option<InviteTtlHours>` that literal stops type-checking; change to
  `Some(parse_invite_ttl_hours("48"))`. (`user_invite_expires_in_optional`'s
  `None` and `main.rs`'s `expires_in: None` are unaffected.)
- **e2e `invite.spec.ts`** fills `expires_in_hours` with `"168"` (in range) —
  unchanged and stays green; the field is now client-validated but 168 is valid.

## Acceptance criteria

1. `InviteTtlHours` exists in `common::invite` with the ADR-0063 numeric
   trailer, `1..=336`, default 168; `"168"` parses,
   `"0"`/`"337"`/`"-1"`/`"abc"`/`"18446744073709551615"` reject with
   `"invite expiry must be between 1 and 336 hours"`; serde is a bare integer
   round-trip rejecting out-of-range on deserialize;
   `InviteTtlHours::default().value() == 168`.
2. `create_invite`'s wire arg is `Option<InviteTtlHours>` and the CLI
   `cmd_user_invite`'s arg is `Option<InviteTtlHours>`; both in-body
   bound/overflow checks are deleted
   (`git grep -n "try_hours\|expires_in_hours too large\|is too large" web/src server/src`
   returns nothing).
3. The invite form's TTL input is a `ValidatedInput<InviteTtlHours>`
   (client-validated, `Field::optional`); **both** an omitted TTL **and** an
   empty-present `expires_in_hours=` default to 168 (asserted by
   `create_invite_omits_hours_uses_default` **and**
   `create_invite_empty_hours_uses_default`); a `u64::MAX` TTL is rejected
   (non-OK) before any email. (Should empty-present reject instead of default,
   decision 4 converts the form to `.dispatch` — see Design/decision 4.)
4. `parse_invite_ttl_hours` exists in `common::test_support` and every
   `cfg(test)` TTL construction uses it (or `Default`).
5. `cargo xtask validate --no-e2e` clean (coverage includes the bound-rejection
   path).

## Verification

- Unit: the `InviteTtlHours` suite in `common::invite`.
- Host/integration: `create_invite_*` (web, dual-backend) and
  `cmd_user_invite_*` (CLI) with the typed arg; the omit→default and
  large-hours→reject cases.
- Browser: the existing `invite.spec.ts` (fills 168) stays green — verified via
  `cargo xtask e2e-local invite`; the field is now client-validated.
