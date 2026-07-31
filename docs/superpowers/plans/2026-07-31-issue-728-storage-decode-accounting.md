# Plan — #728: total accounting of storage decode sites

**Spec:**
[`docs/superpowers/specs/2026-07-31-issue-728-storage-decode-accounting.md`](../specs/2026-07-31-issue-728-storage-decode-accounting.md)
**Issue:** [#728](https://github.com/jaunder-org/jaunder/issues/728) · **Blocked
by:** [#746](https://github.com/jaunder-org/jaunder/issues/746) **For agentic
workers:** drive with `jaunder-iterate`; delegate individual tasks via
`jaunder-dispatch`.

## Review header

**Goal.** Make `sqlx-newtype-decode` police _every_ decode it can read under
`storage/src` by replacing its `i64`-family population with a **derived
approve-set** (spec D1), and clear the residue that widening exposes.

**Scope — in:** the gate rewrite (approve-set, leaf recursion, field-position
rule, self-check, duplicate-key check, categories); `FeedEventStatus` moving to
`common` and `TargetKind` adopting `macros::TextEnum`; `FeedEventRecord`
becoming a `FromRow` target with a narrow purge path; `posts.rs` decoding
`FeedPath` per-row; a `FeedPath` parts accessor; the ADR-0085 amendment. **Scope
— out:** #716, #687, #697, #746 itself; a `SubscriberRef` newtype; the adjacent
same-typed column class (both filed in T1); any change to the feed worker's
resilience or `go_live_pass`'s `last_tick` handling (spec D9).

**Tasks.**

- [x] **T1** — File the two follow-up issues (A11) → **#750** (`SubscriberRef`),
      **#751** (adjacent same-typed columns)
- [x] **T2** — `FeedPath::parts()` accessor (D7) — returns `Option`, not a bare
      tuple; see spec D7 (`expect_used` is denied workspace-wide)
- [x] **T3** — `posts.rs` decodes `FeedPath` per-row, keeping skip-on-corrupt
      (A4, D9)
- [x] **T4** — Gate: `category` + duplicate-key check (A8, D6) — plus the
      `deferred-newtype` category, whose reason must name an issue
- [x] **T5** — Gate: derive self-check (A1a)
- [ ] **T6** — _rebase onto #746_ — **blocking stop**
- [ ] **T7** — `TargetKind` adopts `TextEnum`; `get_post_audiences` decodes it
      (A7) — needs T6
- [ ] **T8** — `FeedEventStatus` moves to `common` + `TextEnum`; delete
      `parse_status` (A5, D5) — needs T6
- [ ] **T9** — `FeedEventRecord` → `FromRow`; `ClaimedRow` narrow purge (D4, A6)
      — needs T6, T8
- [ ] **T10** — Gate: struct-literal field-position rule + peel set (A2) — needs
      **T9**
- [x] **T11** — Gate inventory: run under the new rule, record the raw site list
      — **ran early, pre-#746; 129 sites, stop-and-report fired. See "T11
      result" below. Re-run after T9/T10 for the final number.**
- [ ] **T12** — Gate: approve-set + `APPROVED_FOREIGN` + `ALLOWLIST` (A1, A1b,
      A3, A12) — needs T11
- [ ] **T13** — Module doc + ADR-0085 amendment (A10, D8) — needs T12
- [ ] **T14** — Record the four revert-proofs (A9) — needs T12 (before its
      commit is final)

**Commit ordering is deliberate.** T4, T5, T10 are additive and stay green under
the _old_ `is_i64_family` rule, so each is a real standalone commit. Only
**T12** must land the rule and its allowlist together — neither is verifiable
without the other — and T11 sizes it first. The pre-commit hook runs the full
`cargo xtask check` and `CONTRIBUTING.md` requires history stay green
commit-by-commit, so a red intermediate commit is not an option to be noted
around.

**Key risks / decisions.**

- **Production fixes land before the gate.** Otherwise T12's allowlist needs
  temporary entries for sites T7–T9 are about to type — lies in the one artifact
  whose value is that it is read. The gate's bite is proven instead by T14's
  named reverts.
- **T3 must not wedge the worker** (spec D9). The obvious
  `query_as::<_, (FeedPath, _)>` turns today's per-row skip into a whole-query
  error that stops go-live enqueueing permanently. Per-row `try_get` instead.
- **T11 exists because T12 is otherwise unbounded.** ~180 decode call sites
  across 33 files, and the gate scans `#[cfg(test)]` modules too, so the entry
  count is unknown until the gate runs. Size it before committing to it.
- **T6 is a real stop.** T1–T5 and (with T9's caveat) T10 land without #746.

---

## Global constraints

- **No `Co-Authored-By` trailer** on any commit.
- Every commit: `devtool run -- cargo xtask check` green first (see
  `jaunder-commit`). Never edit while a gated commit is in flight.
- Storage tests follow the dual-backend template (`CONTRIBUTING.md` "backend
  parity").
- No tests in ADR-0019 dialect files (`storage/src/{postgres,sqlite}/*.rs`).
- `xtask` is outside the workspace:
  `cargo nextest run --manifest-path xtask/Cargo.toml`.
- Server package is `jaunder` (`-p jaunder`).
- Coverage policy applies to `xtask` and `macros`; cover error paths or carry a
  trailing `// cov:ignore` **on the executable line** — `report.rs:54-98`
  implements only `// cov:ignore`, `// cov:ignore-start`, `// cov:ignore-stop`.
  There is no `cov:ignore-next-line`.

---

## Task 1 — File the two follow-up issues

**Not code.** Use `jaunder-issues`. Both are referenced later, so they exist
first.

1. **`SubscriberRef` newtype** — `subscriptions.rs:28`'s "channel-scoped opaque
   reference to the subscriber". Touches the subscription/admission seam, the
   `ChannelId` pairing, the wire DTOs. Label `type-safety`.
2. **Adjacent same-typed column transposition** — `SessionRow`
   (`created_at`/`last_used_at`), `InviteRow` (`created_at`/`expires_at`),
   `CacheTuple` (`updated_at`/`generated_at`). No type-identity gate can see
   this. Label `type-safety`; reference ADR-0085.

**Done when:** both exist; numbers recorded here for T12's reasons and T13's ADR
text.

---

## Task 2 — `FeedPath::parts()` accessor

**Files:** `common/src/feed/feed_path.rs` (+ in-file `#[cfg(test)]`).

```rust
impl FeedPath {
    /// The surface and format this path addresses.
    ///
    /// Infallible, but **not** because `FromStr` validated it — [`FeedPath::canonical`]
    /// builds `Self(canonicalize(..))` directly, bypassing `FromStr`, and is the primary
    /// constructor. The guarantee is that `canonicalize` and [`parse`] round-trip, pinned
    /// by `round_trips_all_surfaces_and_formats`. Widening `canonicalize` without widening
    /// `parse` would break this.
    #[must_use]
    pub fn parts(&self) -> (FeedSurface, FeedFormat) {
        parse(&self.0).expect("a FeedPath round-trips through parse by construction") // cov:ignore
    }
}
```

**Tests (in-file):** round-trip through `parts` for the site surface **and** a
tag/user surface, so a non-trivial surface is exercised.

**Run:** `cargo nextest run -p common feed_path` → PASS. **Commit:**
`feat(common): FeedPath::parts() accessor (#728)`

---

## Task 3 — `posts.rs` decodes `FeedPath` per-row

**Files:** `storage/src/posts.rs` (`feed_urls_needing_catchup`, ~1679-1706).

**Read spec D9 first.** `query_as::<_, (FeedPath, DateTime<Utc>)>` is the wrong
shape: it turns today's per-row skip into a whole-query error, and
`go_live_pass` (`server/src/feed/worker.rs:76-93`) never advances `last_tick`
past a failure, so one bad row stops go-live enqueueing permanently.

Use `sqlx::query` and decode per row:

```rust
let rows = sqlx::query("SELECT feed_url, generated_at FROM feed_cache")
    .fetch_all(&self.pool)
    .await?;
let mut needing = Vec::new();
for row in rows {
    // Skip, don't fail: a row written under an older FeedPath grammar must not wedge the
    // whole catch-up scan (and with it every later tick — see the worker's `last_tick`).
    let Ok(feed_path) = row.try_get::<FeedPath, _>("feed_url") else {
        tracing::warn!("skipping feed_cache row with an unparseable feed_url");
        continue;
    };
    let generated_at: DateTime<Utc> = row.try_get("generated_at")?;
    let (surface, _format) = feed_path.parts();
    if let Some(max) = max_published_at_for_surface::<DB>(&self.pool, &surface, now).await? {
        if max > generated_at {
            needing.push(feed_path);
        }
    }
}
```

`generated_at` is unascribed-`try_get`-in-a-`let`, so ascribe it as shown —
under T12's rule an unreadable target is not silently skipped. Update the doc
comment above (it currently says the design "keeps the `feed_url` → surface
parsing in Rust (`common::feed::parse`)").

**Test** (in-file, dual-backend): seed two `feed_cache` rows, corrupt one
`feed_url` with a raw `UPDATE` (the pattern at `feed_cache.rs:227,259`), assert
the scan skips it and still returns the other. This is A4's no-wedge property.

**Run:** `cargo nextest run -p storage feed_urls_needing_catchup` → PASS.
**Commit:**
`refactor(storage): decode feed_url as FeedPath in catch-up scan (#728)`

---

## Task 4 — Gate: `category` + duplicate-key check

**Files:** `xtask/src/steps/sqlx_newtype_decode_check.rs`. Additive; gate stays
green.

1. `Allowed` gains `category: &'static str` — `schema-introspection`,
   `count-or-exists`, `opaque-payload`, `deliberate-lossy`,
   `not-a-decode-target`. Fill in the ten existing entries.
2. The failure footer groups entries by category.
3. Duplicate-key check: two entries with identical (file, function, target,
   what) fail, reported **before** the per-entry count check so the message is
   unambiguous.

**Tests:** two entries identical but for `category` produce identical
match/count behaviour (A8's falsifiable form — do **not** attempt to assert "no
code path branches on it"); two entries with the same key fail with the
duplicate message.

**Run:** `cargo nextest run --manifest-path xtask/Cargo.toml allowlist` → PASS.
**Commit:**
`feat(xtask): categorise and de-duplicate the decode allowlist (#728)`

---

## Task 5 — Gate: derive self-check

**Files:** same. Additive; gate stays green.

**Do not** try to detect which derives reach `sqlx_bridge::bridge()` — that is
two hops through a module-shadowing local fn, and for `StrNewtype` it is
conditional on the derive's own attributes (`no_sqlx`/`secret`), so it is not a
static property. Spec D1 settles this.

Instead: parse `macros/src/lib.rs` with `syn`, collect every
`#[proc_macro_derive(Name)]`, and require each to appear in **either** the
gate's bridge-emitting list **or** a `NON_BRIDGE_DERIVES` list with a written
reason. A derive in neither is one clear failure.

**Tests:** a synthetic derive absent from both lists fails and is named; one
present in each list passes. Use `syn::parse_quote!` for inputs.

**Run:** `cargo nextest run --manifest-path xtask/Cargo.toml derive_enumeration`
→ PASS. **Commit:**
`feat(xtask): the decode gate enumerates the macros crate's derives (#728)`

---

## Task 6 — Rebase onto #746

**Not code.** #746 lands `macros::TextEnum`, deletes `common/src/db_enum.rs`,
retires `impl_string_serde_proxy!` and the `#[serde(into, try_from)]`
attributes.

1. Confirm #746 is merged to `main`.
2. Rebase (foreground, with a timeout, `-c core.editor=true`).
3. Re-run `devtool run -- cargo xtask check` — a clean rebase is **not** a
   verified rebase.
4. Re-read `macros/src/lib.rs` and take the **exact** derive list from source
   for T5's two lists. Update `NON_BRIDGE_DERIVES` if #746 added anything.

---

## Task 7 — `TargetKind` adopts `TextEnum`; `get_post_audiences` decodes it

**Files:** `common/src/visibility.rs`, `storage/src/posts.rs` (~947-969,
~1859-1866).

1. Add `macros::TextEnum` to `TargetKind`'s derive list.
2. Update the convention comment at `visibility.rs:7-12`: the bridge governs
   enums _stored_ as a TEXT token; `target_kinds.name` is FK-normalized on the
   write side but genuinely _read_ as text, so the decode half applies. A
   clarification, not an ADR-0075 reversal.
3. Decode `Vec<(TargetKind, Option<AudienceId>)>`.
4. **Keep a mapper — do not just drop the `filter_map`.**
   `audience_target_from_row` drops a row for _two_ reasons, and only one is
   ours:

   ```rust
   fn audience_target_from_row(kind: TargetKind, audience_id: Option<AudienceId>)
       -> Option<AudienceTarget> {
       match kind {
           TargetKind::Public => Some(AudienceTarget::Public),
           TargetKind::Subscribers => Some(AudienceTarget::Subscribers),
           // Unchanged: a `named` row with a NULL audience_id is still dropped. #728 types
           // the kind; the NULL case is not this issue's business.
           TargetKind::Named => audience_id.map(AudienceTarget::Named),
       }
   }
   ```

   The `Err(_) => None` arm goes — that is now a decode error. Keep
   `posts.rs:2305`'s `audience_target_from_row("named", None) == None`
   assertion, retyped to the new signature.

**Test:** dual-backend in `storage/src/posts.rs` — an unrecognised
`target_kinds.name` surfaces as an error rather than a silently shortened
result. Keep existing coverage.

**Run:** `cargo nextest run -p common visibility`,
`cargo nextest run -p storage get_post_audiences` → PASS. **Commit:**
`refactor(storage): decode target_kinds.name as TargetKind (#728)`

---

## Task 8 — `FeedEventStatus` moves to `common` and adopts `TextEnum`

**Files:** new `common/src/feed/event_status.rs` (+ `common/src/feed/mod.rs`),
`storage/src/feed_events.rs`, `server/src/feed/worker.rs`.

**Read spec D3 first** — it stays in `common` for three verified reasons
(`parse_error!` is `pub(crate)` in a private module; `storage` has no
`macros`/`strum` dep; the bridge is `#[cfg(feature = "sqlx")]` and `storage` has
no such feature, so the derive would silently emit nothing).

```rust
#[derive(
    Clone, Copy, Debug, PartialEq, Eq,
    strum::AsRefStr, strum::Display, strum::EnumString, macros::TextEnum,
)]
#[strum(serialize_all = "snake_case")]
#[strum(parse_err_ty = InvalidFeedEventStatus, parse_err_fn = feed_event_status_parse_err)]
pub enum FeedEventStatus { Pending, Claimed, Done, Failed }

parse_error!(
    InvalidFeedEventStatus,
    feed_event_status_parse_err,
    "feed event status must be \"pending\", \"claimed\", \"done\", or \"failed\""
);
```

Delete `parse_status` and `parse_status_handles_all_statuses` — spec D5
establishes the fallback is unreachable, so removal changes no behaviour. Update
`storage`'s import; either re-export from `storage/src/lib.rs` (keeping
`server/src/feed/worker.rs:13,322` working) or update the two `server` imports.
Prefer re-export — smaller diff, and `worker.rs` already imports the record from
`storage`.

**Test:** in `common`, `FeedEventStatus::from_str("???")` is an `Err` naming the
token. Pure — not dual-backend.

**Run:** `cargo nextest run -p common feed_event_status`,
`cargo nextest run -p jaunder feed` → PASS. **Commit:**
`refactor(common): FeedEventStatus moves to common with the TextEnum bridge (#728)`

---

## Task 9 — `FeedEventRecord` → `FromRow`, with a narrow purge path

**Files:** `storage/src/feed_events.rs`,
`storage/src/{postgres,sqlite}/feed_events.rs`.

1. `#[derive(sqlx::FromRow)]` on `FeedEventRecord`,
   `#[sqlx(rename = "feed_url")]` on `feed_path` — **without it every claim
   fails at runtime** (the derive binds by field name).
2. `ClaimedRow` with the hand-written `FromRow` from **spec D4, exactly that
   shape**. The `else` arm propagates; it must never purge. Restate the derive's
   `impl<'r, R: Row>` bounds (including the `&'r str: ColumnIndex<R>` predicate)
   so both dialects are served.
3. One shared partition helper in `storage/src/feed_events.rs`; both dialects
   call it.
4. Both dialect files: `query_as::<_, ClaimedRow>(…)` then the helper. The
   inline `r.get` mappers and SQLite's
   `i32::try_from(attempts).unwrap_or(i32::MAX)` narrowing both go.

**Tests** (dual-backend, in `storage/src/feed_events.rs`):

- `claim_purges_rows_with_unparseable_feed_url` — **unchanged**, must still
  pass.
- New: a row whose **non**-`feed_url` column fails to decode propagates an error
  and the row is **still present** afterwards. Assert the row count, not just
  the error — this is the property that stops the purge path widening from one
  column to ten.

**Run:** `cargo nextest run -p storage feed_events` → PASS. **Commit:**
`refactor(storage): FeedEventRecord decodes via FromRow (#728)`

---

## Task 10 — Gate: struct-literal field-position rule

**Files:** `xtask/src/steps/sqlx_newtype_decode_check.rs`,
`storage/src/{postgres,sqlite}/backup.rs`.

**Must run after T9.** The rule bites the feed-events mappers
(`postgres/feed_events.rs:97-103`, `sqlite/feed_events.rs:92-97` — 13 bare
`r.get` in field position) that T9 deletes. Landing this first means a red gate.

1. Track struct-literal field-value position, peeling `Expr::Try`,
   `Expr::Await`, `Expr::Paren`, `Expr::Group` — **and nothing else** (spec D2).
2. A `.get`/`try_get` there with no turbofish is a hard failure, message naming
   the fix.
3. Delete `struct_literal_row_get_is_not_collected` and the module-doc paragraph
   claiming the destination struct's declaration polices this shape.
4. Turbofish the live sites: `postgres/backup.rs:279-282`,
   `sqlite/backup.rs:234` gain `::<String, _>`. Green under the old
   `is_i64_family` rule, so this is a standalone commit.

**Not here:** `test_support.rs:1108-1110`'s `HashMap::get`. That is the
_fn-return_ over-bite (D2a), not field position, and no turbofish silences it
once T12 lands — `Option<String>` and `str` are both unapproved. It needs an
`ALLOWLIST` entry (`not-a-decode-target`) in T12.

**Tests:** `?`, `.await`, parens bite; `.unwrap()` does not; a turbofished
field-position get passes.

**Run:** `cargo nextest run --manifest-path xtask/Cargo.toml field_position` →
PASS. **Commit:**
`feat(xtask): the decode gate refuses unreadable struct-literal fields (#728)`

---

## Task 11 — Gate inventory

**Files:** none committed to `xtask` yet — this task **sizes** T12.

Implement T12's approve-set rule locally (do not commit), run
`devtool run -- cargo xtask check`, and capture the parked failure log. Write
the site list into this plan as a checklist grouped by file, one line per site
with its target.

**Done when:** the real entry count is known and written down. If it materially
exceeds the spec's ~39 estimate, stop and report before proceeding — that is a
signal the rule is over-biting, not a licence to write 150 entries.

### T11 result (run 2026-07-31, pre-#746, probe reverted)

**129 sites — the stop-and-report condition fired.** The rule is over-biting,
and the inventory says exactly how. Only ~36 of the 129 are genuine primitives.

| Cause                                                                                                                                                      | Sites | Verdict                      |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------- | ----- | ---------------------------- |
| Composite row targets treated as leaves — `PostRow` ×21, `SessionRow`/`UserRow`/`InviteRow`/`MediaRow`/`CacheTuple`/`UserRecordParts` ×~14                 | ~35   | **Spec gap — see below**     |
| fn-return rule 3 over-bite — `Result<Vec<FeedEventRecord>,_>` ×13, `Result<Option<SmtpConfig>,_>` ×4, `Result<Vec<ColumnInfo>,_>` ×3, `sqlx::Result<_>` ×4 | ~24   | Bigger than D2a assumed      |
| `DateTime<Utc>` in various wrappers                                                                                                                        | ~22   | One `APPROVED_FOREIGN` entry |
| Enums bridged by `impl_text_column_enum!` not a derive — `PostFormat`, `MediaSource`                                                                       | ~3    | **Dissolves with #746**      |
| Genuine primitives — `String` 18, `bool` 11, `&str` 2, `(String,)` 3, `(String,String)` 1                                                                  | ~36   | Real allowlist entries       |

**Spec gap found (blocks T12).** D1 says "every **leaf** type must be approved",
but a `#[derive(FromRow)]` struct or a tuple alias is **not a leaf** — it is a
composite whose fields/elements the gate _already polices separately_
(`visit_item_struct`, `visit_item_type`). Approving it by delegation is not a
loophole: every part is still checked, at the declaration, which is where the
newtype belongs. Without this, `PostRow` alone costs 21 meaningless entries and
the allowlist becomes mostly noise.

**D2a is understated.** The fn-return over-bite is ~24 sites, not the one or two
named. The `smtp.rs` and `test_support.rs` ones are `SiteConfigStorage::get`,
not row reads at all; the `feed_events` and `ColumnInfo` ones are the
struct-literal sites T9/T10 remove. So the residue after T7–T10 is small, but
the ordering is now load-bearing rather than tidy: **T12 cannot be sized until
T9 and T10 have landed.**

**Confirms the #746 dependency is load-bearing**, not bookkeeping: `PostFormat`
and `MediaSource` are flagged today purely because their bridge comes from a
macro invocation rather than a derive.

---

## Task 12 — Gate: the approve-set, `APPROVED_FOREIGN`, and the allowlist

**Files:** `xtask/src/steps/sqlx_newtype_decode_check.rs`.

The rule and its allowlist land together — neither is verifiable without the
other.

1. **Signature.** `problems()` is pure over its inputs today, which is what
   makes the tests synthetic and fast. Keep that:
   `problems(scanned: &[(String, String)], approved: &ApproveSet)`. `run()`
   builds the real set from disk; tests pass a synthetic one.
2. **Declaration scan.** Collect every type under the declaration roots
   (`common/src`, `storage/src`) whose declaration carries a bridge-emitting
   derive. Match on the derive path's **last segment** — existing derives are
   written bare (`use macros::StrNewtype;`) while #746 spells the new one
   `macros::TextEnum`; both occur.
3. **`is_approved(ty)`** replaces `is_i64_family`: walk to leaves, require every
   leaf approved. Recurse `Path` generics, `Tuple`, `Reference`, `Paren`,
   `Group`, **`Slice`, `Array`** (the last two are today's gap).
4. **`APPROVED_FOREIGN`** — `&[(&str, &str)]` of (ident, reason), from T11's
   inventory.
5. **`ALLOWLIST`** — one entry per remaining site. Reasons that must be
   specific:
   - `site_config.rs`/`user_config.rs` values → name **#687**, and state the
     entry survives it because only the key half gets a type;
   - `subscriptions.rs` `subscriber_ref` → name T1's `SubscriberRef` issue;
   - `helpers.rs` `SessionRow` position 4 → `deliberate-lossy`, the
     `SessionLabel` decision (spec A12 — #728 asks for this verdict by name);
   - `UserRecordParts` and any non-query tuple alias, plus `test_support.rs`'s
     `HashMap::get` → `not-a-decode-target`.
6. Rewrite the failure message for the new rule (an unapproved leaf, not "the
   `i64` family"), keeping the "this gate reads no SQL — that judgement is
   yours" framing.

**Tests:** each of the four derive families approves; an undeclared type fails;
`String`, `bool`, `i64`, `Uuid`, `NaiveDate` each fail with no special-casing;
`&[u8]` and `[u8; 32]` are reached; `Vec<(String, Option<PostId>)>` fails on its
`String` leaf while `Vec<(Slug, Option<PostId>)>` passes.

**Run:**
`cargo nextest run --manifest-path xtask/Cargo.toml sqlx_newtype_decode` → PASS,
then `devtool run -- cargo xtask check` → green. **Commit:**
`feat(xtask): sqlx-newtype-decode polices every readable decode (#728)`

---

## Task 13 — Module doc + ADR-0085 amendment

**Files:** `xtask/src/steps/sqlx_newtype_decode_check.rs` (module doc),
`docs/adr/0085-static-type-safety-gates-enumerate.md`.

**Module doc** must state: the approve-set rule and where the set comes from;
the fail-closed asymmetry that licenses it; that approval means _"declared with
a bridge-capable derive"_, **not** _"has a bridge"_
(`#[str_newtype(secret)]`/`no_sqlx` types are approved while emitting none —
harmless, since the compiler rejects a decode into a type with no `Decode` impl,
but a reader will otherwise take approval as proof); the **surviving unreadable
classes** (the unascribed `let` at `postgres/backup.rs:177` and
`sqlite/backup.rs:160`, both genuine `serde_json` map gets; argument/statement
position; decode-typed-only-by-later-use); and the residual adjacent same-typed
column class, pointing at T1's issue.

Delete the paragraph claiming struct-literal field position is policed by the
destination struct's declaration.

**ADR-0085** — per `jaunder-adr`; it is `proposed`, so amend in place: new
Decision subsection (approve-set, fail-closed asymmetry, stated residual);
rewritten `sqlx-newtype-decode` Conformance paragraph (it currently says "the
`i64` family" and "ten allowlist entries"); the unreadable classes re-stated.

`prettier -w` the ADR before staging.

---

## Task 14 — Record the four revert-proofs

For each: revert the one line, `devtool run -- cargo xtask check`, capture the
message, restore.

| Revert                                                             | Expected                                               |
| ------------------------------------------------------------------ | ------------------------------------------------------ |
| T3: retype the per-row `try_get` to `::<String, _>`                | gate names `posts.rs`, unapproved `String` leaf        |
| T8/T9: retype `FeedEventRecord.status` to `String`                 | gate names the declared target                         |
| T7: retype the `rows` `let` to `Vec<(String, Option<AudienceId>)>` | gate names `posts.rs`                                  |
| T10: drop one `ColumnInfo` turbofish                               | gate names field position, says to write the type down |

Run this **before finalising T12's commit** and record the four observed
messages in that commit's message — durable in the repo, unlike a PR body. A
revert that does **not** fail is a defect in the gate: stop and fix.

---

## Self-review

Criterion → task: A1/A1b→T12 · A1a→T5 · A2→T10 · A3→T12 · A4→**T2+T3** ·
A5→**T8+T9** · A6→T9 · A7→T7 · A8→T4 · A9→T14 · A10→T13 · A11→T1 · A12→T12 ·
A13→**cross-cutting** (every commit satisfies the coverage gate; T2's `expect`,
T7/T8's new error paths and T4/T5/T10/T12's gate code each bear on it).

Ordering constraints, all load-bearing: T2 before T3 (accessor) · T9 before T10
(field-position rule would bite the mappers T9 deletes) · T11 before T12
(sizing) · T14 before T12's commit is finalised (its output goes in that
message) · T6 before T7, T8, T9, T12, T13.
