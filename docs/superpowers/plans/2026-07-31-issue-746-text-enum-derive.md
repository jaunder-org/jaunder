# `#[text_enum]` Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make one attribute own the closed-string-enum convention, and leave
exactly one sqlx bridge implementation in the repo.

**Architecture:** `macros/src/sqlx_bridge.rs::bridge()` becomes the single
codegen, parameterised by a `BridgeSpec` with independent type/encode/decode
inners. Four callers use it: the three newtype derives, a new bridge-only
`SqlxBridge` derive, and a new `#[text_enum]` **attribute** macro that injects
strum's derives and generates the named error, serde, and (opt-in) the bridge.

**Tech Stack:** Rust 2021, `syn`/`quote`/`proc-macro2`, `strum` 0.28, `sqlx`
0.8.6 (SQLite + Postgres), `cargo nextest`, `cargo xtask`.

**Spec:**
[`docs/superpowers/specs/2026-07-31-issue-746-text-enum-derive.md`](../specs/2026-07-31-issue-746-text-enum-derive.md).
The plan is "how"; the spec is "what/why". Decisions are cited as **D1**…**D12**
and acceptance criteria as **AC-n** — do not re-derive them here.

## Global Constraints

- **Wire and column representation must not change** for any of the eight enums.
  A test needing an edit to pass means the change is wrong (AC-22).
- **No `where` clause under `storage/` may be edited** (AC-8). If one seems
  necessary, D2a or D3 is wrong — stop, don't edit `storage/`.
- `#[text_enum]` must be the **first** attribute on its item (D1a).
- Injected derives are **path-qualified** (`::strum::AsRefStr`, …); `strum`
  becomes a required dependency of adopting crates (D1b).
- The generated error must **not** require `thiserror` — hand-write `Display` +
  `std::error::Error`, following `num_newtype.rs:114-129`. **Match that
  precedent exactly, including its fully-qualified derive list**
  (`num_newtype.rs:117` renders
  `#[derive(::core::fmt::Debug, ::core::clone::Clone, ::core::marker::Copy, ::core::cmp::PartialEq, ::core::cmp::Eq)]`).
- **Token access is spelled
  `<&#name as ::core::convert::Into<&'static str>>::into(self)`** — the enum's
  concrete ident, fully-qualified `Into`, never `Self` and never a bare `Into`.
  Tasks 6 and 7 both depend on this being uniform.
- Per-commit gate: `devtool run -- cargo xtask check` before every commit
  (**`jaunder-commit`**). **No `Co-Authored-By` trailer.**
- `macros/` has no `sqlx` dependency (its `sqlx` feature is bare so the cfg
  resolves — `macros/Cargo.toml:15-20`), and `common`'s `sqlx` feature is **off
  by default** (`common/Cargo.toml:39`). So bridge output is verified two ways:
  rendered-token assertions in `macros/`, **and**
  `cargo check -p common --features sqlx` in any task that changes bridge
  codegen. `cargo xtask check` alone does not compile it.

## Scope

**In:** `BridgeSpec`; `StrNewtype` decode fix (24 types); `SqlxBridge` derive +
`RenderedHtml`; the `text_enum` attribute macro; migration of all eight enums;
deletion of `db_enum.rs` and `strum_enum.rs`; nine prose sites; a new ADR
draft + four ADR-0075 amendments; the issue body.

**Out:** #758 (`PostTitle`), #759 (TEXT gate) — both filed, both blocked by
this.

## Tasks

- [x] 1. `BridgeSpec` — parameterise `bridge()`, four callers semantically
     unchanged — `d7c6f0d9`
- [x] 2. `StrNewtype` validating kinds decode `&'r str` — `c6c1a226`
- [x] 3. `SqlxBridge` derive + `RenderedHtml` adopts it — `968685ac`
- [x] 4-7. `text_enum` — shape guard, options, injection, named error, serde,
      and the sqlx bridge — `0c1598c2`. **Landed as one commit, not four.**
      Splitting leaves `Opts`' fields dead until the last piece exists, and
      `-D warnings` rejects that; the alternatives were an `#[allow(dead_code)]`
      (needs user approval) or filler written to satisfy a linter.
- [x] 8. Migrate `PostFormat` + `MediaSource`; delete `db_enum.rs` — `1e224f11`
- [x] 9. Migrate `AudienceBase` + `RegistrationPolicy` — `bb6d4ab9`. Also
     retired `impl_string_serde_proxy!`, which the plan had dying later: it had
     exactly four users and this took the last two, so clippy refused it as
     dead.
- [x] 10. Migrate `BackupMode` (D11) — `216944bf`
- [x] 11. Migrate `Channel`/`SubscriptionStatus`/`TargetKind`; delete
      `strum_enum.rs` (D12) — `97e9ab00`
- [x] 12-13. Prose sites, ADR-0075 amendments, ADR draft, issue body —
      `72c51f2d`. Only three prose sites needed work;
      `macros/src/sqlx_bridge.rs`'s module doc was already rewritten in Task 1.
      The ADR draft is **uncommitted by design** — `docs/adr/drafts/` is
      gitignored and `cargo xtask adr promote` numbers and stages it at ship.

**Plan defect found during execution:** Task 11's verification named
`cargo nextest run -p common -p storage`, which cannot pass outside the Nix
harness — `storage`'s postgres cases need a provisioned server, so all six
failures were `ConnectionRefused`. `cargo xtask check` is the command that runs
them.

**Key risks:** Task 5's injection is the novel mechanism — a uniform derive left
above the attribute collides with `E0119`/`E0592` (D1a). Task 2 changes the
generated bound on 24 public types; only a full two-backend `validate` proves it
(AC-23). Task 11 gives three FK-normalized enums a wire surface they lack today
— an accepted cost (D12).

**Per-task prose rule:** Tasks 8-11 each rewrite the doc comment on the enums
they migrate. Task 12 then handles only the sites no migration task touches.
Don't leave prose stale across tasks — the enum doc comments sit _inside_ the
ranges being edited.

---

### Task 1: `BridgeSpec` — parameterise `bridge()`

**Files:**

