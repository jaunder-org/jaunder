# StrNewtype / IdNewtype Derive Macros — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Add `#[derive(StrNewtype)]` and `#[derive(IdNewtype)]` to the `macros`
crate so a domain newtype is a struct + its std `#[derive]`s + a hand-written
`FromStr`, with the ADR-0063 trailer generated.

**Architecture:** Two `#[proc_macro_derive]` entry points in `macros/src/lib.rs`
(entry fns must live at the proc-macro crate root) delegate to `str_newtype.rs`
/ `id_newtype.rs` codegen modules. Proven against **fixture** newtypes in
`macros/tests/`; the four real newtypes are untouched (their retrofit is the
split #404 verticals — Task 1). `compile_fail` doctests lock the secret
negatives and input-shape rejection.

**Tech Stack:** Rust proc-macros — `syn` 2 (default features), `quote`,
`proc-macro2`; `serde`/`serde_json` as **dev**-deps for fixtures.

**Spec:**
[`docs/superpowers/specs/2026-07-12-issue-403-newtype-derive-macros.md`](../specs/2026-07-12-issue-403-newtype-derive-macros.md)
— this plan is "how"; the spec is "what/why". Read its "Generated surface" table
and "Design decisions" before implementing.

## Global Constraints

- **Build-time-only (ADR-0062):** `macros` gains **no runtime deps** beyond
  `syn`/`quote`/`proc-macro2`; `serde`/`serde_json` are **dev-deps only**. No
  coverage surface.
- **Generated impls** carry `#[automatically_derived]` and reference std via
  fully qualified paths (`::core::…`, `::std::string::String`) and serde via
  **`::serde`** (resolves only in a consumer with a direct `serde` dep — true
  for `common`).
- **`FromStr` is never generated** — always hand-written (ADR-0063 §3). The
  derive assumes `Self: FromStr` and, for `StrNewtype`, that
  `<Self as FromStr>::Err: Display` (for serde error mapping via
  `de::Error::custom`); all domain error types are `thiserror` and satisfy this.
- **No inherent `as_str()`** generated.
- **`syn` default features** suffice (single-field-tuple `DeriveInput` +
  `attributes(str_newtype)`); do **not** pull `full`.
- **No clippy `#[allow]`/`#[expect]`** in hand-written code.
- **`compile_fail` doctests** isolate a single offending line with known-good
  surroundings; a positive doctest proves the same surroundings compile.
- **Gate:** run `cargo xtask check` clean before every commit
  (**jaunder-commit**). Commit messages carry **no `Co-Authored-By` trailer**.

## Task list (review layer)

1. **File the #404 split** — four per-newtype retrofit verticals (Username/Slug/
   Tag/Password) via **jaunder-issues**; repurpose #404 as their umbrella. _(no
   code)_
2. **`StrNewtype` (non-secret)** — deps + codegen module + entry point; full
   trailer (`Display`, serde, `AsRef`/`Borrow`/`Deref`, `TryFrom<String>`,
   `From<Self> for String`, `PartialEq<str>`/`<&str>`) + wrong-shape
   `compile_error!`.
3. **`StrNewtype` secret variant** — `#[str_newtype(secret)]`: redacting
   `Debug` + `AsRef` + `TryFrom<String>` only; `compile_fail` doctests for the
   negatives.
4. **`IdNewtype`** — `From<i64>`/`From<Self> for i64`/`Display` +
   transparent-i64 serde + wrong-shape `compile_error!`.
5. **Amend ADR-0063** per the spec's amendment list.

**Key risks/decisions:** (a) secret surface is deliberately tight — `AsRef`
only, no `Deref` (implicit-coercion leak, ADR-0011); (b) `compile_fail` can't
verify _why_ — mitigated by single-line isolation + positive companion; (c)
serde emitted as direct impls, not `#[serde(try_from/into)]`, to avoid two-macro
fragility and the `into="String"` clone.

---

### Task 1: File the #404 split (separable concern)

