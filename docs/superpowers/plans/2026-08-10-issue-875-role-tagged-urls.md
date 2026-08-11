# Role-Tagged Site URLs Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `AbsoluteUrl` with a phantom-tagged `TaggedUrl<T: UrlRole>` so
each URL role is a distinct type and swapping two adjacent URL arguments fails
to compile.

**Architecture:** One generic string newtype carries a zero-sized role marker.
`#[derive(StrNewtype)]` learns generics so the thirteen trait impls (including
the sqlx bridge) are written once and inherited by every role. Roles are minted
only by parsing, by composing from a `Base`, or by an explicit `retag()`.

**Tech Stack:** Rust, `syn`/`quote` proc macros, `sqlx` (SQLite + PostgreSQL),
`serde`, Leptos, `cargo xtask` gates.

Spec:
[`docs/superpowers/specs/2026-08-10-issue-875-role-tagged-urls.md`](../specs/2026-08-10-issue-875-role-tagged-urls.md).
This plan is "how"; the spec is "what/why". Read the spec's _Roles_, _Minting a
role_, and _The alias rule_ sections before starting.

---

## Review header

**Scope — in:** `macros/`, `common/`, `storage/`, `server/` (package `jaunder`),
`web/`, `xtask/`; the `AbsoluteUrl` → `TaggedUrl<T>` migration in full; four
transposition proofs; one ADR draft plus an ADR-0063 amendment.

**Scope — out:** `RootRelativeUrl`, `FeedPath`, and issues #751 / #827 / #879.
No behavioural change — every URL produced at runtime stays byte-identical.

**Tasks:**

1. Teach `StrNewtype` generics; keep `IdNewtype` / `NumNewtype` rejecting them.
2. Teach `sqlx_newtype_decode_check` cross-crate type-alias resolution.
3. Add `TaggedUrl<T>`, `UrlRole`, and the fifteen roles in a new module,
   alongside the untouched `AbsoluteUrl`.
4. **The migration** — retype every site, delete `AbsoluteUrl`, prove the four
   transpositions no longer compile.
5. Write the ADR draft and amend ADR-0063.
6. Full `cargo xtask validate` and an acceptance-criteria walk.

**Key risks and decisions:**

- **Task 4 is one large commit and cannot honestly be split.** Two forces make
  it atomic. (a) `compose` becomes generic in its _output_ tag, and Rust has no
  default type parameter on a function — so the moment `compose` is generic,
  **every** call site must state its role or `U` is un-inferable. Sites consumed
  inline (`compose(...).to_string()` at `mapping.rs:169`, the four
  `format!`-consumed mail sites) break immediately. (b) The retype is viral:
  once `atompub::base_url()` returns `BaseUrl`, every downstream
  `base: &AbsoluteUrl` parameter must change in the same commit. An earlier
  draft of this plan used a temporary `Unclassified` scaffold to fake
  granularity; it does not work, because the scaffold rescues the _input_ type
  and the output parameter is the problem. Task 3 keeps the new type alongside
  the old so the risky work is at least preceded by a green, independently
  reviewable commit that introduces and tests the type in isolation.
- **`server_fn_tracing_check`'s literal must change inside task 4.** It requires
  every un-skipped server-fn argument type to be on `RECORDABLE_TYPES`.
  `web/src/site/api.rs:25` is such an argument, so retyping it and updating
  `xtask/src/steps/server_fn_tracing_check.rs:81` are the same commit or the
  gate fails.
- **Every `compile_fail` needs a positive companion**
  (`CONTRIBUTING.md:469-481`, ADR-0095): the negative carries its fixture as
  `#`-hidden lines, at least one non-empty, and every hidden line appears
  verbatim in a plain fence **in the same doc comment**. A negative without one
  rots into vacuous truth and fails the fence-population gate.
- **No separable concerns to file.** #751, #827, and #879 already exist and are
  untouched.

---

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- **Commit message form:** `type(scope): subject (#875)`.
- **Run the gate before every commit:**
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url -- cargo xtask check`
  (**jaunder-commit**).
- **Stage, then commit.** Never `git commit -- <paths>`.
- **Package names are not directory names.** `server/` is package **`jaunder`**;
  `xtask/` is **excluded from the workspace** (`Cargo.toml:14`) and needs
  `--manifest-path xtask/Cargo.toml`. Other packages: `common`, `storage`,
  `web`, `macros`.
- **Doctests run workspace-wide only:** `cargo test --workspace --doc`. Never
  `-p` — package scoping silently drops the `#[cfg(feature = "sanitize")]`
  fences in `common/src/render.rs` (`CONTRIBUTING.md:447-450`).
- **Only three fence forms are accepted:** ` ``` `, ` ```compile_fail `,
  ` ```text `. Anything else (`ignore`, `no_run`, language tags) fails the gate.
- **Storage tests use the dual-backend template:**

  ```rust
  #[apply(backends)]
  #[tokio::test]
  async fn name(#[case] backend: Backend) {
      let env = backend.setup().await;
      let store = &*env.state.site_config;
      …
  }
  ```

  with `use rstest::*; use rstest_reuse::*;` in the test module. A bare
  `#[tokio::test]` fails the `test-backend-pattern` guard.