- Modify: `macros/src/sqlx_bridge.rs` (whole file)
- Modify: `macros/src/str_newtype.rs:303-329`, `macros/src/id_newtype.rs:88-96`,
  `macros/src/num_newtype.rs:98-109`
- Test: `macros/src/sqlx_bridge.rs`, `macros/src/id_newtype.rs`,
  `macros/src/num_newtype.rs` — in-file `#[cfg(test)]`

**Interfaces:**

- Produces:
  `pub(crate) struct BridgeSpec<'a> { name: &'a syn::Ident, type_inner: TokenStream, encode_inner: TokenStream, to_inner: TokenStream, decode_inner: TokenStream, convert: TokenStream }`
  and `pub(crate) fn bridge(spec: &BridgeSpec<'_>) -> TokenStream`. `to_inner`
  evaluates to `&#encode_inner`; `convert` uses local `v: #decode_inner` and
  evaluates to `Result<Self, ::sqlx::error::BoxDynError>`. Tasks 2, 3 and 7
  consume this.

- [ ] **Step 1: Write the failing tests**

In `macros/src/sqlx_bridge.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use quote::format_ident;

    /// Whitespace-stripped rendering, so assertions survive `TokenStream`'s spacing.
    /// Note: this also strips spaces *inside* string literals.
    pub(crate) fn norm(t: &TokenStream) -> String {
        t.to_string().chars().filter(|c| !c.is_whitespace()).collect()
    }

    fn spec_for(name: &syn::Ident) -> BridgeSpec<'_> {
        BridgeSpec {
            name,
            type_inner: quote! { ::std::string::String },
            encode_inner: quote! { ::std::string::String },
            to_inner: quote! { &self.0 },
            decode_inner: quote! { ::std::string::String },
            convert: quote! { ::core::result::Result::Ok(#name(v)) },
        }
    }

    #[test]
    fn type_impl_delegates_to_type_inner() {
        let n = format_ident!("X");
        let out = norm(&bridge(&spec_for(&n)));
        assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::type_info()"));
        assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::compatible(ty)"));
    }

    #[test]
    fn encode_binds_an_annotated_local_and_keeps_size_hint() {
        let n = format_ident!("X");
        let out = norm(&bridge(&spec_for(&n)));
        assert!(out.contains("letinner:&::std::string::String=&self.0;"));
        assert!(out.contains("::encode_by_ref(inner,buf)"));
        assert!(out.contains("fnsize_hint(&self)->usize"), "size_hint must be emitted");
        assert!(out.contains("::size_hint(inner)"));
    }

    #[test]
    fn decode_delegates_to_decode_inner_then_converts() {
        let n = format_ident!("X");
        let out = norm(&bridge(&spec_for(&n)));
        assert!(out.contains("letv=<::std::string::Stringas::sqlx::Decode<'r,DB>>::decode(value)?;"));
        assert!(out.contains("::core::result::Result::Ok(X(v))"));
    }

    #[test]
    fn the_three_inners_are_independent() {
        let n = format_ident!("X");
        let out = norm(&bridge(&BridgeSpec {
            name: &n,
            type_inner: quote! { ::std::string::String },
            encode_inner: quote! { &'q str },
            to_inner: quote! { &"tok" },
            decode_inner: quote! { &'r str },
            convert: quote! { ::core::result::Result::Ok(X) },
        }));
        assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::type_info()"));
        assert!(out.contains("<&'qstras::sqlx::Encode<'q,DB>>::encode_by_ref(inner,buf)"));
        assert!(out.contains("<&'rstras::sqlx::Decode<'r,DB>>::decode(value)?"));
    }

    #[test]
    fn output_is_feature_gated_and_marked_derived() {
        let n = format_ident!("X");
        let out = norm(&bridge(&spec_for(&n)));
        assert!(out.contains("#[cfg(feature=\"sqlx\")]"));
        assert_eq!(out.matches("#[automatically_derived]").count(), 3);
    }
}
```

In `macros/src/id_newtype.rs` — AC-9 and AC-12 require the `i64` mapping be
_pinned_, not merely stated in a table:

```rust
#[test]
fn id_bridge_uses_i64_for_all_three_inners_and_wraps_infallibly() {
    let n = quote::format_ident!("UserId");
    let out = crate::sqlx_bridge::tests::norm(&sqlx_impls(&n));
    assert!(out.contains("<i64as::sqlx::Type<DB>>::type_info()"));
    assert!(out.contains("letinner:&i64=&self.0;"));
    assert!(out.contains("<i64as::sqlx::Decode<'r,DB>>::decode(value)?"));
    assert!(out.contains("::core::result::Result::Ok(UserId(v))"));
    assert!(!out.contains("str"), "an id never touches the string path");
}
```

In `macros/src/num_newtype.rs`:

```rust
#[test]
fn num_bridge_uses_the_declared_inner_for_all_three_and_checks_bounds() {
    let n = quote::format_ident!("FeedMinItems");
    let inner = quote! { i32 };
    let out = crate::sqlx_bridge::tests::norm(&sqlx_impls(&n, &inner));
    assert!(out.contains("<i32as::sqlx::Type<DB>>::type_info()"));
    assert!(out.contains("letinner:&i32=&self.0;"));
    assert!(out.contains("<i32as::sqlx::Decode<'r,DB>>::decode(value)?"));
    assert!(out.contains("<FeedMinItemsas::core::convert::TryFrom<i32>>::try_from(v)?"));
}
```

(Both call `sqlx_bridge::tests::norm`; mark that module `pub(crate)` under
`#[cfg(test)]`. Adjust the second test's call to `sqlx_impls`' real signature.)

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p macros` Expected: FAIL — `BridgeSpec`
not defined.

- [ ] **Step 3: Implement against the tests**

Rewrite `bridge()` to the **Interfaces** signature, emitting the three impls
exactly as spec **D2** shows — `Type` bounded on `type_inner`; `Encode` on
`encode_inner` using the annotated local in **both** `encode_by_ref` and
`size_hint`; `Decode` on `decode_inner`. Keep `#[cfg(feature = "sqlx")]`, the
`const _: () = { … }` wrapper, and `#[automatically_derived]` on all three.