**Files:** none (tracker only).

**Interfaces:**

- Produces: four GitHub issues in milestone #13 — "types: retrofit Username onto
  StrNewtype", …Slug, …Tag, …Password (secret) — each scoped to that one type's
  derive adoption + its `as_str()`/`String` call-site sweep. #404 repurposed as
  the umbrella (edit title/body to "umbrella: retrofit existing newtypes onto
  the derive"), or closed with pointers, per **jaunder-issues** judgment.

- [x] **Step 1: File the four verticals** via **jaunder-issues** (label
      `type-safety`, milestone "Domain-value type safety (newtypes)"). Each
      body: "Adopt `#[derive(StrNewtype)]` (Password: `#[str_newtype(secret)]`)
      on `<Type>`, keeping its hand-written `FromStr`; delete the obviated
      `.as_str()` sites (`&x`/`x.as_ref()`); no behavior change;
      `cargo xtask validate --no-e2e` clean." Reference the spec's "Separable
      concerns" section and note dependence on #403. → **#407 Username, #408
      Slug, #409 Tag, #410 Password** (all Task type, in Backlog project #1,
      milestone #13, blocked-by #403).
- [x] **Step 2: Repurpose #404** as the umbrella linking the four (or close it
      as split). Set the four as blocked-by #403 (native issue deps). → #404
      retitled "types: retrofit existing newtypes onto the derive (umbrella)";
      #407–410 added as native **sub-issues** of #404 and **blocked-by** #403.
- [x] **Step 3:** No commit — record the issue numbers in the PR description at
      ship. → #407/#408/#409/#410 under umbrella #404.

---

### Task 2: `StrNewtype` (non-secret) derive

**Files:**

- Modify: `Cargo.toml` (root) — add `syn`/`quote`/`proc-macro2` to
  `[workspace.dependencies]`.
- Modify: `macros/Cargo.toml` — add the three as deps; `serde`/`serde_json` as
  dev-deps.
- Create: `macros/src/str_newtype.rs` — codegen.
- Modify: `macros/src/lib.rs` — add `mod str_newtype;` + the
  `#[proc_macro_derive]` entry (with the non-secret path; secret branch lands in
  Task 3).
- Test: `macros/tests/str_newtype.rs`.

**Interfaces:**

- Produces: `#[derive(StrNewtype)]` on a `struct X(String)` with a hand-written
  `FromStr`. Codegen entry
  `pub(crate) fn expand(input: syn::DeriveInput) -> proc_macro2::TokenStream`.
  lib.rs entry:
  `#[proc_macro_derive(StrNewtype, attributes(str_newtype))] pub fn str_newtype_derive(item: proc_macro::TokenStream) -> proc_macro::TokenStream`.
- Shared helper
  `pub(crate) fn newtype_field(input: &syn::DeriveInput) -> syn::Result<()>` (or
  returns the field) — validates single-field tuple struct; **lives in
  `macros/src/lib.rs`** (crate root) so both codegen submodules call
  `crate::newtype_field` without reaching into each other.

- [ ] **Step 1: Add deps.** Root `[workspace.dependencies]`:

  ```toml
  syn = "2"
  quote = "1"
  proc-macro2 = "1"
  ```

  `macros/Cargo.toml`:

  ```toml
  [dependencies]
  syn = { workspace = true }
  quote = { workspace = true }
  proc-macro2 = { workspace = true }

  [dev-dependencies]
  serde = { workspace = true }
  serde_json = { workspace = true }
  ```

