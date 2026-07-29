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
6. Retire the 114 `.bind(i64::from(x))` conversions _(dispatch)_
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
- **Task 7 removes a possible visibility hole.** `subscriber_ref` is
  `TEXT NOT NULL` with no non-empty CHECK, so `subscriber_ref = ''` is
  schema-legal and today's anonymous bind (`""`) would match it. Task 7 traces
  the insert path; if such a row is reachable, this is a visibility **bug** and
  the issue should be re-labelled accordingly.
- **Task 8 must come after task 6** — adding the gate first makes
  `cargo xtask check` fail on the 114 un-swept sites.
- Bind-site type inference may need an occasional turbofish once `i64::from(…)`
  is gone; that is the correct fix, **not** reverting to `i64::from`.

**For agentic workers.** Drive with **`jaunder-iterate`**; delegate task 6 via
**`jaunder-dispatch`**. Tick checkboxes in this file in real time.

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
- [ ] Verify the existing `ByteSize` decode-rejection tests still pass
      **unchanged** — `storage/src/media.rs:453`
      (`find_by_hash_surfaces_a_column_decode_error_for_a_negative_size`) and
      `:484`
      (`get_user_upload_usage_surfaces_a_decode_error_for_a_negative_sum`). They
      are the behavioural contract for this task.
- [ ] `cargo xtask check` → green; commit

**Done when:** `ByteSize` decodes through the bridge and out-of-range columns
still error.

## Task 4 — Type `PostRow` and `build_session_record`

**Files:** `storage/src/helpers.rs`.

- [ ] `PostRow.post_id: i64` → `PostId`, `.user_id: i64` → `UserId` (`:255-256`)
- [ ] `build_session_record(user_id: i64)` → `UserId` (`:79`); drop the now-dead
      conversion at its call site