- **The alias rule:** every use site outside `common/src/tagged_url.rs` spells
  the alias (`HubUrl`), never `TaggedUrl<Hub>` inline. The only exception is a
  turbofish mint (`compose::<Permalink>(…)`), which spells the tag.
- **Explicit role at every mint:** `let x: FeedUrl = compose(&base, p);` — never
  rely on inference.
- **Worktree:**
  `/home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url`. All
  paths below are relative to it; `devtool run --cwd <that path>` pins every
  command.

---

### Task 1: Teach `StrNewtype` generics

**Files:**

- Modify: `macros/src/str_newtype.rs`, `macros/src/sqlx_bridge.rs`
- Modify: `macros/src/lib.rs:480-506` (`require_newtype_shape`), `:549-565`
  (`ord_impls`), and the `StrNewtype` derive docs at `:265`
- Test: `macros/tests/str_newtype.rs`; the negative doctest goes in
  `macros/src/lib.rs`

**Interfaces:**

- Consumes: nothing.
- Produces: `#[derive(StrNewtype)]` accepts
  `struct X<T: Bound>(String, PhantomData<fn() -> T>)` and emits all thirteen
  impls with the user's generics threaded through. `require_newtype_shape` gains
  a `NewtypeShape` parameter; `ord_impls` and `sqlx_bridge::BridgeSpec` gain a
  `&syn::Generics`. `IdNewtype` and `NumNewtype` behaviour is unchanged.

- [ ] **Step 1: Write the failing tests**

In `macros/tests/str_newtype.rs`:

```rust
use std::marker::PhantomData;
use std::str::FromStr;

use macros::StrNewtype;

pub trait Role {}
pub struct Alpha;
impl Role for Alpha {}
pub struct Beta;
impl Role for Beta {}

#[derive(StrNewtype)]
pub struct Tagged<T: Role>(String, PhantomData<fn() -> T>);

#[derive(Debug, PartialEq, Eq)]
pub struct BadTagged;

impl std::fmt::Display for BadTagged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bad tagged value")
    }
}

impl<T: Role> FromStr for Tagged<T> {
    type Err = BadTagged;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(BadTagged);
        }
        Ok(Self(s.to_owned(), PhantomData))
    }
}

// The derive does not emit these; the real type hand-writes them for the same
// reason (no spurious `T: Clone` bound). Hand-written here too so the tests can
// exercise the emitted impls.
impl<T: Role> Clone for Tagged<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}
impl<T: Role> std::fmt::Debug for Tagged<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Tagged").field(&self.0).finish()
    }
}
impl<T: Role> PartialEq for Tagged<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl<T: Role> Eq for Tagged<T> {}
impl<T: Role> std::hash::Hash for Tagged<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

#[test]
fn generic_newtype_displays_inner() {
    let a: Tagged<Alpha> = "value".parse().expect("parses");
    assert_eq!(a.to_string(), "value");
}

#[test]
fn generic_newtype_derefs_to_str() {
    let a: Tagged<Alpha> = "value".parse().expect("parses");
    let s: &str = &a;
    assert_eq!(s, "value");
    assert_eq!(a.as_ref(), "value");
}

#[test]
fn generic_newtype_round_trips_through_string() {
    let a: Tagged<Alpha> = "value".parse().expect("parses");
    let s = String::from(a.clone());
    let back = Tagged::<Alpha>::try_from(s).expect("round-trips");
    assert_eq!(back, a);
}

#[test]
fn generic_newtype_compares_against_str() {
    let a: Tagged<Alpha> = "value".parse().expect("parses");
    assert_eq!(a, *"value");
    assert_eq!(a, "value");
}

#[test]
fn generic_newtype_orders_by_inner() {
    let first: Tagged<Alpha> = "a".parse().expect("parses");
    let second: Tagged<Alpha> = "b".parse().expect("parses");
    assert!(first < second);
}

#[test]
fn generic_newtype_serializes_transparently() {
    let a: Tagged<Alpha> = "value".parse().expect("parses");
    assert_eq!(serde_json::to_string(&a).expect("serializes"), "\"value\"");
}

#[test]
fn generic_newtype_deserializes_through_from_str() {
    let a: Tagged<Alpha> = serde_json::from_str("\"value\"").expect("deserializes");
    assert_eq!(a, *"value");
    assert!(
        serde_json::from_str::<Tagged<Alpha>>("\"\"").is_err(),
        "empty string must fail FromStr"
    );
}

#[test]
fn two_tags_carry_the_same_bytes_independently() {
    let a: Tagged<Alpha> = "value".parse().expect("parses");
    let b: Tagged<Beta> = "value".parse().expect("parses");
    assert_eq!(a.as_ref(), b.as_ref());
}
```

In `macros/src/lib.rs`, on the `IdNewtype` derive's doc comment (`:271-292`) —
**a lib target, so rustdoc actually collects it**, with the positive companion
ADR-0095 requires:

