# Spec — `StrEnum` derive: shared string-enum trailer (#562)

Issue: #562 (milestone #13). Blocks #498. Establishes the string-enum analogue
of the ADR-0063 `StrNewtype` trailer.

## Problem

The string-enum helper `str_enum!` (a private `macro_rules!` in
`common/src/visibility.rs`) is out of step with the newtype derives
(`StrNewtype`, `IdNewtype`, `NumNewtype` in `macros/`) in two ways: it is
**trapped** in `visibility.rs` (so `PostFormat` in `common/src/render.rs`
hand-rolls `Display`/`FromStr`/ `InvalidPostFormat`, and — for #498 — would
hand-roll serde), and its `TryFrom` error is a bare **`()`** rather than a
named, `Display`-carrying error the way the newtype family and
`InvalidPostFormat` are (registrable in `host/src/error.rs`).

## Design

A `#[derive(StrEnum)]` proc-macro in `macros/`, modelled on `NumNewtype`
(self-contained generated error, no `thiserror`; std derives stay in the user's
`#[derive(...)]` list; optional `error = "…"` message override).

**Applies to** an enum whose variants are all unit variants. Any other shape (a
struct, a variant with fields, a duplicate resolved wire literal) is a **spanned
`compile_error!`**.

**Wire string per variant:** the lowercased variant identifier by default;
`#[str_enum(rename = "…")]` on a variant overrides it. (All five current enums
match the lowercase default, so none needs a per-variant attr.)

**Always emitted:**

- `pub fn as_str(&self) -> &'static str` — variant → its wire string.
- `Display` via `as_str`.
- `FromStr` **and** `TryFrom<&str>`, both routing an unknown string to the
  generated error.
- `pub struct Invalid<Name>;` — the error (named
  `format_ident!("Invalid{}", Name)`, e.g. `InvalidPostFormat`), deriving
  `Debug, Clone, Copy, PartialEq, Eq` (matching `NumNewtype`'s error — existing
  tests do `assert_eq!(X::try_from(..), Ok(x))`, so the error must be
  `PartialEq + Debug`), with a hand-written `Display` + `std::error::Error` (no
  `thiserror`). Its message is auto-generated
  (`… must be one of: markdown, org, html`) unless a type-level
  `#[str_enum(error = "…")]` overrides it.

**Opt-in `#[str_enum(serde)]`** (type-level): adds `Serialize`
(`serialize_str(as_str)`) and `Deserialize` (deserialize an **owned `String`**,
then `FromStr`, mapping failure to `serde::de::Error::custom(Invalid<Name>)`).
Owned `String` is required so the serde_qs form transport works (borrowed `&str`
fails there). No `#[serde(rename_all)]` — the wire literals are single-sourced
in `as_str`.

**Not the macro's concern** (left to the user's std derives, like the newtype
derives): `Clone`, `Copy`, `Debug`, `PartialEq`, `Eq`, `Hash`, and `Default`
(via std `#[derive(Default)]` + `#[default]` on a variant). `Default` and
`serde` are orthogonal.

## Migration (this PR)

Delete `macro_rules! str_enum` from `common/src/visibility.rs`; convert all five
enums, **pinning each std-derive list explicitly** (a dropped `Hash`/`Copy` is
silent — not gate-caught):