- [ ] Update `PostRow`'s doc comment (`:242-252`) so its stated exceptions are
      exactly `rendered_html` (#502) and `tags`
- [ ] `cargo xtask check` → green; commit

## Task 5 — Type the tuple row aliases and inline tuples

**Files:** `storage/src/helpers.rs`, `audiences.rs`, `email.rs`, `password.rs`,
`posts.rs`, `subscriptions.rs`, and the per-backend dialect files
(`{postgres,sqlite}/{mod,media,posts}.rs`).

Each site below has a bare position that is immediately re-wrapped by hand;
typing it deletes the re-wrap. Task 1 found 18 further inline-tuple sites beyond
the five aliases — work the spec's two residue tables as the checklist, and
split this into two commits (aliases, then inline tuples) to keep the diff
reviewable.

- [ ] `UserRecordParts:0` (`:31`) → `UserId`; delete `UserId::from(user_id)` in
      `build_user_record` (`:61`)
- [ ] `UserRow:0` (`:187`) → `UserId`
- [ ] `SessionRow:1` (`:203`) → `UserId`
- [ ] `InviteRow:4` (`:229`) → `Option<UserId>`; delete the re-wrap in
      `invite_record_from_row`
- [ ] `MediaRow:0` (`:275`) → `UserId` and `MediaRow:5` → `ByteSize`; delete the
      `ByteSize::try_from` re-wrap in `media_record_from_row` — the bridge now
      does it
- [ ] Leave the documented rejects: `SessionRow:3` (`String`, repaired via
      `SessionLabel::from_lossy`, `:214-218`), `MediaRow:6` (`source_url`,
      #675), `UserRecordParts:7,8` / `UserRow:7,8` (`bool`)
- [ ] `cargo xtask check` → green; commit (aliases)
- [ ] Type the 18 inline-tuple sites from the spec's second residue table;
      delete each accompanying `XId::from(id)` re-wrap
- [ ] `posts.rs:1323` — type both `post_id`/`tag_id` positions; this closes a
      live transposition hazard, so add a note to the fn's doc comment
- [ ] Leave the inline-tuple rejects: `subscriptions.rs:212` (existence flag),
      `subscriptions.rs:226` position 2 (`subscriber_ref`, polymorphic per
      ADR-0020), and the `site_config`/`user_config` key/value strings (#687)
- [ ] `cargo xtask check` → green; commit (inline tuples)

## Task 6 — Retire the 114 bind conversions _(dispatch)_

**Files:** `storage/src/**` (`audiences.rs`, `posts.rs`, `subscriptions.rs`, the
per-backend dialect files, …).

Mechanical and wide — delegate via **`jaunder-dispatch`** to keep the file bulk
out of the driving context. The brief must restate: no `ctx_*` MCP calls (they
hang subagents); worktree absolute paths; `cargo xtask check`, never bare
`nextest`; no `Co-Authored-By`.

- [ ] Rewrite every `.bind(i64::from(x))` → `.bind(x)` across `storage/src` (114
      sites)
- [ ] Where inference then fails, add an explicit type annotation — **do not**
      revert to `i64::from`; report any site where that is not possible
- [ ] Confirm zero remaining matches: `rg -c 'bind\(i64::from\(' storage/src` →
      no hits
- [ ] `cargo xtask check` → green; commit

## Task 7 — `ResolutionBinds`: delete the sentinels

**Files:** `storage/src/posts.rs` (`:1694-1790`), plus dual-backend tests.

Per spec §4 — the fragment has no `NOT`, and `EXISTS` yields FALSE rather than
NULL, so a NULL bind is exactly equivalent to today's sentinels.

- [ ] `ResolutionBinds` → `author_id: Option<UserId>`,
      `channel: Option<ChannelId>`, `subref: Option<String>`
- [ ] `resolution_where`: `ViewerIdentity::Anonymous` → `(None, None, None)`;
      the `Channel` arm uses `subscriber_ref.parse::<UserId>().ok()` in place of
      `parse::<i64>().unwrap_or(-1)` (`:1739`)
- [ ] `bind_onto` binds the `Option` values directly (NULL for `None`)
- [ ] Rewrite the doc comments at `:1699-1707` and `:1714-1717`, which describe
      the sentinel scheme
- [ ] **Trace the insert path** for `subscriptions.subscriber_ref` and record
      whether an empty value is reachable. If it is, note it on #686 — this
      becomes a visibility bug
- [ ] Dual-backend test: an anonymous viewer sees exactly the public posts
      (behaviour unchanged)
- [ ] Dual-backend regression test: seed a subscription with
      `subscriber_ref = ''` (raw SQL, as `media.rs:461` does for tampering
      cases) and assert an anonymous viewer cannot see its posts — this fails
      before the change and passes after
- [ ] `cargo xtask check` → green; commit

## Task 8 — Extend the bind gate

**Files:** `xtask/src/steps/sqlx_newtype_bind_check.rs`.

**Runs after task 6** — otherwise `cargo xtask check` fails on the un-swept
sites.

- [ ] Extend `strips_newtype_in_bind` (`:65`) to also match `i64::from(` in the
      region after the first `.bind(`
- [ ] Unit test `i64_from_bind_is_flagged`, alongside the existing
      `as_ref_strip_is_flagged` (`:175`) and `deref_binds_are_flagged` (`:182`)
      — RED first
- [ ] Update the module doc comment and the failure-detail recovery text to name
      the new case
- [ ] Prove it bites: revert one hunk from task 6, confirm the gate fails,
      restore
- [ ] `cargo nextest run --manifest-path xtask/Cargo.toml`; `cargo xtask check`
      → green; commit

## Task 9 — Amend ADR-0071

**Files:** `docs/adr/0071-sqlx-string-newtype-bridge.md`, `docs/README.md` if
the title changes.

ADR-0071 is `accepted` and scoped to _string_ newtypes; the bridge now covers
all three families. Amend in place (the repo convention, as #400 did for
ADR-0063).

- [ ] Broaden Context/Decision to cover `IdNewtype` (infallible `Decode`) and
      `NumNewtype` (bound-checking `Decode` via `TryFrom<inner>`), and record
      the no-opt-out choice
- [ ] Note the `NumNewtype` behavioural consequence: an out-of-range column is
      now a decode error
- [ ] If the title changes, update `docs/README.md`'s table and run
      `prettier -w docs/README.md`
- [ ] Cross-check ADR-0063 §3 ("the trailer is generated") for wording that
      still implies only string newtypes get the bridge; update in the same
      commit
- [ ] `cargo xtask check` → green; commit

---

## Self-review

- Tasks 2–3 are independent; 4–5 need 2 (and 5 needs 3 for `ByteSize`); 6 needs
  2; 7 needs 2; 8 needs 6; 9 is last.
- Task 1 is deliberately first — its output can add sites to tasks 5 and 6.
- No task changes a wire or serialized shape; the serde bridges are untouched.
- Every task ends at a green `cargo xtask check` and a commit, so the branch is
  bisectable.