Update the four call sites to pass semantically identical specs — this task
changes **no** generated semantics (AC-12):

| caller                               | `type`/`encode`/`decode` inner | `to_inner` | `convert`                                                              |
| ------------------------------------ | ------------------------------ | ---------- | ---------------------------------------------------------------------- |
| `str_newtype::sqlx_impls`            | `::std::string::String`        | `&self.0`  | `Ok(<#name as ::core::str::FromStr>::from_str(&v)?)`                   |
| `str_newtype::sqlx_impls_infallible` | `::std::string::String`        | `&self.0`  | `Ok(<#name as ::core::convert::From<::std::string::String>>::from(v))` |
| `id_newtype`                         | `i64`                          | `&self.0`  | `Ok(#name(v))`                                                         |
| `num_newtype`                        | `#inner`                       | `&self.0`  | `Ok(<#name as ::core::convert::TryFrom<#inner>>::try_from(v)?)`        |

- [ ] **Step 4: Run the tests, then compile the generated code**

Run: `devtool run -- cargo nextest run -p macros` Expected: PASS, including the
existing `str_newtype`/`id_newtype` integration tests unchanged.

Run: `devtool run -- cargo check -p common --features sqlx` Expected: PASS.
**Required** — this task rewrites the codegen for all four families, and
`common`'s `sqlx` feature is off by default, so nothing above actually compiles
a bridge.

- [ ] **Step 5: Commit**

```bash
git add macros/src/sqlx_bridge.rs macros/src/str_newtype.rs macros/src/id_newtype.rs macros/src/num_newtype.rs
git commit -m "refactor(macros): parameterise the sqlx bridge with a BridgeSpec"
```

---

### Task 2: `StrNewtype` validating kinds decode `&'r str`

**Files:**

- Modify: `macros/src/str_newtype.rs:313-318` (`sqlx_impls` only — **not**
  `sqlx_impls_infallible`)
- Test: `macros/src/str_newtype.rs` in-file `#[cfg(test)]`

**Interfaces:**

- Consumes: `BridgeSpec` (Task 1).
- Produces: nothing new. 24 types change their generated `Decode` bound from
  `String: Decode<'r, DB>` to `&'r str: Decode<'r, DB>` (D3).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn validating_bridge_decodes_a_borrowed_str_without_allocating() {
    let n = quote::format_ident!("Slug");
    let out = crate::sqlx_bridge::tests::norm(&sqlx_impls(&n));
    assert!(out.contains("<&'rstras::sqlx::Decode<'r,DB>>::decode(value)?"));
    assert!(out.contains("::from_str(v)?"), "must parse the borrowed str directly");
    assert!(!out.contains("::from_str(&v)"), "the &v form re-borrows an owned String");
    assert!(!out.contains("to_owned"));
}

#[test]
fn validating_bridge_keeps_string_for_type_and_encode() {
    let n = quote::format_ident!("Slug");
    let out = crate::sqlx_bridge::tests::norm(&sqlx_impls(&n));
    assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::type_info()"));
    assert!(out.contains("letinner:&::std::string::String=&self.0;"));
}

