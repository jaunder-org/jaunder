# Plan — #696: `RowLimit` + the `PageSize` fetch-limit accessors

Spec:
[2026-07-29-issue-696-fetch-limit-convention.md](../specs/2026-07-29-issue-696-fetch-limit-convention.md)
· Issue: [#696](https://github.com/jaunder-org/jaunder/issues/696) · Branch:
`worktree-issue-696-fetch-limit-convention` · Fork point: `wt-base-issue-696`
(`0e6c0958`)

## Review header

**Goal.** Give the storage fetch limit a name (`RowLimit`, `inner = i64`), put
the `page_size + 1` rule and its inverse on `PageSize` where they can't drift
apart, and move `PageOffset` to `i64` — which together delete both numeric
ALLOWLIST carve-outs #686 left in the bind gate.

**Scope — in:** `common/src/pagination.rs` (+ `test_support.rs`); the 16
`limit: u32` storage signatures in `storage/src/{posts,media}.rs` and their
dialect files; `web/src/posts/api/listing.rs`, `web/src/tags/api.rs`,
`web/src/media/api.rs`; `server/src/atompub/service.rs`;
`xtask/src/steps/sqlx_newtype_bind_check.rs`; ADR-0063. **Scope — out:** #715
(`query_scalar` id decodes); #716 (`i64`-param laundering, incl.
`claim_pending_batch` and `FeedMinItems`); `PageSize`'s own `1..=50` bounds
(#537).

**Tasks.**

1. `RowLimit` + the three `PageSize` accessors, with tests _(spec §1–§2)_
2. Thread `RowLimit` through the 16 storage signatures and their binds _(§1,
   §5)_
3. Web + AtomPub call sites stop doing arithmetic _(§4, §5)_
4. `PageOffset` → `inner = i64, min = 0` _(§3)_
5. Teach the gate the hoisted-local form; delete both numeric ALLOWLIST entries
   _(§6–§7)_
6. Amend ADR-0063 for the min-only saturating door and the declared-bound
   principle

**Key risks / decisions.**

- **Task 1 and task 3 are one logical change split for reviewability.**
  `has_more`/`page_len` exist only to be called from `page_from_rows` (spec §4).
  If task 3 is dropped, task 1 leaves two unused methods and the inverse stays
  hand-written — the defect half-fixed. Do not ship 1 without 3.
- **Task 5's mutation test must target `list_tags`' hoist** (`posts.rs:1559`),
  which is genuinely within-function and genuinely `i64::from(`. Using
  `claim_pending_batch` would prove nothing — it is cross-function _and_ spelled
  `i64::try_from`, so the new rule cannot see it either way (spec §7).
- **Task 5 runs after tasks 2–4.** Adding the rule first makes
  `cargo xtask check` fail on the un-swept hoist.
- **Task 4 loosens wire validation deliberately** — an offset above `u32::MAX`
  now returns an empty page instead of a validation error. Spec §3 records why
  no `max` is declared; the reason must land in `PageOffset`'s doc, not just the
  spec.
- **`min = 1` is the invariant, not `u32`-ness.** The tempting shortcut in tasks
  1 and 4 is `inner = i64` without a declared `min`, which makes the gate green
  while deleting the guarantee. Both types' `min` is load-bearing.

**For agentic workers.** Drive with **`jaunder-iterate`**; task 2 is wide and
mechanical — delegate via **`jaunder-dispatch`** if useful. Tick checkboxes in
this file in real time.

## Global constraints

- **No `Co-Authored-By` trailer** on any commit.
- Pre-commit runs the full `cargo xtask check` — run it yourself first so the
  hook passes clean (`jaunder-commit`). It auto-fixes formatting, so
  `git status --porcelain` after green.
- `common`'s pagination tests are **in-file `#[cfg(test)]`**
  (`pagination.rs:36`), not a `tests/` file. Build values via
  `common::test_support::parse_page_size` / `parse_page_offset` — never
  `.parse().expect()` (`expect_used` is denied).
- Storage tests use the **dual-backend template**:
  `#[apply(backends)] #[tokio::test]` with `#[case] backend: Backend`, binding
  the whole `TestEnv` (ADR-0053). Never in an ADR-0019 per-backend dialect file.
- `xtask` is excluded from the workspace — test it with
  `cargo nextest run --manifest-path xtask/Cargo.toml`.
- Never bare `nextest` for storage: `cargo xtask check` sets up
  Postgres/seeding.
- Both dialect files change together for every storage signature (ADR-0019
  parity).

---

## Task 1 — `RowLimit` and the `PageSize` accessors

**Files:** `common/src/pagination.rs`, `common/src/test_support.rs`. **Test:**
in-file `#[cfg(test)] mod tests` in `pagination.rs`.

Per spec §1–§2. `RowLimit` is a plain `NumNewtype`; the three accessors are
hand-written on `PageSize`.

```rust
/// How many rows a query fetches — the storage-side quantity, distinct from the
/// [`PageSize`] a reader sees. For a paginated listing use
/// [`PageSize::fetch_limit`], which applies the has-more `+1`; for a flat cap with
/// no page behind it use [`RowLimit::at_most`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(inner = i64, min = 1, error = "fetch limit must be at least 1")]
pub struct RowLimit(i64);

impl RowLimit {
    /// A flat cap — "at most `n` rows", with no page behind it. Saturates a value
    /// below 1 up to 1, so it cannot yield an out-of-range value (the `clamped`
    /// justification in ADR-0063 §2, min-only).
    #[must_use]
    pub const fn at_most(n: i64) -> Self {
        Self(if n < 1 { 1 } else { n })
    }
}

impl PageSize {
    /// Rows to fetch for one page: the page plus **one extra**, so a full page and
    /// one more row proves a next page exists without a second `COUNT(*)`. The
    /// single place this `+1` is derived; [`Self::has_more`] is its inverse.
    #[must_use]
    pub const fn fetch_limit(self) -> RowLimit {
        RowLimit(self.0 as i64 + 1)
    }

    /// Whether an over-fetched row set proves another page exists — the inverse of
    /// [`Self::fetch_limit`].
    #[must_use]
    pub const fn has_more(self, fetched: usize) -> bool {
        fetched > self.page_len()
    }

    /// The page's own length: what an over-fetched set truncates back to.
    #[must_use]
    pub const fn page_len(self) -> usize {
        self.0 as usize
    }
}
```

`fetch_limit` cannot overflow or violate `min`: `PageSize` is `1..=50`, so the
result is `2..=51`. It constructs `RowLimit` via the tuple field rather than
`TryFrom` because a `const fn` cannot `?`, and the bound is proven by
`PageSize`'s own range — note that in a comment so it does not read as a bypass.

- [x] Add `RowLimit` + `at_most`, and the three `PageSize` accessors
- [x] Export `RowLimit` from `common::pagination`; add
      `common::test_support::parse_row_limit` mirroring `parse_page_size`

**Four tests, written RED first** (the items do not exist yet):

- [x] `row_limit_surface` — `FromStr` / serde / `TryFrom` round-trip via
      `parse_row_limit`; `"0"` and `"-1"` both rejected
- [x] `at_most_saturates_below_one` — `at_most(0).value() == 1`,
      `at_most(-5).value() == 1`, `at_most(100).value() == 100`
- [x] `fetch_limit_is_page_plus_one` — for `PageSize`'s min (1) and max/default
      (50): `fetch_limit().value() == page_len() as i64 + 1`
- [x] `has_more_is_the_inverse_of_fetch_limit` — for each of those sizes, a row
      count equal to `page_len()` is **not** more, and `fetch_limit()` rows
      **is**.
- [x] `cargo nextest run -p common pagination` → PASS (6/6)
- [x] `cargo xtask check` → green; commit

**Correction — the coupling needed a fourth assertion the plan did not
anticipate.** As first written, `has_more_is_the_inverse_of_fetch_limit` did
**not** pin the `+1`: mutating `fetch_limit` to `+2` left it green (only
`fetch_limit_is_page_plus_one` caught that), because `has_more(page)` is false
and `has_more(page + n)` is true for every `n >= 1`. Verified by mutation, not
reasoning.

What actually couples the halves is that `fetch_limit` is the **smallest** count
proving a next page, so the test now also asserts `!has_more(fetch_limit - 1)`.
With that, the `+2` mutation fails **both** tests. Re-verified in both
directions:

| Mutation                  | Caught by                                               |
| ------------------------- | ------------------------------------------------------- |
| `fetch_limit` `+1` → `+2` | `fetch_limit_is_page_plus_one` **and** the inverse test |
| `has_more` `>` → `>=`     | the inverse test                                        |

Lesson for the remaining tasks: an assertion that a threshold behaves correctly
_above and below itself_ does not pin _where_ the threshold is. #709's audit of
vacuous assertions is the same failure mode in a different crate.

**Done when:** the `+1` and its inverse exist exactly once each, adjacently,
with a test that fails if they disagree.

## Task 2 — thread `RowLimit` through storage

**Files:** `storage/src/posts.rs` (7 trait + 7 impl signatures),
`storage/src/media.rs` (2), and the dialect files that bind `limit`. **Test:**
the existing dual-backend listing tests must pass unchanged — this is a type
change, not a behaviour change.

Per spec §1/§5. Each `limit: u32` becomes `limit: RowLimit`; each
`.bind(i64::from(limit))` becomes `.bind(limit)`; `list_tags`' hoisted
`let limit_i64 = i64::from(limit);` is deleted and both its binds take `limit`
directly.

- [ ] Retype the 16 signatures (spec's site table has the exact lines)
- [ ] Rewrite the 14 inline `.bind(i64::from(limit))` → `.bind(limit)`
- [ ] Delete `posts.rs:1559`'s hoist; bind `limit` at `:1570`/`:1580`
- [ ] Where inference then fails, add an explicit annotation — **do not**
      reinstate `i64::from`; report any site where that is impossible
- [ ] `rg -n 'i64::from\(limit' storage/src` → no hits
- [ ] `cargo xtask check` → green; commit

**Watch for:** the generic stores restate row tuples and bind types as
`where`-clause bounds (ADR-0019 — supertrait clauses don't propagate). #686
found that changing a `query_as` turbofish without its matching bound puts the
error on _other_ columns. The same applies here: a `RowLimit` bind may need
`for<'q> RowLimit: sqlx::Encode<'q, DB> + sqlx::Type<DB>` restated on each impl
that binds one.

## Task 3 — the call sites stop doing arithmetic

**Files:** `web/src/posts/api/listing.rs`, `web/src/tags/api.rs`,
`web/src/media/api.rs`, `server/src/atompub/service.rs`. **Test:** existing web
tests; plus one new test per spec §4's has-more claim.

Per spec §4–§5. **Task 1's accessors are dead code until this lands** — see the
header's first risk.

- [ ] `page_from_rows(rows, page_size: PageSize, …)`; body uses
      `page_size.has_more(rows.len())` and `page_size.page_len()`
- [ ] Replace the 4 `page_size.value().saturating_add(1)` sites
      (`listing.rs:74`, `:106`, `:232`, `:273`) with `page_size.fetch_limit()`
- [ ] `list_home_feed` (`listing.rs:179-205`) calls `page_from_rows` instead of
      its hand-rolled copy — deletes the 5th derivation site and the 2nd
      `has_more` spelling
- [ ] `web/src/tags/api.rs:36`:
      `.unwrap_or(DEFAULT_TAG_LIMIT).clamp(1, MAX_TAG_LIMIT)` →
      `PageSize::clamped(…)`, then `.fetch_limit()`… **decide during the task**
      whether the typeahead wants the `+1` at all (it has no has-more UI). If
      not, it wants a flat cap from the clamped size — record which, and why, in
      the commit.
- [ ] `server/src/atompub/service.rs:34`:
      `list_tags(None, RowLimit::at_most(100))`
- [ ] `web/src/media/api.rs:77`: pass the media `limit` as a `RowLimit`
- [ ] New test in `listing.rs`'s test module: a page-sized result reports
      `has_more == false` and a page-plus-one result reports `true`, driven
      through `page_from_rows` — pins §4's contract at the call site, not just
      on the type
- [ ] `cargo nextest run -p web listing` → PASS
- [ ] `cargo xtask check` → green; commit

**Note:** `MAX_TAG_LIMIT` is 50 and `DEFAULT_TAG_LIMIT` is 10, so the hand clamp
is exactly `PageSize`'s range — that is why `PageSize::clamped` replaces it
rather than a new bound being invented (spec §2).

## Task 4 — `PageOffset` → `inner = i64, min = 0`

**Files:** `common/src/pagination.rs`, `storage/src/media.rs` + its dialect
files, `web/src/media/api.rs`. **Test:** in-file `pagination.rs` tests; existing
media dual-backend tests.

Per spec §3.

- [ ] `#[num_newtype(inner = i64, min = 0, default = 0, error = …)]`
- [ ] **Rewrite the doc comment.** It currently says _"there is no range bound:
      the full `u32` domain is valid, so this carries no `min`/`max`/`clamp`"_ —
      now false. The replacement must state: the bound is `min = 0`, declared
      rather than implied by the primitive; there is deliberately **no `max`**,
      because an offset's only meaningful cap is the number of rows that exist,
      which is not a constant; and consequently an offset above `u32::MAX`
      yields an empty page rather than a validation error (it is a `#[server]`
      wire arg).
- [ ] `.bind(i64::from(offset.value()))` → `.bind(offset)` at `media.rs:264`,
      `:277`
- [ ] Update the in-file tests that assert through `u32` (`pagination.rs:82`,
      `:106`, `:111` use `u32::from` / `try_from(7u32)`) to `i64`
- [ ] Add `page_offset_rejects_negative` — `"-1"` and
      `serde_json::from_str("-1")` both rejected by the declared `min`, proving
      the bound survived the `inner` change. **This is the test that would fail
      if `min = 0` were forgotten.**
- [ ] `cargo xtask check` → green; commit

## Task 5 — the gate learns the hoisted form

**Files:** `xtask/src/steps/sqlx_newtype_bind_check.rs`. **Test:** in-file unit
tests (the module's existing convention).

Per spec §6–§7. **Runs after tasks 2–4**, or `check` fails on the un-swept
hoist.

`strips_newtype_in_bind` is a per-line predicate today. The hoisted form needs
function-scope state, so `violations` grows a pass that collects locals assigned
from `i64::from(…)` and then flags `.bind(<that ident>)`. Keep it line-based — a
brace-depth reset is enough scoping; do not reach for `syn` here.

- [ ] Unit tests first (RED): - `hoisted_i64_from_bind_is_flagged` —
      `let x = i64::from(user_id);` … `.bind(x)` -
      `hoisted_local_not_from_a_strip_is_clean` — `let x = row.count;` …
      `.bind(x)` - `bind_of_an_unrelated_ident_is_clean` — no assignment in
      scope
- [ ] Implement the local-tracking pass
- [ ] **Delete both numeric ALLOWLIST entries** (`i64::from(limit)`,
      `i64::from(offset.value())`) — tasks 2 and 4 removed their sites. Add
      none: spec §7 establishes `claim_pending_batch` is invisible to this rule,
      not exempted by it.
- [ ] Update the module doc + failure-detail text to name the hoisted case
- [ ] **Prove it bites:** re-hoist `list_tags`' conversion
      (`let limit_i64 = i64::from(limit); … .bind(limit_i64)`), confirm
      `cargo xtask check --no-test` fails naming that line, restore. **Not**
      `claim_pending_batch` — header risk 2.
- [ ] `cargo nextest run --manifest-path xtask/Cargo.toml sqlx_newtype_bind`;
      `cargo xtask check` → green; commit

**Re-audit before finishing:** the spec's Risks note the rule may match shapes
the audit didn't enumerate. Run
`rg -n 'as i64|let .*: i64 =|i64::try_from' storage/src` and confirm every hit
is either swept, genuinely primitive, or a #716 cross-function case — and say
which in the commit message. Do not silently exempt.

## Task 6 — amend ADR-0063

**Files:** `docs/adr/0063-domain-value-newtype-convention.md`, `docs/README.md`
if the title changes (it should not).

Two decisions from this cycle belong in the convention ADR, amended in place
(the repo convention for an accepted/proposed ADR, as #400 did):

- [ ] **The min-only saturating door.** §2 documents `clamped` as requiring both
      bounds. `RowLimit::at_most` is the same idea with only a `min` — a
      validated door that cannot yield an out-of-range value. Record it as
      sanctioned, with the constraint that it is for internally-derived values
      (a literal cap), not user input, which still goes through
      `clamped`/`FromStr`.
- [ ] **An unsigned `inner` is not a substitute for a declared bound.** Record
      that a `NumNewtype` whose value crosses the sqlx boundary should declare
      its `min` rather than lean on `u32`, because sqlx has no Postgres `Encode`
      for unsigned types — so the primitive's range is discarded at the bind,
      while a declared bound is re-run by `FromStr`, serde, and `Decode`. This
      is the principle #715 and #716 will both need.
- [ ] Cross-check ADR-0071's numeric paragraphs for wording that now understates
      the convention; fix in the same commit
- [ ] `prettier -w` the touched docs; `cargo xtask check` → green; commit

---

## Self-review

- **Order:** 1 → 2 → 3 → 4 → 5 → 6. Task 5 must follow 2–4 (its sites must be
  gone first). Task 3 must follow 1 (needs the accessors) and must not be
  skipped. Task 4 is independent of 1–3 and could move earlier; kept here so the
  `RowLimit` work lands as a unit.
- **Each task ends green and committed**, so the branch stays bisectable.
- **No wire shape changes.** `RowLimit` is new; `PageOffset`'s serde stays a
  bare integer. The only wire-visible change is the accepted-range loosening in
  task 4, recorded there and in the type's doc.
- **The two tests that carry the design:**
  `has_more_is_the_inverse_of_fetch_limit` (task 1) fails if the `+1` and its
  inverse disagree; `page_offset_rejects_negative` (task 4) fails if the
  declared `min` is dropped during the `inner` change. Those are the two
  mistakes this plan is most likely to make.
- **Not planned, deliberately:** #715, #716, and typing `claim_pending_batch` —
  all recorded in the spec's Out of scope with reasons.