- [ ] **Step 2: Write the failing test** — `macros/tests/str_newtype.rs`. A
      fixture with a validating, normalizing (lowercasing) `FromStr`, then one
      assertion per generated trait:

  ```rust
  use macros::StrNewtype;
  use std::collections::HashSet;
  use std::str::FromStr;

  #[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
  struct Code(String);

  #[derive(Debug, PartialEq)]
  struct BadCode;

  impl FromStr for Code {
      type Err = BadCode;
      fn from_str(s: &str) -> Result<Self, Self::Err> {
          let s = s.to_lowercase();
          if s.is_empty() { return Err(BadCode); }
          Ok(Code(s))
      }
  }
  // BadCode must impl Display for the serde error path:
  impl std::fmt::Display for BadCode {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
          f.write_str("bad code")
      }
  }

  #[test]
  fn try_from_string_ok_and_err() {
      assert_eq!(Code::try_from("AB".to_owned()), Ok(Code("ab".to_owned())));
      assert_eq!(Code::try_from(String::new()), Err(BadCode));
  }
  #[test]
  fn from_self_for_string() {
      assert_eq!(String::from(Code::from_str("ab").unwrap()), "ab".to_owned());
  }
  #[test]
  fn as_ref_str() {
      let c = Code::from_str("ab").unwrap();
      let r: &str = c.as_ref();
      assert_eq!(r, "ab");
  }
  #[test]
  fn deref_and_coercion() {
      let c = Code::from_str("ab").unwrap();
      assert_eq!(c.len(), 2);            // str method via Deref
      fn take(_: &str) {}
      take(&c);                          // &Code coerces to &str
  }
  #[test]
  fn borrow_probes_hashset_with_str() {
      let mut set: HashSet<Code> = HashSet::new();
      set.insert(Code::from_str("ab").unwrap());
      assert!(set.contains("ab"));       // &str key, no alloc — needs Borrow<str>
  }
  #[test]
  fn display() {
      assert_eq!(format!("{}", Code::from_str("ab").unwrap()), "ab");
  }
  #[test]
  fn partial_eq_str_and_ref_str() {
      let c = Code::from_str("ab").unwrap();
      assert!(c == "ab");                // PartialEq<&str>
      let s: &str = "ab";
      assert!(c == *s);                  // PartialEq<str>
  }
  #[test]
  fn serde_roundtrip_and_wire_validation() {
      let c = Code::from_str("ab").unwrap();
      assert_eq!(serde_json::to_string(&c).unwrap(), "\"ab\"");
      assert_eq!(serde_json::from_str::<Code>("\"AB\"").unwrap(), Code("ab".to_owned()));
      assert!(serde_json::from_str::<Code>("\"\"").is_err()); // FromStr rejects on the wire
  }
  ```

- [ ] **Step 3: Run, verify FAIL.** Run: `cargo nextest run -p macros`.
      Expected: FAIL — `StrNewtype` derive not defined / trait impls missing.

- [ ] **Step 4: Implement.** `str_newtype.rs::expand` emits, for
      `struct X(String)`, the impls below (this is the body — runtime tests pin
      behavior but not the exact tokens, so the token template is the contract).
      All `#[automatically_derived]`:

  ```rust
  // Display
  impl ::core::fmt::Display for #name {
      fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
          f.write_str(&self.0)
      }
  }
  // AsRef / Borrow / Deref
  impl ::core::convert::AsRef<str> for #name { fn as_ref(&self) -> &str { &self.0 } }
  impl ::core::borrow::Borrow<str> for #name { fn borrow(&self) -> &str { &self.0 } }
  impl ::core::ops::Deref for #name {
      type Target = str;
      fn deref(&self) -> &str { &self.0 }
  }
  // Owned conversions
  impl ::core::convert::TryFrom<::std::string::String> for #name {
      type Error = <#name as ::core::str::FromStr>::Err;
      fn try_from(s: ::std::string::String) -> ::core::result::Result<Self, Self::Error> {
          <#name as ::core::str::FromStr>::from_str(&s)
      }
  }
  impl ::core::convert::From<#name> for ::std::string::String {
      fn from(v: #name) -> Self { v.0 }
  }
  // Comparisons
  impl ::core::cmp::PartialEq<str> for #name {
      fn eq(&self, other: &str) -> bool { self.0 == *other }
  }
  impl ::core::cmp::PartialEq<&str> for #name {
      fn eq(&self, other: &&str) -> bool { self.0 == **other }
  }
  // serde (direct, borrow-not-clone; deserialize routes through FromStr)
  impl ::serde::Serialize for #name {
      fn serialize<S: ::serde::Serializer>(&self, s: S)
          -> ::core::result::Result<S::Ok, S::Error> { s.serialize_str(&self.0) }
  }
  impl<'de> ::serde::Deserialize<'de> for #name {
      fn deserialize<D: ::serde::Deserializer<'de>>(d: D)
          -> ::core::result::Result<Self, D::Error> {
          let s = <::std::string::String as ::serde::Deserialize>::deserialize(d)?;
          <#name as ::core::str::FromStr>::from_str(&s)
              .map_err(::serde::de::Error::custom)
      }
  }
  ```

  `newtype_field` returns
  `syn::Error::new_spanned(input, "StrNewtype requires a single-field tuple struct like `struct
  X(String)`")` when the input is not a struct with exactly one unnamed field;
  `expand` returns `err.to_compile_error()`. lib.rs entry parses via
  `syn::parse_macro_input!` and calls `expand`.

