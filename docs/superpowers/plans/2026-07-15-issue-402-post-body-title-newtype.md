# `PostBody` / `PostTitle` Newtypes — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Introduce infallible `PostBody`/`PostTitle` string newtypes in
`common` and thread them through storage, the render pipeline, the web
`#[server]` boundary, and atompub mapping, so raw post source is a distinct type
from the already-shipped `RenderedHtml`.

**Architecture:** Both types use the ADR-0063 str-newtype trailer via a **new
`#[str_newtype(infallible)]` derive mode** (this plan's Task 1) — `From<String>`
is the single hand-written construction chokepoint (pure-wrap for `PostBody`,
whitespace-trim for `PostTitle`), the derive generates the rest and routes
`Deserialize` through that `From<String>`. Threading proceeds inside-out:
persistence + render pipeline first (Task 3), then the web/atompub boundaries
(Task 4).

**Tech Stack:** Rust, `macros` proc-macro crate (syn/quote), `serde`, `sqlx`
dual-backend storage, Leptos `#[server]` fns, `cargo nextest` /
`cargo test --doc`.

**Spec:**
[`docs/superpowers/specs/2026-07-15-issue-402-post-body-title-newtype.md`](../specs/2026-07-15-issue-402-post-body-title-newtype.md)
— this plan is the "how"; consult the spec for "what/why" (decisions D1–D5,
acceptance AC-1–7). Do not re-derive them here.

## Global Constraints

- **No length bound, no rejection** — construction is infallible
  (`From<String>`, never `TryFrom`/`FromStr`) (spec D1).
- **`PostBody` wraps verbatim; `PostTitle` trims outer whitespace** (case +
  inner whitespace preserved) (spec D2).
- **Backend parity:** storage tests are dual-backend per `CONTRIBUTING.md`; do
  not add a bare `#[tokio::test]` where a dual-backend test is required.
- **Coverage:** the `macros` crate is coverage-measured — new derive arms need
  in-crate `parse_quote!` unit tests (not just usage from `common`).
- **Commit gate:** each task's commit runs the full `cargo xtask check` via the
  pre-commit hook; run it first so it passes clean (**jaunder-commit**). **No
  `Co-Authored-By` trailer.**
- **Excluded from the sweep** (not post domain values — leave as-is):
  Atom-native `Text` and feed-label `String`s in `common/src/atompub/entry.rs`
  (`CollectionMeta.title`, `MediaEntryMeta.title`), and
  `common/src/feed/metadata.rs`'s `FeedItem.title`.

---

## Review layer (summary — this is what the plan-approval gate reads)

**Scope in:** the `infallible` macro mode; the two newtypes; retyping every
post-`body`/`title` site across `common` (render), `storage`, `web`, and
`server/atompub`. **Scope out:** any validation/length bound;
`ValidatedInput`/client-side validation (infallible ⇒ nothing to pre-validate);
`RenderedHtml` changes; feed-native title types (see Global Constraints). No
separable follow-up issues.

**Tasks:**

1. **Macro `infallible` mode** — teach `StrNewtype` `#[str_newtype(infallible)]`
   (emit `From<X> for String` + serde-via-`From<String>`, omit
   `TryFrom`/`FromStr`; reject `infallible+secret`/`infallible+serde`);
   rustdoc + `parse_quote!` tests + ADR-0063 note. → AC-macro-infallible,
   AC-macro-reject.
2. **`PostBody` + `PostTitle`** — two `common` modules, infallible
   `From<String>` (pure-wrap / trim), unit tests, `compile_fail` transposition
   doctest, lib.rs registration. → AC-postbody-type, AC-posttitle-trim,
   AC-transposition.
3. **Persistence & render pipeline speak the newtypes** — `render(&PostBody)`,
   `DerivedPostMetadata.title: Option<PostTitle>`, storage
   records/inputs/service/ helpers/test_support; web/atompub adapt at their
   seams with temporary conversions. One compilable commit; storage + render
   suites green. → AC-boundary (storage/render half).
4. **Boundaries speak the newtypes** — web `create_post`/`update_post` wire
   args + DTOs, atompub `mapping.rs`; remove Task 3's seam conversions. One
   compilable commit; web + server suites green. → AC-boundary (web/atompub
   half).

Full-branch `cargo xtask validate --no-e2e` (AC-gate) is confirmed at ship
(**jaunder-ship**).

**Key risks/decisions:** (a) the derive must **omit** `TryFrom` in infallible
mode — a hand-written `From<String>` + the fallible `TryFrom` would collide via
std's blanket `impl<T,U:Into<T>> TryFrom<U>` (spec §Risks). (b) The org render
path feeds `render()` the `canonicalize_org_body` `String`, which is re-wrapped
`.into()` a `PostBody` before rendering (spec D4). (c)
`Option<PostTitle>::as_deref()` still yields `Option<&str>` (PostTitle:
`Deref<str>`), so existing `metadata.title` assertions need no change.

---

## Task 1: Macro `infallible` mode

**Files:**

- Modify: `macros/src/str_newtype.rs` (add `infallible` to `Opts`/`parse_opts`;
  new `expand` branch; infallible serde)
- Modify: `macros/src/lib.rs:24-143` (rustdoc for `str_newtype_derive`) and
  `macros/src/lib.rs:208-296` (`#[cfg(test)] mod tests`)
- Modify: `docs/adr/0063-domain-value-newtype-convention.md` (one-line note)

**Interfaces:**

- Consumes: the existing `StrNewtype` derive machinery (`expand`, `parse_opts`,
  `serde_impls`, `require_newtype_shape`).
- Produces: `#[str_newtype(infallible)]` — a mode where the derive emits
  `Display`, `AsRef<str>`, `Borrow<str>`, `Deref<Target=str>`,
  `From<X> for String`, `PartialEq<str>`, `PartialEq<&str>`, a borrowing
  `Serialize`, and a `Deserialize` that routes a deserialized `String` through
  the type's own `From<String>`; it **omits** `TryFrom<String>`, `FromStr`
  routing. The type author hand-writes `From<String>`. Rejects `infallible`
  combined with `secret` or `serde`.

- [ ] **Step 1: Write the failing tests** — add to `macros/src/lib.rs`
      `mod tests` (mirroring `str_newtype_secret_selects_redacting_trailer`,
      lib.rs:250-261):

```rust
#[test]
fn str_newtype_infallible_emits_from_string_serde_and_omits_fallible_door() {
    // Infallible mode: Display/AsRef/Deref/Serialize/Deserialize present; the
    // fallible door (TryFrom / FromStr routing) is absent — the author writes
    // From<String> and Deserialize routes through it.
    let input: DeriveInput = parse_quote! {
        #[str_newtype(infallible)]
        struct X(String);
    };
    let out = str_newtype::expand(&input).to_string();
    assert!(out.contains("Display"));
    assert!(out.contains("Deref"));
    assert!(out.contains("Serialize"));
    assert!(out.contains("Deserialize"));
    assert!(!out.contains("TryFrom"));
    assert!(!out.contains("FromStr"));
}

#[test]
fn str_newtype_infallible_with_secret_emits_compile_error() {
    let input: DeriveInput = parse_quote! {
        #[str_newtype(infallible, secret)]
        struct X(String);
    };
    assert!(str_newtype::expand(&input).to_string().contains("compile_error"));
}

#[test]
fn str_newtype_infallible_with_serde_emits_compile_error() {
    let input: DeriveInput = parse_quote! {
        #[str_newtype(infallible, serde)]
        struct X(String);
    };
    assert!(str_newtype::expand(&input).to_string().contains("compile_error"));
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p macros infallible` Expected: FAIL — `parse_opts`
rejects the unknown `infallible` option (all three assert on behavior not yet
implemented).

- [ ] **Step 3: Implement against the tests** — in `macros/src/str_newtype.rs`:
  1. Add `infallible: bool` to `struct Opts` (line 10-13) and to `parse_opts`
     (line 171-196): accept `meta.path.is_ident("infallible")`; **before** the
     existing `serde && !secret` guard, add
     `if infallible && (secret || serde) { return Err(syn::Error::new_spanned(input, "`str_newtype(infallible)`is exclusive with`secret`/`serde`")); }`
     (ordered first so `infallible, serde` reports the infallible message rather
     than falling through to the serde-needs-secret guard — the reject test then
     proves the infallible path).
  2. In `expand` (line 19-106), after the `if opts.secret { … }` block and
     before the default trailer, add
     `if opts.infallible { return infallible_trailer(name); }`.
  3. Add `fn infallible_trailer(name) -> TokenStream` emitting the default
     trailer's
     `Display`/`AsRef`/`Borrow`/`Deref`/`From<#name> for String`/`PartialEq<str>`/
     `PartialEq<&str>` (identical to lines 46-102) **minus** the `TryFrom` impl
     (lines 75-81), plus a `serde_impls_infallible(name)`.
  4. Add `fn serde_impls_infallible(name)` — same `Serialize` as `serde_impls`
     (lines 114-122); a `Deserialize` whose body is
     `let s = <String as Deserialize>::deserialize(deserializer)?; Ok(<#name as ::core::convert::From<String>>::from(s))`.

  The three Step-1 tests pin the impl set (present/absent tokens) and both
  rejection branches, so the contract determines the body.

  Also update the `str_newtype_derive` rustdoc (`macros/src/lib.rs:24-143`) with
  a short `#[str_newtype(infallible)]` paragraph + a passing doctest fixture (a
  `struct Inf(String)` with a hand-written `impl From<String>`), mirroring the
  existing secret fixtures. And add a one-line note to
  `docs/adr/0063-domain-value-newtype-convention.md` (§Implementation / §3) that
  the derive now implements the trailer's "`From<String>` when infallible" half
  via `#[str_newtype(infallible)]`.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p macros` then `cargo test -p macros --doc` Expected:
PASS (nextest for the unit tests; `--doc` for the rustdoc fixtures — nextest
does not run doctests).

- [ ] **Step 5: Commit**

Run `cargo xtask check` first (jaunder-commit), then:

```bash
git add macros/src/str_newtype.rs macros/src/lib.rs docs/adr/0063-domain-value-newtype-convention.md
git commit -m "tooling(macros): add str_newtype infallible mode (#402)"
```

---

## Task 2: `PostBody` + `PostTitle` newtypes

**Files:**

- Create: `common/src/post_body.rs`, `common/src/post_title.rs`
- Modify: `common/src/lib.rs` (add `pub mod post_body;` and
  `pub mod post_title;`, alphabetically — after `pub mod password;` / before
  `pub mod render;`)
- Test: in-file `#[cfg(test)] mod tests` in each new module (the crate
  convention, per `username.rs`)

