# `PageSize` range-newtype for pagination — Implementation Plan (issue #537)

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful — Task 5 is a good candidate). Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hand-clamped `u32` page size
(`limit.unwrap_or(50).clamp(1,50)`, `const PAGE_SIZE`, AtomPub
`DEFAULT/MAX_PAGE_SIZE`) with one `common::pagination::PageSize` range newtype
(`1..=50`), so the bound lives in the type.

**Architecture:** Extend the `NumNewtype` derive with an opt-in `clamp` flag
(emits `const MIN`/`MAX` + a `const fn clamped`), define `PageSize` with it,
then adopt the type at every page-size site: web `#[server]` args become
`Option<PageSize>` (out-of-range now rejects on the wire — clients only send the
constant), AtomPub keeps clamp semantics via `PageSize::clamped(...)` on a
still-`u32` query field (owner-approved flatten, #470).

**Tech Stack:** Rust, `macros` proc-macro crate (`syn`/`quote`), `common` crate,
`serde`, `cargo nextest`, `cargo xtask`.

Spec: `docs/superpowers/specs/2026-07-20-issue-537-page-size-newtype.md`
(referenced by AC number below; not restated).

## Global Constraints

- No `Co-Authored-By` trailer on commits. Do not commit without the per-task
  gate green.
- Accessor is `.value()` (not `.get()`). Bound `1..=50` and web default `50`
  live only in `PageSize`; AtomPub's `25` is its policy default, expressed as
  `PageSize::clamped(25)`.
- `clamped` is opt-in per type (the `clamp` flag) so
  `RetentionCount`/`FeedMin*`/media newtypes do **not** gain coercion.
- The macros crate **is** coverage-measured — new codegen branches need
  unit-test reach or a `// cov:ignore` on a genuinely unreachable arm (block
  form for reflow-prone lines).
- Web has host + wasm + server-feature targets; server-gated web code only
  compiles under `--all-features` — verify web with
  `cargo check -p web --all-features --all-targets`.
- Per-commit gate: `cargo xtask check` (fmt + clippy + Nix coverage/tests). Run
  it clean before each commit (**jaunder-commit**). Final gate:
  `cargo xtask validate --no-e2e` (AC10).

---

## Review header

**Scope — in:** the `NumNewtype` `clamp` affordance; `PageSize` + its test
helper + surface test; adoption at the web posts listing/drafts `#[server]` fns
and their fetchers + all Rust callers (pages, projector); AtomPub
`collection_get`; ADR-0063 amendment.

**Scope — out:** cursor/keyset logic, storage query signatures (stay raw `u32`),
AtomPub feed format. `web/src/media/api.rs` `list_my_media` (media default-50,
**no** `1..=50` bound today) — filed as a follow-up (Task 1), not folded in.

**Tasks:**

1. File the separable media-pagination-bound follow-up issue.
2. `macros`: add the `clamp` flag → `const MIN`/`MAX` + `const fn clamped`; fix
   stale `get()` doc comments to `value()`.
3. `common`: `PageSize` newtype (`pagination` module), `parse_page_size` helper,
   surface test.
4. AtomPub `collection_get`: adopt `PageSize::clamped`, drop the two consts, add
   a clamp regression test.
5. Web: `Option<PageSize>` across listing/drafts `#[server]` fns + fetchers; fix
   every Rust caller; delete `const PAGE_SIZE`.
6. Amend ADR-0063 with the `clamp` affordance.

**Key risks / decisions:**

- **`const fn clamped`** must hand-roll min/max (`Ord::clamp` isn't const) —
  const-legal for integer inners.
- **Web wire contract tightens** (Task 5): out-of-range `limit` now rejects, not
  clamps — safe only because every first-party client sends the constant;
  deliberate (ADR-0065).
- **Task 5 is broad but mechanical** — one atomic commit (signatures + all
  callers), or the tree won't compile. Delegate-friendly.

---

## Task 1: File the media-pagination follow-up (filed #556)

**Files:** none in-repo (GitHub issue via **jaunder-issues**).

- [x] **Step 1: File the follow-up issue.** Using **jaunder-issues** conventions
      (type `Task`, label `type-safety`, milestone "Domain-value type safety
      (newtypes)", reference #537 and this family), file: _"types: bounded
      page-size newtype for media listing (`list_my_media`)"_. Body:
      `web/src/media/api.rs` `list_my_media` uses `limit.unwrap_or(50)` with
      **no** upper bound; deferred from #537 because adopting `PageSize` would
      newly impose `max = 50` (a behavior change — clients can currently
      request >50 media items). Decide whether media pagination should gain a
      bounded page size (reuse `PageSize` or a media-specific bound) as a
      separate change. Add it to project #1.

- [x] **Step 2: Record the issue number** in this plan (edit the Task 1 heading
      to append `(filed #556)`) so ship-time archiving links it. No commit
      (issue-only task).

---

## Task 2: `NumNewtype` gains the opt-in `clamp` affordance

**Files:**

- Modify: `macros/src/num_newtype.rs` (add `clamp` to `Opts`, parse it,
  validate, emit `clamped_impl`; fix the stale `get()` doc at the module
  header).
- Modify: `macros/src/lib.rs:202` (stale `get()` → `value()` in the derive doc)
  and the `#[cfg(test)] mod tests` block (new unit tests).

**Interfaces:**

- Produces: `#[num_newtype(..., clamp)]` (requires `min` + `max`) → generates,
  on the newtype `X(I)`: `pub const MIN: I`, `pub const MAX: I`, and
  `#[must_use] pub const fn clamped(v: I) -> X` coercing `v` into `MIN..=MAX`.
  `clamp` without both bounds is a `compile_error!`.

- [x] **Step 1: Write the failing macro unit tests** (in `macros/src/lib.rs`
      tests module, alongside the existing `num_newtype_*` tests):

```rust
#[test]
fn num_newtype_clamp_emits_bounds_and_clamped_constructor() {
    let input: DeriveInput = parse_quote! {
        #[num_newtype(inner = u32, min = 1, max = 50, default = 50, clamp)]
        struct X(u32);
    };
    let out = num_newtype::expand(&input).to_string();
    assert!(!out.contains("compile_error"));
    assert!(out.contains("const MIN"));
    assert!(out.contains("const MAX"));
    assert!(out.contains("fn clamped"));
}

#[test]
fn num_newtype_clamp_without_both_bounds_emits_compile_error() {
    let input: DeriveInput = parse_quote! {
        #[num_newtype(inner = u32, min = 1, clamp)]
        struct X(u32);
    };
    assert!(num_newtype::expand(&input)
        .to_string()
        .contains("compile_error"));
}

#[test]
fn num_newtype_without_clamp_omits_clamped_constructor() {
    let input: DeriveInput = parse_quote! {
        #[num_newtype(inner = u32, min = 1, max = 50)]
        struct X(u32);
    };
    let out = num_newtype::expand(&input).to_string();
    assert!(!out.contains("fn clamped"));
    assert!(!out.contains("const MAX"));
}
```

- [x] **Step 2: Run them, verify they fail.** Run:
      `cargo nextest run --manifest-path macros/Cargo.toml num_newtype_clamp num_newtype_without_clamp`
      Expected: FAIL — `clamp` is an unknown option (today `parse_opts` rejects
      it) / `clamped` not emitted.

- [x] **Step 3: Implement the `clamp` support.** In `macros/src/num_newtype.rs`:
  - Add `clamp: bool` to `struct Opts`; in `parse_opts` add
    `let mut clamp = false;`, a branch
    `} else if meta.path.is_ident("clamp") { clamp = true; Ok(()) }` (a bare
    flag — do not call `meta.value()`), include `clamp` in the returned `Opts`,
    and extend the unknown-option message to list `clamp`. Update the
    `parse_opts` doc-comment option list.
  - In `expand`, after `require_field_matches_inner`, add the validation guard:
    ```rust
    if opts.clamp && (opts.min.is_none() || opts.max.is_none()) {
        return syn::Error::new_spanned(
            &input.ident,
            "num_newtype `clamp` requires both `min` and `max`",
        )
        .to_compile_error();
    }
    ```
    and emit `let clamped = clamped_impl(name, &opts);`, adding `#clamped` to
    the final `quote! { ... }`.
  - Add:

    ```rust
    /// The opt-in `clamp` affordance: `const MIN`/`MAX` and an infallible `const fn clamped`
    /// that coerces into `MIN..=MAX`. `Ord::clamp` isn't `const`, so the bound is hand-rolled;
    /// `<`/`>` on integer inners are const-evaluable. Emitted only under the `clamp` flag, which
    /// `expand` has already proven carries both bounds.
    fn clamped_impl(name: &Ident, opts: &Opts) -> TokenStream {
        if !opts.clamp {
            return quote! {};
        }
        let (Some(min), Some(max)) = (opts.min.as_ref(), opts.max.as_ref()) else {
            // cov:ignore-start unreachable: `expand` rejects `clamp` without both bounds
            return quote! {};
            // cov:ignore-stop
        };
        let inner = &opts.inner;
        quote! {
            impl #name {
                #[doc = "Inclusive lower bound of the declared range."]
                pub const MIN: #inner = #min;
                #[doc = "Inclusive upper bound of the declared range."]
                pub const MAX: #inner = #max;

                #[doc = "Coerce `v` into `MIN..=MAX`; infallible (the result is always in range)."]
                #[must_use]
                pub const fn clamped(v: #inner) -> Self {
                    let v = if v < Self::MIN {
                        Self::MIN
                    } else if v > Self::MAX {
                        Self::MAX
                    } else {
                        v
                    };
                    Self(v)
                }
            }
        }
    }
    ```

  - Fix stale docs (per docs-track-late-API-changes, AC9): `num_newtype.rs`
    module header line 5 `a \`get()\` accessor`→`a \`value()\`
    accessor`; `macros/src/lib.rs:202` `error type, \`get()\`,`→`error type,
    \`value()\`,`.

- [x] **Step 4: Run the macro tests, verify they pass.** Run:
      `cargo nextest run --manifest-path macros/Cargo.toml` Expected: PASS (new
      tests + all existing `num_newtype_*`).

- [x] **Step 5: Commit.** Run `cargo xtask check` first (must be clean).
  ```bash
  git add macros/src/num_newtype.rs macros/src/lib.rs
  git commit -m "feat(macros): add opt-in clamp affordance to NumNewtype (#537)"
  ```

---

## Task 3: `PageSize` newtype + test helper

**Files:**

- Create: `common/src/pagination.rs`
- Modify: `common/src/lib.rs` (add `pub mod pagination;`, alphabetical — between
  `media` and `password`).
- Modify: `common/src/test_support.rs` (add `parse_page_size` + its import).

**Interfaces:**

- Consumes: the Task 2 `clamp` affordance.
- Produces: `common::pagination::PageSize` —
  `#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]`, `1..=50`,
  `Default` = 50, with `PageSize::MIN`/`MAX`/`clamped`/`value()`,
  `From<PageSize> for u32`, `FromStr`, serde.
  `common::test_support::parse_page_size(&str) -> PageSize`.

- [x] **Step 1: Write `common/src/pagination.rs` with the type and its failing
      surface test** (AC1, AC2, AC8). _Deliberately monomorphic, not the generic
      `assert_\*_newtype::<T>()`shape:`PageSize`is a lone type whose`clamped`/`MIN`/`MAX` surface the byte newtypes lack, so a generic-over-`T`
      helper would have exactly one caller (a generic helper is warranted only
      to collapse 2+ near-identical tests)._

```rust
//! Pagination page size — the `1..=50` range newtype (#537, ADR-0063).

use macros::NumNewtype;

/// A pagination page size, bounded to `1..=50` (the bound lives here, once).
///
/// `default()` is `50`, the web listing default. AtomPub's default of `25` is its own
/// policy, expressed as [`PageSize::clamped`]`(25)`. The `clamp` affordance means an
/// out-of-range request coerces into range rather than rejecting — used by the public
/// AtomPub `?limit=` param; the web `#[server]` args instead reject out-of-range on the
/// wire via the serde bridge.
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(
    inner = u32,
    min = 1,
    max = 50,
    default = 50,
    clamp,
    error = "page size must be between 1 and 50"
)]
pub struct PageSize(u32);

#[cfg(test)]
mod tests {
    use super::PageSize;

    #[test]
    fn page_size_surface() {
        // value()/From<Self> for u32, and trim
        assert_eq!("10".parse::<PageSize>().map(u32::from).ok(), Some(10));
        assert_eq!("  50  ".parse::<PageSize>().map(PageSize::value).ok(), Some(50));
        // FromStr rejects out-of-range and non-integers...
        for bad in ["0", "51", "abc", "-1", "1.5"] {
            assert!(bad.parse::<PageSize>().is_err(), "{bad} should reject");
        }
        // ...with the domain message
        assert!("0"
            .parse::<PageSize>()
            .err()
            .is_some_and(|e| e.to_string().starts_with("page size")));
        // Default is the web default (50), and Display round-trips
        let d = PageSize::default();
        assert_eq!(d.value(), 50);
        assert_eq!(d.to_string().parse::<PageSize>().ok(), Some(d));
        // serde: bare integer, round-trip, wire-rejection of out-of-range
        assert_eq!(serde_json::to_string(&d).ok(), Some("50".to_owned()));
        assert_eq!(serde_json::from_str::<PageSize>("25").map(u32::from).ok(), Some(25));
        assert!(serde_json::from_str::<PageSize>("0").is_err());
        assert!(serde_json::from_str::<PageSize>("51").is_err());
        // clamp affordance: bounds + coercion
        assert_eq!(PageSize::MIN, 1);
        assert_eq!(PageSize::MAX, 50);
        assert_eq!(PageSize::clamped(0).value(), 1);
        assert_eq!(PageSize::clamped(999).value(), 50);
        assert_eq!(PageSize::clamped(25).value(), 25);
    }
}
```

Add `pub mod pagination;` to `common/src/lib.rs` between `media` and `password`.

- [x] **Step 2: Run it, verify it passes** (the type + test compile together;
      this is a characterization of the whole generated surface — no separate
      red step for a pure data-type addition). Run:
      `cargo nextest run -p common pagination` Expected: PASS.

- [x] **Step 3: Add `parse_page_size` to `common/src/test_support.rs`** (AC7):
      add `use crate::pagination::PageSize;` to the imports, and:

  ```rust
  /// Parse `s` into a [`PageSize`] for tests — the single place a test page-size literal
  /// is parsed, so a malformed fixture (e.g. `"0"`/`"51"`) fails loudly and the parse isn't
  /// re-spelled at every pagination call site.
  ///
  /// # Panics
  ///
  /// Panics if `s` is not an integer in `1..=50`.
  #[must_use]
  pub fn parse_page_size(s: &str) -> PageSize {
      s.parse().expect("valid test page size")
  }
  ```

- [x] **Step 4: Verify the crate builds with the helper.** Run:
      `cargo check -p common --features test-support` Expected: PASS.

- [x] **Step 5: Commit.** Run `cargo xtask check` first.
  ```bash
  git add common/src/pagination.rs common/src/lib.rs common/src/test_support.rs
  git commit -m "feat(common): add PageSize 1..=50 range newtype (#537)"
  ```

---

## Task 4: AtomPub `collection_get` adopts `PageSize` (clamp preserved)

**Files:**

- Modify: `server/src/atompub/posts.rs` (imports; drop
  `DEFAULT_PAGE_SIZE`/`MAX_PAGE_SIZE` consts at `:30-31`; the page-size
  computation at `:136-139`; the `+1`/`try_from` sites at `:156,159,161`; the
  `CollectionPaging.limit` doc at `:115`).
- Test: `server/tests/atompub/atompub_posts.rs` (add the clamp regression).

**Interfaces:**

- Consumes: `common::pagination::PageSize`.
- `CollectionPaging.limit` **stays** `Option<u32>` (owner-approved flatten,
  #470).

- [x] **Step 1: Write the failing regression test** in
      `server/tests/atompub/atompub_posts.rs` (mirror the existing `?limit=1`
      collection test's setup) (AC5). **Seed 51 posts** for one user (so the
      `1..=50` cap is observable), then: GET `/atompub/{user}/posts?limit=999`,
      assert the feed entry count is exactly 50 (clamped to MAX, not 51); GET
      `?limit=0`, assert the entry count is exactly 1 (clamped to MIN). Follow
      the file's existing harness/helpers; name it
      `collection_clamps_out_of_range_limit`. _(Characterization: this passes on
      the pre-refactor code too — it pins the behavior the refactor must
      preserve. Do **not** weaken it to a "same count as default" assertion —
      with a small seed that passes vacuously without proving the 50-cap; the
      51-post seed is what makes AC5 real.)_

- [x] **Step 2: Run it, verify it passes on current code.** Run:
      `cargo nextest run -p jaunder --test integration -E 'test(collection_clamps_out_of_range_limit)'`
      (the `server/` dir's package is `jaunder`, and `autotests = false` gives
      it a single integration binary `integration` = `tests/main.rs`; select
      individual tests with nextest's `-E 'test(<substr>)'` filter — there is no
      per-module test binary). Expected: PASS (behavior exists today).

- [x] **Step 3: Refactor `collection_get` to `PageSize`.** In
      `server/src/atompub/posts.rs`:
  - Add `use common::pagination::PageSize;`.
  - Replace the two consts (`:30-31`) with
    `const DEFAULT_PAGE_SIZE: PageSize = PageSize::clamped(25);` (delete
    `MAX_PAGE_SIZE` — 50 is now `PageSize::MAX`).
  - Replace the `:136-139` computation with:
    ```rust
    let limit = paging.limit.map_or(DEFAULT_PAGE_SIZE, PageSize::clamped);
    ```
  - At the storage call (`:156`) use `limit.value() + 1`; at `:159`/`:161` use
    `usize::try_from(limit.value())`.
  - Update the `CollectionPaging.limit` doc (`:115`) to
    `Requested page size (clamped to \`PageSize::MAX\`).`

- [x] **Step 4: Run the regression + AtomPub suite, verify still green.** Run:
      `cargo nextest run -p jaunder --test integration -E 'test(atompub)'`
      Expected: PASS (behavior unchanged).

- [x] **Step 5: Commit.** Run `cargo xtask check` first.
  ```bash
  git add server/src/atompub/posts.rs server/tests/atompub/atompub_posts.rs
  git commit -m "refactor(atompub): page size via PageSize::clamped, drop clamp literals (#537)"
  ```

---

## Task 5: Web listing/drafts adopt `Option<PageSize>` (one atomic commit)

**Files:**

- Modify: `web/src/posts/listing.rs` — 6 `#[server]`/fetcher pairs: fetchers
  `fetch_user_posts` (:99/:102), `fetch_local_timeline` (:128/:131),
  `fetch_posts_by_tag` (~:243/:248), `fetch_user_posts_by_tag` (:278/:284), the
  inline `list_home_feed` body (:192/:200); `#[server]` args `list_user_posts`
  (:149), `list_local_timeline` (:171), `list_home_feed` (:192),
  `list_posts_by_tag` (:306), `list_user_posts_by_tag` (:330). `page_from_rows`
  stays `u32`.
- Modify: `web/src/posts/mod.rs` — `list_drafts` (:532 arg, :539 body).
- Modify callers: `web/src/pages/posts.rs` (:355, :440, :936, :1125, :1202,
  :1304, :1387), `server/src/projector/mod.rs` (:190, :245, :281, :315),
  `web/src/pages/timeline.rs` (:17 delete `const PAGE_SIZE`, :91 caller),
  `web/src/pages/home.rs` (:7 import, :35), `web/src/pages/cockpit.rs` (:16
  import, :35).

**Interfaces:**

- Consumes: `common::pagination::PageSize`.
- Produces: every listed `#[server]` fn / fetcher takes
  `limit: Option<PageSize>`.

_This is one commit: changing a signature breaks its callers, so signatures +
all callers must land together for a compiling tree. Mechanical (type swap +
`.value()` + `.unwrap_or_default()`). Good candidate to delegate via
**jaunder-dispatch**._

- [x] **Step 1: Change the fetcher + `#[server]` signatures and bodies** in
      `web/src/posts/listing.rs` and `web/src/posts/mod.rs`:
  - Every `limit: Option<u32>` → `limit: Option<PageSize>`.
  - Every `let page_size = limit.unwrap_or(50).clamp(1, 50);` →
    `let page_size = limit.unwrap_or_default();` (a `PageSize`).
  - Every raw-int use of `page_size` goes through `.value()`:
    `page_size.saturating_add(1)` → `page_size.value().saturating_add(1)`;
    `page_size as usize` → `page_size.value() as usize`;
    `page_from_rows(rows, page_size, …)` →
    `page_from_rows(rows, page_size.value(), …)`;
    `list_drafts_by_user(…, page_size, …)` → `… , page_size.value(), …`.
  - Add `use common::pagination::PageSize;` to `listing.rs` and `mod.rs` imports
    (place with the other `common::` imports; `mod.rs` may already glob — add
    explicitly if not).

- [x] **Step 2: Fix every caller** the compiler now flags:
  - `web/src/pages/posts.rs`: the four direct calls (`Some(50)` →
    `Some(PageSize::default())`) and the three action-struct `limit: Some(50)`
    field inits → `limit: Some(PageSize::default())`; add
    `use common::pagination::PageSize;`.
  - `server/src/projector/mod.rs`: the four `fetch_*(..., Some(50))` →
    `Some(PageSize::default())`; add `use common::pagination::PageSize;`.
  - `web/src/pages/timeline.rs`: delete `pub(crate) const PAGE_SIZE: u32 = 50;`
    (:17); at :91 `Some(PAGE_SIZE)` → `Some(PageSize::default())`; add the
    `PageSize` import.
  - `web/src/pages/home.rs` and `web/src/pages/cockpit.rs`: drop the
    `use crate::pages::timeline::{…, PAGE_SIZE}` import of `PAGE_SIZE`, add
    `use common::pagination::PageSize;`, and `Some(PAGE_SIZE)` →
    `Some(PageSize::default())`.
  - Fix any additional caller the compiler flags (e.g. Rust tests that pass
    `limit: Some(<n>)` to a fetcher/server fn) by building the fixture through
    `common::test_support::parse_page_size("…")` or `PageSize::default()` (AC7).
    Do **not** touch integer `limit=<n>` values in wire/query-string test
    fixtures.

- [x] **Step 3: Compile all web + server targets (including server-gated web
      code).** Run: `cargo check -p web --all-features --all-targets` Then:
      `cargo check -p jaunder --all-targets` Expected: PASS — no residual
      `Option<u32>` `limit` on these fns, no `PAGE_SIZE`.

- [x] **Step 4: Run the web + projector test suites.** Run:
      `cargo nextest run -p web --all-features` and
      `cargo nextest run -p jaunder --test integration -E 'test(web)'` Expected:
      PASS.

- [x] **Step 5: Commit.** Run `cargo xtask check` first.
  ```bash
  git add web/src/posts/listing.rs web/src/posts/mod.rs web/src/pages/posts.rs \
          web/src/pages/timeline.rs web/src/pages/home.rs web/src/pages/cockpit.rs \
          server/src/projector/mod.rs
  git commit -m "refactor(web): thread PageSize through posts listing/drafts pagination (#537)"
  ```

---

## Task 6: Amend ADR-0063 with the `clamp` affordance

**Files:**

- Modify: `docs/adr/0063-domain-value-newtype-convention.md` (§2 numeric values
  / §3 macro).

- [x] **Step 1: Amend the ADR** (per **jaunder-adr**; 0063 is
      `Status: proposed`, edit in place — no new number). In §2's numeric-value
      description and §3's `NumNewtype` list, add a short paragraph: the range
      case may opt into `clamp` (requires `min` + `max`), which emits
      `const MIN`/`MAX` and an infallible `const fn clamped(inner) -> Self`
      coercing into range. It is a validated door (cannot yield an out-of-range
      value) and **opt-in**, so non-range / non-clamping numeric newtypes are
      unaffected. Use case: a public bound that should coerce (AtomPub
      `?limit=`) rather than reject on the wire; `PageSize` is the first
      adopter. Correct any lingering `get()` mention to `value()` if present.

- [x] **Step 2: Prettier the ADR before staging** (per the
      precommit-prettier-prose note):
      `prettier -w docs/adr/0063-domain-value-newtype-convention.md`.

- [x] **Step 3: Commit.** Run `cargo xtask check` first.
  ```bash
  git add docs/adr/0063-domain-value-newtype-convention.md
  git commit -m "docs(adr-0063): document the NumNewtype clamp affordance (#537)"
  ```

---

## Final gate

- [x] Run `cargo xtask validate --no-e2e` (AC10). Expected: PASS. Then hand off
      to **jaunder-ship** (final review, archive spec+plan, PR, merge — releases
      the issue to Done).

## Self-review notes

- **Spec coverage:** AC1/AC2 → Task 2+3; AC3 → Tasks 3/4/5 (bound only in type;
  25 as `clamped`); AC4 → Task 5 (full enumeration); AC5 → Task 4; AC6 → Tasks
  4/5 (`.value()` arithmetic); AC7 → Task 3 + Task 5 Step 2; AC8 → Task 2
  (macro) + Task 3 (surface); AC9 → Task 2 (doc fix) + Task 6 (ADR); AC10 →
  Final gate. Media exclusion → Task 1.
- **Type consistency:** accessor `.value()` and `From<PageSize> for u32` used
  throughout; `PageSize::clamped`/`MIN`/`MAX`/`default()` names match the Task 2
  codegen.