- [ ] **Step 5: Run, verify PASS.** Run: `cargo nextest run -p macros`.
      Expected: PASS.

- [ ] **Step 6: Wrong-shape doctest.** On the `str_newtype_derive` doc comment
      add a `compile_fail` doctest applying `#[derive(StrNewtype)]` to a
      **named-field** struct (`struct X { s: String }`) — must fail to compile —
      and a positive companion on a tuple struct that compiles. Run:
      `cargo test -p macros --doc`. Expected: PASS **with the "running N tests"
      count == the number of doctest blocks added (2 here)** — a `compile_fail`
      suite that is never collected also reports success with 0 tests, so assert
      the count, not just the exit status.

- [ ] **Step 7: Commit** (run `cargo xtask check` clean first).
  ```bash
  git add Cargo.toml macros/Cargo.toml macros/src/lib.rs macros/src/str_newtype.rs macros/tests/str_newtype.rs
  git commit -m "feat(macros): StrNewtype derive generates the ADR-0063 trailer"
  ```

---

### Task 3: `StrNewtype` secret variant

**Files:**

- Modify: `macros/src/str_newtype.rs` — parse `#[str_newtype(secret)]`; branch
  codegen.
- Modify: `macros/src/lib.rs` — extend the derive's doc comment with the secret
  `compile_fail` doctests.
- Test: `macros/tests/str_newtype.rs` (append secret fixture + tests).

**Interfaces:**

- Consumes: Task 2's `expand` / `newtype_field`.
- Produces: `#[str_newtype(secret)]` support. Secret emits **only** redacting
  `Debug`, `AsRef<str>`, `TryFrom<String>`; omits `Display`, serde, `Borrow`,
  `Deref`, `From<Self> for String`, `PartialEq`.

- [ ] **Step 1: Write the failing test** — append to
      `macros/tests/str_newtype.rs`:

  ```rust
  #[derive(Clone, StrNewtype)]           // NOTE: no Debug derive
  #[str_newtype(secret)]
  struct Secret(String);

  #[derive(Debug, PartialEq)]
  struct BadSecret;
  impl std::fmt::Display for BadSecret {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("bad") }
  }
  impl FromStr for Secret {
      type Err = BadSecret;
      fn from_str(s: &str) -> Result<Self, Self::Err> {
          if s.is_empty() { return Err(BadSecret); }
          Ok(Secret(s.to_owned()))
      }
  }

  #[test]
  fn secret_debug_redacts() {
      let s = Secret::from_str("hunter2").unwrap();
      let d = format!("{:?}", s);
      assert_eq!(d, "Secret([redacted])");
      assert!(!d.contains("hunter2"));
  }
  #[test]
  fn secret_as_ref_and_try_from() {
      let s = Secret::try_from("hunter2".to_owned()).unwrap();
      assert_eq!(s.as_ref() as &str, "hunter2");
      assert!(Secret::try_from(String::new()).is_err());
  }
  ```