**Interfaces:**

- Consumes: `#[str_newtype(infallible)]` from Task 1;
  `common::render::RenderedHtml` (for the transposition doctest).
- Produces: `common::post_body::PostBody` and `common::post_title::PostTitle` —
  `#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)] #[str_newtype(infallible)]`
  tuple structs over `String`, each with a hand-written `impl From<String>`
  (`PostBody` pure-wrap; `PostTitle` `s.trim().to_owned()`). Both `Deref<str>`,
  `Display`, serde-as-plain-string, `From<Self> for String`.

- [ ] **Step 1: Write the failing tests** — `common/src/post_body.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_body_wraps_verbatim_without_trimming() {
        let b = PostBody::from("  # Heading\n\nbody  ".to_owned());
        assert_eq!(b, "  # Heading\n\nbody  ");
    }

    #[test]
    fn post_body_display_and_deref_expose_inner() {
        let b = PostBody::from("hello".to_owned());
        assert_eq!(b.to_string(), "hello");
        assert_eq!(&*b, "hello");
        assert!(b.contains("ell")); // str method via Deref
    }

    #[test]
    fn post_body_serde_round_trips_as_plain_string() {
        let b = PostBody::from("raw *body*".to_owned());
        assert_eq!(serde_json::to_string(&b).unwrap(), "\"raw *body*\"");
        assert_eq!(
            serde_json::from_str::<PostBody>("\"raw *body*\"").unwrap(),
            b
        );
    }

    #[test]
    fn post_body_into_string_extracts_inner() {
        assert_eq!(String::from(PostBody::from("x".to_owned())), "x");
    }
}
```