````rust
/// `IdNewtype` has no generic form; it rejects a phantom-tagged struct.
///
/// ```compile_fail
/// # use std::marker::PhantomData;
/// # use macros::IdNewtype;
/// # pub trait Role {}
/// #[derive(IdNewtype)]
/// pub struct Generic<T: Role>(i64, PhantomData<fn() -> T>);
/// ```
///
/// The non-generic form is what it accepts:
///
/// ```
/// # use std::marker::PhantomData;
/// # use macros::IdNewtype;
/// # pub trait Role {}
/// #[derive(IdNewtype)]
/// pub struct PostId(i64);
/// ```
````

- [ ] **Step 2: Run the tests, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url -- cargo nextest run -p macros`

Expected: FAIL — the `Tagged<T>` derive emits `impl Display for Tagged` without
generics, so the test crate does not compile.

- [ ] **Step 3: Implement against the tests**

In `macros/src/lib.rs`, split the shape check so the relaxation is scoped:

```rust
/// Field shapes a newtype derive will accept.
pub(crate) enum NewtypeShape {
    /// Exactly one field, no generics. `IdNewtype` and `NumNewtype`.
    Plain,
    /// One data field, optionally followed by a `PhantomData<_>` marker field,
    /// with generics permitted. `StrNewtype` only (#875).
    PhantomTagged,
}

pub(crate) fn require_newtype_shape(
    input: &syn::DeriveInput,
    shape: NewtypeShape,
) -> syn::Result<&syn::Field>;
```

`Plain` keeps today's behaviour, including the generics rejection and its error
message. Correct the doc comment at `:480-485`, which currently states that
rejection is unconditional.

Thread generics through the emitters. `default_trailer` (8 impls) and
`ord_impls` (2 impls) introduce no generics of their own, so plain
`split_for_impl()` suffices:

```rust
let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
quote! {
    impl #impl_generics ::std::fmt::Display for #name #ty_generics #where_clause { … }
}
```

Four impls need a **merge** because they introduce their own parameters — serde
`Deserialize` (`'de`, `str_newtype.rs:212`) and the three `sqlx_bridge` impls
(`DB`, `'q`, `'r`). For each, clone the user's generics, push the extra
parameter, and split the merged copy; `ty_generics` and `where_clause` still
come from the user's original generics:

```rust
fn merged(base: &syn::Generics, extra: syn::GenericParam) -> syn::Generics {
    let mut g = base.clone();
    g.params.push(extra);
    g
}
```

`ord_impls` (`macros/src/lib.rs:549`) is shared with `IdNewtype` and
`NumNewtype`; give it a `&syn::Generics` parameter and pass
`&syn::Generics::default()` from those two call sites. `sqlx_bridge::BridgeSpec`
gains a `generics: &syn::Generics` field; its other three call sites (including
the standalone `SqlxBridge` derive) pass an empty `Generics`.

The tests pin every emitted impl — `Display`, `Deref`/`AsRef`, `TryFrom<String>`
and `From<Self> for String`, both `PartialEq` forms, `Ord`, and both serde
directions — plus the negative that `IdNewtype` still rejects generics. No impl
body changes; only the headers gain generics.

- [ ] **Step 4: Run the tests, verify they pass**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url -- cargo nextest run -p macros`
then:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url -- cargo test --workspace --doc`

Expected: PASS — nine `str_newtype` tests, and the `IdNewtype` negative plus its
companion.

- [ ] **Step 5: Commit**

```bash
git add macros/src/lib.rs macros/src/str_newtype.rs macros/src/sqlx_bridge.rs macros/tests/str_newtype.rs
git commit -m "feat(macros): let StrNewtype derive generic phantom-tagged newtypes (#875)"
```

---

### Task 2: Cross-crate type-alias resolution in `sqlx_newtype_decode_check`

**Files:**

- Modify: `xtask/src/steps/sqlx_newtype_decode_check.rs:969-1146`
- Test: in-file `#[cfg(test)]` in the same file

**Interfaces:**

- Consumes: nothing (unit tests use synthetic `syn` input, so this lands before
  the aliases exist).
- Produces: the gate approves a decode into `Option<HubUrl>` by resolving the
  alias to `TaggedUrl`, and still rejects a decode into an underived type.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn resolves_an_alias_to_its_approved_underlying_newtype() {
    let mut set = ApprovedSet::default();
    set.approved.insert("TaggedUrl".to_owned());
    set.aliases.insert("HubUrl".to_owned(), "TaggedUrl".to_owned());

    let ty: syn::Type = syn::parse_quote!(Option<HubUrl>);
    assert!(unapproved_leaves(&ty, &set).is_empty());
}

#[test]
fn rejects_an_alias_to_an_unapproved_type() {
    let mut set = ApprovedSet::default();
    set.aliases.insert("Mystery".to_owned(), "NotDerived".to_owned());

    let ty: syn::Type = syn::parse_quote!(Option<Mystery>);
    assert_eq!(unapproved_leaves(&ty, &set), vec!["NotDerived".to_owned()]);
}

#[test]
fn still_rejects_a_bare_unapproved_type() {
    let set = ApprovedSet::default();
    let ty: syn::Type = syn::parse_quote!(Option<Undeclared>);
    assert_eq!(unapproved_leaves(&ty, &set), vec!["Undeclared".to_owned()]);
}

