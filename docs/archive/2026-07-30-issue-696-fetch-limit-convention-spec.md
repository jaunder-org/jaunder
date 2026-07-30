# Spec — #696: name the storage fetch-limit convention with a `RowLimit` newtype

- Issue: [#696](https://github.com/jaunder-org/jaunder/issues/696)
- Milestone: Domain-value type safety (newtypes)
- Governing ADR: [ADR-0063](../adr/0063-domain-value-newtype-convention.md)
  §1–§2, [ADR-0071](../adr/0071-sqlx-string-newtype-bridge.md) (the bridge
  #686 broadened)
- Follows: [#686](https://github.com/jaunder-org/jaunder/issues/686) — this
  issue owns the 2 ALLOWLIST carve-outs that one left behind
- Spun out: [#715](https://github.com/jaunder-org/jaunder/issues/715)
  (`query_scalar` id decodes),
  [#716](https://github.com/jaunder-org/jaunder/issues/716) (`i64` params
  launder a strip past the gate)
- Date: 2026-07-29

## Problem

The issue as filed asks a question — "is this a `PageSize` adoption gap?" — and
answers it: no, because `web` deliberately passes **page size + 1** as the
has-more sentinel, so the value crossing the storage boundary can be `51`,
outside `PageSize`'s `1..=50`. It then offers two resolutions and asks for a
decision.

Two independent problems are tangled in those 16 signatures. Both are real; only
one is what the issue's title is about.

**A — the `+1` convention has no name.** It is re-derived at five call sites
(`page_size.value().saturating_add(1)` ×4 plus a `fetch_limit` local at
`web/src/posts/api/listing.rs:179`), and its **inverse** is re-derived twice
more, in two different spellings: `rows.len() > page_size as usize` (`:37`,
inside `page_from_rows`) and `rows.len() > page_size.value() as usize` (`:191`,
in a hand-rolled copy of `page_from_rows` that `list_home_feed` uses instead of
calling it). Seven sites carry a convention nothing names, and the two halves
can drift independently.

**B — `u32` costs two permanent holes in a gate that is otherwise absolute.**
#686 taught `sqlx-newtype-bind` to reject `i64::from(` inside a `.bind(`. sqlx
implements no Postgres `Encode` for unsigned types, so `limit: u32` and
`PageOffset(u32)` force a widening at every bind, which had to become two
ALLOWLIST entries. Today's code is **safe** — `u32` holds all the way to the
`i64::from`, a lossless widening of a value that cannot be negative — but the
guarantee lives in the primitive's range rather than anywhere durable.

**Which resolution.** Option (1) — the newtype — but the issue does not say what
`inner` it should have, and that is the load-bearing part: a `RowLimit(u32)`
fixes A and leaves B exactly where it is. `RowLimit(i64)` with a declared `min`
fixes both.

**Why the lower bound is worth more than it looks.** `u32` expresses `>= 0`,
which is weaker than what these values need (`>= 1`). And **SQLite treats a
negative `LIMIT` as "no limit"**, returning every row, where Postgres errors. A
negative limit reaching storage would not crash on SQLite — it would silently do
a full-table read on a paginated endpoint, and diverge per backend (an ADR-0019
parity hazard). That is worth holding in a validating type rather than a
primitive range that is discarded at the boundary.

## The sites — every one

**Storage signatures taking `limit: u32` — 16** (the issue says 18; its own
enumeration lists 16):

| File                         | Lines                                                         |
| ---------------------------- | ------------------------------------------------------------- |
| `storage/src/posts.rs` trait | `:601`, `:615`, `:632`, `:644`, `:666`, `:682`, `:694`        |
| `storage/src/posts.rs` impl  | `:1053`, `:1125`, `:1195`, `:1258`, `:1364`, `:1456`, `:1552` |
| `storage/src/media.rs`       | `:87`, `:245` (`list_media`)                                  |

**Bind sites reached — 16, in two shapes.** 14 inline `.bind(i64::from(limit))`
(12 in `posts.rs`, 2 in `media.rs`) — these are the gate's exempted set. Plus
`list_tags` (`posts.rs:1559`) hoists it: `let limit_i64 = i64::from(limit);`
then binds that local at `:1570` and `:1580`. **The gate cannot see the hoisted
form** — it inspects only the region after `.bind(`, so `.bind(limit_i64)` reads
as clean. Harmless today (a genuine `u32` widening) but it means a hoisted
_newtype_ strip would pass, so the gate is weaker than #686's commit message
implies.

**`PageOffset` binds — 2:** `media.rs:264`, `:277`, as
`i64::from(offset.value())`.

**Three distinct kinds of `limit` reach these signatures.** This is why the `+1`
derivation cannot be `RowLimit`'s only door:

| Kind                              | Sites                                                                                                                                                                        | Value              |
| --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------ |
| Page-size-derived (`+1` sentinel) | the 5 web listing call sites                                                                                                                                                 | `2..=51`           |
| A plain cap                       | `server/src/atompub/service.rs:34` `list_tags(None, 100)` — a literal; `web/src/tags/api.rs:36` typeahead — runtime, clamped `1..=50` off an `Option<u32>` wire arg (see §2) | `100`, or `1..=50` |
| A window / batch count            | `storage/src/feed_events.rs` `claim_pending_batch(limit: usize)`                                                                                                             | worker batch size  |

The third is the concept the issue fences off ("do not merge it in") and this
spec honours that — see Out of scope.

## Decision

### 1. `RowLimit` — a `NumNewtype` with `inner = i64`, `min = 1`

```rust
// common/src/pagination.rs
#[derive(Clone, Copy, Debug, Eq, PartialEq, NumNewtype)]
#[num_newtype(inner = i64, min = 1, error = "fetch limit must be at least 1")]
pub struct RowLimit(i64);
```

No `default` — there is no sensible default number of rows to fetch, and
`NumNewtype` emits `Default` only when `default` is declared. No `max`: the
legitimate values span the `+1` sentinel (up to 51) and the AtomPub tag cap
(100), so any `max` would be arbitrary, and `clamp` requires both bounds so it
is unavailable and unwanted.

`inner = i64` is the whole point: it binds through the #686 bridge with no
conversion, so both ALLOWLIST entries go. `min = 1` is what preserves the
guarantee `u32` was carrying — **and it is stronger**, because `u32` only ever
said `>= 0`.

**`RowLimit` is not a replacement for `PageSize`; it is the storage-side
quantity.** They are deliberately different numbers: `PageSize` is how many
posts the reader sees (a UI policy, `1..=50`), `RowLimit` is how many rows the
query fetches. For a paginated listing the second is the first plus one, because
over-fetching a single row is how a next page is detected without a second
`COUNT(*)`. That is why the issue's opening finding — web passes `51`, outside
`PageSize`'s range — rules out simply adopting `PageSize` at these signatures.

`RowLimit` also has a second, independent source: a **flat cap** with no page
behind it (AtomPub's `list_tags(None, 100)`). So it is the general "how many
rows" concept, and `PageSize + 1` is one way to arrive at one — see §2.

### 2. The `+1` rule lives on `PageSize`, both halves together

The `+1` is a function from `PageSize` to `RowLimit`, so it can be written at
either end: a constructor on the destination (`RowLimit::for_page(size)`) or an
accessor on the source (`size.fetch_limit()`).

**It goes on `PageSize`**, because the rule has **two halves that must agree**
and only this placement puts them in the same file:

| Half        | Question it answers                                                    |
| ----------- | ---------------------------------------------------------------------- |
| **Forward** | how many rows do I ask for? (`page_size + 1`)                          |
| **Inverse** | given the rows back, is there another page? and what do I truncate to? |

If the forward half were `RowLimit::for_page` in `common/src/pagination.rs`
while the inverse stayed in `page_from_rows` in `web/src/posts/api/listing.rs`,
the two halves would sit in **different crates** — which is the same drift risk
this issue exists to remove, merely relocated. On `PageSize` they are adjacent
methods a reviewer sees together.

```rust
impl PageSize {
    /// Fetch one row past the page, so a full page plus one proves another exists.
    /// The single place the `+1` is derived.
    #[must_use]
    pub const fn fetch_limit(self) -> RowLimit { … }

    /// Whether an over-fetched row set proves another page exists — the inverse of
    /// [`Self::fetch_limit`], and the reason both live here.
    #[must_use]
    pub const fn has_more(self, fetched: usize) -> bool { fetched > self.page_len() }

    /// The page's own length: what an over-fetched set truncates back to.
    #[must_use]
    pub const fn page_len(self) -> usize { … }
}

impl RowLimit {
    /// A flat cap — "at most `n` rows", with no page behind it. Saturates a value
    /// below 1 up to 1, so it is a **validated** door that cannot yield an
    /// out-of-range value (ADR-0063's `clamped` justification, min-only).
    #[must_use]
    pub const fn at_most(n: i64) -> Self { … }
}
```

`fetch_limit` is infallible by construction: `PageSize` is `1..=50`, so `+1` is
`2..=51`, always `>= 1`.

`page_len` is the smallest of the three and still earns its place: it deletes
the `as usize` casts that appear at every truncate and comparison today, and
unchecked `as` casts are part of what this milestone is removing.

`at_most` is hand-written and `const` deliberately: `expect_used`/`unwrap_used`
are both `deny` (`Cargo.toml:117-118`), so a fallible constructor at a site that
cannot fail forces either dead error handling or a lint exemption.

**The cost of this placement, stated so it is not discovered later.**
Discoverability inverts: someone holding a `RowLimit`-shaped hole reads
`RowLimit`'s constructors, finds only `at_most`, and may hand-roll a `+1` rather
than find `PageSize::fetch_limit` on the other type — which a constructor named
`RowLimit::for_page` would have surfaced to exactly that search. Mitigation:
`RowLimit`'s type-level doc must point at `PageSize::fetch_limit` as the way to
get one for a paginated listing. That is a cheaper problem than a cross-crate
split.

**Correction — only one of the two cap sites is a literal.** The first draft
justified `at_most` as "both sites pass compile-time literals". That is true of
`server/src/atompub/service.rs:34` (`list_tags(None, 100)`) and **false** of the
typeahead: `web/src/tags/api.rs:36` computes
`limit.unwrap_or(DEFAULT_TAG_LIMIT).clamp(1, MAX_TAG_LIMIT)` from that
`#[server]` fn's own `limit: Option<u32>` wire argument. So one `at_most` caller
is a runtime value off a public boundary.

**And that site should not use `at_most` at all.** `DEFAULT_TAG_LIMIT` is `10`,
`MAX_TAG_LIMIT` is `50`, and the expression is `.clamp(1, 50)` — which is
exactly `PageSize`'s declared `1..=50` and exactly what `PageSize::clamped`
does. That line is a hand-rolled re-implementation of an affordance `PageSize`
already has, including the clamp-rather-than-reject policy `PageSize`'s doc
describes for public `?limit=` params. It should become `PageSize::clamped(…)`,
in the same way `PageSize`'s doc records AtomPub's default of 25 as
`PageSize::clamped(25)`.

That leaves `at_most` for the genuine flat caps — values with no page behind
them, which is the case a saturating `const fn` is for.

**Correction (found during implementation): there are three such callers, not
one.** The draft said "one — AtomPub's literal `100`". Two more surfaced, both
missed by the call-site audit because neither is a `limit` _parameter_; both are
internal scan batches:

| Caller                             | Cap    | Why it is a flat cap                   |
| ---------------------------------- | ------ | -------------------------------------- |
| `server/src/atompub/service.rs:34` | `100`  | service-document category list         |
| `storage/src/posts.rs:474`         | `50`   | draft-permalink scan batch             |
| `web/src/media/api.rs`             | `1000` | scans an author's posts for media refs |

Three real callers is a **stronger** justification for the door than the one the
draft argued from, and it leaves the alternative — declaring an arbitrary `max`
on `RowLimit` purely to unlock the generated `clamped` — clearly worse.

**A third derivation was also needed: `PageSize::exact_limit`.** Three call
sites want a row limit that is _exactly_ one page with **no** has-more probe:
the media listing, the draft listing, and the tags typeahead, none of which has
a "load more" affordance. The draft's two doors did not cover this —
`fetch_limit` would fetch a row the caller must then know to discard, and
`at_most(i64::from(size.value()))` reintroduces a conversion at the boundary and
severs the link to the page. So `PageSize` carries both: `fetch_limit` (probing)
and `exact_limit` (not), with a test asserting `fetch_limit == exact_limit + 1`
so the pair still cannot drift.

### 3. `PageOffset` moves to `inner = i64, min = 0`

Its `>= 0` guarantee is currently carried **entirely by `inner = u32`** — its
own doc says _"there is no range bound: the full `u32` domain is valid, so this
carries no `min`/`max`/`clamp`."_ Moving to `i64` without declaring `min = 0`
would delete that guarantee while making the gate green, which is the trap this
change must not fall into. With `min = 0` declared, the bound is re-run by
`FromStr`, the serde bridge, and (since #686) the sqlx `Decode` — a better home
than a primitive range that is discarded at the bind.

Two consequences to state plainly:

- The doc comment must be rewritten: the type now _does_ carry a bound.
- `min = 0` on an `i64` gives **no upper cap**, where `u32` implicitly capped at
  ~4.29e9.

**And `PageOffset` is a wire type, which makes that second point sharper than
first drafted.** `web/src/media/api.rs:70` takes `offset: Option<PageOffset>` as
a `#[server]` parameter, and `xtask/src/steps/server_fn_tracing_check.rs:67`
registers it as one. So its bounds are **input validation on a client-facing
boundary**, not merely an internal invariant.

The serialized shape does not change — the serde bridge is transparent-integer
either way, so `42` stays `42`. The accepted **range** changes in one direction:

| Input           | Today (`u32`)                     | Proposed (`i64, min = 0`) |
| --------------- | --------------------------------- | ------------------------- |
| `-1`            | rejected (integer range)          | rejected (declared `min`) |
| `5_000_000_000` | **rejected** (exceeds `u32::MAX`) | **accepted**              |

An absurd offset returns zero rows, so this is not a safety hole — but it
**loosens upper-bound validation on a `#[server]` argument**, which the first
draft of this spec missed by reasoning only about internal callers.

**Decision (2026-07-29, user): handle `PageOffset` in this issue and accept the
loosening.** Both carve-outs go, and the gate is left with none.

**A `max` was considered and rejected**, and the reason is worth recording
because it will be asked again: an offset's only meaningful upper bound is _the
number of rows that exist_, which is not a constant and cannot be a declared
bound. Any literal `max` would be an invented number wearing the authority of a
validated invariant — worse than an honest absence of one. `PageSize` can
declare `max = 50` because 50 is a real policy about what a page may contain;
there is no equivalent policy about how far into a list a reader may skip.

So the residual exposure is precisely: a client may pass an offset between
`u32::MAX` and `i64::MAX` and receive an empty page instead of a validation
error. That must be **recorded in `PageOffset`'s own doc** rather than left as
folklore, alongside the reason (there is no principled cap) so a future reader
does not "fix" it by inventing one. If a cap is ever wanted it is a
one-attribute change.

**Why bite the bullet rather than defer.** Deferring would leave the gate with
one carve-out whose stated reason ("owned by #696") had just become false, and
would split one mechanical change — every `u32` pagination value at the sqlx
boundary becomes `i64` — across two cycles for no benefit. Done here, the gate's
ALLOWLIST returns to holding only genuine non-newtype binds.

### 4. The web call sites stop doing arithmetic

`page_from_rows(rows, page_size: u32, …)` becomes `page_size: PageSize` and uses
§2's accessors, so no half of the rule is spelled at a call site:

```rust
// storage call — was page_size.value().saturating_add(1)
.list_published_by_user(username, cursor, page_size.fetch_limit(), viewer, now)

// page_from_rows — was rows.len() > page_size as usize, then a matching truncate
let has_more = page_size.has_more(rows.len());
rows.truncate(page_size.page_len());
```

**§2's accessors and this section are one change, not two.** `has_more` and
`page_len` exist only to be called here; if §4 is skipped, §2 emits two methods
nobody uses and the inverse stays hand-written — which leaves problem A
half-fixed, since the defect was never the forward half alone.

`list_home_feed` (`listing.rs:179-205`) hand-rolls `page_from_rows` instead of
calling it; it must call the shared helper. That deletes the fifth derivation
site and the second `has_more` spelling in one move.

### 5. `list_tags` adopts `RowLimit`

`RowLimit` means "fetch at most N rows"; `PageSize::fetch_limit` is one way to
arrive at one and `at_most` is the other. A flat cap is the same concept without
a `PageSize` behind it, so `list_tags` takes `RowLimit` rather than growing a
second type for the same idea. This also removes the hoisted conversion at
`posts.rs:1559`, which is the one site the current gate cannot see.

### 6. The gate learns the hoisted form

`strips_newtype_in_bind` currently inspects only the text after `.bind(`. Extend
the check so a `.bind(<ident>)` whose `<ident>` was assigned from `i64::from(…)`
earlier in the same function is flagged. Scope it to within-a-function analysis;
the cross-function case is **#716** and is explicitly not solved here.

Prove it bites, per this repo's convention: hoist one swept conversion into a
local, confirm the gate fails, restore.

### 7. No new ALLOWLIST entry is needed — the first draft was wrong

The first draft claimed `claim_pending_batch(limit: usize)` hoists
`let limit_i = …` and binds it, so §6 would flag it and it would need a
documented exemption. **That is false on two independent grounds**, each
verified:

- The conversion is `i64::try_from(limit).unwrap_or(i64::MAX)`
  (`storage/src/feed_events.rs:198`) — textually `i64::try_from(`, not the
  `i64::from(` shape §6 matches.
- The `.bind(limit_i)` calls are in **different functions in different files**
  (`storage/src/postgres/feed_events.rs:70`,
  `storage/src/sqlite/feed_events.rs:70`), where `limit_i: i64` arrives as a
  plain trait-method **parameter** (`feed_events.rs:114`). No conversion happens
  in the binding function at all.

So under §6 as scoped, this site is **invisible, not exempted** — it is a second
instance of the parameter-laundering shape that is **#716**, not something #696
allowlists. Recorded on #716.

**Net ALLOWLIST change: both entries go and none is added.** `i64::from(limit)`
is deleted by §1/§5 and `i64::from(offset.value())` by §3, leaving the gate with
only its two pre-existing `Option<PostTitle>`/`Option<PostSummary>` `as_ref()`
entries — i.e. nothing exempted for numeric reasons at all. **That is the
strongest form of this issue's outcome**: the rule #686 added becomes absolute
for numerics, rather than absolute-with-footnotes.

This correction also matters for the plan: the §6 "prove it bites" step must use
`list_tags`' hoist (`posts.rs:1559`, which genuinely is within-function and
genuinely is `i64::from(`), **not** this site — a mutation test against
`claim_pending_batch` would silently prove nothing.

## Out of scope

- **#715** — the ~8 `query_scalar::<_, i64>` id decodes #686's tuple audit could
  not see. The direct sequel to #686 and larger than this issue; bundling it
  would make this unreviewable.
- **#716** — strips laundered through an `i64` function parameter, which no
  within-a-function gate rule can catch. Two instances now: `FeedMinItems` at
  `posts.rs:1611`, and `claim_pending_batch`'s `limit_i` (§7).
- **Typing `claim_pending_batch`** — the issue fences it off as a worker batch
  size paired with `server/src/feed/worker.rs BATCH_LIMIT: usize = 200`, with no
  pagination behind it. §7 explains why it needs nothing from #696.
- **`PageSize`'s own bounds.** `1..=50` and its `clamp` affordance are #537's
  decision and are not revisited.

## Risks

- **§2's `at_most` is the soft spot.** A saturating constructor is a validated
  door, but it is also a place where a caller's mistake (passing `0`) becomes
  `1` silently rather than loudly. Acceptable for compile-time literals; would
  not be acceptable for user input, which is why the public AtomPub `?limit=`
  keeps going through `PageSize::clamped`.
- **§3 changes a documented "no bound" type.** If review prefers `PageOffset`
  keep `u32`, §3 drops out and one ALLOWLIST entry stays — §1/§2/§4/§5 are
  unaffected.
- **§6 may surface sites this spec has not enumerated.** The audit above covers
  `i64::from` hoists; a stricter rule could also match `as i64` or
  `let x: i64 =` shapes. The plan's first task re-runs the enumeration against
  the rule as implemented, and any new site is either swept or allowlisted with
  a reason — not silently exempted.
- **Backend parity.** Every touched storage method is dual-backend; the
  `limit`/`offset` binds exist in both dialect files, so both must change
  together (ADR-0019).