`common/src/post_title.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn post_title_trims_outer_whitespace_preserving_inner_and_case() {
        let t = PostTitle::from("  Hello  World  ".to_owned());
        assert_eq!(t, "Hello  World");
    }

    #[test]
    fn post_title_deserialize_trims() {
        assert_eq!(
            serde_json::from_str::<PostTitle>("\"  Trimmed \"").unwrap(),
            PostTitle::from("Trimmed".to_owned())
        );
    }

    #[test]
    fn post_title_serializes_as_plain_string() {
        assert_eq!(
            serde_json::to_string(&PostTitle::from("Title".to_owned())).unwrap(),
            "\"Title\""
        );
    }

    #[test]
    fn post_title_display_exposes_inner() {
        assert_eq!(PostTitle::from(" Hi ".to_owned()).to_string(), "Hi");
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run: `cargo nextest run -p common post_body post_title` Expected: FAIL —
`PostBody`/`PostTitle` not defined.

- [ ] **Step 3: Implement against the tests** — `common/src/post_body.rs`:

````rust
use macros::StrNewtype;

/// A post's raw source body. Infallible wrapper (no length bound — spec D1) that
/// keeps raw source distinct from rendered output ([`crate::render::RenderedHtml`]),
/// so the two ends of the render pipeline cannot be transposed. The ADR-0063
/// trailer is generated by `#[derive(StrNewtype)] #[str_newtype(infallible)]`;
/// `From<String>` is the single (pure-wrap) construction door.
///
/// A `PostBody` is not a `RenderedHtml` and neither converts to the other:
/// ```compile_fail
/// fn want_html(_: common::render::RenderedHtml) {}
/// want_html(common::post_body::PostBody::from("x".to_owned()));
/// ```
/// ```compile_fail
/// fn want_body(_: common::post_body::PostBody) {}
/// want_body(common::render::RenderedHtml::from_trusted("x"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
#[str_newtype(infallible)]
pub struct PostBody(String);

