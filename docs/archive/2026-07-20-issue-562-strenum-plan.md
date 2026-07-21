# Plan — `StrEnum` derive: shared string-enum trailer (#562)

Spec:
[`2026-07-20-issue-562-strenum.md`](../specs/2026-07-20-issue-562-strenum.md).
ADR draft: [`str-enum-trailer.md`](../../adr/drafts/str-enum-trailer.md). The
spec is "what/why"; this is "how."

## Review header

**Goal.** Add a `#[derive(StrEnum)]` proc-macro to `macros/` (modelled on
`NumNewtype`) and route all five string enums through it, deleting the private
`visibility.rs` `macro_rules! str_enum` and `PostFormat`'s hand-written surface.
`PostFormat` gains serde (unblocks #498).

**Scope — in:** `macros/src/{lib.rs,str_enum.rs}`, `macros/tests/str_enum.rs`,
`common/src/{render.rs,visibility.rs}`,
`storage/src/{posts.rs,post_service.rs}`, `server/src/atompub/mapping.rs`,
`docs/adr/drafts/str-enum-trailer.md` (already written). **Out:** web-boundary
`PostFormat` threading (#498); any wire/DB form change; new variants.

**Tasks (one line each):**

1. Build the `StrEnum` derive in `macros/` (+ its `macros/tests` integration
   suite and in-crate error-path unit tests). No real enum migrated yet.
2. Migrate `PostFormat` (`render.rs`) onto `StrEnum`; delete its hand-written
   surface; absorb the `Copy` `clone_on_copy` cleanup (`post_service.rs`,
   `atompub/mapping.rs`).
3. Migrate the four `visibility.rs` enums onto `StrEnum`; delete the
   `macro_rules!`; fix the `Err(())` match in `storage/src/posts.rs`.
4. Verification: `validate --no-e2e`, `nextest -p macros`, coverage, acceptance
   greps.

**Key risks / decisions:**

- **Macros crate is coverage-measured** (ADR-0062's "no gate lines" is wrong).
  Task 1's derive error paths (non-enum, fielded variant, duplicate literal)
  must be hit by in-crate `syn::parse_quote!` unit tests; a `?`-fall-through
  brace gets a `// cov:ignore`. Model on `str_newtype.rs`'s existing in-`lib.rs`
  error-path tests + `macros/tests/str_newtype.rs`.
- **`Copy` on `PostFormat` trips `clippy::clone_on_copy`** at nine sites —
  `post_service.rs:510` (×1) and eight in `atompub/mapping.rs`
  (`wire_to_format_is_lenient`, lines 222/224/228/243×2/244×2/245) — fixed in
  Task 2; the `cargo xtask check` clippy gate (`--all-targets`) enforces none
  remain. `web/src/pages/posts.rs:703` is **not** a site (that clone is a
  `String`, not a `PostFormat`, until #498) — web stays out of scope.
- **Deleting the `macro_rules!` must land with all four conversions** in one
  commit (Task 3) — a lone deletion breaks the four uses.
- **Message preservation:** `PostFormat` keeps its exact message via
  `#[str_enum(error = "…")]`, so `render.rs`'s message test stays green; the
  auto message format is pinned by a `macros/tests` sample instead.
- Tasks 2 and 3 are independent (disjoint files) and each builds on Task 1.

**For agentic workers:** execute with `jaunder-iterate`, delegating a task to
`jaunder-dispatch` when useful. Tick the checkboxes here in real time.

## Global constraints

- No `Co-Authored-By` trailer. No new `#[allow]`/`#[expect]`/`crap:allow`;
  `cov:ignore` only for a genuinely-unreachable `?`-fall-through brace, with a
  one-line reason.
- Each task ends by running `cargo xtask check` clean, then commits
  (`jaunder-commit`). Adding a dep (none expected) triggers a full vendor
  rebuild — none here.
- Review base: `git diff wt-base-issue-562...HEAD`.

---

## Task 1 — the `StrEnum` derive (`macros/`)

**Files:** `macros/src/str_enum.rs` (new), `macros/src/lib.rs` (register),
`macros/tests/str_enum.rs` (new integration suite).

**Interfaces.** `macros/src/lib.rs`:

```rust
mod str_enum;

/// Derive the standard string-enum trailer: `as_str`/`Display`/`FromStr`/`TryFrom<&str>`
/// + a generated `Invalid<Name>` error; opt-in `#[str_enum(serde)]`; per-variant
/// `#[str_enum(rename = "…")]`; type-level `#[str_enum(error = "…")]`. See ADR (str-enum-trailer).
#[proc_macro_derive(StrEnum, attributes(str_enum))]
pub fn str_enum_derive(item: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(item as syn::DeriveInput);
    str_enum::expand(&input).into()
}
```

`macros/src/str_enum.rs` — `pub fn expand(input: &DeriveInput) -> TokenStream2`:

1. **Shape guard.** `Data::Enum` with ≥1 variant, every variant `Fields::Unit`.
   Else a spanned `syn::Error::new_spanned(..).to_compile_error()` ("StrEnum
   requires a unit-variant enum").
2. **Type-level opts** from `#[str_enum(...)]` on the enum: `serde: bool`,
   `error: Option<String>`. Unknown key → spanned error (mirror `str_newtype`'s
   `parse_opts`).
3. **Per variant**: read optional `#[str_enum(rename = "lit")]`; the wire
   literal is
   `rename.unwrap_or_else(|| variant_ident.to_string().to_lowercase())`. Collect
   `(ident, literal)`. **Duplicate literal across variants → spanned error.**
4. **Emit** (all always):

```rust
let err_name = quote::format_ident!("Invalid{}", name);
let message = opts.error.unwrap_or_else(|| format!("must be one of: {}", literals.join(", ")));
// as_str
impl #name { pub fn as_str(&self) -> &'static str { match self { #(Self::#idents => #lits,)* } } }
// Display via as_str
impl ::core::fmt::Display for #name { fn fmt(..) { f.write_str(self.as_str()) } }
// error: NumNewtype-style, no thiserror
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct #err_name;
impl ::core::fmt::Display for #err_name { fn fmt(..) { f.write_str(#message) } }
impl ::std::error::Error for #err_name {}
// FromStr + TryFrom<&str>, both -> Result<Self, #err_name>
impl ::core::str::FromStr for #name {
    type Err = #err_name;
    fn from_str(s: &str) -> Result<Self, #err_name> { match s { #(#lits => Ok(Self::#idents),)* _ => Err(#err_name) } }
}
impl ::core::convert::TryFrom<&str> for #name {
    type Error = #err_name;
    fn try_from(s: &str) -> Result<Self, #err_name> { s.parse() }
}
```

Under `opts.serde`, additionally emit `Serialize`
(`serialize_str(self.as_str())`) and `Deserialize`
(`let s = String::deserialize(d)?; s.parse().map_err(|e| serde::de::Error::custom(e))`
— owned `String`, fully-qualified paths as `NumNewtype` does).

**Tests.** `macros/tests/str_enum.rs` (integration — a real dependent crate):

```rust
use macros::StrEnum;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, StrEnum)]
#[str_enum(serde)]
enum Fmt { #[default] Markdown, Org, Html }

#[derive(Clone, Copy, PartialEq, Eq, Debug, StrEnum)]     // no serde, no default
enum Kind { Public, Subscribers, Named }

#[derive(Clone, Copy, PartialEq, Eq, Debug, StrEnum)]
#[str_enum(error = "bad fmt")]
enum WithMsg { A, #[str_enum(rename = "zee")] Z }

#[test] fn roundtrip_and_wire() {
    for f in [Fmt::Markdown, Fmt::Org, Fmt::Html] {
        assert_eq!(f.as_str().parse(), Ok(f));
        assert_eq!(serde_json::to_string(&f).unwrap(), format!("\"{}\"", f.as_str()));
        assert_eq!(serde_json::from_str::<Fmt>(&format!("\"{}\"", f.as_str())).unwrap(), f);
    }
    assert_eq!(Fmt::Markdown.as_str(), "markdown");
    assert_eq!(Kind::Subscribers.as_str(), "subscribers");
    assert_eq!("zee".parse::<WithMsg>(), Ok(WithMsg::Z));
}
#[test] fn rejects_unknown_with_auto_message() {
    let e = "xml".parse::<Fmt>().unwrap_err();
    assert_eq!(e.to_string(), "must be one of: markdown, org, html");   // pins AC 3b auto format
}
#[test] fn error_override() { assert_eq!("q".parse::<WithMsg>().unwrap_err().to_string(), "bad fmt"); }
#[test] fn default_and_deser_rejects() {
    assert_eq!(Fmt::default(), Fmt::Markdown);
    assert!(serde_json::from_str::<Fmt>("\"nope\"").is_err());
}
```

**In-crate `#[cfg(test)]` unit tests in `str_enum.rs`** — these are what the
coverage-measured `macros` crate actually instruments (the `macros/tests`
integration enums expand at _compile time_, inside the compiler, so their
execution is **not** counted). Call `expand(&syn::parse_quote!{ … })` at runtime
and assert on the emitted token string, modelled on **`num_newtype.rs`'s ~12
positive `num_newtype_*` tests** (not just `str_newtype.rs`'s error-path tests):

- **Positive branches** (each must be hit for coverage): serde on vs off
  (emitted tokens contain / omit `Serialize`); `rename` applied (`"zee"`
  appears); `error =` override vs the auto `"must be one of: …"` message; the
  `as_str`/`Display`/`FromStr`/`TryFrom` emission.
- **Error branches** (assert `.to_string().contains("compile_error")`): (a)
  applied to a struct; (b) an enum with a fielded variant; (c) two variants
  resolving to the same literal; (d) an unknown `#[str_enum(x)]` key. Generated
  code uses fully-qualified paths (`::core::fmt::Debug`,
  `::std::string::String`, …) as `num_newtype.rs` does, including the error
  type's `#[derive(...)]`.

**Steps:**

- [x] Write `str_enum.rs` (`expand` + opts parsing + shape guard).
- [x] Register the derive in `lib.rs`.
- [x] Add `macros/tests/str_enum.rs` (needs `serde`, `serde_json` as macros
      dev-deps — confirm/add, version-matched to the vendor).
- [x] Add the in-crate error-path unit tests (positive + error branches).
- [x] `cargo nextest run -p macros` — 70 passed. Confirm coverage of the error
      arms (a `?`-fall-through brace, if any, gets `// cov:ignore` with a
      reason).
- [x] `cargo xtask check` clean → commit `69e70239`
      (`feat(macros): StrEnum derive for string enums (#562)`).

---

## Task 2 — migrate `PostFormat` (`common/src/render.rs`)

**Files:** `common/src/render.rs`, `storage/src/post_service.rs`,
`server/src/atompub/mapping.rs`.

**Interfaces.** Replace the hand-written enum + `Display` + `FromStr` +
`struct InvalidPostFormat` with:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, macros::StrEnum)]
#[str_enum(serde)]
#[str_enum(error = "post format must be \"markdown\", \"org\", or \"html\"")]
pub enum PostFormat {
    #[default]
    Markdown,
    Org,
    Html,
}
```

The macro re-emits `InvalidPostFormat` (same name/path), `Display`, `FromStr`,
`TryFrom<&str>`, `as_str`, and serde. Keep the doc comment on the enum (the
variants lose their per-variant docs — consistent with the visibility enums).

- `common/src/render.rs` tests: `post_format_rejects_invalid_value` (`:489`)
  stays green (message preserved); the `"markdown".parse()` round-trip tests
  stay green (`FromStr` preserved). Add nothing new here — the derive is
  unit-tested in `macros`.
- **`Copy` cleanup (nine sites):** `storage/src/post_service.rs:510` (×1) and
  the eight `d.clone()` in `server/src/atompub/mapping.rs`
  (`wire_to_format_is_lenient`, lines 222/224/228/243×2/244×2/245) → replace
  each with a plain copy. (`web/src/pages/posts.rs:703` is a `String` clone, not
  `PostFormat` — untouched; web is #498.)

**Steps:**

- [x] Convert `PostFormat` to the derive; delete the hand-written
      `impl Display`, `impl FromStr`, and `struct InvalidPostFormat`.
- [x] Fix the nine `clone_on_copy` sites (`post_service.rs` ×1,
      `atompub/mapping.rs` ×8); `format_to_wire` takes `PostFormat` by value.
- [x] `cargo nextest run -p common post_format` and `-p storage`
      (format-touching) — PASS (via full `cargo xtask check`).
- [x] `cargo xtask check` clean (clippy `--all-targets` proves no stray
      `clone_on_copy`) → commit `a22be84f`
      (`refactor(common): PostFormat rides the StrEnum trailer (#562)`).

---

## Task 3 — migrate the four `visibility.rs` enums + delete the macro

**Files:** `common/src/visibility.rs`, `storage/src/posts.rs`.

**Interfaces.** Delete `macro_rules! str_enum` and its four invocations; declare
the enums directly with the derive (std-derive lists pinned per spec):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, StrEnum)]
pub enum Channel { Local }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, StrEnum)]
pub enum SubscriptionStatus { Active, Pending, Blocked }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, StrEnum)]
pub enum TargetKind { Public, Subscribers, Named }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, StrEnum)]
#[str_enum(serde)]
pub enum AudienceBase { #[default] Private, Public, Subscribers }
```

All wire literals are the lowercase default → no `rename` attrs needed. The
existing `visibility.rs` tests (`display_matches_as_str`,
`audience_base_deserializes_from_literal`,
`audience_base_deserialize_rejects_unknown`) stay green — the derive supplies
`as_str`, `Display`, `TryFrom`, serde, and the error derives `PartialEq + Debug`
for their `assert_eq!(try_from(..), Ok(..))`.

- **`storage/src/posts.rs:1842`**: `Err(()) => None` → `Err(_) => None` (the new
  error is a struct, not `()`).

**Steps:**

- [x] Replace the four `str_enum!` invocations with explicit
      `#[derive(StrEnum)]` enums; delete the `macro_rules! str_enum` definition.
- [x] Fix `storage/src/posts.rs:1842`.
- [x] `cargo nextest run -p common visibility` and `-p storage` — PASS (via full
      check).
- [x] `cargo xtask check` clean → commit `b8b7918c`
      (`refactor(common): visibility enums ride the StrEnum trailer (#562)`).

---

## Task 4 — verification

No source changes expected.

**Steps:**

- [x] `rg 'macro_rules! str_enum' common/` empty;
      `rg 'InvalidPostFormat' common/src/render.rs` shows the hand-written
      `impl`/`struct` is gone.
- [x] `cargo xtask validate --no-e2e --allow-dirty` — PASS (untracked planning
      docs only; source all committed).
- [x] `cargo nextest run -p macros` — PASS (70 tests: derive suite + units).
- [x] Confirm no new `#[allow]`/`crap:allow`; the only `cov:ignore` added are
      the two justified `?`-fall-through braces in `str_enum.rs` (mirroring
      `num_newtype`).

## Self-review

- No placeholders; each task names concrete files, signatures, `cargo` commands.
- Spec acceptance → task map: §Acc1 → T1+T3(delete); 2 → T1 (unit) + T2/T3 (real
  enums); 3 → T2/T3 + existing web test; 3b → T1 (auto msg) + T2 (preserved
  msg); 3c → T3; 4 → T2; 5 → T1; 6 → T2; 7 → T4; 8 → ADR draft (already
  written), promoted at ship.
- Separable concerns: none — the ADR is written inline; #498 is the downstream
  consumer, already filed and blocked-by this.