- [ ] **Step 2: Run, verify FAIL.** Run: `cargo nextest run -p macros`.
      Expected: FAIL — `#[str_newtype(secret)]` unknown / redacting `Debug`
      missing.

- [ ] **Step 3: Implement.** In `expand`, parse the `str_newtype` attribute (via
      `input.attrs`, matching `path().is_ident("str_newtype")` and looking for
      the `secret` ident in its meta-list). When `secret`:

  ```rust
  // redacting Debug (exact form matches Password's contract style)
  impl ::core::fmt::Debug for #name {
      fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
          f.write_str(concat!(stringify!(#name), "([redacted])"))
      }
  }
  // AsRef + TryFrom<String> only (same token templates as Task 2)
  ```

  Emit **none** of
  Display/serde/Borrow/Deref/`From<Self> for String`/`PartialEq`. Non-secret
  path is unchanged.

- [ ] **Step 4: Run, verify PASS.** Run: `cargo nextest run -p macros`.
      Expected: PASS.

- [ ] **Step 5: Negative `compile_fail` doctests.** On the derive's doc comment,
      one `compile_fail` block per negative, each isolating a single offending
      line with the fixture (a secret `S` with a hand `FromStr`) as known-good
      surroundings: `format!("{}", s)` (no `Display`),
      `serde_json::to_string(&s)` (no serde), `String::from(s)` (no owned
      extraction), `s == "x"` (no `PartialEq`), and `let _: &str = &s;` (no
      `Deref` coercion). Add one positive companion doctest where the identical
      surroundings + `s.as_ref()` **and** a `serde_json::to_string(s.as_ref())`
      call **compile** — touching `serde_json` in the positive proves the
      negative's serde failure is the missing `Serialize`, not an unresolved
      crate. Run: `cargo test -p macros --doc`. Expected: PASS **with "running N
      tests" == number of doctest blocks (6: five `compile_fail` + one
      positive)** — assert the count so a never-collected suite can't pass
      silently.

- [ ] **Step 6: Commit** (`cargo xtask check` clean first).
  ```bash
  git add macros/src/str_newtype.rs macros/src/lib.rs macros/tests/str_newtype.rs
  git commit -m "feat(macros): StrNewtype secret variant redacts and omits leak paths"
  ```

---

### Task 4: `IdNewtype` derive

**Files:**

- Create: `macros/src/id_newtype.rs` — codegen.
- Modify: `macros/src/lib.rs` — `mod id_newtype;` + entry + doctests.
- Test: `macros/tests/id_newtype.rs`.

**Interfaces:**

- Consumes: `crate::newtype_field` (single-field-tuple check, in `lib.rs` from
  Task 2).
- Produces: `#[proc_macro_derive(IdNewtype)] pub fn id_newtype_derive(...)`. For
  `struct X(i64)`: generates `From<i64>`, `From<Self> for i64`, `Display`, and a
  transparent-i64 serde bridge. `Copy` etc. are **user-derived**.

- [ ] **Step 1: Write the failing test** — `macros/tests/id_newtype.rs`:

  ```rust
  use macros::IdNewtype;

  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, IdNewtype)]
  struct Id(i64);

  #[test]
  fn from_i64_and_into_i64() {
      let id = Id::from(42);
      assert_eq!(id, Id(42));
      let n: i64 = id.into();
      assert_eq!(n, 42);
  }
  #[test]
  fn copy_semantics() {
      let a = Id(7);
      let b = a;           // Copy
      assert_eq!(a, b);    // a still usable
  }
  #[test]
  fn display() { assert_eq!(format!("{}", Id(42)), "42"); }
  #[test]
  fn serde_transparent_roundtrip() {
      assert_eq!(serde_json::to_string(&Id(42)).unwrap(), "42");
      assert_eq!(serde_json::from_str::<Id>("42").unwrap(), Id(42));
  }
  ```