- `Channel`, `SubscriptionStatus`, `TargetKind` →
  `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, StrEnum)]`, no serde, no
  Default (identical to today's std derives). Their `TryFrom` error `()` →
  `Invalid<Name>`. Consumers are **almost** all payload-agnostic
  (`.ok()`/`is_ok()`), with **one exception that must be edited**:
  `storage/src/posts.rs:1842` matches `Err(()) => None` — the unit pattern won't
  match the new struct error, so rewrite it to `Err(_) => None`.
- `AudienceBase` →
  `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, StrEnum)] #[str_enum(serde)]`,
  `#[default]` on `Private`. Preserves its current serde + Default. Its
  deserialize error changes `unknown_variant` → `custom(InvalidAudienceBase)`;
  observably safe — `audience_base_deserialize_rejects_unknown` asserts only
  `.is_err()`.
- `PostFormat` (`common/src/render.rs`) →
  `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, StrEnum)] #[str_enum(serde)] #[str_enum(error = "post format must be \"markdown\", \"org\", or \"html\"")]`,
  `#[default]` on `Markdown`. **Deletes** the hand-written `Display`/`FromStr`/
  `InvalidPostFormat` and the manual serde. The `error = …` override **preserves
  the exact current message**, so `post_format_rejects_invalid_value`
  (`render.rs:489`) stays green; the `FromStr` round-trip tests likewise pass
  unchanged. The generated `InvalidPostFormat` (same name, same `common::render`
  path, still `Display + Error`) keeps the `host/src/error.rs` registration
  (`:386`, `:669`), the two storage `.parse::<PostFormat>()` DB-decode sites
  (`storage/src/helpers.rs`, `storage/src/user_config.rs`), **and** the three
  web `.parse::<PostFormat>()?` sites (`web/src/posts/mod.rs:250,:412`,
  `web/src/profile/mod.rs:93` — routed through host's `validation_from!`; #498
  removes these) working unchanged.
- **`Copy` ripple:** `PostFormat` is not `Copy` today; adding it (wanted by
  #498) trips `clippy::clone_on_copy` on **nine** existing `PostFormat` clones
  (line numbers as of `main` @ e77ced3d) — `storage/src/post_service.rs:510`
  (×1) and the eight `d.clone()` in `server/src/atompub/mapping.rs`
  `wire_to_format_is_lenient` (lines 222, 224, 228, 243×2, 244×2, 245). Replace
  each with a plain copy; the `cargo xtask check` clippy gate (`--all-targets`,
  so test bodies too) enforces none remain. (This absorbs what was #498's Task 1
  clone cleanup.) `web/src/pages/posts.rs:703`'s `fetched.format.clone()` is
  **not** among them — that value is `PostResponse.format`, still a `String`
  until #498, so `Copy` doesn't touch it; web stays out of scope.

## ADR

Record the trailer as an ADR (draft in `docs/adr/drafts/`, promoted at ship):
the standard string-enum surface, lowercase-default + `rename`, the orthogonal
`serde`/std-`Default` split, the auto-generated `Invalid<Name>` error, and the
"known variant set ⇒ the macro owns `FromStr`" divergence from `StrNewtype`
(which delegates to the author's `FromStr`).

## Acceptance criteria

1. **Derive exists.** `#[derive(StrEnum)]` is registered in `macros/src/lib.rs`
   with a `str_enum.rs` module. `rg 'macro_rules! str_enum' common/` returns
   nothing (old macro deleted).
2. **Surface per enum.** For each of the five enums, `as_str`, `Display`,
   `FromStr`, `TryFrom<&str>`, and a `pub Invalid<Name>` error exist; a
   round-trip (`x.as_str().parse() == Ok(x)`) holds for every variant, and
   parsing an unknown string yields `Err(Invalid<Name>)`.
3. **Wire form byte-identical.** Every variant's `as_str`/serialized string is
   unchanged from today (`"markdown"`, `"local"`, `"private"`, …). For the two
   `serde` enums (`AudienceBase`, `PostFormat`), a `serde_json` round-trip of
   each variant yields the lowercase string and deserializing an unknown string
   errors. (The owned-`String` / serde*qs form-path decode is exercised by the
   `macros/tests` round-trip and matters for #498, where `PostFormat` becomes a
   wire arg; in *this* PR `set_default_post_format` is still `String`-based, so
   its server test stays green trivially.) 3b. **Error messages pinned.**
   `PostFormat`'s message is preserved exactly (the `error =` override) —
   `post_format_rejects_invalid_value` (`render.rs:489`) stays green. The
   **auto-generated** message format (`… must be one of: a, b, c`) is pinned by
   a `macros/tests` case on a sample derive (no override), so a conformance
   reviewer can falsify it. 3c. **Migration compiles.**
   `storage/src/posts.rs:1842` is rewritten `Err(()) => None` → `Err(*) =>
   None`; the full workspace builds under the new struct errors.
4. **Error is named + host-wired.** `InvalidPostFormat` remains a `pub` type at
   `common::render::InvalidPostFormat`; `host/src/error.rs` still compiles and
   registers it (`:386`, `:669`). The generated error impls `Display` +
   `std::error::Error`.
5. **Macro robustness.** Applying `StrEnum` to a non-enum, to an enum with a
   fielded variant, or to two variants that resolve to the same wire literal is
   a spanned `compile_error!`, covered by `macros/tests` + in-crate
   `syn::parse_quote!` unit tests (the macros crate is coverage-measured).
6. **`PostFormat` is not hand-written.** `common/src/render.rs` no longer
   hand-writes `impl Display`/`impl FromStr`/`struct InvalidPostFormat`/serde
   for `PostFormat`; it is a `#[derive(…, StrEnum)]` enum. The two storage
   `.parse::<PostFormat>()` sites are untouched and compile.
7. **Gate green.** `cargo xtask validate --no-e2e` passes (static + clippy +
   coverage, including the macros crate's error-path coverage);
   `cargo nextest run -p macros` passes. No new
   `#[allow]`/`cov:ignore`/`crap:allow`.
8. **ADR added** (draft, promoted at ship).

## Non-goals

- Threading `PostFormat` through the web boundary — that is #498 (depends on
  this).
- Any wire/DB form change; any new enum variants; converting non-string enums.
