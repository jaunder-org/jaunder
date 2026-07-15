# Spec — issue #401: `DisplayName` newtype

- Issue: [#401](https://github.com/jaunder-org/jaunder/issues/401) — _types:
  DisplayName newtype (mirror AudienceName #350)_
- Milestone: Domain-value type safety (newtypes)
- ADRs: adopts **ADR-0063** (newtype convention + `StrNewtype` trailer) and
  **ADR-0065** (typed `#[server]` wire args + client-side pre-validation). **No
  new ADR** — this is another adopter of both.

## Problem

A user's `display_name` is a bare `String`/`Option<String>` end-to-end — storage
record (`UserRecord`), the update DTO (`ProfileUpdate`), the web response DTO
(`ProfileData`), the `update_profile` `#[server]` fn, and the CLI. The only
validation today is trim-to-`None` at the web edge (`common::text::non_empty`,
`web/src/profile/mod.rs:58`). There is **no length bound** and no type carrying
the invariant. Per ADR-0063 §1 the value qualifies on the **invariant** axis
(trim, non-empty-or-nullable, length bound).

## Deliberate departure from the issue body

The issue (and #350) prescribe the **`String` wire-arg + parse-on-entry** shape
and explicitly reject `Deserialize`-time validation "because it degrades the
`ActionForm` error message." That reasoning is the **pre-ADR-0065 stopgap**.
ADR-0065 (#414) exists precisely to retire it — and names #350 as a motivating
flag. This spec implements the **ADR-0065** shape instead: a typed wire arg plus
client-side pre-validation through the newtype's own `FromStr`. (Maintainer-
directed; recorded here because it contradicts the issue text.)

## Decisions (settled in the design interview)

1. **Length bound = 255 chars.** `MAX_DISPLAY_NAME_CHARS = 255` (counted as
   `chars()`, matching `Slug`). Generous enough that the read-time bound (see
   §Scope) is extremely unlikely to reject any pre-existing row.
2. **Scope = full thread, strict read.** The newtype is threaded through the
   storage record, the update DTO, `create_user`, and the CLI — per ADR-0063 §4
   ("storage record fields _are_ the newtype") and the issue's acceptance
   ("Profile record + update path speak `DisplayName`"). The row→record read
   path **parses strictly**; a hypothetical legacy row longer than 255 chars
   would fail to load. Accepted given the generous bound.
3. **No new ADR.** ADR-0063 + ADR-0065 already govern this; `DisplayName` is an
   adopter.
4. **`bio` is unchanged.** It stays `String`/`Option<&str>` — no newtype is
   filed for it (free-form, no invariant). Only `display_name` changes.

## The type — `common/src/display_name.rs`

Mirror `Slug` (length bound) + `TagLabel` (trim, **preserve casing** — a display
name is not lowercased):

```rust
pub const MAX_DISPLAY_NAME_CHARS: usize = 255;

#[derive(Clone, Debug, PartialEq, Eq, StrNewtype)]
pub struct DisplayName(String);

#[derive(Debug, Error)]
#[error("display name must be non-empty and at most {MAX_DISPLAY_NAME_CHARS} characters")]
pub struct InvalidDisplayName;

impl FromStr for DisplayName {
    type Err = InvalidDisplayName;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() || trimmed.chars().count() > MAX_DISPLAY_NAME_CHARS {
            return Err(InvalidDisplayName);
        }
        Ok(DisplayName(trimmed.to_owned()))
    }
}
```

- `#[derive(StrNewtype)]` generates the full trailer (`Display`, `AsRef<str>`,
  `Borrow<str>`, `Deref<Target = str>`, `TryFrom<String>`,
  `From<Self> for String`, `PartialEq<str>`/`<&str>`, serde bridge). Only
  `FromStr` + the std derives are hand-written.
- Register `pub mod display_name;` in `common/src/lib.rs` (alphabetical).
- **No** `validation_from!` entry in `host/src/error.rs`: with a typed wire arg
  there is no body parse, so `From<InvalidDisplayName> for InternalError` would
  be dead code. (Contrast: `Slug`/`Username` are listed there because they parse
  in a fn body.)

## Threading surface

**`common`** — new `display_name` module (above).

**storage** —

- `UserRecord.display_name: Option<DisplayName>` (`storage/src/users.rs:22-28`).
- `ProfileUpdate.display_name: Option<&'a DisplayName>`
  (`storage/src/users.rs:111-116`).
- `create_user(..., display_name: Option<&'a DisplayName>, ...)` trait +
  generic/postgres/sqlite impls; bind via the `Deref<str>` (`.map(|d| &**d)` /
  `AsRef::as_ref`) — no SQL/migration change (column stays `TEXT`).
- Row→record helper (`storage/src/helpers.rs`) parses the `TEXT` column into
  `Option<DisplayName>` **strictly**, surfacing a parse failure as a storage
  decode error (not a panic).
- `atomic.rs` wrapper forwards the new types.

**web (ADR-0065)** —

- `update_profile` wire arg: `display_name: String` → `Option<DisplayName>`
  (`web/src/profile/mod.rs:53-71`); import `DisplayName` **ungated** (the
  generated arg struct references it on both client + server builds). Drop the
  server-side `common::text::non_empty(&display_name)` — empty→`None` is handled
  client-side by `Field::optional`. Server passes `display_name.as_ref()` into
  `ProfileUpdate`.
- `ProfileData.display_name: Option<DisplayName>` (response DTO,
  `web/src/profile/mod.rs:22-28`).
- Form (`web/src/pages/profile.rs`): convert the profile `<ActionForm>` to a
  **`.dispatch`** form, mirroring `slug_override` (#408,
  `web/src/pages/posts.rs:632,695,737`):
  - `let dn_field = Field::<DisplayName>::optional_prefilled(&existing);`
  - **direct-bind** the existing `<input name="display_name">` (preserve the e2e
    selector): `prop:value=dn_field.value`, `on:input` → set value +
    `dn_field.error_for`, `on:blur` → `dn_field.touch()`, touched-gated inline
    error.
  - `bio` moves to a plain `RwSignal<String>` bound to its `<textarea>`.
  - submit button `on:click` →
    `update_action.dispatch(UpdateProfile { display_name: dn_field.parsed(), bio: bio_sig.get() })`.
  - Optional field ⇒ `is_valid()` leaves submit **enabled** when blank (clearing
    the name stays possible); a non-empty over-long entry gates it.

**CLI (`server`)** — `UserCreate.display_name: Option<DisplayName>` (clap parses
via `FromStr`); thread through `commands.rs`/`main.rs` seed paths (`None`
unchanged).

## Test strategy

- **`common`** — inline `#[cfg(test)] mod tests`: parse valid; reject
  empty/whitespace-only/over-255; `Display`/`to_string`; the
  `*_serde_serializes_as_plain_string_and_validates_on_deserialize` test
  (pattern from `username.rs`/`slug.rs`).
- **`web/src/forms.rs`** host test — a `field_error::<DisplayName>` case (valid
  → `None`; over-long → the newtype's message), matching the existing
  `Username`/`Slug` cases.
- **Server integration tests** (`server/tests/web/web_account.rs`) — a typed
  wire arg moves empty/invalid rejection into arg-**decode** (generic
  `WebError::ServerFunction`), not `Validation`. So assert
  `assert_ne!(status, OK)` **plus the store side-effect**, and **drop** any
  `body.contains("<message>")` assertion (precedent:
  `web_auth.rs::register_invalid_username_returns_error`). Keep/adjust the
  happy-path + clear-to-`None` assertions.
- **e2e** — the profile page still submits a valid display name and shows the
  inline validation error for an over-long entry (selector unchanged:
  `input[name="display_name"]`).

## Acceptance

- The trim/bound rule lives in exactly one tested place —
  `DisplayName::FromStr`.
- Storage record, update DTO, `create_user`, CLI, the `update_profile` wire arg,
  and `ProfileData` all speak `DisplayName`, not `String`.
- Client-side pre-validation via the shared `FromStr` (no re-implemented rule).
- `cargo xtask validate --no-e2e` clean (e2e via the full gate at ship).

## Out of scope

- `AudienceName` (#350) and the other sibling newtypes.
- Any `bio` newtype (`PostBody`/`PostTitle` are #402).
- A `Deserialize`-time / server-body validation path (retired by ADR-0065).
