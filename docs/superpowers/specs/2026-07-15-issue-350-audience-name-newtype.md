# Spec — #350: validated `AudienceName` newtype (typed wire arg, ADR-0065)

**Issue:** [#350](https://github.com/jaunder-org/jaunder/issues/350) — _common:
validated AudienceName newtype to de-duplicate the create/rename trim-and-empty
check_. Milestone: _Domain-value type safety (newtypes)_. Depends on #314
(closed).

## Problem

`create_audience` and `rename_audience` (`web/src/audiences/mod.rs`) each repeat
the same inline rule:

```rust
let name = name.trim();
if name.is_empty() {
    return Err(InternalError::validation("audience name must not be empty"));
}
```

The "non-empty, trimmed audience name" invariant is duplicated and lives inline
in the web layer instead of being encoded in a type.

## Superseding note: the issue body predates ADR-0065

The issue's "honest caveat" prescribes the **String-arg + parse-on-entry** shape
and explicitly rejects a `Deserialize`-time reject "because it degrades the
`ActionForm` error message." That reasoning is exactly what
[ADR-0065](../../adr/0065-client-side-domain-validation.md) (issue #414) was
written to **retire** — it even names #350 as one of the flags that led to the
stringly-typed stopgap (ADR-0065 Context, line 17). The maintainer has directed
this issue to follow the ADR instead.

**ADR-0065 decision:** _type `#[server]` wire args as domain newtypes, and
require client-side pre-validation using the same newtype `FromStr`._ The
`Deserialize` failure UX is a non-issue because a legitimate browser never
submits an invalid value — the submit button is disabled-until-valid, and an
inline error (the newtype's own message) shows at the field. The only requests
that reach decode-time rejection are malformed/malicious/non-browser ones, for
which the generic transport error is the correct (defense-in-depth) outcome.

So #350 becomes: **introduce `AudienceName`, type it as the `create_audience` /
`rename_audience` wire arg, thread it through the store, and adopt client-side
pre-validation in the two audience forms.** This is a direct application of
ADR-0063 (string newtype) + ADR-0065 (typed wire args). **No new ADR.**

## Design

### 1. The newtype (`common/src/audience.rs`, new)

```rust
use std::str::FromStr;
use macros::StrNewtype;
use thiserror::Error;

/// A validated audience name: non-empty after trimming, original casing preserved.
#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct AudienceName(String);

/// Error returned when a string cannot be parsed as an [`AudienceName`].
#[derive(Debug, Error)]
#[error("audience name must not be empty")]
pub struct InvalidAudienceName;

impl FromStr for AudienceName {
    type Err = InvalidAudienceName;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(InvalidAudienceName);
        }
        Ok(AudienceName(trimmed.to_owned()))
    }
}
```

- `#[derive(StrNewtype)]` (ADR-0063) generates the whole trailer: `Display`,
  `AsRef<str>`, `Borrow<str>`, `Deref<Target = str>`, `TryFrom<String>`,
  `From<Self> for String`, `PartialEq<str>`/`PartialEq<&str>`, **and the
  validating serde bridge** (serialize as a plain string; deserialize routes
  through `FromStr`). That serde `Deserialize` is what rejects an empty name at
  the `#[server]` wire boundary. Only the trimming `FromStr` is hand-written —
  the one validating chokepoint.
- **Casing preserved, not lowercased** — audience names are author-facing
  display labels, not identity keys (mirrors `TagLabel`; contrast
  `Username`/`Tag`).
- **No `Hash`/`Ord`** — never a map/set key; DB uniqueness is
  `UNIQUE (author_user_id, name)`, enforced by the store.
- The error's `Display` — `"audience name must not be empty"` — **is** the
  message shown inline at the field on the client (via
  `field_error::<AudienceName>`), preserving the original wording. One source of
  truth.

Register: add `pub mod audience;` to `common/src/lib.rs`. No new crate deps
(`macros`/`thiserror` already used by `username`/`tag`).

Unit tests in `common/src/audience.rs` (host-compiled, coverage-measured — like
`username`/`tag`): accepts a normal name, trims surrounding whitespace, rejects
`""` and whitespace-only, preserves casing, `Display` round-trips, serde
serializes as a plain string and rejects empty on deserialize,
`InvalidAudienceName` `Display` is exactly the message.

### 2. No `host/src/error.rs` change

With a **typed wire arg**, the server-fn body never constructs `AudienceName`
from a raw string, so no `From<InvalidAudienceName> for InternalError` lift is
used anywhere — adding one to `validation_from!` would be dead, un-hit code.
Deliberately omitted. (Rejection now happens at arg-decode, surfacing as the
generic `WebError::ServerFunction` — ADR-0065 Consequences, accepted as the
malicious-only path.)

### 3. Thread through storage (`storage/src/audiences.rs`)

Change the two mutating signatures `name: &str` → `name: &AudienceName` (trait +
`AudienceStore` impl; `mockall::automock` regenerates the mock):

```rust
async fn create_audience(&self, author_user_id: i64, name: &AudienceName) -> Result<i64, AudienceError>;
async fn rename_audience(&self, author_user_id: i64, audience_id: i64, name: &AudienceName) -> Result<(), AudienceError>;
```

SQL binds become `.bind(name.as_ref())` (`AudienceName: AsRef<str>`; the
existing `for<'q> &'q str: Encode<'q, DB>` where-clause already covers it — one
edit per bind). Add `use common::audience::AudienceName;` (`storage` already
depends on `common`).

A **concrete `&AudienceName`** is required — a generic `impl AsRef<str>` would
let a caller pass a raw `&str` again and defeat the forcing property. Read path
stays `String` (`AudienceRecord.name`, `AudienceSummary`): the invariant is a
write-time parse; stored names are already valid. Re-typing the read path is out
of scope.

### 4. Web boundary (`web/src/audiences/mod.rs`)

- **Typed args, ungated import.** Params become `name: AudienceName`:

  ```rust
  pub async fn create_audience(name: AudienceName) -> WebResult<i64>
  pub async fn rename_audience(audience_id: i64, name: AudienceName) -> WebResult<()>
  ```

  `use common::audience::AudienceName;` goes at the top **ungated** (not under
  `#[cfg(feature = "server")]`) — the `#[server]`-generated arg struct
  references it on both the client and server builds (exactly as `auth/mod.rs`
  does for `Username`).

- **Bodies.** Delete both duplicated `trim`/`is_empty` hunks; pass the
  already-parsed value by reference:
  `audiences.create_audience(auth.user_id, &name).await?` /
  `audiences.rename_audience(auth.user_id, audience_id, &name).await?`. The
  `#[cfg(feature = "server")]` `InternalError` import becomes unused (both
  inline `Err(InternalError::validation(...))` are gone) → drop it.

- **Client-side pre-validation via direct-bind (ADR-0065's bespoke-layout
  path).** The audience forms are compact, bespoke layouts (a bare placeholder
  input for create; an inline input beside a Rename button per row) — **not**
  the standard labelled auth form. ADR-0065 §"Rendering: component or direct
  bind" sanctions binding the same `Field<T>` directly to the form's own
  `<input>` for this case. Direct-bind is chosen because it **preserves the
  existing markup** (placeholder, inline row) — so the visual design and the
  existing e2e selectors are unchanged — while adding the two behaviors ADR-0065
  requires:
  - a `Field::<AudienceName>` per input (`::new()` for create,
    `::prefilled(&name)` for the rename row's existing value);
  - `<input … name="name" prop:value=field.value on:input=… on:blur=move|_| field.touch() />`
    where `on:input` sets `field.error = field.error_for(&v)`;
  - a touched-gated inline error node
    (`field.is_touched().then(|| field.error.get())…`) rendering
    `<p class="error">{msg}</p>` at the field;
  - submit gated `prop:disabled=move || !field.is_valid()`.

  `use crate::forms::Field;` (the `ValidatedInput` component is not used here).
  The `required` attribute on the create input is removed (disable-until-valid
  replaces it; the newtype rule is the single source of truth).

### 5. Tests to update

- **`server/tests/web/audiences.rs` — the two empty-name tests change
  semantics.** With a typed arg, an empty/whitespace `name` fails
  **arg-decode**, not the body, so the response no longer carries
  `"audience name must not be empty"` (that message is now client-side). Follow
  the auth precedent (`web_auth.rs::register_invalid_username_returns_error`):
  assert `assert_ne!(status, StatusCode::OK)` + the store side-effect (no
  audience created / name unchanged), and **drop** the
  `body.contains("audience name must not be empty")` assertion. Rename them to
  `…_empty_name_is_rejected`.
- **Store-seeding call sites** that pass `&str` literals to the store now pass
  an `&AudienceName` (mechanical): `server/tests/storage/mod.rs` (~17),
  `server/tests/web/audiences.rs` (direct-store seeds),
  `server/tests/web/web_posts.rs`, `server/tests/misc/backup_fixture.rs`. Use a
  tiny per-module helper
  (`fn an(s: &str) -> AudienceName { s.parse().unwrap() }`) or inline
  `&"Friends".parse().unwrap()`. Finish in this PR — no deferral. HTTP-level
  `/api/create_audience` form-body posts are unaffected (the wire body is still
  `name=<string>`).
- **`web/src/forms.rs` host test** — extend the existing `field_error` tests
  with an `AudienceName` case (valid → `None`; empty/whitespace → the message),
  mirroring the `Username`/`Tag`/`Slug` cases, so the client-validation wiring
  for this newtype is covered host-side.
- **`end2end/tests/audiences.spec.ts` — add a client-validation test**
  (mirroring `auth.spec.ts`): on the create form, an empty/whitespace name keeps
  the **Create** button disabled; typing then clearing + blur shows the inline
  `p.error` "audience name must not be empty"; a valid name enables submit and
  creates. Existing tests keep their selectors (placeholder/`name` preserved by
  direct-bind) — verify they stay green after the `required`-attribute removal
  (disable-until-valid now gates the empty submit the `required` attribute used
  to).

## Acceptance

- The trim/empty rule exists in exactly one place — `AudienceName::from_str` —
  unit tested in `common`.
- `create_audience`/`rename_audience` take a typed `AudienceName` wire arg; both
  duplicated inline hunks are gone; the store cannot be called without a
  validated name (`&AudienceName`).
- The `"audience name must not be empty"` wording is preserved as the
  client-side inline message (the newtype's `Display`), shown
  disable-until-valid at the field; asserted by the `forms.rs` host test and the
  new e2e test.
- Legitimate submissions can't reach decode-time rejection (submit
  disabled-until-valid); a malicious empty POST is rejected non-OK with no side
  effect (updated server tests).
- `cargo xtask validate` green (host + coverage + all four e2e combos, incl. the
  audience flows).

## Out of scope

- Re-typing the audience read path / `AudienceSummary` wire DTO.
- Any charset/length rule beyond trim + non-empty (parse the _existing_
  invariant exactly).
- Adopting the `<ValidatedInput>` component look for these forms (direct-bind
  preserves the current design; switching to the component is a separate UI
  decision).
- Changing ADR-0065's status (a separate concern); #350 is simply another
  adopter.