impl From<String> for PostBody {
    fn from(s: String) -> Self {
        Self(s)
    }
}
````

`common/src/post_title.rs`:

```rust
use macros::StrNewtype;

/// A post's title. Infallible wrapper that trims outer whitespace (case and inner
/// whitespace preserved — spec D2); no length bound. The ADR-0063 trailer is
/// generated by `#[derive(StrNewtype)] #[str_newtype(infallible)]`; `From<String>`
/// is the single (trimming) construction door, through which the derived
/// `Deserialize` also routes so wire values are trimmed identically.
#[derive(Clone, Debug, PartialEq, Eq, Hash, StrNewtype)]
#[str_newtype(infallible)]
pub struct PostTitle(String);

impl From<String> for PostTitle {
    fn from(s: String) -> Self {
        Self(s.trim().to_owned())
    }
}
```

Then register both modules in `common/src/lib.rs`. Every test branch
(wrap-vs-trim, Display/Deref, serde both directions, into-String) is pinned, so
the contract determines the bodies.

- [ ] **Step 4: Run the tests, verify they pass**

Run: `cargo nextest run -p common post_body post_title` then
`cargo test -p common --doc post_body` Expected: PASS (unit tests + the two
`compile_fail` transposition doctests).

- [ ] **Step 5: Commit**

Run `cargo xtask check` first, then:

```bash
git add common/src/post_body.rs common/src/post_title.rs common/src/lib.rs
git commit -m "types(common): add PostBody and PostTitle newtypes (#402)"
```

---

## Task 3: Persistence & render pipeline speak the newtypes

One atomic, compilable commit: the render pipeline and storage layer adopt the
newtypes; the web/atompub boundaries stay `String` and adapt at their seams with
**temporary** conversions removed in Task 4. Pure type-propagation — no behavior
change — so the existing storage + render suites are the contract.

**Files:**

- Modify: `common/src/render.rs` — `render` (line 123), `DerivedPostMetadata`
  (133), `derive_post_metadata` (142) + the `render(…)` call sites in that
  file's tests (render.rs:561/567/736).
- Modify: `storage/src/posts.rs` — `PostRecord.title/body` (48/52),
  `PostRevisionRecord.title/body` (113/117), `CreatePostInput.title/body`
  (194/196), `UpdatePostInput.title/body` (215/218), the SQL row-mappers
  (`.get`→`.into()`), `fallback_summary_label` (88-98, reads via `Deref` —
  likely no change).
- Modify: `storage/src/post_service.rs` — `PostCreation`/`PostUpdate` (411-417 /
  254-262), the three `render(&body, …)` sites (75/160/322), metadata reads.
- Modify: `storage/src/helpers.rs` (row→record trusted-rebuild),
  `storage/src/test_support.rs`.
- Modify (**temporary seam conversions, removed in Task 4**):
  `web/src/posts/mod.rs`, `web/src/posts/listing.rs`,
  `server/src/atompub/mapping.rs`, `server/src/atompub/posts.rs` (builds
  `PostCreation`/`PostUpdate` from `PostFields` at 380-388 / 502-506 —
  `body: fields.body.into()`; the `post.title.as_deref()` / `&post.body` reads
  at 86-87 work unchanged via `Deref`/`as_deref`) — only where the crate now
  needs an owned `String`/newtype conversion. Do **not** change their
  struct/param types here.

**Interfaces:**

- Consumes: `PostBody`, `PostTitle` (Task 2).
- Produces: `render(body: &PostBody, format: &PostFormat) -> RenderedHtml`;
  `DerivedPostMetadata { title: Option<PostTitle>, slug_seed: String, summary_label: String }`;
  storage `PostRecord`/`PostRevisionRecord`/`CreatePostInput`/ `UpdatePostInput`
  with `body: PostBody` and `title: Option<PostTitle>`;
  `PostCreation`/`PostUpdate` carrying `body: PostBody` and
  **`title: Option<&'a str>` unchanged** — the transient perform-aggregate
  borrows the owned input/record `PostTitle` as `&str` via
  `Deref`/`.as_deref()`, so its title type and every construction site
  (`… .title.as_deref()`) stay exactly as they are. Only owned title
  _storage/DTO fields_ become `Option<PostTitle>`; borrowed/transient title
  params stay `Option<&str>`.

