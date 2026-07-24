# `InviteTtlHours` newtype (#582) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

Spec:
[`docs/superpowers/specs/2026-07-23-issue-582-invite-ttl.md`](../specs/2026-07-23-issue-582-invite-ttl.md).
Issue: [#582](https://github.com/jaunder-org/jaunder/issues/582).

**Goal:** Move the invite-TTL bound (default 168, overflow) from two in-body
checks into a bounded `InviteTtlHours` newtype (`1..=336`, default 168), on the
web `create_invite` and the CLI `cmd_user_invite`.

**Architecture:** A bounded `NumNewtype` (inner `i64`) in `common::invite`. The
web wire arg and the CLI arg become `Option<InviteTtlHours>`; the form's plain
number input becomes a `ValidatedInput<InviteTtlHours>` inside the existing
`<ActionForm>`. Storage is unaffected.

**Tech Stack:** Rust, `macros::NumNewtype` (#464/ADR-0063), Leptos (ADR-0065
`Field`/ `ValidatedInput`), clap, `chrono`, dual-backend tests.

## Review header

**Scope (in):** `common::invite` (newtype), `common::test_support`
(`parse_invite_ttl_hours`), `web/src/invites/{api,component}.rs`,
`server/src/{cli,commands}.rs`, and the test sites
(`server/tests/web/web_account.rs`, `server/tests/misc/commands.rs`,
`server/src/cli.rs` unit tests).

**Scope (out):** storage `create_invite` (takes `DateTime`, unaffected); the 168
default value (unchanged); the `ProfferedInviteCode`/`InviteCode` types
(unrelated).

**Tasks:**

1. `InviteTtlHours` bounded `NumNewtype` in `common::invite` +
   `parse_invite_ttl_hours` + unit test.
2. Web `create_invite`: typed wire arg + `ValidatedInput` form +
   `web_account.rs` tests (incl. the empty-present TDD arbiter for
   keep-ActionForm-vs-dispatch).
3. CLI `cmd_user_invite`: typed arg + body + `cli.rs`/`commands.rs` tests.

**Key risks/decisions:**

- **Empty-present decode (Task 2)** — the keep-ActionForm design hinges on an
  empty `expires_in_hours=` decoding to `None`→default. #581 proved the _string_
  case; this is _numeric_. Task 2 writes
  `create_invite_empty_hours_uses_default` as the arbiter; if it rejects instead
  of defaulting, Task 2's contingency converts the form to `.dispatch`.
- **Bounds tighten**: `0` and `> 336` now rejected at the boundary (spec
  decision 3).

## Global Constraints

- **Bounded newtype**:
  `#[num_newtype(inner = i64, min = 1, max = 336, default = 168, error = "invite expiry must be between 1 and 336 hours")]`.
- **No `Co-Authored-By` trailer** on commits.
- **Test construction** via `common::test_support::parse_invite_ttl_hours` or
  `Default`.
- **Per-commit gate:** the pre-commit hook runs `cargo xtask check`; run it
  first (**jaunder-commit**). Web/form touches: also
  `cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`.
- **Storage/CLI tests are dual-backend** (`#[apply(backends)]`) — keep them so.

---

### Task 1: `InviteTtlHours` newtype + test helper

**Files:**

- Modify: `common/src/invite.rs` (add `InviteTtlHours` +
  `use macros::NumNewtype;`)
- Modify: `common/src/test_support.rs` (add `parse_invite_ttl_hours`)

**Interfaces:**

- Produces:
  - `common::invite::InviteTtlHours` — bounded `NumNewtype(i64)`, `1..=336`,
    default 168. Trailer: `value() -> i64`, `From<Self> for i64`,
    `TryFrom<i64>`, `FromStr`/serde (transparent integer, rejects
    out-of-range/non-integer), `Display`, `Default` (168),
    `InvalidInviteTtlHours`.
  - `common::test_support::parse_invite_ttl_hours(s: &str) -> InviteTtlHours`.

- [ ] **Step 1: Write the failing test** — append to `common/src/invite.rs`'s
      `#[cfg(test)] mod tests`:

```rust
#[test]
fn invite_ttl_hours_surface() {
    use super::InviteTtlHours;
    // value()/From<Self>, trim, in-range parse.
    assert_eq!("168".parse::<InviteTtlHours>().map(i64::from).ok(), Some(168));
    assert_eq!("  1  ".parse::<InviteTtlHours>().map(InviteTtlHours::value).ok(), Some(1));
    assert_eq!("336".parse::<InviteTtlHours>().map(InviteTtlHours::value).ok(), Some(336));
    // FromStr rejects out-of-range / non-integer / u64::MAX (doesn't fit i64)...
    for bad in ["0", "337", "-1", "abc", "1.5", "18446744073709551615"] {
        assert!(bad.parse::<InviteTtlHours>().is_err(), "{bad} should reject");
    }
    // ...with the domain message.
    assert!("0".parse::<InviteTtlHours>().err()
        .is_some_and(|e| e.to_string().starts_with("invite expiry")));
    // Default is 168 and Display round-trips.
    let d = InviteTtlHours::default();
    assert_eq!(d.value(), 168);
    assert_eq!(d.to_string().parse::<InviteTtlHours>().ok(), Some(d));
    // serde: bare integer, round-trip, wire-rejection of out-of-range.
    assert_eq!(serde_json::to_string(&d).ok(), Some("168".to_owned()));
    assert_eq!(serde_json::from_str::<InviteTtlHours>("24").map(i64::from).ok(), Some(24));
    assert!(serde_json::from_str::<InviteTtlHours>("0").is_err());
    assert!(serde_json::from_str::<InviteTtlHours>("337").is_err());
    // The generated TryFrom<i64>.
    assert_eq!(InviteTtlHours::try_from(48_i64).map(i64::from), Ok(48));
    assert!(InviteTtlHours::try_from(0_i64).is_err());
    // The shared fixture.
    assert_eq!(crate::test_support::parse_invite_ttl_hours("48").value(), 48);
}
```

- [ ] **Step 2: Run, verify FAIL**

Run: `cargo nextest run -p common invite::tests::invite_ttl_hours` Expected:
FAIL — `InviteTtlHours` / `parse_invite_ttl_hours` not defined.

- [ ] **Step 3: Implement**

In `common/src/invite.rs` add `use macros::NumNewtype;` (beside the `StrNewtype`
import) and the `InviteTtlHours` struct from the spec's "the newtype" section
(doc comment + the `#[num_newtype(...)]` line). In `common/src/test_support.rs`,
add `use crate::invite::InviteTtlHours;` (a new import line) and:

```rust
/// Parse `s` into an [`InviteTtlHours`] for tests — the single place a test invite-TTL literal
/// is parsed, so a malformed fixture fails loudly.
///
/// # Panics
///
/// Panics if `s` is not an integer in `1..=336`.
#[must_use]
pub fn parse_invite_ttl_hours(s: &str) -> InviteTtlHours {
    s.parse().expect("valid test invite TTL")
}
```

- [ ] **Step 4: Run, verify PASS**

Run: `cargo nextest run -p common invite::tests::invite_ttl_hours` — Expected:
PASS.

- [ ] **Step 5: Commit**

```bash
git add common/src/invite.rs common/src/test_support.rs
git commit -m "feat(common): bounded InviteTtlHours NumNewtype in common::invite (#582)"
```

Run `cargo xtask check` first (**jaunder-commit**).

---

### Task 2: Web `create_invite` — typed wire arg + form + tests

TDD: write the boundary tests first (they encode the omit/empty/reject
contract), then type the arg + form. The empty-present test is the arbiter for
keep-ActionForm vs dispatch.

**Files:**

- Modify: `web/src/invites/api.rs` (`:40` arg, `:53-58` body)
- Modify: `web/src/invites/component.rs` (`:20` field decl, `:49-52` input,
  `:56` submit disable)
- Modify: `server/tests/web/web_account.rs` (`:418` large-hours + add two
  default-path tests)

**Interfaces:**

- Consumes: `common::invite::InviteTtlHours` (Task 1).
- Produces:
  `web::invites::create_invite(expires_in_hours: Option<InviteTtlHours>, recipient_email: Email)`.

- [ ] **Step 1: Write/adjust the integration tests**
      (`server/tests/web/web_account.rs`):
  - `create_invite_large_hours_returns_error` (~:437): change
    `assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR)` to
    `assert_ne!(status, StatusCode::OK)` (keep the no-email assertion below it).
  - Add two tests mirroring `create_invite_emails_link_and_appears_in_list`
    (same operator-cookie
    - base-url + `CapturingMailSender` setup), differing only in the POST body,
      both asserting `status == OK` and the invite is created (list non-empty /
      mailer sent):
    * `create_invite_omits_hours_uses_default`: body
      `"recipient_email=invitee@example.com"` (no `expires_in_hours`).
    * `create_invite_empty_hours_uses_default`: body
      `"expires_in_hours=&recipient_email=invitee@example.com"`.

- [ ] **Step 2: Run the new tests, verify they FAIL as expected**

Run: `cargo nextest run -p jaunder --test integration create_invite` Expected:
compile FAIL (arg still `Option<u64>` — the assertions/type don't yet reflect
the change) or the large-hours test still 500. This is red.

- [ ] **Step 3: Type the wire arg + body** (`web/src/invites/api.rs`)
  - Import `use common::invite::InviteTtlHours;`.
  - `:40`: `expires_in_hours: Option<u64>` → `Option<InviteTtlHours>`.
  - `:53-58`: replace the
    `let hours = …unwrap_or(168); let duration = i64::try_from(hours)…try_hours… ok_or_else(…)?;`
    block with:
    ```rust
    let hours = expires_in_hours.unwrap_or_default().value();
    let expires_at = Utc::now() + chrono::Duration::hours(hours);
    ```
    (keep `hours` — the email body at `:76` interpolates it).

- [ ] **Step 4: Convert the form input to `ValidatedInput`**
      (`web/src/invites/component.rs`)
  - Import `use common::invite::InviteTtlHours;`.
  - After `let recipient = Field::<Email>::new();` add
    `let ttl = Field::<InviteTtlHours>::optional();`.
  - Replace the
    `<label>"Expires in hours" <input type="number" name="expires_in_hours" /></label>`
    with:
    ```rust
    <ValidatedInput<
    InviteTtlHours,
    >
        label="Expires in hours"
        name="expires_in_hours"
        input_type="number"
        field=ttl
    />
    ```
  - Submit `prop:disabled`: `move || !recipient.is_valid()` →
    `move || !recipient.is_valid() || !ttl.is_valid()`.

- [ ] **Step 5: Run the gate**

Run: `cargo nextest run -p jaunder --test integration create_invite` Expected:
PASS — including **`create_invite_empty_hours_uses_default`**. **If that test
fails (empty-present rejects instead of defaulting): CONTINGENCY** — convert the
form off `<ActionForm>` to the `.dispatch` pattern (a bespoke `<div>` +
`type="button"` submit dispatching
`CreateInvite { expires_in_hours: ttl.parsed(), recipient_email: recipient.parsed().unwrap() }`,
mirroring `site.rs::site_settings_form`), and re-run. `ttl.parsed()` yields
`None` for empty → default. Run:
`cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings` — Expected:
clean. Run: `cargo xtask check` — Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add web/src/invites/api.rs web/src/invites/component.rs server/tests/web/web_account.rs
git commit -m "refactor(web): type invite TTL as InviteTtlHours on create_invite (#582)"
```

---

### Task 3: CLI `cmd_user_invite` — typed arg + tests

**Files:**

- Modify: `server/src/cli.rs` (`:191` arg + unit tests ~`:600-616`)
- Modify: `server/src/commands.rs` (`:293` fn sig, `:298-301` body, + own
  `cfg(test)` sites `:961`/`:995`)
- Modify: `server/tests/misc/commands.rs` (`:336`/`:359` tests)

**Interfaces:**

- Consumes: `common::invite::InviteTtlHours`,
  `common::test_support::parse_invite_ttl_hours` (Task 1).
- Produces:
  `cmd_user_invite(storage: &StorageArgs, expires_in: Option<InviteTtlHours>)`.

- [ ] **Step 1: Type the CLI arg + body**
  - `server/src/cli.rs:191`: `expires_in: Option<u64>` →
    `Option<InviteTtlHours>` (import `common::invite::InviteTtlHours`; clap
    parses via its `FromStr`).
  - `server/src/commands.rs:293`:
    `cmd_user_invite(storage: &StorageArgs, expires_in: Option<InviteTtlHours>)`.
  - `:298-301`: replace the
    `let hours_u64 = …unwrap_or(168); let hours = i64::try_from(hours_u64)…"too large"?;`
    block with
    `let expires_at = Utc::now() + chrono::Duration::hours(expires_in.unwrap_or_default().value());`.

- [ ] **Step 2: Update the tests** — sweep **every** `cfg(test)`
      `cmd_user_invite(…, Some(<int>))` and CLI-arg literal (the macro has no
      `From<i64>`, so a bare integer no longer coerces):
  - `server/tests/misc/commands.rs`: `cmd_user_invite_default_expires_in`
    (passes `None`) unchanged. `cmd_user_invite_creates_retrievable_invite`
    (`:336`, `Some(48)`) → `Some(parse_invite_ttl_hours("48"))`.
    `cmd_user_invite_too_large_expires_in_returns_error` (`:359`) can no longer
    pass `u64::MAX` to a typed arg → replace with
    `cmd_user_invite_with_explicit_hours` passing
    `Some(parse_invite_ttl_hours("48"))` and asserting success (covers the
    `Some` arm; the too-large rejection is Task 1's newtype test). Import
    `parse_invite_ttl_hours`.
  - `server/src/commands.rs` **own `#[cfg(test)] mod`**:
    `cmd_user_invite_creates_invite_expiring_in_the_future` (`:961`, `Some(24)`)
    and `cmd_user_invite_with_base_url_configured_prints_link` (`:995`,
    `Some(24)`) → `Some(parse_invite_ttl_hours("24"))`. Add
    `use common::test_support::parse_invite_ttl_hours;` to that test module.
  - `server/src/cli.rs` unit tests (~`:600-616`):
    `user_invite_parses_expires_in`'s `assert_eq!(expires_in, Some(48))`
    (`:606`) → `assert_eq!(expires_in, Some(parse_invite_ttl_hours("48")))`
    (import `parse_invite_ttl_hours` in that test module).
    `user_invite_expires_in_optional` (`None`) unchanged.

- [ ] **Step 3: Run the gate**

Run: `cargo nextest run -p jaunder --test integration cmd_user_invite` and
`cargo nextest run -p jaunder --lib cli` (or `cargo xtask check`). Verify AC2:
`git grep -n "try_hours\|expires_in_hours too large\|is too large" web/src server/src`
returns nothing. Run: `cargo xtask check` (the per-task gate; its Nix coverage
step satisfies AC5's coverage — the exact `cargo xtask validate --no-e2e` the AC
cites runs at ship). Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add server/src/cli.rs server/src/commands.rs server/tests/misc/commands.rs
git commit -m "refactor(server): type CLI invite TTL as InviteTtlHours (#582)"
```
