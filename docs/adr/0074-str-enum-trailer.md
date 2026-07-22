# ADR-0074: `StrEnum` derive — the standard string-enum trailer

- Status: proposed
- Date: 2026-07-20
- Issue: [#562](https://github.com/jaunder-org/jaunder/issues/562)

## Context

ADR-0063 established the newtype trailers (`StrNewtype`, `IdNewtype`,
`NumNewtype` in the `macros` crate): one derive owns a value type's whole
conversion surface so invalid states are unrepresentable and the surface is
uniform. Closed **string-backed enums** — a domain value that is one of a fixed
set of lowercase tokens on the wire and in the DB (`Channel`,
`SubscriptionStatus`, `TargetKind`, `AudienceBase`, `PostFormat`) — had no such
trailer. Instead there was `str_enum!`, a private `macro_rules!` in
`common/src/visibility.rs`, and it diverged from the newtype family:

- It was **module-local**, so a string enum outside `visibility.rs` (e.g.
  `PostFormat` in `render.rs`) could not use it and hand-rolled
  `Display`/`FromStr`/its error/serde.
- Its `TryFrom<&str>` error was a bare **`()`** — no message, not registrable in
  `host/src/error.rs` the way the newtype errors and `InvalidPostFormat` are.

## Decision

Promote the string-enum trailer to a **`#[derive(StrEnum)]` proc-macro in
`macros/`**, alongside the newtype derives, and route all five string enums
through it.

- **Applies to** a unit-variant enum; any other shape (struct, fielded variant,
  or two variants that resolve to the same wire literal) is a spanned
  `compile_error!`.
- **Wire string = the `snake_case` of the variant identifier by default**
  (`InviteOnly` → `invite_only`), overridable per variant with
  `#[str_enum(rename = "…")]`. (The five original enums are all single-word,
  where `snake_case` == the lowercased identifier;
  `RegistrationPolicy::InviteOnly` (#576) is the first multi-word variant and
  relies on this default for its `invite_only` token. A consecutive-capital
  acronym snake-cases one underscore per letter — override those with `rename`.)
- **Always emitted:** `as_str(&self) -> &'static str`, `Display` (via `as_str`),
  `FromStr` **and** `TryFrom<&str>`, and a generated `pub struct Invalid<Name>;`
  error (`format_ident!("Invalid{}", Name)`; derives
  `Debug, Clone, Copy, PartialEq, Eq`; hand-written `Display` +
  `std::error::Error`, no `thiserror`). The message is auto-generated
  (`… must be one of: a, b, c`) unless `#[str_enum(error = "…")]` overrides it.
- **`serde` and `Default` are orthogonal opt-ins.** `#[str_enum(serde)]`
  (type-level) adds the `Serialize`/`Deserialize` bridge — serialize `as_str`,
  deserialize an **owned `String`** through `FromStr` (owned so the serde_qs
  form transport works; no `rename_all`, so the wire literals stay
  single-sourced in `as_str`). `Default` is the user's own std
  `#[derive(Default)]` + `#[default]` on a variant. `Copy`/`Hash`/etc. likewise
  stay in the user's `#[derive(...)]` list — the macro owns only the trailer.

**Divergence from `StrNewtype`, deliberately:** `StrNewtype` makes the author
hand-write `FromStr` and routes `TryFrom`/serde through its `Err`. Because a
string enum's valid set is _known to the macro_, `StrEnum` instead **generates**
both `FromStr` and the named error — strictly less hand-written code, with the
same "one named error, host-registrable" outcome.

## Consequences

- The private `macro_rules! str_enum` in `common/src/visibility.rs` is deleted;
  all five enums carry `#[derive(StrEnum)]`. Each `TryFrom` error goes `()` →
  `Invalid<Name>`; this is non-breaking (consumers use
  `.ok()`/`is_ok()`/`match _`), the one structural exception being a
  `Err(()) => …` arm rewritten to `Err(_) => …`.
- `PostFormat` stops hand-rolling its surface and **gains serde**, unblocking
  the #498 web-boundary threading. Its generated `InvalidPostFormat` keeps the
  same name and `common::render` path, so the `host/src/error.rs` registration
  and every existing `.parse::<PostFormat>()` site keep working. Its message is
  preserved via `error = "…"`.
- New public `Invalid<Name>` error types appear for the other four enums
  (previously errorless). They are inert unless a caller inspects them.
- Future string enums get the whole surface — `as_str`, `Display`, `FromStr`,
  `TryFrom`, a named error, opt-in serde — from one derive, matching the newtype
  family.
