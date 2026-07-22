# Spec — #576: `RegistrationPolicy` enum on the wire

- Issue: [#576](https://github.com/jaunder-org/jaunder/issues/576)
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../adr/0063-domain-value-newtype-convention.md)
  (domain-value type safety), [ADR-0065](../adr/0065-typed-wire-args.md)
  (typed wire args), [ADR-0074](../adr/0074-str-enum-trailer.md) (`StrEnum` trailer —
  amended by this issue)
- Related: [#562](https://github.com/jaunder-org/jaunder/issues/562) (`StrEnum`
  derive)
- Date: 2026-07-22

## Problem

`get_registration_policy()` returns the site registration policy as a bare
`String` (`web/src/registration/api.rs`, documented `"open"` / `"invite_only"` /
`"closed"`). The wire hop erases the already-typed `RegistrationPolicy` enum
(`storage/src/auth.rs`) that the server fn uses internally, and its two client
consumers compare against string literals:

- `web/src/registration/component.rs` — `p.as_deref() == Ok("invite_only")`
- `web/src/pages/invites.rs` — `if policy_str != "invite_only"`

This is the classic stringly-enum drift hazard: a variant rename silently breaks
the literal compares and the compiler cannot see them. The enum lives in
`storage`, which the wasm client cannot import — that is *why* the hop is
stringly today.

## Decision

Move `RegistrationPolicy` to `common` (importable ungated from wasm, the ADR-0065
typed-wire requirement), give it the `StrEnum` trailer with serde, return it
typed, and delete the literals.

### Scene-setting: `StrEnum` defaults to `snake_case` (macros)

`RegistrationPolicy::InviteOnly` is the **first multi-word `StrEnum` adopter**, and
it exposes a latent gap in the #562 derive: `StrEnum` defaulted each variant's wire
token to the *lowercased identifier*, so `InviteOnly` → `"inviteonly"` — never the
snake_case token an author wants, and here it would silently break the
`site.registration_policy` DB value `"invite_only"`.

So the **first commit** of this issue fixes the tooling at the source: `StrEnum`'s
default token becomes the **`snake_case`** of the identifier (`InviteOnly` →
`invite_only`, `Open` → `open`). This changes **zero existing tokens** — all five
prior adopters (`Channel`, `SubscriptionStatus`, `TargetKind`, `AudienceBase`,
`PostFormat`) are single-word, where snake_case == lowercased. It aligns with
serde's `rename_all = "snake_case"` convention and removes the footgun for every
future multi-word variant. A consecutive-capital acronym snake-cases one underscore
per letter (`HTMLPage` → `h_t_m_l_page`); those override with `#[str_enum(rename)]`.
ADR-0074's token-default bullet is amended accordingly, and the derive gains
multi-word coverage (in-crate unit test + integration fixture).

### The enum — new `common::registration` module

A dedicated `common/src/registration.rs`, matching the one-module-per-domain-type
pattern (`email`, `invite`, `password`, `slug`, …):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, StrEnum)]
#[str_enum(serde)]
pub enum RegistrationPolicy {
    Open,
    InviteOnly, // -> "invite_only" via the snake_case default; NO rename needed
    Closed,
}
```

- **No `#[str_enum(rename)]`** — the snake_case default now yields `invite_only`.
  A test asserts `as_str()`/serde == `"invite_only"` and rejects `"inviteonly"`,
  pinning that behavior.
- `#[str_enum(serde)]` because the type is a `#[server]` return value crossing the
  wire (serialize the token; deserialize an owned `String` via the generated
  `FromStr`).
- The derive generates the named error `InvalidRegistrationPolicy` — the same name
  the hand-written error carried, so no external reference breaks.

### `storage` re-uses it (no duplicate)

`storage/src/auth.rs`: delete the hand-rolled enum/`Display`/`FromStr`/error;
`pub use common::registration::RegistrationPolicy;` so the `storage::RegistrationPolicy`
path in `web/src/invites/mod.rs` keeps resolving. `load_registration_policy(store)
-> RegistrationPolicy` stays in storage (it needs `SiteConfigStorage`); body
unchanged.

`api.rs` currently imports `RegistrationPolicy` from `storage` inside its
`#[cfg(feature = "server")]` block. Because the typed return makes `web` name the
type ungated (from `common::registration`), that server-gated import is **dropped**
to avoid an E0252 collision.

### `web` — typed return, literals deleted

- `get_registration_policy() -> WebResult<RegistrationPolicy>` (ungated import;
  body returns `Ok(policy)` directly). Doc lists the variants.
- `component.rs`: `matches!(p, Ok(RegistrationPolicy::InviteOnly))`.
- `invites.rs`: `if policy.await != Ok(RegistrationPolicy::InviteOnly)` — folds the
  `Err` case into the not-invite-only → 404 branch, dropping `unwrap_or_default()`.

## Out of scope

- **`host::metrics::RegistrationPolicy`** is a *separate* `enum_attr!` enum that
  adds a `CliBypass` variant not present in the domain enum. It genuinely diverges;
  `register()`'s manual storage→metrics mapping stays.
- The `storage/src/atomic.rs` raw `.set("site.registration_policy", "open")` DB-seed
  write stays as a literal (not a policy comparison in `web/`).

## Tests

- `macros`: in-crate unit test drives `expand` with a multi-word variant (covers the
  new `to_snake_case` conversion → `invite_only`, not `inviteonly`); integration
  fixture `Policy { Open, InviteOnly, Closed }` asserts `as_str`/parse round-trip.
- `common::registration`: `FromStr` accept for all three tokens, `Display`/`as_str`
  round-trip, reject unknown (and explicitly `"inviteonly"`), serde round-trip, and
  the snake_case-token guard.
- `storage`: the `load_registration_policy` dual-backend tests stay, now exercising
  the re-exported type.
- `web`: existing registration/invites behavior unchanged (invite-only gates; a
  non-invite-only site 404s the invites page).

## Acceptance

- `StrEnum`'s default token is the `snake_case` of the identifier; ADR-0074 amended;
  no existing adopter token changes; multi-word coverage added.
- `RegistrationPolicy` defined once in `common::registration` with `#[str_enum(serde)]`
  and **no rename**; `storage` re-exports it (no duplicate enum).
- The `"invite_only"` DB/wire token is preserved (test asserts token + serde).
- `get_registration_policy` returns `WebResult<RegistrationPolicy>`; no `"open"` /
  `"invite_only"` / `"closed"` policy string comparisons remain in `web/` source.
- `cargo xtask validate --no-e2e` clean.
