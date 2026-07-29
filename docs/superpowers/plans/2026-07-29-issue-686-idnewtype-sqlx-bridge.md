# Plan — #686: sqlx bridges for `IdNewtype` / `NumNewtype`

Spec:
[2026-07-29-issue-686-idnewtype-sqlx-bridge.md](../specs/2026-07-29-issue-686-idnewtype-sqlx-bridge.md)
· Issue: [#686](https://github.com/jaunder-org/jaunder/issues/686) · Branch:
`worktree-issue-686-idnewtype-sqlx-bridge` · Fork point: `wt-base-issue-686`
(`da17c14a`)

## Review header

**Goal.** Give `IdNewtype` and `NumNewtype` the sqlx bridge `StrNewtype` already
has, then delete the primitive residue it was forcing: ~29 untyped declaration
positions (task 1 raised this from 9), 114 `i64::from(…)` bind conversions, and
the `ResolutionBinds` sentinels.

**Scope — in:** `macros/src/{id,num}_newtype.rs`;
`storage/src/{helpers,posts,media}.rs` and the per-backend dialect files;
`xtask/src/steps/sqlx_newtype_bind_check.rs`; ADR-0071. **Scope — out:** #697's
adoption gate; `PostRow.rendered_html` (#502) and `.tags`;
`MediaRow:6 source_url` (#675); the storage fetch-limit question (#696).

**Tasks.**

1. ✅ Audit the 31 inline `query_as::<_, ( … )>` tuples; finalise the residue
   list
2. `IdNewtype` sqlx bridge (infallible `Decode`)
3. `NumNewtype` sqlx bridge (bound-checking `Decode` via `TryFrom<inner>`)
4. Type `PostRow` + `build_session_record`
5. Type the five tuple row aliases **and 18 inline-tuple sites**; delete the
   hand re-wraps
6. Retire the `.bind(i64::from(x))` conversions — 98 of 114; see task 6
7. `ResolutionBinds`: delete the `-1`/`""` sentinels, bind NULL
8. Extend `sqlx_newtype_bind_check` to flag `i64::from(` in a `.bind(`
9. Amend ADR-0071 to cover all three newtype families

**Key risks / decisions.**

- **Task 3 is a behaviour change**, not a refactor: a bound-checking `Decode`
  turns a silently-accepted out-of-range column into a decode error.
  `media.rs:453` and `:484` already pin exactly that behaviour for `ByteSize`
  (negative `size_bytes` → column-decode error) via a hand-written
  `ByteSize::try_from`, so those tests are the safety net — they must keep
  passing with the hand conversion removed. Confirm per adopting type before
  extending further.
- ~~**Task 7 removes a possible visibility hole.**~~ **Refuted — see task 7.**
  Admission needs both sentinels, and the other one (`channel_id = -1`) is
  FK-unstorable, so the empty `subscriber_ref` never mattered. Proved by running
  the new regression test against the old sentinels. No re-labelling.
- **Task 8 must come after task 6** — adding the gate first makes
  `cargo xtask check` fail on the 114 un-swept sites.
- Bind-site type inference may need an occasional turbofish once `i64::from(…)`
  is gone; that is the correct fix, **not** reverting to `i64::from`.

**For agentic workers.** Drive with **`jaunder-iterate`**; delegate task 6 via
**`jaunder-dispatch`**. Tick checkboxes in this file in real time.

## Base moved during execution — #445 superseded the `rendered_html` carve-out

`origin/main` advanced twice while this branch was in flight; the second move
brought **#445 (RenderedHtml sanitization, ADR-0079)**, which invalidates a
premise stated below. This plan and the spec both scope `PostRow.rendered_html`
out "per #502", on the grounds that its sqlx bridge was deliberately
**write-only** — a `Decode` would have laundered an untrusted column into
trusted unescaped HTML. #445 moved sanitization onto the type, which removes
that objection: `rendered_html` now has a `Decode` and `PostRow.rendered_html`
is `RenderedHtml`, not `String`.

So the scope-out below is **stale, not wrong**: the column was already typed by
someone else, for a better reason than this issue had. After the rebase the only
`PostRow` column that is not a decoded domain type is `tags` (the JSON
aggregate), and `helpers.rs` plus ADR-0071's Consequences were corrected to say
so. Read every "#502 / stays `String`" mention below as historical.

## Global constraints

- **No `Co-Authored-By` trailer** on any commit.
- Pre-commit runs the full `cargo xtask check` — run it yourself first so the
  hook passes clean (`jaunder-commit`). It auto-fixes formatting, so
  `git status --porcelain` after green.
- Storage tests use the **dual-backend template**:
  `#[apply(backends)] #[tokio::test]` with `#[case] backend: Backend`, binding
  the whole `TestEnv` (ADR-0053). A bare `#[tokio::test]` that should be
  dual-backend fails the `test-backend-pattern` guard.
- Build newtype values in tests via `common::test_support::parse_<name>()` —
  never `.parse().expect()` (`expect_used` is denied).
- `macros` **is** coverage-measured; cover the new bridges' paths.
  Derive-expansion tests use `syn::parse_quote!`.
- `xtask` is excluded from the workspace — test it with
  `cargo nextest run --manifest-path xtask/Cargo.toml`.
- Never bare `nextest` for storage: `cargo xtask check` sets up
  Postgres/seeding.

---

## Task 1 — Audit the inline tuple `query_as` sites

**Files:** none changed (investigation). Update the spec's residue table with
any additions.

`storage/src` has 31 inline `query_as::<_, ( … )>` tuples, which the field-name
audit could not see (tuple positions have no names) — the same blind spot that
hid five of the nine known sites. Enumerate them and record any position that is
a bare primitive but carries an ID or a bounded numeric.

- [x] List every `query_as::<_, ( … )>` in `storage/src` with its tuple element
      types
- [x] For each bare `i64`/`Option<i64>`/`u32`/`usize`, identify the column and
      whether it is an ID, a bounded numeric, or genuinely primitive (a
      `COUNT(*)`, a `bool` flag)
- [x] Append confirmed sites to the spec's residue table; record rejects with a
      reason
- [x] Commit the spec update (the list changed substantially)

**Result:** 31 scanned, 23 with bare positions, **18 sites / 20 positions**
carrying an ID or bounded numeric — see the spec's inline-tuple table. Rejects:
`subscriptions.rs:212` (existence flag), `subscriptions.rs:226` position 2
(`subscriber_ref`, deliberately polymorphic per ADR-0020), and the four
`site_config`/`user_config` key/value strings (#687).

Two consequences for later tasks: the residue is ~29 positions rather than 9, so
**task 5 is roughly three times its original scope**; and `posts.rs:1323` turns
out to be a live transposition hazard (adjacent bare `post_id`/`tag_id`).

**Done when:** the residue list is complete and the remaining tasks know their
full site set. ✅

## Task 2 — `IdNewtype` sqlx bridge

**Files:** `macros/src/id_newtype.rs`, `macros/tests/` (derive-expansion),
`common/src/ids.rs` (round-trip test).

**Interface.** Append to `expand`'s emitted tokens, modelled on
`str_newtype.rs`'s `sqlx_impls_inner` (`:331`). `Decode` is an infallible wrap —
an ID has no invariant beyond "is an integer" (ADR-0063 §2).

```rust
#[cfg(feature = "sqlx")]
const _: () = {
    #[automatically_derived]
    impl<DB: ::sqlx::Database> ::sqlx::Type<DB> for #name
    where
        i64: ::sqlx::Type<DB>,
    {
        fn type_info() -> <DB as ::sqlx::Database>::TypeInfo {
            <i64 as ::sqlx::Type<DB>>::type_info()
        }
        fn compatible(ty: &<DB as ::sqlx::Database>::TypeInfo) -> bool {
            <i64 as ::sqlx::Type<DB>>::compatible(ty)
        }
    }

    #[automatically_derived]
    impl<'q, DB: ::sqlx::Database> ::sqlx::Encode<'q, DB> for #name
    where
        i64: ::sqlx::Encode<'q, DB>,
    {
        fn encode_by_ref(
            &self,
            buf: &mut <DB as ::sqlx::Database>::ArgumentBuffer<'q>,
        ) -> ::core::result::Result<::sqlx::encode::IsNull, ::sqlx::error::BoxDynError> {
            <i64 as ::sqlx::Encode<'q, DB>>::encode_by_ref(&self.0, buf)
        }
        fn size_hint(&self) -> ::core::primitive::usize {
            <i64 as ::sqlx::Encode<'q, DB>>::size_hint(&self.0)
        }
    }

    #[automatically_derived]
    impl<'r, DB: ::sqlx::Database> ::sqlx::Decode<'r, DB> for #name
    where
        i64: ::sqlx::Decode<'r, DB>,
    {
        fn decode(
            value: <DB as ::sqlx::Database>::ValueRef<'r>,
        ) -> ::core::result::Result<Self, ::sqlx::error::BoxDynError> {
            ::core::result::Result::Ok(#name(<i64 as ::sqlx::Decode<'r, DB>>::decode(value)?))
        }
    }
};
```

No opt-out attribute (spec §1) — `id_newtype.rs` parses no options today and
gains none.

- [x] Add a derive-expansion test asserting the emitted tokens contain the three
      impls (RED — `cargo nextest run -p macros`, 4 new tests failed as
      expected)
- [x] Emit the bridge from `expand`; update the module doc comment to list it
      (GREEN — 56/56 pass)
- [~] **Dropped as redundant churn.** A bespoke round-trip test was planned
  here, but tasks 4–6 type the _real_ sites, so the existing dual-backend suite
  proves `Decode` (typed row structs) and `Encode` (binds without `i64::from`)
  on live queries. A dedicated test would duplicate that with weaker coverage
  and be deleted later. Instead, the bridges were verified to compile against
  real sqlx via
  `cargo clippy -p common --features sqlx --all-targets -- -D warnings` (exit
  0), which the default gate does **not** do — the bridge is behind
  `#[cfg(feature = "sqlx")]` and is otherwise never type-checked.
- [x] `cargo xtask check --no-test` → green; committed with task 3

**Done when:** a `UserId` binds and decodes with no manual conversion on both
backends. ✅ (bridge landed; exercised on real sites by tasks 4–6)

## Task 3 — `NumNewtype` sqlx bridge

**Files:** `macros/src/num_newtype.rs`, `macros/tests/`, `common/src/media.rs`
(`ByteSize`).

**Interface.** Same `Type`/`Encode` shape as task 2, parameterised on
`opts.inner` rather than `i64`. `Decode` **re-runs the bound** by delegating to
the generated `TryFrom<#inner>` (`try_from_inner_impl`, `:174`); its error type
already implements `std::error::Error` (`:105`), so it boxes straight into
`BoxDynError`.

```rust
fn decode(
    value: <DB as ::sqlx::Database>::ValueRef<'r>,
) -> ::core::result::Result<Self, ::sqlx::error::BoxDynError> {
    let v = <#inner as ::sqlx::Decode<'r, DB>>::decode(value)?;
    ::core::result::Result::Ok(<#name as ::core::convert::TryFrom<#inner>>::try_from(v)?)
}
```

A `Decode` that skipped the bound would make the column a hole in an invariant
the serde bridge already enforces (spec §1).

- [x] Derive-expansion test for the three impls, plus one asserting `Decode`
      routes through `TryFrom`, and one pinning that the bridge uses the
      declared `inner` type rather than a hardcoded `i64` (RED)
- [x] Emit the bridge via a new `sqlx_impls(name, inner)`; update the module doc
      comment (GREEN)
- [x] Verify the existing `ByteSize` decode-rejection tests still pass —
      `find_by_hash_surfaces_a_column_decode_error_for_a_negative_size` passes
      **unchanged**. The `get_user_upload_usage` twin needed its _assertion_
      updated, in task 5: moving the bound from a hand `ByteSize::try_from` on
      the returned `i64` to the column's `Decode` changes the error from
      `sqlx::Error::Decode` to `sqlx::Error::ColumnDecode`. The contract this
      task cares about — an out-of-range column is rejected, not silently
      accepted — holds, and the two `ByteSize` sites now agree on one variant
      instead of two. Renamed to `…_surfaces_a_column_decode_error_for_a_…`.
- [x] `cargo xtask check` → green; commit

**Done when:** `ByteSize` decodes through the bridge and out-of-range columns
still error. ✅

## Task 4 — Type `PostRow` and `build_session_record`

**Files:** `storage/src/helpers.rs`.

- [x] `PostRow.post_id: i64` → `PostId`, `.user_id: i64` → `UserId`
- [x] `build_session_record(user_id: i64)` → `UserId`; dropped the now-dead
      `UserId::from(user_id)` inside it
- [x] Update `PostRow`'s doc comment so its stated exceptions are exactly
      `rendered_html` (#502) and `tags`
- [x] **Pulled forward from task 5:** `SessionRow:1` → `UserId`.
      `session_record_from_row` feeds `build_session_record` directly, so
      leaving it `i64` would have meant adding a `UserId::from(…)` wrapper
      purely to delete it in the next commit.
- [x] `cargo xtask check --no-test` → green; commit

**Note:** the 21 `query_as::<_, PostRow>` sites needed no changes — `FromRow`
plus the new bridge decode the typed columns transparently.

## Task 5 — Type the tuple row aliases and inline tuples

**Files:** `storage/src/helpers.rs`, `audiences.rs`, `email.rs`, `password.rs`,
`posts.rs`, `subscriptions.rs`, and the per-backend dialect files
(`{postgres,sqlite}/{mod,media,posts}.rs`).

Each site below has a bare position that is immediately re-wrapped by hand;
typing it deletes the re-wrap. Task 1 found 18 further inline-tuple sites beyond
the five aliases — work the spec's two residue tables as the checklist, and
split this into two commits (aliases, then inline tuples) to keep the diff
reviewable.

- [x] `UserRecordParts:0` (`:31`) → `UserId`; delete `UserId::from(user_id)` in
      `build_user_record` (`:61`)
- [x] `UserRow:0` (`:187`) → `UserId`
- [x] `SessionRow:1` (`:203`) → `UserId` _(pulled forward into task 4)_
- [x] `InviteRow:4` (`:229`) → `Option<UserId>`; delete the re-wrap in
      `invite_record_from_row`
- [x] `MediaRow:0` (`:275`) → `UserId` and `MediaRow:5` → `ByteSize`; delete the
      `ByteSize::try_from` re-wrap in `media_record_from_row` — the bridge now
      does it
- [x] Leave the documented rejects: `SessionRow:3` (`String`, repaired via
      `SessionLabel::from_lossy`, `:214-218`), `MediaRow:6` (`source_url`,
      #675), `UserRecordParts:7,8` / `UserRow:7,8` (`bool`)
- [x] `cargo xtask check` → green; commit (aliases) — `8b79b452`
- [x] Type the 18 inline-tuple sites from the spec's second residue table;
      delete each accompanying `XId::from(id)` re-wrap
- [x] `posts.rs:1323` — type both `post_id`/`tag_id` positions; this closes a
      live transposition hazard, so add a note to the fn's doc comment
- [x] Leave the inline-tuple rejects: `subscriptions.rs:212` (existence flag),
      `subscriptions.rs:226` position 2 (`subscriber_ref`, polymorphic per
      ADR-0020), and the `site_config`/`user_config` key/value strings (#687)
- [x] `cargo xtask check` → green; commit (inline tuples)

**The generic-impl where-clause trap, again.** Every generic
`impl<DB> …Storage for …Store<DB>` restates its row tuples as `FromRow` bounds
(`ADR-0019`, "supertrait where-clauses don't propagate"), so each retyped tuple
had to change in **two** places — the `query_as` turbofish and the bound. This
is the same trap `users.rs::authenticate` sprang in task 5a; here it hit
`audiences.rs:154`, `email.rs:101`, `password.rs:78`, `posts.rs:821-822` and
`subscriptions.rs:149-150`. Changing only the turbofish removes the `FromRow`
impl for the new shape, and the error surfaces as unrelated
"`String`/`DateTime`: `Decode` not satisfied" noise on the _other_ columns
rather than at the id. Two bounds also had to **split**: `audiences.rs`'s single
`(i64,)` served both an `AudienceId` and a `SubscriptionId` query, and
`subscriptions.rs` keeps a bare `(i64,)` for the existence flag alongside the
two new id shapes.

**`get_user_upload_usage` went further than a turbofish.** The `MediaDialect`
twin returned `sqlx::Result<i64>` and `MediaStore` re-ran the bound by hand
(`ByteSize::try_from(sum).map_err(sqlx::Error::Decode)`). Typing the tuple
`(ByteSize,)` makes the dialect return `sqlx::Result<ByteSize>` and the wrapper
a straight delegation — see the task 3 note for the error-variant consequence.

## Task 6 — Retire the 114 bind conversions

**Files:** `storage/src/**` (`audiences.rs`, `posts.rs`, `subscriptions.rs`, the
per-backend dialect files, …).

Mechanical and wide. Planned as a **`jaunder-dispatch`** delegation to keep the
file bulk out of the driving context; done instead as one scripted rewrite over
the 17 files, which keeps the bulk out just as effectively and is verified by
the compiler plus the full gate rather than by a subagent's report.

- [x] Rewrite every `.bind(i64::from(x))` → `.bind(x)` across `storage/src` —
      **98 of 114**, see the carve-out below
- [x] Where inference then fails, add an explicit type annotation — **do not**
      revert to `i64::from`. Not needed at any site: every swept bind resolves
      through the newtype's own `Encode` impl, so no turbofish was required.
- [x] Confirm only the carve-out remains:
      `rg -c 'bind\(i64::from\(' storage/src` → 16 (`posts.rs` 12, `media.rs` 4)
- [x] `cargo xtask check` → green; commit

**16 sites are NOT newtype strips and must stay** — `.bind(i64::from(limit))`
(×14) and `.bind(i64::from(offset.value()))` (×2). `limit` is a bare `u32` and
`PageOffset`'s declared `inner` is `u32`; sqlx has no Postgres `Encode` for
unsigned types, so both are genuine `u32 → i64` widenings, not conversions the
bridge can absorb. These are exactly the storage fetch-limit family the spec
scopes out to **#696**.

**This constrains task 8.** A gate that flags `i64::from(` anywhere inside a
`.bind(` fires on all 16. It needs to allow the primitive widening — the
narrowest rule that bites the residue without false positives is to flag
`i64::from(` only when its argument is not a bare `u32`-typed local, which a
syntactic gate cannot see. Reconsider the rule when task 8 is written: either
gate on the newtype-typed spellings, or leave #696 to remove the last 16 first.

## Task 7 — `ResolutionBinds`: delete the sentinels

**Files:** `storage/src/posts.rs` (`:1694-1790`), plus dual-backend tests.

Per spec §4 — the fragment has no `NOT`, and `EXISTS` yields FALSE rather than
NULL, so a NULL bind is exactly equivalent to today's sentinels.

- [x] `ResolutionBinds` → `author_id: Option<UserId>`,
      `channel: Option<ChannelId>`, `subref: Option<String>`
- [x] `resolution_where`: `ViewerIdentity::Anonymous` → `(None, None, None)`;
      the `Channel` arm uses `subscriber_ref.parse::<UserId>().ok()` in place of
      `parse::<i64>().unwrap_or(-1)` (`:1739`)
- [x] `bind_onto` binds the `Option` values directly (NULL for `None`)
- [x] Rewrite the doc comments at `:1699-1707` and `:1714-1717`, which describe
      the sentinel scheme
- [x] **Trace the insert path** for `subscriptions.subscriber_ref` — see below
- [x] Dual-backend test: an anonymous viewer sees exactly the public posts —
      **already covered**, no new test written. `server/tests/storage/mod.rs`'s
      `resolution_matrix` is exactly this: 5 viewers × 6 audience targetings,
      asserted through both `get_post_by_id` and `list_published`, dual-backend.
      A bespoke anonymous-only test would be a strict subset of it.
- [x] Dual-backend regression test:
      `anonymous_is_not_admitted_by_an_empty_subscriber_ref`
- [x] `cargo xtask check` → green; commit

**The visibility hypothesis is refuted — this is not a bug, and #686 does not
need re-labelling.** The plan reasoned that a schema-legal `subscriber_ref = ''`
row would be matched by the anonymous `""` bind. It would not: admission needs
**both** halves of the sentinel pair, and the other half is `channel_id = -1`.
`subscriptions.channel_id` is `INTEGER NOT NULL REFERENCES channels(channel_id)`
and `channels` hands out positive autoincrement keys, so no row can carry `-1`
and the subscribers/named `EXISTS` branches were already dead for `Anonymous`.

Proved rather than argued: with the new test in place, `resolution_where`'s
`Anonymous` arm was temporarily set back to
`(Some(UserId::from(-1)), Some(ChannelId::from(-1)), Some(String::new()))` — the
old sentinels expressed in the new types — and `cargo xtask check` still passed.
The test does **not** bite on the old code, so it pins a property rather than
demonstrating a fix.

The insert-path trace agrees: `SubscriptionStorage::subscribe` has exactly one
caller, `web/src/subscriptions/api.rs:30`, which binds an authenticated
`auth.user_id`, so `''` is unreachable through the application in the first
place.

What the change is worth, then, is not a fix but the removal of two unstated
dependencies: the sentinel scheme was correct only because `-1` is unstorable in
two different tables. NULL is correct because of what NULL means. That is the
same reason the rest of this issue exists.

## Task 8 — Extend the bind gate

**Files:** `xtask/src/steps/sqlx_newtype_bind_check.rs`.

**Runs after task 6** — otherwise `cargo xtask check` fails on the un-swept
sites.

- [x] Extend `strips_newtype_in_bind` (`:65`) to also match `i64::from(` in the
      region after the first `.bind(`
- [x] Unit test `i64_from_bind_is_flagged`, alongside the existing
      `as_ref_strip_is_flagged` and `deref_binds_are_flagged` — plus
      `i64_from_outside_a_bind_is_ignored` (the widening is policed only at a
      bind site) and `allowlisted_primitive_widenings_are_clean`
- [x] Update the module doc comment and the failure-detail recovery text to name
      the new case
- [x] Prove it bites: `.bind(record.user_id)` in `storage/src/media.rs:186` was
      temporarily reverted to `.bind(i64::from(record.user_id))`;
      `cargo xtask check --no-test` failed with
      `[FAIL] sqlx-newtype-bind — storage/src/media.rs:186: …`, then restored
- [x] `cargo nextest run --manifest-path xtask/Cargo.toml` (12/12);
      `cargo xtask check` → green; commit

**Task 6's carve-out is handled by the existing ALLOWLIST, not by weakening the
rule.** The gate already exempts by bind-expression **substring** (reflow-proof,
unlike a line number or an inline marker), so the 16 forced `u32 → i64`
widenings get two entries — `i64::from(limit)` and `i64::from(offset.value())` —
each carrying its reason and pointing at #696, which should delete them along
with the underlying `u32`. The rule itself stays absolute: any other
`i64::from(` in a `.bind(` fails the gate.

## Task 9 — Amend ADR-0071

**Files:** `docs/adr/0071-sqlx-string-newtype-bridge.md`, `docs/README.md` if
the title changes.

ADR-0071 is `accepted` and scoped to _string_ newtypes; the bridge now covers
all three families. Amend in place (the repo convention, as #400 did for
ADR-0063).

- [x] Broaden Context/Decision to cover `IdNewtype` (infallible `Decode`) and
      `NumNewtype` (bound-checking `Decode` via `TryFrom<inner>`), and record
      the no-opt-out choice
- [x] Note the `NumNewtype` behavioural consequence: an out-of-range column is
      now a decode error
- [x] Title → "Transparent sqlx bridge for **domain** newtypes";
      `docs/README.md`'s table cell updated and prettier'd. **Filename left as
      `0071-sqlx-string-newtype-bridge.md`** — renaming would break links from
      two archived docs, and `adr-readme-parity` compares the number/link/status
      mechanically while treating the title cell as hand-curated, so the slug
      never has to track the title.
- [x] Cross-check ADR-0063 for wording that implies only string newtypes get the
      bridge. **Nothing was wrong** — 0063 never mentions sqlx, and every "the
      bridge" in it means the serde bridge. What it had was an _omission_: §2's
      id and numeric-value trailers listed their serde bridge and stopped. Both
      now name the sqlx bridge and point at ADR-0071.
- [x] `cargo xtask check` → green; commit

---

## Self-review

- Tasks 2–3 are independent; 4–5 need 2 (and 5 needs 3 for `ByteSize`); 6 needs
  2; 7 needs 2; 8 needs 6; 9 is last.
- Task 1 is deliberately first — its output can add sites to tasks 5 and 6.
- No task changes a wire or serialized shape; the serde bridges are untouched.
- Every task ends at a green `cargo xtask check` and a commit, so the branch is
  bisectable.