#[test]
fn collects_generic_aliases_from_a_declarations_only_root() {
    let file: syn::File = syn::parse_str("pub type HubUrl = TaggedUrl<Hub>;").expect("parses");
    let mut set = ApprovedSet::default();
    collect_declarations(&file, Root::DeclarationsOnly, &mut set);
    assert_eq!(set.aliases.get("HubUrl"), Some(&"TaggedUrl".to_owned()));
}

#[test]
fn tuple_alias_collection_is_unchanged() {
    let file: syn::File = syn::parse_str("pub type MediaRow = (i64, String);").expect("parses");
    let mut set = ApprovedSet::default();
    collect_declarations(&file, Root::Policed, &mut set);
    assert!(set.aliases.is_empty(), "tuple aliases keep their existing handling");
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url -- cargo nextest run --manifest-path xtask/Cargo.toml sqlx_newtype_decode`

Expected: FAIL — `ApprovedSet` has no `aliases` field.

- [ ] **Step 3: Implement against the tests**

Add `aliases: HashMap<String, String>` to `ApprovedSet`. In
`collect_declarations` (`:1071-1092`), alongside the existing tuple-alias arm,
collect `syn::Item::Type` whose `ty` is a `Type::Path`, mapping the alias ident
to the **last path segment ident** of its target (`HubUrl` → `TaggedUrl`). Do
this for **both** root kinds: the aliases live in `common/src`
(`Root::DeclarationsOnly`) while the policed decodes live in `storage/src`, so
`DeclarationsOnly`-scoped collection is what makes it cross-crate. Leave the
tuple-alias arm's behaviour exactly as it is —
`tuple_alias_collection_is_unchanged` pins that.

In `unapproved_leaves` (`:1098-1146`), before reporting a leaf as unapproved,
resolve it through `aliases` once and re-check `approved`. Report the
**resolved** name so the failure message names the underlying type.

- [ ] **Step 4: Run the tests, verify they pass**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url -- cargo nextest run --manifest-path xtask/Cargo.toml sqlx_newtype_decode`

Expected: PASS — five tests.

- [ ] **Step 5: Commit**

```bash
git add xtask/src/steps/sqlx_newtype_decode_check.rs
git commit -m "feat(xtask): resolve type aliases in the sqlx decode gate (#875)"
```

---

### Task 3: Introduce `TaggedUrl<T>`, `UrlRole`, and the fifteen roles

`AbsoluteUrl` is untouched in this task. The new type lands alongside it, fully
tested in isolation, so task 4's large migration is preceded by a green commit
that proves the type itself works.

**Files:**

- Create: `common/src/tagged_url.rs`
- Modify: `common/src/lib.rs` (add `pub mod tagged_url;`)
- Test: in-file `#[cfg(test)]` in `common/src/tagged_url.rs`

**Interfaces:**

- Consumes: task 1's generic-capable `StrNewtype`.
- Produces:
  - `pub trait UrlRole {}`
  - `pub struct TaggedUrl<T: UrlRole>(String, PhantomData<fn() -> T>);`
  - `pub struct InvalidUrl;`
  - Fifteen marker structs and aliases: `Base`/`BaseUrl`, `Hub`/`HubUrl`,
    `Feed`/`FeedUrl`, `CollectionHref`/`CollectionHrefUrl`,
    `Canonical`/`CanonicalUrl`, `Permalink`/`PermalinkUrl`,
    `MediaOrigin`/`MediaSourceUrl`, `Pagination`/`PaginationUrl`,
    `EntryId`/`EntryIdUrl`, `EditUri`/`EditUriUrl`,
    `EditMediaUri`/`EditMediaUriUrl`, `ContentSrc`/`ContentSrcUrl`,
    `ServiceDoc`/`ServiceDocUrl`, `Homepage`/`HomepageUrl`,
    `MailConfirm`/`MailConfirmUrl`
  - `pub fn compose<U: UrlRole>(base: &BaseUrl, path: &str) -> TaggedUrl<U>`
  - `impl<T: UrlRole> TaggedUrl<T> { pub fn join<U: UrlRole>(&self, path: &str) -> Result<TaggedUrl<U>, InvalidUrl>; pub fn with_query_pairs<U: UrlRole>(&self, pairs: &[(&str, &str)]) -> TaggedUrl<U>; pub fn retag<U: UrlRole>(self) -> TaggedUrl<U>; }`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn parses_and_normalizes() {
    let b: BaseUrl = "HTTPS://Example.COM:443".parse().expect("parses");
    assert_eq!(b, *"https://example.com/");
}

#[test]
fn rejects_non_http_schemes() {
    assert!("ftp://example.com".parse::<BaseUrl>().is_err());
    assert!("nonsense://x".parse::<HubUrl>().is_err());
    assert!("".parse::<HubUrl>().is_err());
}

#[test]
fn compose_mints_the_ascribed_role() {
    let base: BaseUrl = "https://example.com".parse().expect("parses");
    let feed: FeedUrl = compose(&base, "/feed.xml");
    assert_eq!(feed, *"https://example.com/feed.xml");
}

#[test]
fn with_query_pairs_mints_a_new_role() {
    let feed: FeedUrl = "https://example.com/posts".parse().expect("parses");
    let next: PaginationUrl = feed.with_query_pairs(&[("page", "2")]);
    assert_eq!(next, *"https://example.com/posts?page=2");
}

#[test]
fn join_mints_a_new_role() {
    let base: BaseUrl = "https://example.com".parse().expect("parses");
    let edit: EditUriUrl = base.join("/edit/1").expect("joins");
    assert_eq!(edit, *"https://example.com/edit/1");
}

#[test]
fn retag_preserves_the_bytes() {
    let feed: FeedUrl = "https://example.com/feed.xml".parse().expect("parses");
    let id: EntryIdUrl = feed.clone().retag();
    assert_eq!(id.as_ref(), feed.as_ref());
}

#[test]
fn roles_are_clonable_and_hashable_without_bounds_on_the_tag() {
    use std::collections::HashSet;
    let a: HubUrl = "https://hub.example.com".parse().expect("parses");
    let mut set = HashSet::new();
    set.insert(a.clone());
    assert!(set.contains(&a));
}

#[test]
fn serde_round_trips_transparently() {
    let a: HubUrl = "https://hub.example.com/".parse().expect("parses");
    let json = serde_json::to_string(&a).expect("serializes");
    assert_eq!(json, "\"https://hub.example.com/\"");
    assert_eq!(serde_json::from_str::<HubUrl>(&json).expect("deserializes"), a);
}
```

Port the remaining cases from `absolute_url.rs:128-251` onto `BaseUrl` — the
existing normalization, `join`, and `with_query_pairs` coverage must not be
lost.

Add the roles-do-not-unify proof, with its companion:

````rust
/// Distinct roles are distinct types.
///
/// ```compile_fail
/// # use common::tagged_url::{BaseUrl, HubUrl};
/// # fn takes_hub(_: &HubUrl) {}
/// # let base: BaseUrl = "https://example.com".parse().unwrap();
/// takes_hub(&base);
/// ```
///
/// The matching role compiles:
///
/// ```
/// # use common::tagged_url::{BaseUrl, HubUrl};
/// # fn takes_hub(_: &HubUrl) {}
/// # let base: BaseUrl = "https://example.com".parse().unwrap();
/// let hub: HubUrl = "https://hub.example.com".parse().unwrap();
/// takes_hub(&hub);
/// ```
````

And the proof that `compose` starts only at a `Base`:

````rust
/// `compose` starts from the site root and nothing else.
///
/// ```compile_fail
/// # use common::tagged_url::{compose, BaseUrl, FeedUrl, HubUrl};
/// # let hub: HubUrl = "https://hub.example.com".parse().unwrap();
/// let feed: FeedUrl = compose(&hub, "/feed.xml");
/// ```
///
/// From a `BaseUrl` it compiles:
///
/// ```
/// # use common::tagged_url::{compose, BaseUrl, FeedUrl, HubUrl};
/// # let hub: HubUrl = "https://hub.example.com".parse().unwrap();
/// let base: BaseUrl = "https://example.com".parse().unwrap();
/// let feed: FeedUrl = compose(&base, "/feed.xml");
/// ```
````

- [ ] **Step 2: Run the tests, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url -- cargo nextest run -p common tagged_url`

Expected: FAIL — `common::tagged_url` does not exist.

- [ ] **Step 3: Implement against the tests**

```rust
pub trait UrlRole {}

#[derive(StrNewtype)]
pub struct TaggedUrl<T: UrlRole>(String, PhantomData<fn() -> T>);
```

Hand-write `Clone`, `Debug`, `PartialEq`, `Eq`, and `Hash` rather than deriving
them — `std`'s derives add `T: Clone`-style bounds on a marker that is never
stored, which `roles_are_clonable_and_hashable_without_bounds_on_the_tag` pins.

`FromStr` is the existing `AbsoluteUrl` body verbatim
(`url::Url::parse(s.trim())`, scheme must be `http`/`https`, store
`url.to_string()`), now `impl<T: UrlRole> FromStr for TaggedUrl<T>` returning
`InvalidUrl`. This is the single validation chokepoint every role shares.

`join`, `with_query_pairs`, and `compose` keep their existing bodies; only their
signatures gain the free output parameter `U`. `retag` is
`TaggedUrl(self.0, PhantomData)`, with the mandatory doc comment:

```rust
/// Assert that this URL plays a different role.
///
/// **Every call site MUST carry a comment stating the domain identity it
/// asserts** — e.g. "the collection URL *is* the feed id". A retag with no
/// such justification is a review failure (#875).
```

Declare the fifteen markers and aliases. The tag is `MediaOrigin`, **not**
`MediaSource` — `common/src/media.rs:608` already declares
`pub enum MediaSource`, and both the sqlx decode gate and
`server_fn_tracing_check` reduce types to a bare ident.

- [ ] **Step 4: Run the tests, verify they pass**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url -- cargo nextest run -p common`
then:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url -- cargo test --workspace --doc`

Expected: PASS. `AbsoluteUrl` still exists and every other crate is untouched.

- [ ] **Step 5: Commit**

```bash
git add common/src/tagged_url.rs common/src/lib.rs
git commit -m "feat(common): add TaggedUrl<T> and the fifteen URL roles (#875)"
```

---

### Task 4: The migration — retype every site and delete `AbsoluteUrl`

**This is one commit.** See the Review header for why it cannot be split: the
generic output tag makes every `compose` site change at once, and the `Base`
retype is viral through every `base: &AbsoluteUrl` parameter.

**Files:**

- Delete: `common/src/absolute_url.rs`; remove `pub mod absolute_url;` from
  `common/src/lib.rs`
- Modify (`common`): `site.rs:17`, `config_key.rs:165,169`, `feed/mod.rs:37`,
  `feed/metadata.rs:13-26`, `feed/{atom,rss,json}.rs`,
  `atompub/entry.rs:340-356,410-420`, `atompub/service.rs:20,117,123`,
  `atompub/rsd.rs:24`, `test_support/mod.rs:49`, `root_relative_url.rs:1`,
  `media.rs:757` (doc link)
- Modify (`storage`): `site_config.rs:6,166,174,209,229,285`,
  `media.rs:5,32,39,189,193,220`, `helpers.rs:12,324,332`
- Modify (`jaunder`): `websub/{mod,http,file_capture,noop}.rs`,
  `feed/{worker,regenerate}.rs`,
  `atompub/{mod,posts,media,service,rsd,mapping}.rs`, `commands.rs:302`,
  `cli.rs:402`, `tests/helpers/websub_capturing.rs`,
  `tests/feed/feed_worker.rs`, `tests/atompub/`, `tests/web/web_site.rs:122,171`
- Modify (`web`): `site/{api,component}.rs`, `mail.rs:21`, `email/api.rs:45`,
  `invites/api.rs:71`, `password_reset/api.rs:62`, `posts/component.rs:1454`,
  `media/component.rs:114`
- Modify (`xtask`): `steps/server_fn_tracing_check.rs:81`
- Modify: `end2end/tests/admin-site.spec.ts:36,43` (comments)

**Interfaces:**

- Consumes: everything from tasks 1–3.
- Produces:
  - `async fn send_publish(&self, hub_url: &HubUrl, feed_url: &FeedUrl) -> Result<(), WebSubError>`
    on the trait and all five impls
  - `pub fn render_rsd_document(service_url: &ServiceDocUrl, homepage_url: &HomepageUrl) -> String`
  - `FeedMetadata { canonical_url: CanonicalUrl, self_url: FeedUrl, hub_url: Option<HubUrl>, … }`;
    `FeedItem.permalink: PermalinkUrl`
  - `FeedMeta { id: EntryIdUrl, self_url: FeedUrl, first/next/previous: Option<PaginationUrl> }`
  - `MediaLinkEntry { id: EntryIdUrl, edit_uri: EditUriUrl, edit_media_uri: EditMediaUriUrl, content_src: ContentSrcUrl }`
  - `CollectionDecl.href: CollectionHrefUrl`;
    `SiteIdentity.base_url: Option<BaseUrl>`;
    `FeedsConfig.websub_hub_url: Option<HubUrl>`;
    `MediaRecord.source_url: Option<MediaSourceUrl>`
  - `pub fn parse_url<T: UrlRole>(s: &str) -> TaggedUrl<T>` replacing
    `parse_absolute_url`

- [ ] **Step 1: Write the failing transposition proofs**

These four are the issue's acceptance. Each carries `#`-hidden fixture lines and
a positive companion in the same doc comment (`CONTRIBUTING.md:469-481`).

On `WebSubClient::send_publish` in `server/src/websub/mod.rs` — note the crate
is **`jaunder`**, not `server`:

````rust
/// Ping a WebSub hub with a feed URL.
///
/// The two parameters are distinctly typed, so transposing them is a compile
/// error (#875):
///
/// ```compile_fail
/// # use jaunder::websub::{NoopWebSubClient, WebSubClient};
/// # use common::tagged_url::{FeedUrl, HubUrl};
/// # async fn f(client: &NoopWebSubClient, hub: &HubUrl, feed: &FeedUrl) {
/// client.send_publish(feed, hub).await.ok();
/// # }
/// ```
///
/// The correct order compiles:
///
/// ```
/// # use jaunder::websub::{NoopWebSubClient, WebSubClient};
/// # use common::tagged_url::{FeedUrl, HubUrl};
/// # async fn f(client: &NoopWebSubClient, hub: &HubUrl, feed: &FeedUrl) {
/// client.send_publish(hub, feed).await.ok();
/// # }
/// ```
````

On `render_rsd_document` in `common/src/atompub/rsd.rs`:

````rust
/// ```compile_fail
/// # use common::atompub::rsd::render_rsd_document;
/// # use common::tagged_url::{HomepageUrl, ServiceDocUrl};
/// # fn f(service: &ServiceDocUrl, homepage: &HomepageUrl) {
/// let _ = render_rsd_document(homepage, service);
/// # }
/// ```
///
/// The correct order compiles:
///
/// ```
/// # use common::atompub::rsd::render_rsd_document;
/// # use common::tagged_url::{HomepageUrl, ServiceDocUrl};
/// # fn f(service: &ServiceDocUrl, homepage: &HomepageUrl) {
/// let _ = render_rsd_document(service, homepage);
/// # }
/// ```
````

On `FeedMetadata` in `common/src/feed/metadata.rs` and on `MediaLinkEntry` in
`common/src/atompub/entry.rs`, the same two-fence shape, swapping
`canonical_url`/`self_url` and `edit_uri`/`edit_media_uri` respectively.

- [ ] **Step 2: Run the doctests, verify they fail**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url -- cargo test --workspace --doc`

Expected: FAIL — the parameters and fields are still one type, so all four
`compile_fail` fences compile and the doctests fail.

- [ ] **Step 3: Migrate**

Delete `common/src/absolute_url.rs` and its `pub mod` line, then retype every
site above. The compiler drives this: each error is a value whose role must be
named.

Mints get their role by **ascription**:

```rust
// server/src/atompub/rsd.rs:36,38 — the site inference would have silently broken
let service_url: ServiceDocUrl = compose(&base, ATOMPUB_SERVICE_PATH);
let homepage_url: HomepageUrl = compose(&base, &format!("/~{username}"));

// server/src/feed/worker.rs:230
let absolute: FeedUrl = compose(base, feed_url);

// web/src/email/api.rs:45 and the three sibling mail sites
let verify: MailConfirmUrl = compose(&base_url, "/verify-email");
let link = format!("{verify}?token={token}");
```

Sites consumed inline take the turbofish exception:

```rust
// server/src/atompub/mapping.rs:169
compose::<Permalink>(base_url, &alt_path).to_string()
```

The four `retag` sites, each with the mandatory justification comment:

```rust
// server/src/atompub/media.rs
let edit: EditUriUrl = compose(&base, &edit_path);
let binary: EditMediaUriUrl = compose(&base, &binary_path);
MediaLinkEntry {
    // The member URL *is* the entry's atom:id — the edit URI is the canonical
    // identifier in the AtomPub member representation.
    id: edit.clone().retag(),
    edit_uri: edit,
    edit_media_uri: binary.clone(),
    // The content source *is* the media binary; one resource, two link rels.
    content_src: binary.retag(),
}
```

```rust
// server/src/atompub/posts.rs
let collection_url: FeedUrl = compose(&base, &collection_path);
let next: Option<PaginationUrl> = has_next.then(|| collection_url.with_query_pairs(&[…]));
let meta = FeedMeta {
    // The collection URL *is* the feed's atom:id.
    id: collection_url.clone().retag(),
    self_url: collection_url.clone(),
    // The collection URL *is* its own first page.
    first: Some(collection_url.retag()),
    next,
    previous,
};
```

Rename `test_support::parse_absolute_url` to `parse_url<T: UrlRole>` and give
every one of its ~18 call-site files its role. The rename is atomic — it lands
here.

Carve-outs that stay flattened, both ADR-0063 §5-sanctioned and unchanged:
`websub/http.rs:46-49` hands `.as_ref()` to `reqwest`'s sealed `IntoUrl`;
`atompub/mapping.rs:175`'s `id: edit_uri.to_string()` feeds
`atom_syndication::Entry.id`, a `String`.

`config_key.rs:165,169` take `HubUrl` and `BaseUrl` — single idents, so the
`$value:tt` matcher needs no macro edit. `web/src/site/component.rs:67,94-95`
become `Field::<BaseUrl>` and `<ValidatedInput<BaseUrl,>>`; the alias keeps the
Leptos `view!` generic argument a single ident.

`xtask/src/steps/server_fn_tracing_check.rs:81` becomes
`("BaseUrl", "operator-configured site base URL")` — **in this commit**, because
`web/src/site/api.rs:25` is a server-fn argument and the gate would otherwise
fail.

`file_capture.rs:32-36` keeps its JSON keys, so `end2end/tests/websub.ts:40-41`
needs no edit.

Add storage coverage in the repo's dual-backend shape:

```rust
#[apply(backends)]
#[tokio::test]
async fn websub_hub_url_round_trips_as_hub_role(#[case] backend: Backend) {
    let env = backend.setup().await;
    let store = &*env.state.site_config;
    store
        .set(SiteConfigKey::FeedsWebsubHubUrl, "https://hub.example.com/")
        .await
        .unwrap();
    let want: HubUrl = "https://hub.example.com/".parse().unwrap();
    assert_eq!(store.get_feeds_websub_hub_url().await.unwrap(), Some(want));
}

#[apply(backends)]
#[tokio::test]
async fn a_malformed_hub_url_row_is_purged(#[case] backend: Backend) {
    let env = backend.setup().await;
    let store = &*env.state.site_config;
    store
        .set(SiteConfigKey::FeedsWebsubHubUrl, "nonsense://x")
        .await
        .unwrap();
    assert_eq!(store.get_feeds_websub_hub_url().await.unwrap(), None);
}
```

and in `storage/src/media.rs`'s test module, exercising the `MediaSourceUrl`
decode through `create_media`, which already binds `record.source_url`
(`storage/src/media.rs:220`) — no raw-seed helper is needed or exists.

- [ ] **Step 4: Run everything, verify it passes**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url -- cargo test --workspace --doc`
then:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url -- cargo nextest run -p common -p storage -p jaunder -p web`
then:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url -- cargo xtask check`

Expected: PASS — all four transposition proofs now fail to compile as required,
every backend is green, and rendered feed/XML output is byte-identical.

- [ ] **Step 5: Commit**

```bash
git add common storage server web xtask/src/steps/server_fn_tracing_check.rs end2end/tests/admin-site.spec.ts
git commit -m "feat(common): replace AbsoluteUrl with role-tagged TaggedUrl<T> (#875)"
```

---

### Task 5: ADR draft and ADR-0063 amendment

**Files:**

- Create: `docs/adr/drafts/role-tagged-site-urls.md` (gitignored — see step 3)
- Modify: `docs/adr/0063-domain-value-newtype-convention.md`

**Interfaces:**

- Consumes: the finished implementation.
- Produces: the recorded decision. Do **not** number the ADR —
  `cargo xtask adr promote` assigns it at ship (**jaunder-adr**).

- [ ] **Step 1: Write the draft**

`cp docs/adr/template.md docs/adr/drafts/role-tagged-site-urls.md`. Line 1 must
be exactly `# ADR-DRAFT: Role-tagged site URLs`; leave `- Status: proposed`.

_Context_ — the #688 residual and the four measured hazards. _Decision_ records:

- `TaggedUrl<T: UrlRole>`, tag as pure role label, one shared validator.
- The three minting doors, and that **the output role is stated explicitly at
  every compose** — with the `rsd.rs` example showing why inference is
  insufficient there and why `send_publish` was protected only incidentally.
- **The alias rule**, and the two gates that depend on it.
- **The `retag` justification rule.**
- The **§1 cost-model argument** from the spec: a role costs two lines, so §1's
  per-type cost model does not apply; partial classification is unstable; four
  hazards is a lower bound.

_Consequences_ — fifteen roles today and the two-line cost of a sixteenth; the
four `compile_fail` doctests as the regression surface; `RootRelativeUrl` and
`FeedPath` stayed out; #751, #827, #879 unaffected.

- [ ] **Step 2: Amend ADR-0063**

Add a short subsection to §1 cross-referencing
`docs/adr/drafts/role-tagged-site-urls.md`, stating that URL roles are handled
by that scheme and that §1's per-type cost model should not be applied to a
role. Cite it **by path** — there is no bare `ADR-DRAFT` token, and `promote`
rewrites the path at ship. Do **not** touch `docs/README.md`; the table is
generated.

- [ ] **Step 3: Verify and commit**

Run:
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url -- cargo xtask check`

Expected: PASS. The draft is gate-invisible until promotion.

`docs/adr/drafts/` is gitignored, so only the amendment is staged. **The draft
is therefore not in the PR diff** — a reviewer checking spec criterion 16 must
look at the working tree, or wait for `cargo xtask adr promote` at ship, which
stages the numbered file.

```bash
git add docs/adr/0063-domain-value-newtype-convention.md
git commit -m "docs(adr): cross-reference the role-tagged URL scheme from ADR-0063 (#875)"
```

---

### Task 6: Full validation and acceptance walk

- [ ] **Step 1: Run the full gate**

Run in Bash background mode (long, cold):
`devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-875-hub-feed-url -- cargo xtask validate`

Expected: PASS — static, clippy, coverage, and all four
`{sqlite,postgres}×{chromium,firefox}` e2e combos.

- [ ] **Step 2: Walk the spec's seventeen criteria**

Three are verified by command rather than by a test; run them and record the
output:

```bash
rg '\bAbsoluteUrl\b' || true                                  # criterion 1 — expect matches only under docs/adr/
rg 'TaggedUrl<' --glob '!common/src/tagged_url.rs' || true      # criterion 10 — expect only turbofish mints
rg '\.retag\(\)|\.retag::<' --glob '!common/src/tagged_url.rs' || true  # criterion 9 — expect exactly four, each commented
```

Check the remaining fourteen against the tree.

- [ ] **Step 3: Hand off**

Work is complete; **jaunder-ship** takes it from here (final review,
`cargo xtask adr promote`, push, PR).

---

## Self-review

**Spec coverage.** All seventeen criteria map to tasks — 1→4 (verified in 6),
2→3/4, 3→3, 4→4, 5→4, 6→1/3/4 (every `compile_fail` carries a companion), 7→1,
8→4, 9→4 (verified in 6), 10→4 (verified in 6), 11→2, 12→4, 13→4, 14→4, 15→4,
16→5, 17→6. Criteria 1, 9, and 10 are properties verified by command in task 6,
not by an automated gate; that is deliberate and stated.

**Placeholders.** None. Every command names a real package (`jaunder`, not
`server`; `--manifest-path xtask/Cargo.toml`), every storage test uses the
`#[apply(backends)]` template, and no invented helper (`set_raw_source_url`,
`site_config_store`) survives.

**Type consistency.** `compose<U: UrlRole>(base: &BaseUrl, …)` has one
signature, introduced in task 3 and used unchanged in task 4. The tag
`MediaOrigin` and its alias `MediaSourceUrl` differ by design; task 3 states
why. `parse_absolute_url` → `parse_url<T>` happens once, atomically, in task 4.
