# Centralize Fallible String Serde Emission

## Outcome

`str_newtype` and `text_enum` generate their validating string Serde
implementations through one macros-crate helper without changing generated
behavior or public macro interfaces.

## Load-bearing decisions

- Add one private `fallible_string_serde_impls` helper in `macros/src/lib.rs`,
  beside the existing shared generics helpers.
- The helper accepts the type `Ident`, its `Generics`, and a caller-supplied
  borrowed-string serialization expression, then returns both complete
  `Serialize` and `Deserialize` impl token streams.
- The helper owns `split_for_impl` and `with_leading_param(..., 'de)` so the
  deserialize lifetime always precedes existing lifetime, type, and const
  parameters while preserving type arguments and where clauses.
- Serialization continues to call `Serializer::serialize_str` with borrowed data
  and performs no allocation.
- Deserialization continues to decode an owned `String`, call the generated
  type's `FromStr` implementation, and map its error through
  `serde::de::Error::custom`.
- Both generated impls retain `#[automatically_derived]` and the current fully
  qualified Serde, `String`, `FromStr`, and error-mapping paths so downstream
  lint and diagnostic behavior is unchanged.
- `str_newtype::serde_impls` delegates with `&self.0`, retaining generic and
  phantom-tagged newtype support.
- `text_enum::serde_impls` delegates with its allocation-free `&'static str`
  conversion expression; its non-generic enum restriction remains owned by
  `text_enum`.
- Both macro modules address the nonlocal helper through the immediate parent
  owner path rather than importing the free function directly.
- Preserve default, `secret`, `secret, serde`, `no_serde`, `no_sqlx`, and
  `no_ord` feature/option routing exactly as implemented today.
- Preserve ADR-0063's `FromStr` validation chokepoint, ADR-0091's text-enum
  token ownership, and ADR-0062's macros-crate ownership and direct codegen
  coverage.

## Acceptance

- `str_newtype::serde_impls` and `text_enum::serde_impls` contain no duplicated
  Serde impl emission.
- Generated serialization remains borrowed and allocation-free for both macro
  families.
- Generated deserialization still uses owned `String`, `FromStr`, and custom
  Serde error mapping.
- Generic tagged string newtypes retain valid lifetime ordering, type arguments,
  and where clauses.
- Normal and secret-inbound string newtypes retain round-trip and invalid-wire
  behavior.
- Text enums retain declared token round trips and parse-error reporting.
- Normalized-token assertions for both callers cover both derived markers,
  `&self.0`, the exact static-token conversion, owned `String`, qualified
  `FromStr`, custom error mapping, and generic impl/type/where-clause spelling.
- Existing macro invocation syntax, generated public interfaces, serialized
  shapes, and diagnostics remain unchanged.
- Focused runtime and code-generation macros tests pass and `cargo xtask check`
  passes.

## Boundaries

- Do not change numeric newtype or ID newtype Serde generation; their wire
  formats and fallibility differ.
- Do not change SQLx emission, enum token derivation, `FromStr` implementations,
  secret redaction, macro parsing, or option validation.
- Do not add Serde attributes or a second deserialization representation.
- Do not absorb #857, #913, or #709.
- No domain glossary or ADR change is required; this implements existing macro
  ownership and validation decisions.