- [ ] **Step 2: Run, verify FAIL.** Run: `cargo nextest run -p macros`.
      Expected: FAIL — `IdNewtype` not defined.

- [ ] **Step 3: Implement.** `id_newtype.rs::expand` emits (all
      `#[automatically_derived]`):

  ```rust
  impl ::core::convert::From<i64> for #name { fn from(v: i64) -> Self { #name(v) } }
  impl ::core::convert::From<#name> for i64 { fn from(v: #name) -> Self { v.0 } }
  impl ::core::fmt::Display for #name {
      fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
          ::core::fmt::Display::fmt(&self.0, f)
      }
  }
  impl ::serde::Serialize for #name {
      fn serialize<S: ::serde::Serializer>(&self, s: S)
          -> ::core::result::Result<S::Ok, S::Error> { s.serialize_i64(self.0) }
  }
  impl<'de> ::serde::Deserialize<'de> for #name {
      fn deserialize<D: ::serde::Deserializer<'de>>(d: D)
          -> ::core::result::Result<Self, D::Error> {
          ::core::result::Result::Ok(#name(<i64 as ::serde::Deserialize>::deserialize(d)?))
      }
  }
  ```

  Reuse `newtype_field` for the single-field-tuple check (wrong shape →
  `compile_error!` "IdNewtype requires a single-field tuple struct like
  `struct X(i64)`").

- [ ] **Step 4: Run, verify PASS.** Run: `cargo nextest run -p macros`.
      Expected: PASS.

- [ ] **Step 5: Wrong-shape doctest** on `id_newtype_derive` (named-field struct
      → `compile_fail`; tuple struct → positive companion). Run:
      `cargo test -p macros --doc`. Expected: PASS **with the "running N tests"
      count == the total doctest blocks across all three derives** (assert the
      count, not just exit status, so a never-collected `compile_fail` can't
      pass silently).

- [ ] **Step 6: Commit** (`cargo xtask check` clean first).
  ```bash
  git add macros/src/lib.rs macros/src/id_newtype.rs macros/tests/id_newtype.rs
  git commit -m "feat(macros): IdNewtype derive for i64-backed id newtypes"
  ```

---

### Task 5: Amend ADR-0063

**Files:**

- Modify: `docs/adr/0063-domain-value-newtype-convention.md`.

**Interfaces:** none (docs).

- [ ] **Step 1: Edit the ADR** per the spec's "ADR amendment" section:
  1. Broaden the **Secret-bearing exception** — secret omits `Display`, the
     serde bridge, `Deref<str>` + `Borrow<str>` (implicit-coercion/`to_owned`
     leak, ADR-0011), `From<Self> for String`, and `PartialEq<str>`/`<&str>`;
     retains redacting `Debug`, explicit `AsRef<str>`, and `TryFrom<String>`.
  2. Add "+ a transparent-i64 serde bridge" to the **numeric-ID trailer** (§2).
  3. Reconcile **§3**: the derive generates the trailer except `FromStr` **and
     the std `#[derive]`s** (which stay user-written so per-type variation is
     idiomatic).
  4. Note the trailer is emitted as **direct impls** (no `#[serde]` attribute)
     and that **no inherent `as_str()`** is generated.
- [ ] **Step 2:** `prettier -w docs/adr/0063-domain-value-newtype-convention.md`
      before staging (pre-commit prettier restages prose otherwise).
- [ ] **Step 3: Commit** (`cargo xtask check` clean first).
  ```bash
  git add docs/adr/0063-domain-value-newtype-convention.md
  git commit -m "docs(adr): amend ADR-0063 for the derive's secret/id/serde shape (#403)"
  ```

---

## Definition of done

- `cargo xtask validate --no-e2e` clean.
- Spec acceptance criteria 1–8 all demonstrable: the three fixtures + the
  `compile_fail` doctests cover the full positive surface and the secret/shape
  negatives; ADR-0063 amended; `macros` still build-time-only.
- The four #404 verticals are filed and blocked-by #403.
