# Spec — #728: total accounting of storage decode sites

- Issue: [#728](https://github.com/jaunder-org/jaunder/issues/728)
- Date: 2026-07-31
- Depends on: #715 (merged — `sqlx-newtype-decode`, ADR-0085)
- **Blocked by: [#746](https://github.com/jaunder-org/jaunder/issues/746)** —
  converges the stored-enum sqlx ceremony onto a `macros::TextEnum` derive.
  Recorded as a native GitHub dependency. See _Sequencing_ below.

## Problem

`sqlx-newtype-decode` (#715) polices only the `i64` family. Two consequences,
one root: **the population is defined by the primitive type we happened to be
sweeping, not by the set of decodes that exist.**

1. String-family decodes are unguarded, with live residue — most visibly
   `posts.rs:1685`, which decodes `feed_cache.feed_url` as `String` and
   re-parses it, while `feed_cache.rs`'s `CacheTuple` decodes the same column
   straight into `FeedPath`.
2. Sites the gate does not look at are recorded nowhere durable. #715's spec
   enumerated them; that spec is now in `docs/archive/`.

Investigation found a **third** partition the issue does not name: decode
targets the gate cannot see, because the type is written on a struct it does not
read.

```rust
// postgres/feed_events.rs:93-104 — a struct literal over a raw PgRow
FeedEventRecord { attempts: r.get("attempts"), … }
// storage/src/feed_events.rs:35 — a plain struct, no #[derive(FromRow)]
pub struct FeedEventRecord { pub attempts: i32, pub last_error: Option<String>, … }
```

Neither the call nor the field is in any population. #715's module doc asserts
this shape is safe — "the destination field's declared type pins the decode, and
that declaration is itself policed as a declared target" — true for
`#[derive(FromRow)]` structs, false here, and a passing test
(`struct_literal_row_get_is_not_collected`) enshrines the false half. Same shape
at `postgres/backup.rs:279-282` and `sqlite/backup.rs:234` (plain `ColumnInfo`,
`backup.rs:104`).

## Decisions

### D1 — The population is every decode; the pass condition is a _derived_ approve-set

**Rule: a decode target passes iff every leaf type in it is an approved column
type. Everything else is a failure unless allowlisted.**

A type is approved iff its declaration carries a derive that emits the shared
sqlx bridge (`macros/src/sqlx_bridge.rs::bridge()`) — after #746, the four
families `StrNewtype`, `IdNewtype`, `NumNewtype`, `TextEnum`. **The gate derives
this set by scanning declaration sites**, so adding a newtype approves it
automatically and costs no gate edit.

This predicate is exact rather than approximate: "approved column type" _is_ "a
type we generated an sqlx bridge for," which is the same question the compiler
answers when the decode type-checks.

#### Why searching for approval spellings is legitimate here

ADR-0085 forbids deciding _violations_ or _exemptions_ by searching for
anticipated spellings. This search is neither, and the difference is the failure
direction:

- An incomplete **violation** detector is **silent** — the site passes, and a
  green run falsely implies it was examined. That is the defect ADR-0085 exists
  to prevent.
- An incomplete **approval** detector is **loud** — an unrecognised declaration
  form means the type is not approved, so every decode into it **fails**. The
  author sees the gate bite and either teaches it the form or writes an entry.

The approve-set **fails closed**. That asymmetry is the whole argument, and it
is what makes this design strictly better than the alternative considered first
(deny over the closed Rust primitive vocabulary), which left `Uuid`, `NaiveDate`
and every other non-primitive non-newtype target passing silently.

#### The three parts of the rule

1. **Derived approve-set.** Scan declaration sites for the bridge-emitting
   derives. Source- visible: `syn` reads derive attributes directly, without
   macro expansion.
2. **`APPROVED_FOREIGN`.** Legitimate decode targets not declared in this repo —
   `DateTime<Utc>`, `serde_json::Value`, and whatever the first gate run
   surfaces. Each carries a written reason. This is the only hand-maintained
   part, and it is small (~3 entries) because the ~35 domain types derive
   automatically.
3. **Per-site `ALLOWLIST`.** Unchanged in shape from #715 — every unapproved
   leaf needs an entry naming the site, with a reason and a multiplicity.

#### A marker on the wrapper is not needed, and a _new_ one would not work

A marker trait emitted **by** the derive would be invisible: `syn` parses
source, not expansion, so it never sees a generated
`impl DomainColumn for Slug`. A separate hand-written marker would be
bookkeeping that can drift from the real `Decode` impl. The derive already
**is** the marker, in the source, at the type.

#### Self-checking: the gate must not silently under-approve

Under-approval fails closed, but noisily — a forgotten family would produce
dozens of confusing failures. So the gate cross-checks its own enumeration:
**every `#[proc_macro_derive(Name)]` declared in `macros/` must appear either in
the gate's bridge-emitting list or in a `NON_BRIDGE_DERIVES` list carrying a
reason.** A derive in neither is one clear failure instead of thirty obscure
ones.

The check is deliberately _not_ "which derives reach `sqlx_bridge::bridge()`".
That is not determinable by a `syn` scan — the call is two hops deep through a
module-shadowing local function, and for `StrNewtype` it is **conditional on the
derive's own attributes** (`no_sqlx` / `secret` suppress it), so "reaches
`bridge()`" is not a static property of the derive at all. Enumerating the
derives and forcing each into one of two lists gets the same guarantee from a
property `syn` can actually read.

This is ADR-0085 principle 6 turned inward — a gate that quietly shrinks its own
enumeration is the failure mode, whichever direction it shrinks.

**Consequence worth stating:** the approve-set means "declared with a
bridge-capable derive," _not_ "has a bridge." A `#[str_newtype(secret)]` or
`no_sqlx` type carries `StrNewtype` and emits no bridge, so it is approved here
while being undecodable in fact. Harmless — the compiler rejects a decode into a
type with no `Decode` impl — but the module doc must say so, or a reader will
take approval for proof of a bridge.

#### Leaf recursion

Recursion to reach leaves goes through `Vec`/`Option`/`Result`/references/tuples
**and `Type::Slice`/`Type::Array`** — the last two are a gap today
(`sqlx_newtype_decode_check.rs:234-257` handles
`Path`/`Tuple`/`Reference`/`Paren`/`Group` only), so `&[u8]` and `[u8; 32]`
targets would slip through. No such decode exists today; closed now rather than
inherited.

#### Accepted residual, stated honestly

Type identity cannot catch **adjacent same-typed columns**. `DateTime<Utc>` is
approved, so these transpose invisibly and compile:

- `helpers.rs:204-211` `SessionRow` — `created_at`, `last_used_at`
- `helpers.rs:230-236` `InviteRow` — `created_at`, `expires_at`
- `feed_cache.rs:40-47` `CacheTuple` — `updated_at`, `generated_at`

Same class as ADR-0063/#686, and no type-identity gate can see it — it needs
column-order correspondence, a different mechanism. **Deferred to a follow-up
issue** (A11), not silently accepted.

#### Costs on the record

- The gate's population now depends on a **second tree** (declaration roots). A
  missing or unparseable file there is a hard failure, consistent with how
  `storage/src` is already treated — but it is new coupling.
- `syn` sees identifiers, not resolved paths, so two same-named types in
  different modules would be conflated. Not live today; stated as a boundary.
- A newtype that should never be a decode target is nonetheless approved.
  Harmless: the compiler rejects a decode into a type with no `Decode` impl.

### D2 — Unreadable in struct-literal field position is a hard failure

A `.get`/`try_get` as a struct-literal field value with no turbofish fails:
_write the type down_. Rule 1 then reads it.

**Peel set** (so conformance is checkable): field position after peeling `?`
(`Expr::Try`), `.await` (`Expr::Await`), and parens/groups. Both live sites are
`name: row.try_get("column_name")?`, i.e. `Try(MethodCall)`. Anything else —
`.unwrap()`, a cast, a nested call — is **not** field position and is out of
this rule.

Rejected: turbofishing the live sites with no gate rule (fixes today, leaves the
class open); policing every struct field regardless of derive (drags every error
enum and DTO in); cross-file field-map resolution (population depends on two
files, needs a policy for structs defined outside the root).

**Over-bite in field position is nil today** — verified: the only
struct-literal-position `.get`/`try_get` calls under the root are the two
`ColumnInfo` builds and the feed-events mappers, all genuine row reads.

#### D2a — The fn-return over-bite goes live and must be handled up front

#715 documents a "latent over-bite": an unascribed `.get` on a non-row inside a
fn whose return type is in population is recorded via rule 3. Widening the
population under D1 makes it **live**, and it must not be discovered at first
gate run.

- `test_support.rs:1108-1110` —
  `async fn get(&self, key: &str) -> sqlx::Result<Option<String>>` whose body is
  a `HashMap::get`. Rule 3 supplies `String`. **Remedy: ascribe/turbofish at the
  call**, per #715's own stated fix.
- `visit_item_type` polices **every** tuple alias under the root, including
  aliases that are not `query_as` targets. `helpers.rs:32-42` `UserRecordParts`
  is a plain function-parameter tuple carrying two `bool`s. **Remedy: allowlist
  entries** in a `not-a-decode-target` category (D6) — narrowing
  `visit_item_type` would require guessing which aliases are query targets, the
  pattern search ADR-0085 forbids.

### D3 — Missing type infrastructure is fixed here when it is local

**Rule:** fix it here when the missing type is what makes the decode untypeable
and the fix is local; allowlist with a reason and file an issue when the missing
type is a vertical.

- **`FeedEventStatus` — fix here, and it moves to `common/src/feed/`.** A bare
  enum with a hand-rolled `parse_status(&str)`. It gets the ADR-0075 strum stack
  plus `#[derive(macros::TextEnum)]` (#746's form).

  An earlier revision of this spec left it in `storage/src/feed_events.rs`,
  reasoning that #746 makes "the ceremony reachable outside `common`." That
  reasoning was wrong on three verified counts, all of which make leaving it in
  `storage` the _expensive_ option:
  1. **The named parse error is still `common`-internal.**
     `common/src/lib.rs:15,40` declare `mod db_enum;` and `mod strum_enum;` —
     both **private** — and `strum_enum.rs:51` is `pub(crate) use parse_error;`.
     `parse_error!` is exactly as unreachable from `storage` as
     `impl_text_column_enum!` was, and #746's own "Out of scope" says _"Do not
     fold `parse_error!` into the derive."_
  2. **`storage` has neither dependency.** `storage/Cargo.toml` pulls in no
     `macros` and no `strum`; both would have to be added.
  3. **The bridge would silently compile out.** `macros/src/sqlx_bridge.rs:28`
     wraps the emitted impls in `#[cfg(feature = "sqlx")]`, evaluated **in the
     consuming crate**. `storage`'s features are `test-utils`, `seed-posts`,
     `test-support` — no `sqlx` — so the derive would emit nothing and
     `query_as::<_, FeedEventStatus>` would fail to type-check with no
     indication why.

  Moving it costs one import update in `server/src/feed/worker.rs` (or a
  re-export from `storage`). Leaving it costs two new dependencies, a new crate
  feature, and a hand-rolled error type — work this issue never authorised. D1's
  declaration scan finds it either way, so the gate is indifferent; the crate
  boundary is not.

- **`TargetKind` — fix here.** Already in `common/src/visibility.rs` with the
  full stack; needs `#[derive(macros::TextEnum)]`. `visibility.rs:7-12`'s
  comment gets clarified: ADR-0075 scopes the bridge to enums _stored_ as a TEXT
  token, and `target_kinds.name` is genuinely _read_ as text — a clarification,
  not a reversal (checked; no ADR-0075 conflict).
- **`subscriber_ref` — allowlist, follow-up.** A `SubscriberRef` newtype touches
  the subscription/admission seam, the `ChannelId` pairing and the wire DTOs — a
  vertical.
- **Config values — allowlist, reason names #687.** `(String, String)` at
  `site_config.rs:404` is a real transposition hazard, but #687 owns the key
  half. Once it lands the pair is `(SiteConfigKey, String)` and is no longer
  transposable — **the entry does not go away**, since the value half stays
  `String`; the reason must say so.

### D4 — `FeedEventRecord` becomes a `FromRow` target, with the purge path kept narrow

`FeedEventRecord` gains `#[derive(sqlx::FromRow)]`, with
**`#[sqlx(rename = "feed_url")]` on `feed_path`** — the derive binds by field
name and the column is `feed_url`; without it every claim fails at runtime.

The only reason `claim_pending_batch` reads raw rows is the corrupt-`feed_url`
diversion (`postgres/feed_events.rs:85-92`): a row whose `feed_url` will not
parse must yield **its `id`** so the worker can purge it rather than wedging the
batch forever.

**The wrapper must not widen that diversion.** Today exactly one column diverts
to purge; every other column uses infallible `.get`, so a decode failure
elsewhere propagates rather than deleting data. A naive
`ClaimedRow(Result<FeedEventRecord, FeedEventId>)` diverting on _any_ derive
failure would let a schema or driver regression on a timestamp column silently
DELETE the queue. So:

```
FromRow for ClaimedRow:
  match FeedEventRecord::from_row(row):
    Ok(rec) -> Ok(Ok(rec))
    Err(e)  -> if row.try_get::<FeedPath,_>("feed_url").is_err():
                   Ok(Err(row.try_get::<FeedEventId,_>("id")?))   // id must decode
               else:
                   Err(e)                                          // propagate, never purge
```

`purge_corrupt` therefore deletes on exactly the same condition as today. If
`id` itself will not decode there is no third state: the error propagates and
the batch fails — stated here so it is a decision, not an accident.

The query becomes `query_as::<_, ClaimedRow>`, the loop becomes a partition.
Both dialects' partition logic is identical, so it lands in one shared helper in
`storage/src/feed_events.rs`; the hand-written `FromRow` must restate the
derive's generic `impl<'r, R: Row>` bounds to serve both dialects.

**`attempts` decodes as `i32` on both backends** — verified: `sqlx-sqlite`
implements `Decode<Sqlite>` for `i32` (`compatible` accepts `Int4 | Integer`),
and both migrations declare `attempts INTEGER`
(`0015_create_feed_events.sql:5`). This is _stricter_ than today's SQLite path,
which saturates (`i32::try_from(attempts).unwrap_or(i32::MAX)`): an out-of-range
value becomes a decode error that propagates. Accepted — it needs 2³¹ retries to
reach, and silent saturation is the same coercion class this issue is about.

**Net accounting effect:** SQLite's `attempts` entry moves rather than vanishes
— `attempts: i32` **and** `last_error: Option<String>` become declared decode
targets on the `FromRow` struct and need entries in `feed_events.rs`.

### D5 — `parse_status`'s fallback is dead code; removing it is not a behaviour change

`parse_status` coerces any unrecognised token to `Failed`. **That fallback is
unreachable.** Eligibility is
`WHERE (status = 'pending' AND …) OR (status = 'claimed' AND …)`, and the
statement is `UPDATE … SET status = 'claimed' … RETURNING … status`, so a row
with an unrecognised status is never selected, and every returned row carries
the just-written `'claimed'`. `claim_pending_batch` is the only
`FeedEventRecord` producer.

Removing it removes a coercion that is currently dead but would become live if
the eligibility predicate ever widened. **No observable behaviour changes**, and
no test can purge a row for a bad status — the precondition is unconstructible.

### D6 — Allowlist entries carry a reading category

`Allowed` gains `category`: `schema-introspection`, `count-or-exists`,
`opaque-payload`, `deliberate-lossy`, `not-a-decode-target` (for D2a's over-bite
entries). The failure footer groups by it. **Nothing branches on it** — matching
and the count check are untouched, so it cannot become a region exemption.

Entries stay one-per-site: the backup-introspection reads are **not** collapsed
into one multiplicity entry, because the key distinguishes them and a collapsed
entry would silently absorb a twelfth read.

**The gate also gains a duplicate-key check**: two entries with identical (file,
function, target, what) are a failure. Today matching is `.any(...)` and the
count check is per-entry, so two entries each declaring 1 would both pass while
double-covering one site — a gate blind to its own allowlist.

### D7 — `FeedPath` gains an accessor to its parts

`feed_urls_needing_catchup` needs `(FeedSurface, FeedFormat)` to call
`max_published_at_for_surface`. `FeedPath` exposes only `canonical()` and the
free `parse()`, with no way back — so decoding as `FeedPath` alone would remove
the _rebuild_ but leave a re-parse of an already-validated value, the very shape
this issue is about.

`FeedPath` gains a `parts()` accessor returning its surface and format.

**It returns `Option`, not a bare tuple.** The first draft specified an
_infallible_ accessor, reasoning that a `FeedPath` is validated at construction.
That is true but not expressible: recovering the parts means calling `parse`,
which is partial, and the workspace denies `clippy::expect_used` —
`CONTRIBUTING.md` is categorical ("Never use `.unwrap()` or `.expect()` in
production code… Use Rust's type system to make invalid states impossible").
Silencing the lint would be the wrong reading of that rule, because the
invariant genuinely spans two functions (`canonicalize` emits, `parse` accepts)
rather than being enforced by the type. `Option` keeps the impossible case a
value the caller handles.

The unreachable `None` costs nothing at the one call site: D9's loop already
skips a row whose `feed_url` will not decode, so the two skips fold into one
branch.

### D9 — The catch-up scan keeps skip-on-corrupt; typing it must not wedge the worker

`feed_urls_needing_catchup` currently does
`let Some(..) = common::feed::parse(&feed_url) else { continue }` — an
unparseable `feed_cache.feed_url` is **skipped**. The obvious way to satisfy A4
is `query_as::<_, (FeedPath, DateTime<Utc>)>`, which turns that skip into a
whole-query `ColumnDecode` error. **That would wedge the feed worker
permanently**, so it is rejected:

`server/src/feed/worker.rs:76-93` — `go_live_pass` runs the catch-up branch only
while `last_tick` is `None`, and `*last_tick = Some(now)` sits **after** the `?`
on line 80. `tick` logs the error and continues. So one unparseable row means
catch-up fails, `last_tick` never advances, every later tick retries catch-up
and fails again, and the incremental `list_posts_gone_live_between` branch is
**never reached**. Go-live feed enqueueing stops for good, from one bad row.

This is reachable without tampering: `FeedPath`'s grammar has been tightened
before (`feed_path.rs:226`), so a row written under an older grammar can stop
parsing.

Note the asymmetry this avoids — D4 spends a whole section making sure a corrupt
`feed_url` cannot wedge the _event_ queue. Introducing exactly that wedge one
file over would be incoherent.

**Shape:** iterate rows and decode per-row with a turbofished
`row.try_get::<FeedPath, _>("feed_url")`, skipping (and `tracing::warn`-ing) a
row that fails. The decode is typed — so A4 is satisfied and the gate sees an
approved target — while the blast radius stays one row, exactly as today.
Widening the worker's resilience, or changing `go_live_pass`'s `last_tick`
handling, is **out of scope**.

### D8 — ADR-0085 is amended, not supplemented

A new subsection under Decision states the approve-set rule (D1), the
fail-closed asymmetry that licenses it, and the residual adjacent-same-type
class. The Conformance paragraph for `sqlx-newtype-decode` is updated for the
derived population, D2/D2a, the self-check, the duplicate-key check and
`category`. It must also **re-state the surviving unreadable classes** — the
unascribed `let` (`postgres/backup.rs:177`, `sqlite/backup.rs:160`, both genuine
`serde_json` map gets), argument/statement position, and
decode-typed-only-by-later-use — since D2 removes one and ADR-0085 obliges a
conforming gate to state what it cannot see.

No new ADR: same decision area, and 0085 is still `proposed`.

## Sequencing with #746

#746 replaces `impl_text_column_enum!(X)` (a macro invocation after the type)
with `#[derive(macros::TextEnum)]` (on the type), deletes
`common/src/db_enum.rs`, and makes the bridge reachable outside `common`. It
lands first.

- **The gate targets #746's world**: four bridge-emitting derives, uniformly
  source-visible. No transitional support for the `impl_text_column_enum!` form
  — writing it would mean shipping a form that is deleted before this merges.
- **If #746 slips**, the fallback is to add the macro-invocation form as a fifth
  recognised declaration and delete it on rebase. The self-check in D1 makes
  that a one-line change.
- **D3's `TargetKind` and `FeedEventStatus` work is written in #746's
  spelling**, so this branch rebases onto it rather than racing it.

## Method note — the inventory comes from the gate, not from a search

Every site count quoted during design came from a regex sweep, which is the kind
of search this issue exists to distrust. **The authoritative inventory is the
failure output of the widened gate on its first run.** This spec commits to the
rule, not to a list of sites, and no acceptance criterion is phrased as a count.

## Acceptance criteria

1. **A1 — Derived approve-set.** The gate builds its approved-type set by
   scanning declaration sites for the bridge-emitting derives. Unit tests: a
   type declared with each of the four derives is approved; a decode into an
   undeclared type fails; `String`, `bool`, `i64`, `Uuid` and `NaiveDate` all
   fail with no special-casing.
2. **A1a — Self-check.** The gate fails with one clear message if its list of
   bridge-emitting derives does not match the derives in `macros/` that call
   `sqlx_bridge::bridge()`. Unit-tested by feeding a mismatched pair.
3. **A1b — Leaf recursion.** `&[u8]` and `[u8; 32]` targets are reached
   (`Type::Slice`/`Type::Array`), unit-tested.
4. **A2 — Field position.** A `.get`/`try_get` in struct-literal field position
   with no turbofish fails with a message naming the fix; the peel set is
   unit-tested (`?`, `.await`, parens bite; `.unwrap()` does not).
   `struct_literal_row_get_is_not_collected` is deleted along with the
   module-doc claim it encoded.
5. **A3 — Accounting.** `cargo xtask check` is green, and every decode the gate
   can read is either approved or named by an allowlist entry, verified by the
   staleness check plus the new duplicate-key check. _Not_ claimed: coverage of
   the unreadable classes, which are excluded by construction and stated in the
   module doc.
6. **A4 — `posts.rs:1685`.** `feed_urls_needing_catchup` decodes `feed_url` as
   `FeedPath` (per-row, per D9); the `FeedPath::canonical` rebuild is gone; the
   `common::feed::parse` re-parse is replaced by D7's accessor. A test proves
   one unparseable row is skipped and the scan still returns the other feeds —
   the D9 no-wedge property.
7. **A5 — `FeedEventStatus`.** Lives in `common/src/feed/`, carries the ADR-0075
   strum stack and `#[derive(macros::TextEnum)]`; `server/src/feed/worker.rs`'s
   import is updated (or `storage` re-exports). `parse_status` and
   `parse_status_handles_all_statuses` are gone, replaced by a
   `FromStr`-rejects-unknown-token test. `FeedEventRecord` derives `FromRow`
   with `#[sqlx(rename = "feed_url")]`; neither `postgres/feed_events.rs` nor
   `sqlite/feed_events.rs` contains a `.get`/`try_get` (the `try_get("id")`
   **moves into** `ClaimedRow`'s impl — it does not disappear).
8. **A6 — Corrupt-row behaviour preserved exactly.**
   `claim_purges_rows_with_unparseable_feed_url` passes unchanged. A new test
   proves a derive failure on a **non**-`feed_url` column propagates as an error
   and does **not** delete the row — the D4 narrow-purge property.
9. **A7 — `TargetKind`.** `get_post_audiences` decodes `tk.name` as
   `TargetKind`, so an unrecognised kind is a decode error rather than a
   silently shortened result; `visibility.rs`'s comment is updated. **The mapper
   is not simply deleted**: `audience_target_from_row` (`posts.rs:1859-1866`)
   drops a row for _two_ reasons — an unrecognised kind **and** a `named` kind
   whose `audience_id` is NULL. Only the first is this issue's business; the
   NULL case keeps its current behaviour and its existing assertion
   (`posts.rs:2305`).
10. **A8 — Categories.** `Allowed` carries `category` and the footer groups by
    it. Proven behaviourally: two entries identical but for `category` produce
    identical match and count results.
11. **A9 — The gate bites on a reverted fix.** Each a _named one-line revert_,
    with the four observed failure messages recorded in the commit message
    (durable in the repo), not only the PR body:
    - A4: retype the per-row `try_get` to `::<String, _>`
    - A5: retype `FeedEventRecord.status` to `String` (the pre-#728 `&str`
      turbofish no longer exists — the mapper it lived in is deleted by D4)
    - A7: retype the `rows` `let` to `Vec<(String, Option<AudienceId>)>`
    - A2: drop one `ColumnInfo` turbofish
12. **A10 — ADR-0085 amended** per D8, including the re-stated surviving
    unreadable classes.
13. **A11 — Follow-ups filed.** (a) a `SubscriberRef` newtype issue, referenced
    by the `subscriptions.rs` entry's reason; (b) an issue for the adjacent
    same-typed column transposition class of D1, referenced from the ADR
    amendment.
14. **A12 — Verdicts for the issue's named sites.** Each string target #728
    lists carries an entry or a fix, explicitly including `helpers.rs`
    `SessionRow` position 4 (the deliberate lossy `SessionLabel`, category
    `deliberate-lossy`).
15. **A13 — Coverage.** The coverage gate is green. `ClaimedRow`'s error arms
    are reachable from A6's tests; anything genuinely unreachable carries a
    `cov:ignore` with a written reason.

## Out of scope

- The `.bind(` direction — #716. `SiteConfigKey` — #687. The stored-enum derive
  itself — #746.
- The `#[server]`-param / struct-field adoption gate — #697. Its "rejects
  registry" is this allowlist; under D1 there is no separate registry, and #697
  should adopt the `category`-carrying shape rather than invent one.
- Widening the policed root beyond `storage/src` — unchanged from #715. (D1's
  _declaration_ scan reads more trees; the _policed_ root is unchanged.)
- A `SubscriberRef` newtype and the adjacent same-typed column class (A11 files
  both).