#[test]
fn infallible_bridge_is_untouched_on_all_three_inners() {
    // AC-11 + D3: PostBody's From<String> MOVES the value; borrowing would ADD an
    // allocation. This test is the standing guard that the #758 boundary is not crossed.
    let n = quote::format_ident!("PostBody");
    let out = crate::sqlx_bridge::tests::norm(&sqlx_impls_infallible(&n));
    assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::type_info()"));
    assert!(out.contains("letinner:&::std::string::String=&self.0;"));
    assert!(out.contains("<::std::string::Stringas::sqlx::Decode<'r,DB>>::decode(value)?"));
    assert!(!out.contains("&'rstras::sqlx::Decode"));
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p macros str_newtype` Expected: FAIL on
the first two; the third already passes and must keep passing.

- [ ] **Step 3: Implement against the tests**

In `sqlx_impls` only, set `decode_inner: quote! { &'r str }` and
`convert: quote! { ::core::result::Result::Ok(<#name as ::core::str::FromStr>::from_str(v)?) }`.
Leave `type_inner`/`encode_inner`/`to_inner` as `String`/`String`/`&self.0`
(AC-11), and leave `sqlx_impls_infallible` untouched.

- [ ] **Step 4: Run the tests, then prove it across both backends**

Run: `devtool run -- cargo nextest run -p macros` Expected: PASS.

Run: `devtool run -- cargo xtask validate --no-e2e` Expected: PASS. This is the
real gate — 24 public types changed their bound, including `InviteCode` in
`host/` (D3). If anything under `storage/` needs a `where` edit, **stop**: AC-8
says D2a or D3 is wrong.

- [ ] **Step 5: Commit**

```bash
git add macros/src/str_newtype.rs
git commit -m "perf(macros): decode string newtypes from a borrowed str"
```

---

### Task 3: `SqlxBridge` derive + `RenderedHtml`

**Files:**

- Create: `macros/src/sqlx_bridge_derive.rs`
- Modify: `macros/src/lib.rs` (register + export the derive)
- Modify: `common/src/render.rs` — delete the hand-written block at 276-323, add
  the derive, retain and re-point the rationale prose
- Test: `macros/src/sqlx_bridge_derive.rs` in-file `#[cfg(test)]`

**Interfaces:**

- Consumes: `BridgeSpec` (Task 1);
  `crate::require_newtype_shape(input: &DeriveInput, macro_name: &str, example: &str) -> syn::Result<()>`
  (`macros/src/lib.rs:237-241`).
- Produces: `#[derive(SqlxBridge)]` for a single-field tuple struct. All three
  inners = the field's type; `to_inner = &self.0`; `convert = Ok(Self(v))`. No
  options.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn emits_only_the_three_bridge_impls() {
    let input: syn::DeriveInput = syn::parse_quote! { pub struct RenderedHtml(String); };
    let out = crate::sqlx_bridge::tests::norm(&expand(&input));
    assert!(out.contains("::sqlx::Type<DB>forRenderedHtml"));
    assert!(out.contains("::sqlx::Encode<'q,DB>forRenderedHtml"));
    assert!(out.contains("::sqlx::Decode<'r,DB>forRenderedHtml"));
    for forbidden in ["FromStr", "TryFrom", "Deserialize", "Serialize", "Display", "Deref"] {
        assert!(!out.contains(forbidden), "{forbidden} must not be emitted");
    }
    assert!(!out.contains("From<::std::string::String>forRenderedHtml"));
}

#[test]
fn all_three_inners_are_the_field_type_and_decode_moves() {
    let input: syn::DeriveInput = syn::parse_quote! { pub struct RenderedHtml(String); };
    let out = crate::sqlx_bridge::tests::norm(&expand(&input));
    assert!(out.contains("<Stringas::sqlx::Type<DB>>::type_info()"));
    assert!(out.contains("letinner:&String=&self.0;"));
    assert!(out.contains("<Stringas::sqlx::Decode<'r,DB>>::decode(value)?"));
    assert!(out.contains("::core::result::Result::Ok(Self(v))"));
}

#[test]
fn wrong_shape_is_a_spanned_error_naming_the_derive() {
    let input: syn::DeriveInput = syn::parse_quote! { pub enum E { A } };
    let out = expand(&input).to_string();
    assert!(out.contains("compile_error"));
    assert!(out.contains("SqlxBridge"), "the message must name the derive");
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p macros sqlx_bridge_derive` Expected:
FAIL — module does not exist.

- [ ] **Step 3: Implement against the tests**

`expand(&DeriveInput) -> TokenStream`: call
`require_newtype_shape(input, "SqlxBridge", "struct X(Inner)")`, returning
`e.to_compile_error()` on error as `str_newtype.rs:38-39` does (AC-14 —
unit-tested, **not** a `compile_fail` doctest; `macros/src/lib.rs:259-263`
records that those are invisible to coverage). Otherwise read the single field's
type and call `bridge()` with all three inners set to it, `to_inner = &self.0`,
`convert = Ok(Self(v))`. Register with `#[proc_macro_derive(SqlxBridge)]` in
`lib.rs`.

- [ ] **Step 4: Migrate `RenderedHtml`, run, and compile the bridge**

Delete `common/src/render.rs:276-323`. Add `macros::SqlxBridge` to
`RenderedHtml`'s derive list. **Keep every line of rationale** (AC-17): the
block comment at 264-275 (rejected sanitizing decode, its revisit condition,
#701), the `Self(..)`/door reasoning from 301-304, and the
`compatible`-delegation note from 285-291 — all re-pointed at the derive. Add
the derive's own doc comment warning that its `Decode` is an **inbound door
re-establishing no invariant** (D10).

Run: `devtool run -- cargo nextest run -p macros -p common` Expected: PASS,
including `RenderedHtml`'s `compile_fail` doctests at `render.rs:98-104`
(AC-16).

Run: `devtool run -- cargo check -p common --features sqlx` Expected: PASS.
**Required** — the previous step does not build the replaced impls.

- [ ] **Step 5: Commit**

```bash
git add macros/src/sqlx_bridge_derive.rs macros/src/lib.rs common/src/render.rs
git commit -m "refactor(macros): add a bridge-only SqlxBridge derive, adopt it in RenderedHtml"
```

---

### Task 4: `text_enum` — shape guard and option parsing

**Files:**

- Create: `macros/src/text_enum.rs`
- Modify: `macros/src/lib.rs` (register `#[proc_macro_attribute]`; add
  `require_enum_shape` beside `require_newtype_shape`)
- Test: `macros/src/text_enum.rs` in-file, and `macros/src/lib.rs` for the
  helper

**Interfaces:**

- Produces:
  `pub(crate) fn expand(attr: TokenStream, item: TokenStream) -> TokenStream`,
  parsing `{ error: syn::Ident, message: syn::LitStr, sqlx: bool }`. At this
  task it validates and re-emits the item **unchanged**; Tasks 5-7 add output.
  Also
  `crate::require_enum_shape(input: &DeriveInput, macro_name: &str, example: &str) -> syn::Result<()>`,
  rejecting non-enums and any non-unit variant.

- [ ] **Step 1: Write the failing tests**

In `macros/src/lib.rs`, beside the `require_newtype_shape` tests at 269-285 —
AC-2 wants the helper tested directly, as its sibling is:

```rust
#[test]
fn require_enum_shape_rejects_a_struct() {
    let input: syn::DeriveInput = syn::parse_quote! { struct S(String); };
    assert!(require_enum_shape(&input, "text_enum", "enum X { A }").is_err());
}

#[test]
fn require_enum_shape_rejects_a_non_unit_variant() {
    let input: syn::DeriveInput = syn::parse_quote! { enum X { A(u8) } };
    assert!(require_enum_shape(&input, "text_enum", "enum X { A }").is_err());
}

#[test]
fn require_enum_shape_accepts_a_unit_enum() {
    let input: syn::DeriveInput = syn::parse_quote! { enum X { A, B } };
    assert!(require_enum_shape(&input, "text_enum", "enum X { A }").is_ok());
}
```

In `macros/src/text_enum.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    pub(crate) use crate::sqlx_bridge::tests::norm;

    /// Same normalization for a literal needle written in source form.
    pub(crate) fn norm_s(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    /// Parse `attr` and `item` from source text and run the expansion.
    pub(crate) fn expand_str(attr: &str, item: &str) -> TokenStream {
        expand(attr.parse().expect("attr must lex"), item.parse().expect("item must lex"))
    }

    #[test]
    fn missing_or_unpaired_options_are_spanned_errors() {
        for attr in [
            "",
            "error = InvalidX",
            r#"message = "bad""#,
            r#"error = InvalidX, message = "b", bogus"#,
        ] {
            let out = expand_str(attr, "pub enum X { A }").to_string();
            assert!(out.contains("compile_error"), "attr {attr:?} must be rejected");
        }
    }

    #[test]
    fn const_into_str_is_rejected() {
        // D4: it suppresses the From<&X> for &'static str the Encode side needs.
        let out = expand_str(
            r#"error = InvalidX, message = "b""#,
            r#"#[strum(const_into_str)] pub enum X { A }"#,
        ).to_string();
        assert!(out.contains("compile_error"));
        assert!(out.contains("const_into_str"));
    }

    #[test]
    fn wrong_shape_is_a_spanned_error_naming_the_macro() {
        for item in ["pub struct S(String);", "pub enum X { A(u8) }"] {
            let out = expand_str(r#"error = InvalidX, message = "b""#, item).to_string();
            assert!(out.contains("compile_error"));
            assert!(out.contains("text_enum"));
        }
    }

    #[test]
    fn a_valid_invocation_re_emits_the_item_intact() {
        let out = norm(&expand_str(
            r#"sqlx, error = InvalidX, message = "bad x""#,
            r#"#[derive(Clone)] #[strum(serialize_all = "snake_case")] pub enum X { A, B }"#,
        ));
        assert!(out.contains("pubenumX{A,B,}") || out.contains("pubenumX{A,B}"));
        assert!(out.contains("#[derive(Clone)]"));
        assert!(out.contains("serialize_all=\"snake_case\""));
        assert!(!out.contains("compile_error"));
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p macros` Expected: FAIL —
`require_enum_shape` and the module do not exist.

- [ ] **Step 3: Implement against the tests**

Write `require_enum_shape` mirroring `require_newtype_shape`'s shape and error
style. Write `expand` to parse the three options (`error`/`message` mandatory
and paired, `sqlx` a bare flag, anything else a spanned error), run the shape
guard, reject `#[strum(const_into_str)]` on the item, and otherwise re-emit the
item verbatim. Every branch is pinned by a Step 1 test.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p macros` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add macros/src/text_enum.rs macros/src/lib.rs
git commit -m "feat(macros): add the text_enum attribute with shape and option validation"
```

---

### Task 5: `text_enum` — derive injection and the named error

**Files:**

- Modify: `macros/src/text_enum.rs`
- Modify: `macros/Cargo.toml` — add `strum = { workspace = true }` to
  `[dev-dependencies]`. **Workspace inheritance is required**: `Cargo.toml:80`
  carries `features = ["derive"]`, and a bare `strum = "0.28"` would drop it so
  `::strum::AsRefStr` would not resolve. Matches `macros/Cargo.toml:23` and
  `common/Cargo.toml:21`.
- Create: `macros/tests/text_enum.rs`

**Interfaces:**

- Consumes: Task 4's `expand` and options.
- Produces: the four injected derives, the injected
  `#[strum(parse_err_ty, parse_err_fn)]` pair, the public unit error type, and
  its private parse fn.

- [ ] **Step 1: Write the failing tests**

In `macros/src/text_enum.rs`:

```rust
#[test]
fn injects_the_four_uniform_derives_path_qualified() {
    let out = norm(&expand_str(r#"error = InvalidX, message = "bad x""#, "pub enum X { A, B }"));
    for d in ["::strum::AsRefStr", "::strum::Display", "::strum::EnumString", "::strum::IntoStaticStr"] {
        assert!(out.contains(&norm_s(d)), "{d} must be injected, path-qualified");
    }
}

#[test]
fn injects_the_strum_parse_err_pair_naming_the_declared_error() {
    let out = norm(&expand_str(r#"error = InvalidX, message = "bad x""#, "pub enum X { A }"));
    assert!(out.contains("parse_err_ty=InvalidX"));
    assert!(out.contains("parse_err_fn="));
}

#[test]
fn generates_a_unit_error_matching_the_num_newtype_precedent() {
    let out = norm(&expand_str(r#"error = InvalidX, message = "bad x""#, "pub enum X { A }"));
    // Shape per AC-20 and num_newtype.rs:114-129 — fully-qualified derives, no thiserror.
    assert!(out.contains("pubstructInvalidX;"), "must be a bare unit struct");
    assert!(out.contains(&norm_s(
        "#[derive(::core::fmt::Debug, ::core::clone::Clone, ::core::marker::Copy, \
          ::core::cmp::PartialEq, ::core::cmp::Eq)]"
    )));
    assert!(out.contains("::core::fmt::DisplayforInvalidX"));
    assert!(out.contains("::std::error::ErrorforInvalidX"));
    assert!(!out.contains("thiserror"), "must not require a thiserror dependency");
    assert!(out.contains("\"badx\""), "norm strips the space inside the literal");
}

#[test]
fn preserves_author_attributes_and_derives() {
    let out = norm(&expand_str(
        r#"error = InvalidX, message = "bad x""#,
        r#"#[derive(Clone, Copy, ::strum::VariantArray)]
           #[strum(serialize_all = "snake_case")]
           pub enum X { A }"#,
    ));
    assert!(out.contains("::strum::VariantArray"));
    assert!(out.contains("serialize_all=\"snake_case\""));
}

#[test]
fn does_not_duplicate_a_uniform_derive_written_below_the_attribute() {
    let out = norm(&expand_str(
        r#"error = InvalidX, message = "bad x""#,
        "#[derive(::strum::Display)] pub enum X { A }",
    ));
    assert_eq!(out.matches("::strum::Display").count(), 1, "AC-5: no duplicate");
}
```

In `macros/tests/text_enum.rs` — the real-strum compile proof:

```rust
#[macros::text_enum(error = InvalidColour, message = "colour must be \"red\" or \"blue\"")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[strum(serialize_all = "snake_case")]
pub enum Colour { Red, Blue }

#[test]
fn token_round_trips_through_the_injected_derives() {
    assert_eq!(Colour::Red.as_ref(), "red");
    assert_eq!(Colour::Red.to_string(), "red");
    assert_eq!("blue".parse::<Colour>(), Ok(Colour::Blue));
    let s: &'static str = (&Colour::Blue).into();
    assert_eq!(s, "blue");
}

#[test]
fn parse_failure_carries_the_declared_message() {
    let err = "green".parse::<Colour>().unwrap_err();
    assert_eq!(err, InvalidColour);
    assert_eq!(err.to_string(), "colour must be \"red\" or \"blue\"");
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p macros text_enum` Expected: FAIL —
nothing is injected or generated yet.

- [ ] **Step 3: Implement against the tests**

Add the four uniform derives **path-qualified**, skipping any already present
among the attributes the macro can see (D1a: only those below it). Append
`#[strum(parse_err_ty = #error, parse_err_fn = #generated_fn)]`. Emit the unit
error struct per the `num_newtype.rs:114-129` precedent, and the private
`fn #generated_fn(_: &str) -> #error`. Name that fn from the enum in snake_case
with a leading `__` so it cannot collide with hand-written code.

Write the macro's doc comment to state **two** things (AC-4a, AC-6):

1. `#[text_enum]` must be the item's **first** attribute, and the real failure
   if it is not — a duplicate-impl compile error (`E0119`/`E0592`) from an
   invisible derive above it. Do **not** write "injection will not run"; that
   was disproved.
2. An adopting crate must depend on `strum` **under that name**, because the
   injected derives are path-qualified as `::strum::…`. Without it the error is
   "cannot find derive macro in this scope", which points nowhere useful.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p macros` Expected: PASS, including the
new integration test compiling against real strum.

- [ ] **Step 5: Commit**

```bash
git add macros/src/text_enum.rs macros/Cargo.toml macros/tests/text_enum.rs
git commit -m "feat(macros): inject the strum derives and generate the named error"
```

---

### Task 6: `text_enum` — serde

**Files:**

- Modify: `macros/src/text_enum.rs`
- Test: same file's `#[cfg(test)]`, plus `macros/tests/text_enum.rs`

**Interfaces:**

- Produces: unconditional `Serialize`/`Deserialize`, reading the token via the
  Global Constraints spelling
  `<&#name as ::core::convert::Into<&'static str>>::into(self)` and writing back
  through `FromStr` (D5).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn serialize_writes_the_static_token_without_allocating() {
    let out = norm(&expand_str(r#"error = InvalidX, message = "b""#, "pub enum X { A }"));
    assert!(out.contains("serializer.serialize_str"));
    assert!(out.contains("<&Xas::core::convert::Into<&'staticstr>>::into(self)"));
    assert!(!out.contains("to_owned"));
    assert!(!out.contains("clone()"));
}

#[test]
fn deserialize_routes_an_owned_string_through_from_str() {
    let out = norm(&expand_str(r#"error = InvalidX, message = "b""#, "pub enum X { A }"));
    assert!(out.contains("<::std::string::Stringas::serde::Deserialize>::deserialize(deserializer)?"));
    assert!(out.contains("::from_str(&s).map_err(::serde::de::Error::custom)"));
}
```

In `macros/tests/text_enum.rs`:

```rust
#[test]
fn serde_round_trips_the_token_and_reports_the_declared_message() {
    assert_eq!(serde_json::to_string(&Colour::Red).unwrap(), "\"red\"");
    assert_eq!(serde_json::from_str::<Colour>("\"blue\"").unwrap(), Colour::Blue);
    let err = serde_json::from_str::<Colour>("\"green\"").unwrap_err();
    assert!(err.to_string().contains("colour must be"));
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p macros text_enum` Expected: FAIL — no
serde impls emitted.

- [ ] **Step 3: Implement against the tests**

Emit `Serialize` (serialize_str of the `&'static str`) and `Deserialize` (owned
`String` → `FromStr` → `de::Error::custom`). This is deliberately the same path
`#[serde(into = "String", try_from = "String")]` takes today, which is what
keeps `serde_qs` form transport working.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p macros` Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add macros/src/text_enum.rs macros/tests/text_enum.rs
git commit -m "feat(macros): emit the serde bridge from text_enum"
```

---

### Task 7: `text_enum` — the sqlx bridge

**Files:**

- Modify: `macros/src/text_enum.rs`
- Test: same file's `#[cfg(test)]`

**Interfaces:**

- Consumes: `BridgeSpec` (Task 1), Task 4's option parsing.
- Produces: with `sqlx`, the bridge with `type_inner = ::std::string::String`,
  `encode_inner = &'q str`,
  `to_inner = &<&#name as ::core::convert::Into<&'static str>>::into(self)`,
  `decode_inner = &'r str`, `convert = Ok(<#name as FromStr>::from_str(v)?)`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn sqlx_flag_emits_the_bridge_with_the_three_declared_inners() {
    let out = norm(&expand_str(r#"sqlx, error = InvalidX, message = "b""#, "pub enum X { A }"));
    // D2a: Type delegates to String, NOT str — storage binds `String: Type<DB>`.
    assert!(out.contains("<::std::string::Stringas::sqlx::Type<DB>>::type_info()"));
    assert!(!out.contains("<stras::sqlx::Type"));
    // D4: encode borrows a 'static token, no allocation. The `&&` is a reference to the
    // `&'q str` local, which is what Encode::encode_by_ref(&self, ..) takes.
    assert!(out.contains("letinner:&&'qstr=&<&Xas::core::convert::Into<&'staticstr>>::into(self);"));
    assert!(out.contains("<&'qstras::sqlx::Encode<'q,DB>>::encode_by_ref(inner,buf)"));
    assert!(!out.contains("to_owned"));
    // D3: decode borrows.
    assert!(out.contains("<&'rstras::sqlx::Decode<'r,DB>>::decode(value)?"));
    assert!(out.contains("::from_str(v)?"));
}

#[test]
fn without_the_sqlx_flag_no_bridge_is_emitted() {
    let out = norm(&expand_str(r#"error = InvalidX, message = "b""#, "pub enum X { A }"));
    assert!(!out.contains("::sqlx::"));
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p macros text_enum` Expected: FAIL — the
`sqlx` flag is parsed but ignored.

- [ ] **Step 3: Implement against the tests**

When `sqlx` is set, call `bridge()` with the spec above and append it.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p macros` Expected: PASS. (Compilation
of this output arrives in Task 8, the first adopter with `sqlx`.)

- [ ] **Step 5: Commit**

```bash
git add macros/src/text_enum.rs
git commit -m "feat(macros): emit the sqlx bridge from text_enum(sqlx)"
```

---

### Task 8: Migrate `PostFormat` + `MediaSource`; delete `db_enum.rs`

**Files:**

- Modify: `common/src/render.rs:20-71`, `common/src/media.rs:530-571` and its
  import at `common/src/media.rs:66`
- Delete: `common/src/db_enum.rs`; remove its `mod` line from
  `common/src/lib.rs:15`
- Test: existing tests in both files, unmodified

- [ ] **Step 1: Rewrite both declarations**

For each: put `#[text_enum(sqlx, error = …, message = …)]` **first** (D1a),
copying the message verbatim from the existing `parse_error!`; **remove** the
four uniform strum derives from the author list (AC-19a) while keeping
`VariantArray`/`EnumMessage`/ `Default`/`Copy`/…; remove `serde::Serialize`,
`serde::Deserialize`, `#[serde(into, try_from)]`, the `parse_error!` invocation,
the `impl_string_serde_proxy!` line, and the `impl_text_column_enum!` line.

**Also rewrite the doc comment above each enum** — `render.rs:20-26` and
`media.rs:530-537` both narrate `impl_string_serde_proxy!` /
`impl_text_column_enum!` by name (part of AC-25; doing it here rather than in
Task 12 avoids leaving it stale for four tasks and avoids line-number drift).

**Also remove the now-unused import** at `media.rs:66`
(`use crate::strum_enum::{impl_string_serde_proxy, parse_error};`) — leaving it
fails the per-commit `cargo xtask check`.

- [ ] **Step 2: Delete `db_enum.rs` and run the existing tests unmodified**

Run: `devtool run -- cargo nextest run -p common` Expected: PASS with **no test
edits** (AC-22). `render.rs:772-792` and `media.rs:1783` are the ones that would
catch a representation change.

- [ ] **Step 3: Prove the storage path, and that `host` still resolves**

Run: `devtool run -- cargo xtask validate --no-e2e` Expected: PASS. This
compiles `host/src/error.rs:380-387` and its `check!` block (665-671), which
construct `InvalidPostFormat` and `InvalidMediaSource` as **bare unit-struct
expressions** — the shape AC-20 pins. If the generated error is not a bare unit
struct, this is where it fails.

Run: `rg 'impl.*sqlx::(Type|Encode|Decode)' common/` Expected: no matches
(AC-15). Six lines matched before this work — three in `db_enum.rs`, three in
`render.rs`, both blocks now gone.

- [ ] **Step 4: Commit**

```bash
git add common/src/render.rs common/src/media.rs common/src/lib.rs
git rm common/src/db_enum.rs
git commit -m "refactor(common): move the stored enums onto text_enum, drop db_enum"
```

---

### Task 9: Migrate `AudienceBase` + `RegistrationPolicy`

**Files:**

- Modify: `common/src/visibility.rs:84-114`, `common/src/registration.rs:13-46`
  and its import at `common/src/registration.rs:6`
- Test: existing tests, unmodified

- [ ] **Step 1: Rewrite both declarations**

As Task 8 but **without** `sqlx` — neither is stored. `AudienceBase` keeps its
`#[default]` variant and `Default` derive. Rewrite `registration.rs:13-14`'s doc
comment (AC-25) and drop the `registration.rs:6` import. `visibility.rs`'s
module-level comment and its own import are handled in Task 11, which migrates
the rest of that file.

- [ ] **Step 2: Run the existing tests unmodified**

Run: `devtool run -- cargo nextest run -p common -p web` Expected: PASS.
`visibility.rs:335-365` pins `AudienceBase`'s JSON; the form-transport test at
`web/src/profile/api.rs:121-123` pins the `serde_qs` path (AC-22).

- [ ] **Step 3: Commit**

```bash
git add common/src/visibility.rs common/src/registration.rs
git commit -m "refactor(common): move the wire-only enums onto text_enum"
```

---

### Task 10: Migrate `BackupMode` (D11)

**Files:**

- Modify: `common/src/backup.rs:17-44`
- Test: `common/src/backup.rs` `#[cfg(test)]`

**Interfaces:**

- Produces: `InvalidBackupMode`, a new public error type.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn backup_mode_json_bytes_are_unchanged() {
    // AC-21a: the one type swapping serde `rename_all` for strum `serialize_all`,
    // and it crosses a #[server] boundary via BackupConfig.
    assert_eq!(serde_json::to_string(&BackupMode::Directory).unwrap(), "\"directory\"");
    assert_eq!(serde_json::to_string(&BackupMode::Archive).unwrap(), "\"archive\"");
    for m in [BackupMode::Directory, BackupMode::Archive] {
        let j = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<BackupMode>(&j).unwrap(), m);
    }
}

#[test]
fn backup_mode_rejects_unknown_with_the_named_error() {
    // AC-21: must beat strum's generic "Matching variant not found".
    let err = "sideways".parse::<BackupMode>().unwrap_err();
    assert_eq!(err, InvalidBackupMode);
    assert_eq!(err.to_string(), "backup mode must be \"directory\" or \"archive\"");
    let de = serde_json::from_str::<BackupMode>("\"sideways\"").unwrap_err();
    assert!(de.to_string().contains("backup mode must be"));
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p common backup` Expected: FAIL —
`InvalidBackupMode` not defined.

- [ ] **Step 3: Migrate**

Add
`#[text_enum(error = InvalidBackupMode, message = "backup mode must be \"directory\" or \"archive\"")]`
first; remove `Serialize`, `Deserialize`, `#[serde(rename_all)]`, and the three
uniform strum derives it already has (`AsRefStr`, `IntoStaticStr`,
`EnumString`), keeping `VariantArray`, `Default`, `Clone`, `Copy`, `Debug`,
`Eq`, `PartialEq`. It **gains** `Display`, which it lacks today — additive, and
`label()` (46-53) is an inherent method, so no collision (D11). Rewrite the doc
comment at `backup.rs:17-20` (AC-25).

- [ ] **Step 4: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run -p common -p web -p storage` Expected:
PASS. The three parse sites all discard the error type
(`storage/src/site_config.rs:103`, `web/src/backup/component.rs:144`,
`common/src/backup.rs:226`), so the `FromStr::Err` change is invisible to them.

- [ ] **Step 5: Commit**

```bash
git add common/src/backup.rs
git commit -m "refactor(common): move BackupMode onto text_enum with a named parse error"
```

---

### Task 11: Migrate the three FK enums; delete `strum_enum.rs` (D12)

**Files:**

- Modify: `common/src/visibility.rs` — the module comment at 7-12, the import at
  5, and the three enums at 13-78
- Delete: `common/src/strum_enum.rs`; remove its `mod` line from
  `common/src/lib.rs:40`
- Test: `common/src/visibility.rs` `#[cfg(test)]`

- [ ] **Step 1: Write the failing tests**

These three have **no** serde tests today, so without this a broken `Serialize`
passes every other criterion (AC-21b).

```rust
#[test]
fn fk_enums_round_trip_through_serde() {
    for c in [Channel::Local] {
        assert_eq!(serde_json::from_str::<Channel>(&serde_json::to_string(&c).unwrap()).unwrap(), c);
    }
    for s in [SubscriptionStatus::Active, SubscriptionStatus::Pending, SubscriptionStatus::Blocked] {
        assert_eq!(serde_json::to_string(&s).unwrap(), format!("\"{}\"", s.as_ref()));
        assert_eq!(serde_json::from_str::<SubscriptionStatus>(&serde_json::to_string(&s).unwrap()).unwrap(), s);
    }
    for k in [TargetKind::Public, TargetKind::Subscribers, TargetKind::Named] {
        assert_eq!(serde_json::to_string(&k).unwrap(), format!("\"{}\"", k.as_ref()));
        assert_eq!(serde_json::from_str::<TargetKind>(&serde_json::to_string(&k).unwrap()).unwrap(), k);
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `devtool run -- cargo nextest run -p common visibility` Expected: FAIL —
the three do not implement `Serialize`.

- [ ] **Step 3: Migrate all three, clean the file, delete `strum_enum.rs`**

As Task 9, no `sqlx`. `SubscriptionStatus` and `TargetKind` already derive
`IntoStaticStr` — remove it with the other uniform derives, since the attribute
injects it (AC-19a). Rewrite the module comment at 7-12 (AC-25) and drop the
import at 5. With the last `parse_error!` gone, delete `strum_enum.rs`.

- [ ] **Step 4: Run, and verify nothing references the retired macros**

Run: `devtool run -- cargo nextest run -p common -p storage` Expected: PASS.
`storage/src/posts.rs:1844-1863` binds `TargetKind` as `&'static str` into the
lookup column and parses it back — unchanged by this.

Run:
`rg 'impl_text_column_enum|impl_string_serde_proxy|parse_error!' --type rust`
Expected: no matches (AC-18).

- [ ] **Step 5: Commit**

```bash
git add common/src/visibility.rs common/src/lib.rs
git rm common/src/strum_enum.rs
git commit -m "refactor(common): move the FK-normalized enums onto text_enum, drop strum_enum"
```

---

### Task 12: Four remaining prose sites

**Files:**

- Modify: `storage/src/media.rs:464`, `macros/src/sqlx_bridge.rs:1-13`,
  `xtask/src/steps/sqlx_newtype_bind_check.rs:4`,
  `xtask/src/steps/rendered_html_from_trusted_check.rs:80-84`

The other five sites were rewritten by Tasks 8-11, alongside the code they
describe. **Anchor these four by content, not line number** — the earlier tasks
shift line numbers.

- [ ] **Step 1: Rewrite all four**

- `storage/src/media.rs` — the comment naming `PostFormat`'s
  `impl_text_column_enum!` instantiation.
- `macros/src/sqlx_bridge.rs` module doc — claims the bridge is "emitted by all
  three newtype derives" and carries a three-row family→inner table. There are
  now four callers and three independent inners; replace the table with D3's
  per-conversion rule.
- `xtask/src/steps/sqlx_newtype_bind_check.rs` — "All three newtype derives now
  emit an `sqlx::Encode`/`Type`/`Decode` bridge".
- `xtask/src/steps/rendered_html_from_trusted_check.rs` — the `ALLOWED_FNS`
  comment narrating `RenderedHtml`'s "own `sqlx::Decode`", which is now derived.

**Comments only** in the two xtask files — no gate logic changes. #759 owns any
change to what those gates actually police.

- [ ] **Step 2: Verify and commit**

Run: `devtool run -- cargo xtask check` Expected: PASS (doc tests and clippy
included).

```bash
git add -u
git commit -m "docs: retire the impl_text_column_enum/three-derive narration"
```

---

### Task 13: ADR draft, ADR-0075 amendments, issue body

**Files:**

- Create: `docs/adr/drafts/<slug>.md` (numberless — `cargo xtask adr promote`
  numbers it at ship; see **`jaunder-adr`**)
- Modify: `docs/adr/0075-adopt-strum-retire-str-enum.md`

- [ ] **Step 1: Write the ADR draft**

Decision: `#[text_enum]` is the standard shape for a closed string enum. Argue
the **engine-vs-periphery** line from D7 explicitly and **concede** that the
named error and serde bridge — two things `StrEnum` also generated (ADR-0074:47,
52-56) — come back; the claim is that the duplicated engine (token mapping,
`Display`, `FromStr`) stays deleted, not that overlap is zero. Carry the
`parse_error!` precedent forward in the ADR's own text, since Task 11 deletes
the file that records it.

- [ ] **Step 2: Amend ADR-0075**

Add `- Amended: 2026-07-31` and `_Amended by #746._` on four passages (AC-24):
the Decision sentence (81-83); the `impl_text_column_enum!` Consequences bullet
(99-100), **rewriting** its "explicitly NOT a return to a bespoke proc-macro"
clause; the `BackupMode`-as-precedent paragraph (72-77); and the
accepted-minor-cost bullet (110-113).

- [ ] **Step 3: Update issue #746 (AC-26)**

```bash
gh issue edit 746 --repo jaunder-org/jaunder --body-file <path>
```

The filed body names no derive, scopes serde to two enums, omits the
`StrNewtype`, `RenderedHtml`, `BackupMode` and FK-enum work, and carries the
retracted gate claim.

- [ ] **Step 4: Full gate**

Run: `devtool run -- cargo xtask validate` Expected: PASS — both backends, all
four e2e combos (AC-23).

- [ ] **Step 5: Commit**

```bash
git add docs/adr
git commit -m "docs(adr): record text_enum as the closed-string-enum convention"
```