- [ ] **Step 1: Retype the render pipeline** (`common/src/render.rs`)
  - `render`: `body: &str` → `body: &PostBody` (line 123). Internal
    `render_markdown(body)` / `render_org(body)` / `body.to_string()` take
    `&str` — pass `&**body` / `body` (Deref) unchanged.
  - `DerivedPostMetadata.title`: `Option<String>` → `Option<PostTitle>` (line
    135).
  - `derive_post_metadata`: keep `explicit_title: Option<&str>` and `body: &str`
    (they accept `&PostTitle`/`&PostBody` via `Deref` at call sites). At each of
    the three return sites (155/170/178) wrap the derived title into
    `PostTitle`: `title: Some(PostTitle::from(title.clone()))` while keeping
    `slug_seed: title` (a `String`). The `extract_*` helpers stay
    `String`-returning (private).
  - Update the in-file tests: `render("…", &fmt)` →
    `render(&PostBody::from("…".to_owned()), &fmt)` at render.rs:561/567/736.
    The `metadata.title.as_deref()` assertions (579/592/602/…) need **no**
    change — `Option<PostTitle>::as_deref()` yields `Option<&str>`.

- [ ] **Step 2: Retype the storage records/inputs + service** (`storage/src/*`)
  - `posts.rs`: the four structs' `body: String` → `PostBody`,
    `title: Option<String>` → `Option<PostTitle>`. Row-mappers:
    `row.get::<String, _>("body")` → `…get::<String,_>("body").into()`; `title`
    → `opt.map(PostTitle::from)`. SQL binds take `&str` via `&*record.body` /
    `record.title.as_deref()`.
  - `fallback_summary_label` (posts.rs:88-98): `self.body.lines()` reads fine
    via `Deref`, but the `.or_else(|| self.title.clone())` arm now yields
    `Option<PostTitle>` where `Option<String>` is expected — change it to
    `.or_else(|| self.title.clone().map(String::from))`.
  - `post_service.rs`: `PostCreation`/`PostUpdate` `body` → `PostBody`; leave
    `title: Option<&'a str>` **unchanged** — construction sites keep
    `title: input.title.as_deref()` (owned `PostTitle` → `&str`). The three
    `render(&body, …)` calls (75/160/322): markdown/html pass `&body` (now
    `&PostBody`); the **org** branch rebinds `body` from `canonicalize_org_body`
    (two sites, ~307-311 / ~466-470) — make that rebind produce a `PostBody`:
    `let body: PostBody = if matches!(format, PostFormat::Org) { canonicalize_org_body(&body).into() } else { body };`
    (or `.into()` on the existing org-branch assignment) so the later
    `render(&body, …)` sees `&PostBody` on both branches (spec D4).
    `derive_post_metadata(title, &body, …)` — `title` is already `Option<&str>`
    and `&body` derefs to `&str`, so these calls are unchanged.
  - `helpers.rs`, `test_support.rs`: follow — construct records/inputs with
    `.into()`.

