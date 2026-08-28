# ADR-0102: Config keys are a closed registry carrying validators

- Status: accepted
- Date: 2026-08-05
- Issue: [#687](https://github.com/jaunder-org/jaunder/issues/687)

## Context

Site config was a `&str`-keyed key/value store. Three problems compounded:

1. **`set(key, value)` was transposable.** Two adjacent `&str` parameters —
   swapping them compiled and silently wrote a garbage row.
2. **The key space had accreted across three files** and nobody could point at
   it. The issue that opened this work counted 11 keys; the real number was 19,
   spread over `site_config.rs` consts, `media.rs` consts, and bare `smtp.*`
   literals. A twentieth was declared and never read or written.
3. **The CLI was a documented free-form door** — "values are not validated" — so
   a typo in an operator's key or value became a row that nothing would reject
   until something later tried to parse it, if anything ever did.

Rust has no dependent types, so the obvious fix does not exist: `get(key)`
cannot vary its return type by key. That is what makes this a decision rather
than a refactor.

## Decision

**A `macro_rules!` table declares every config key, and each row carries a
validator.** The table emits the key enum (a `#[text_enum(sqlx, …)]`, so the key
is a first-class column) and a `validate(self, raw: &str) -> Result<(), _>`
whose return type is fixed. A validator runs the value's real parser and
discards the parsed value — which sidesteps dependent types entirely while still
enforcing the real rule.

**Raw `get`/`set`/`delete` take the key enum, never `&str`**, so transposition
is a compile error. They remain the primitive that typed accessors are built
from, and are legal only inside those accessor bodies.

**A key cannot exist without a validator**, because both come from the same
table row. The whole supported surface is one scannable block, and adding a key
is one line.

`list()` deliberately stays `Vec<(String, String)>` — a faithful dump of what is
physically stored. Typing it would hide the orphan rows an operator most needs
to see.

> **Annotation (2026-08-27).** As of #847, both closed configuration registries,
> `SiteConfigKey` and `UserConfigKey`, moved to `host`. Their closed-registry,
> validator, and SQLx-binding contracts remained unchanged. Current ownership:
> [ARCHITECTURE.md](../ARCHITECTURE.md).

## Consequences

- **The registry lives in `common`, not `storage`.** `#[text_enum]` needs
  `strum` and emits its sqlx bridge under `#[cfg(feature = "sqlx")]`; `storage`
  has neither, so the bridge would have compiled out **silently**. The `sqlx`
  form is also load-bearing rather than decorative: without `Encode`, the impls
  must write `.bind(key.as_ref())`, which the `sqlx-newtype-bind` gate forbids.
- **Validation is a write-time door, not a read-time guarantee.** Rows can
  predate the registry or be written straight to the database, so read paths
  stay defensive — some accessors deliberately default or purge an unparseable
  value, and that behaviour is unchanged.
- **`list` gained a second job.** Because every key now has a validator, the CLI
  dump can report two classes of junk it previously could not see: unrecognised
  keys, and recognised keys whose stored value no longer parses.
- **Errors name the key and the reason, never the value.** Credentials are in
  the registry; a validator that echoed its input would leak a secret into logs
  and operator output.
- **`{ optional }` is part of the contract, not a convenience.** Several setters
  store `""` to mean "absent", so optional keys' validators must accept empty.
  Removing that marker silently breaks three shipped behaviours.
- The same table shape applies to per-user config, which was given the identical
  treatment. Browser `localStorage` has the same defect and is tracked
  separately ([#827](https://github.com/jaunder-org/jaunder/issues/827)) — its
  registry cannot sit beside the primitive, because ADR-0069 charters `client`
  as raw browser infrastructure with no domain types.
