# Plan — issue #535: numeric-value newtypes for feeds (co-lands #464)

Spec:
[`2026-07-19-issue-535-feeds-numeric-newtypes.md`](../specs/2026-07-19-issue-535-feeds-numeric-newtypes.md).
This plan is "how"; the spec is "what/why" — read it for the attribute API,
generated trailer, and decisions.

## Review header

**Goal.** Extract a `#[derive(NumNewtype)]` into `macros/`, convert
`RetentionCount` to it, and introduce `FeedMinItems`/`FeedMinDays` (min-1,
`u32`) threaded through the feed config + windowing path — the deliberate second
case that satisfies #464.

**Scope — in:** the `NumNewtype` derive + tests; `RetentionCount` conversion;
`FeedMinItems`/`FeedMinDays` + full propagation (`FeedsConfig`, `HybridWindow`,
`site_config`, `posts.rs`, feed worker/regenerate, all touched tests);
test-support helpers; ADR-0063 amendment. **out:** media byte-limits (#536),
pagination `PageSize` (#537) — the macro is built general enough for both,
neither converted here.

**Tasks (one line each):**

1. `NumNewtype` derive — codegen, option/shape/inner-type validation, unit +
   doctests (`macros/`).
2. Convert `RetentionCount` to the derive (`common/src/backup.rs`).
3. Add `FeedMinItems`/`FeedMinDays` + `parse_feed_min_*` test helpers + unit
   tests (`common/src/feed/`, `common/src/test_support.rs`).
4. Thread the feed newtypes through everything, atomically (one compiling
   commit): `FeedsConfig`, `HybridWindow`, `site_config`, `posts.rs`, server
   feed consumers, and all touched tests.
5. Amend ADR-0063 with the numeric-value subsection.
6. Full-gate pass (`cargo xtask validate --no-e2e`) + verify #464 acceptance
   met.

**Key risks / decisions.**

- **Atomic compile at Task 4.** Changing `FeedsConfig`/`HybridWindow` field
  types breaks `storage`/`server` until every consumer is updated, so Task 4 is
  one commit spanning `common` + `storage` + `server` (cannot be sub-split
  without a red gate — the pre-commit `cargo xtask check` requires green). Tasks
  1–3 each compile the whole workspace on their own (new `pub` types are not
  dead-code errors; `RetentionCount`'s surface is preserved).
- **clippy `absurd_extreme_comparisons`.** The generated lower-bound check is
  emitted **only when `min` is present** — never a vacuous `v < 0` on an
  unsigned inner. `min = 0` on an unsigned type is pointless and simply omitted
  by the author. (Same for `max`.)
- **No `thiserror` in generated code** (spec §Part 1) — the emitted error type
  is a hand-written `Display` + `impl std::error::Error`, so future adopters
  (#536/#537) need no extra dep.
- **`RetentionCount` derive drops `Serialize, Deserialize`** from its
  `#[derive]` list — the macro emits the validating serde bridge; leaving the
  std derives on would double-impl.
- Post-merge, **#464 closes with this PR**; #536/#537 are unblocked.

**For agentic workers:** drive with `jaunder-iterate`, delegating a task to
`jaunder-dispatch` where useful. Tick checkboxes live.

## Global constraints

- No `Co-Authored-By` trailer on commits (global preference).
- Each task ends green: run `cargo xtask check` before committing
  (`jaunder-commit`) — the pre-commit hook runs the full check; run it first so
  it passes clean.
- Rust throughout; exact crate names: `macros`, `common`, `storage`, server
  package is **`jaunder`** (`-p jaunder`).
- Storage tests keep the dual-backend `#[apply(backends)]` template (backend
  parity); do not convert a dual-backend test to a bare `#[tokio::test]`.
- Newtype construction in `cfg(test)` uses the `common::test_support::parse_*`
  helpers, never inline `.parse().unwrap()` (repo convention).

---

## Task 1 — `#[derive(NumNewtype)]` in `macros/`

**Files**

- `macros/src/num_newtype.rs` (new) —
  `pub(crate) fn expand(input: &DeriveInput) -> proc_macro2::TokenStream`.
- `macros/src/lib.rs` — `mod num_newtype;`, a
  `#[proc_macro_derive(NumNewtype, attributes(num_newtype))]` entry with rustdoc
  (incl. doctests), and new `#[cfg(test)]` unit tests.

**Interfaces / codegen** (spec §Part 1 is the contract)

- Reuse `crate::require_newtype_shape(input, "NumNewtype", "struct X(u32)")`.
- `parse_opts` (mirroring `str_newtype::parse_opts`) reads
  `#[num_newtype(...)]`:
  - `inner = <Type>` — **required**; error if absent.
  - `min = <LitInt>`, `max = <LitInt>`, `default = <LitInt>`, `error = <LitStr>`
    — optional.
  - Unknown key → `meta.error(...)`.
- Validate the tuple field type equals `inner` via
  `quote!(#field_ty).to_string() == quote!(#inner_ty).to_string()`; mismatch →
  spanned `compile_error!`.
- Emit:
  - `struct Invalid#name;` + hand-written `Display` (the `error` message, or a
    generated default naming type + bounds) + `impl ::std::error::Error`.
  - `impl #name { #[must_use] pub fn get(self) -> #inner { self.0 } }` (by
    value; the inners here are `Copy`).
  - `impl ::core::str::FromStr` — trim, `<#inner as FromStr>::from_str`, then
    the bound check(s) (only the sides whose attribute is present), mapping any
    failure to `Invalid#name`.
  - `impl ::core::fmt::Display` — delegate to `self.0`.
  - When `default` present: `impl ::core::default::Default` returning
    `#name(<lit>)`, preceded by a
    `const _: () = assert!(<default satisfies bounds>);` compile guard.
  - Validating serde: `impl ::serde::Serialize` (serialize the inner) +
    `impl<'de> ::serde::Deserialize<'de>` (deserialize the inner, run the bound
    check, `map_err(serde::de::Error::custom)`).

**Test — `macros/src/lib.rs` `#[cfg(test)]` (parse_quote pattern, matching the
str/id tests)**

Add unit tests asserting
`num_newtype::expand(&input).to_string().contains(...)`:

- `num_newtype_wrong_shape_emits_compile_error` — named struct →
  `compile_error`.
- `num_newtype_missing_inner_emits_compile_error`.
- `num_newtype_inner_type_mismatch_emits_compile_error` — field `u32`,
  `inner = i64`.
- `num_newtype_unknown_option_emits_compile_error`.
- `num_newtype_min_max_default_emit_full_trailer` —
  `inner=u32, min=1, max=100, default=20`: asserts `FromStr`, `Default`,
  `Serialize`, `Deserialize`, and both bound checks present.
- `num_newtype_min_only_omits_max_check` /
  `num_newtype_max_only_omits_min_check`.
- `num_newtype_no_default_omits_default_impl`.

Rustdoc doctests on the derive entry: a passing fixture
(`#[num_newtype(inner = u32, min = 1, default = 1)] struct Ok(u32);` with
`#[derive(Clone, Copy, PartialEq, Eq, Debug, NumNewtype)]`) exercising
`"1".parse()` ok, `"0".parse()` err, `Ok::default().get() == 1`, `serde_json`
round-trip + `from_str("0")` err; plus a `compile_fail` on a named struct.

**Run**

- `cargo nextest run -p macros` → new tests **PASS** (implement codegen until
  green).
- `cargo test -p macros --doc` → doctests **PASS**.
- `cargo xtask check` clean → commit.

## Task 2 — convert `RetentionCount` (`common/src/backup.rs`)

**Files**

- `common/src/backup.rs` — replace the hand-written type + impls (spec §Part 2):

  ```rust
  #[derive(Clone, Copy, Debug, Eq, PartialEq, macros::NumNewtype)]
  #[num_newtype(inner = usize, min = 1, default = 7,
      error = "backup retention count must be a whole number of at least 1")]
  pub struct RetentionCount(usize);
  ```

  Delete: the `NonZeroUsize` import if now unused, `InvalidRetentionCount`, the
  hand-written `FromStr`/`Display`/`Default`/`get`, the
  `const DEFAULT: NonZeroUsize` dance, and `DEFAULT_BACKUP_RETENTION_COUNT` if
  it becomes unreferenced (the `default = 7` literal replaces it) — audit and
  remove.

- Keep the existing `#[cfg(test)] mod tests` for `RetentionCount` **unchanged**
  — they are the conversion's integration proof (parse accept/reject, error
  prefix, `Default::get() == 7`, `Display` round-trip, serde `"5"`/rejects
  `"0"`).

**Run**

- `cargo nextest run -p common backup` → **PASS** (behavior preserved).
- `cargo nextest run -p common` → **PASS** (no other `common` breakage).
- `cargo xtask check` clean → commit.

## Task 3 — `FeedMinItems` / `FeedMinDays` + helpers (`common`)

**Files**

- `common/src/feed/settings.rs` (new) — the two types (spec §Part 3), each
  `#[derive(Clone, Copy, Debug, Eq, PartialEq, macros::NumNewtype)]` +
  `#[num_newtype(inner = u32, min = 1, default = 20|30)]`, with an in-file
  `#[cfg(test)] mod tests`: parse accept, reject `0`, `Default::get()` = 20/30,
  `Display` round-trip, serde round-trip + wire-reject `0`.
- `common/src/feed/mod.rs` —
  `pub mod settings; pub use settings::{FeedMinItems, FeedMinDays};`.
- `common/src/test_support.rs` — `parse_feed_min_items(&str) -> FeedMinItems` /
  `parse_feed_min_days(&str) -> FeedMinDays`, siblings of
  `parse_retention_count`.

These types are unused outside tests until Task 4; the workspace still compiles
(`pub` items are not dead-code errors).

**Run**

- `cargo nextest run -p common feed::settings` → **PASS**.
- `cargo xtask check` clean → commit.

## Task 4 — thread the feed newtypes everywhere (atomic)

One commit; the whole workspace must compile at its end. Order the edits so the
type definitions change first, then every consumer.

**Files — `common`**

- `common/src/feed/mod.rs` —
  `FeedsConfig { min_items: FeedMinItems, min_days: FeedMinDays, websub_hub_url: Option<String> }`.
- `common/src/feed/window.rs` —
  - `HybridWindow { min_items: FeedMinItems, min_days: FeedMinDays }` (keep
    `#[derive(... Copy ...)]`).
  - `Default` →
    `{ min_items: FeedMinItems::default(), min_days: FeedMinDays::default() }`.
  - `cutoff_date`: `Duration::days(i64::from(self.min_days.get()))`.
  - `select`: `i < self.min_items.get() as usize`.
  - the module's `#[cfg(test)]` `HybridWindow { min_items: 20, .. }` literals →
    `parse_feed_min_items("20")` / `parse_feed_min_days("30")` (and the
    `min_items: 3`, `min_days: 1` etc. cases).

**Files — `storage`**

- `storage/src/site_config.rs` —
  - `get_feeds_min_items() -> sqlx::Result<FeedMinItems>` /
    `get_feeds_min_days() -> sqlx::Result<FeedMinDays>`:
    `...and_then(|v| v.trim().parse().ok()).unwrap_or_default()`.
  - `get_feeds_config` builds the typed `FeedsConfig`.
  - `set_feeds_config` — `config.min_items.to_string()` (unchanged via
    `Display`).
  - Remove `DEFAULT_FEEDS_MIN_ITEMS`/`DEFAULT_FEEDS_MIN_DAYS` consts and their
    doc-links (now superseded by the newtype `Default`); update the tests that
    referenced them (`== DEFAULT_FEEDS_MIN_ITEMS` →
    `== FeedMinItems::default()`; literal `50`/`60`/`42`/`7` constructions →
    `parse_feed_min_*`). Keep the `#[apply(backends)]` dual-backend shape.
- `storage/src/posts.rs:~1613` —
  `let min_items = i64::from(window.min_items.get());`.

**Files — `server`**

- `server/src/feed/regenerate.rs:44-45` —
  `min_items: feeds.min_items, min_days: feeds.min_days` (newtypes move straight
  through; no `.get()`). Its `#[cfg(test)]`
  `HybridWindow { min_items: 10, min_days: 30 }` → `parse_feed_min_*`.
- `server/src/feed/worker.rs:~437` — same test-construction fix.
- `server/tests/storage/mod.rs` — the
  `HybridWindow { min_items: N, min_days: M }` literals (≈ lines 2984–3118) →
  `parse_feed_min_*`.
- `server/src/cli.rs` doc comment (243-244) needs no code change (keys
  unchanged).

**Run**

- `cargo nextest run -p common` → **PASS**.
- `cargo nextest run -p storage site_config` (SQLite + Postgres) → **PASS**.
- `cargo nextest run -p jaunder feed` → **PASS**.
- `cargo build --all-features --all-targets` → clean (catches any missed
  consumer, incl. server-gated web code).
- `cargo xtask check` clean → commit.

## Task 5 — amend ADR-0063

**Files**

- `docs/adr/0063-domain-value-newtype-convention.md` — add a **"Numeric
  values"** paragraph to §2 (the trailer: validating `FromStr` from declarative
  bounds, `.get()`, `Display`, compile-checked `Default`, validating
  transparent-integer serde) and a sentence to §3 (the trailer is a third
  derive, `NumNewtype`, whose bounds/`inner`/`default` are attributes — unlike
  `StrNewtype`, it _generates_ the validating `FromStr` because a numeric bound
  is declarative, not per-type prose). Note the distinction from `IdNewtype` (an
  id has no value invariant; a numeric _value_ does). Keep status `proposed`.
- If the repo has a doc-formatting gate, `prettier -w docs/adr/0063-*.md` before
  staging (per the prose-restage note).

**Run**

- `cargo xtask check` clean (doc-only) → commit.

## Task 6 — full gate + close-out

- `cargo xtask validate --no-e2e` → clean (run foreground, `timeout: 600000`).
- Confirm #464 acceptance: derive exists with `compile_error!` guard; two
  adopters (`RetentionCount` + feeds) converted; error/validation paths covered.
  Record that #464 closes with this PR (handled at `jaunder-ship`).
- `git status --porcelain` clean (no fmt drift left unstaged).

## Self-review

- Every field/consumer of `min_items`/`min_days` from the spec's grep is covered
  by Task 4 (common: mod/window; storage: site_config/posts; server:
  regenerate/worker/tests).
- No placeholders; each task names concrete files, the exact derive attributes,
  and run commands with expected PASS.
- Atomic-compile boundary is explicit (Task 4). Tasks 1–3 independently compile.
- Coverage: macro error paths (Task 1 unit tests), newtype validation (Tasks 2–3
  unit tests) — satisfies the "macros crate is coverage-measured" acceptance
  bullet.