- [ ] **Step 3: Adapt the web/atompub seams (temporary)** Build the workspace;
      wherever `web` or `server/atompub` now fails because a `String`
      field/param meets a newtype value (e.g. a DTO built from
      `record.body`/`record.title`, or `CreatePostInput` built from a `String`),
      insert a conversion: `String::from(x)` / `x.into()` /
      `opt.map(String::from)`. Leave all `web`/`server` struct and `#[server]`
      param **types unchanged** — Task 4 retypes them and deletes these
      conversions.

- [ ] **Step 4: Verify** — compiles + existing suites green

Run: `cargo xtask check` (fmt/clippy/coverage), then
`cargo nextest run -p common -p storage` and `cargo test -p common --doc`.
Expected: PASS — no behavior change; the dual-backend storage tests and render
tests exercise the new types end to end.

- [ ] **Step 5: Commit**

```bash
git add common/src/render.rs storage/src/ web/src/posts/ server/src/atompub/
git commit -m "types(storage): thread PostBody/PostTitle through storage and render (#402)"
```

---

## Task 4: Boundaries speak the newtypes

Retype the web `#[server]` boundary + DTOs and the atompub mapping to the
newtypes, deleting Task 3's temporary seam conversions. One compilable commit;
web + server suites are the contract (behavior unchanged — infallible decode).

**Files:**

- Modify: `web/src/posts/mod.rs` — `create_post`/`update_post` `body: String` →
  `PostBody`; DTOs `PostResponse.title/body` (190/192), `DraftSummary.title`
  (152).
- Modify: `web/src/posts/listing.rs` — `TimelinePostSummary.title` (35).
- Modify: `web/src/pages/posts.rs` — DTO consumers that break once the DTOs
  retype: `fetched.body.clone()` (686), `fetched.title.clone()` (219/551), and
  `draft.title.clone().unwrap_or(draft.summary_label…)` (992). Convert to
  `String` where a `String` signal/value is needed (`String::from(...)` /
  `.map(String::from)` before `.unwrap_or`), or read as `&str` via `Deref` where
  that suffices.
- Modify: `server/src/atompub/mapping.rs` — `PostFields.title/body` (15-19),
  `entry_to_post_fields` (72/82, inbound: build `PostFields` from Atom `Text`
  via `.into()`), `post_to_entry` (131/149, read post fields as `&str` via
  `Deref`/ `AsRef` into Atom `Text`).
- Modify: `server/src/atompub/posts.rs` — drop Task 3's `.into()` at the
  `PostCreation`/`PostUpdate` construction (`body: fields.body` is now already
  `PostBody`).
- Modify: the affected tests (`server/tests/web/web_posts.rs`,
  `server/tests/helpers/mod.rs`, and any web unit tests) — bodies built via
  `PostBody::from(...)` / `impl Into<PostBody>`.

**Interfaces:**

- Consumes: `PostBody`/`PostTitle` (Task 2); the retyped storage layer (Task 3).
- Produces: `create_post`/`update_post` with `body: PostBody` (a valid typed
  wire arg — infallible, so no client pre-validation, spec D1); web DTOs and
  atompub `PostFields` whose `body`/`title` are the newtypes. All Task 3 seam
  conversions deleted.

- [ ] **Step 1: Retype web boundary + DTOs**
  - `create_post`/`update_post`: `body: String` → `body: PostBody`. The
    `#[server]` serde bridge decodes any JSON string (`PostBody`'s derived
    `Deserialize`), so the compose/edit form needs no change. Where the fn built
    `CreatePostInput { body: body.into(), … }` in Task 3, `body` is already
    `PostBody` — drop the conversion.
  - `PostResponse`/`DraftSummary`/`TimelinePostSummary`: `title`/`body` fields
    adopt `Option<PostTitle>`/`PostBody`; delete the Task 3
    `String::from`/`.map(String::from)` conversions at their construction sites
    — assign the record fields directly.
  - `web/src/pages/posts.rs` DTO consumers: where a retyped field flows into a
    `String` signal/value (`fetched.body.clone()` at 686,
    `fetched.title.clone()` at 219/551,
    `draft.title.clone().unwrap_or(draft.summary_label…)` at 992), insert
    `String::from(...)` / `.map(String::from)` (the `.map` before `.unwrap_or`
    so the arms agree), or read via `Deref` where a `&str` suffices.

- [ ] **Step 2: Retype atompub mapping** (`server/src/atompub/mapping.rs`)
  - `PostFields.title/body` → `Option<PostTitle>`/`PostBody`.
    `entry_to_post_fields` builds them from Atom `Text`/content `String` via
    `.into()` (the inbound boundary parse). `post_to_entry` reads them as `&str`
    (`Deref`/`AsRef`) into Atom `Text`/content. Delete Task 3's seam conversions
    here.

- [ ] **Step 3: Update boundary tests** Update `server/tests/web/web_posts.rs`
      and `server/tests/helpers/mod.rs` body/title builders to construct
      `PostBody`/`PostTitle` (or accept `impl Into<PostBody>`). No new behavior
      — the assertions stand; only the constructed types change.

- [ ] **Step 4: Verify**

Run: `cargo xtask check`, then `cargo nextest run -p web -p server`. Expected:
PASS — the whole post create/edit/read + atompub round-trip flows through the
newtypes; behavior unchanged.

- [ ] **Step 5: Commit**

```bash
git add web/src/posts/ web/src/pages/posts.rs server/src/atompub/ server/tests/
git commit -m "types(web,server): PostBody/PostTitle at the web and atompub boundaries (#402)"
```

---

## Self-review notes

- **Spec coverage:** AC-macro-infallible/reject → Task 1; AC-postbody-type,
  AC-posttitle-trim, AC-transposition → Task 2; AC-boundary → Tasks 3+4; AC-gate
  → ship-time `validate --no-e2e`. All spec ACs mapped.
- **Type consistency:** `render(&PostBody) -> RenderedHtml`,
  `DerivedPostMetadata.title: Option<PostTitle>`, storage owned fields
  `body: PostBody` / `title: Option<PostTitle>`; transient perform-aggregate
  `title` stays `Option<&str>` (Deref-fed) — owned title fields become the
  newtype, borrowed title params do not. Used consistently across Tasks 3–4.
- **No placeholders:** every retype site carries a file:line; the two novel
  modules and the macro arm carry full code + tests.
