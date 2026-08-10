# Issue #891 — `PgHasArrayType` for the newtype bridge: Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating an individual task to a subagent via `jaunder-dispatch` when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-08-10-issue-891-pg-array-newtype.md` — the
_what_ and _why_, including the opt-in table and the trivial-bound correction. This
plan is the _how_ and does not restate it.

**Goal:** Let a slice of an ADR-0071 newtype bind as a Postgres array, so typed call
sites stop stripping to raw `Vec<i64>`.

**Architecture:** A fourth impl in `macros/src/sqlx_bridge.rs`'s `bridge()`, emitted
only when the caller sets a new `pg_array` flag on `BridgeSpec`. Proof is the deletion
of `raw_ids` and its five call sites.

**Tech Stack:** Rust proc macros (`syn`/`quote`), sqlx 0.8.6, rstest / rstest_reuse,
cargo-nextest, `cargo xtask` gate.

## Review header

**Scope — in:**

- `macros/src/sqlx_bridge.rs` — the `pg_array` field, the impl, unit tests, module doc.
- Seven `BridgeSpec` construction sites across five macro files.
- `storage/src/postgres/feed_events.rs` — delete `raw_ids`, bind typed slices.
- `storage/src/postgres/mod.rs` — one `postgres_only` test for the string case.

**Scope — out:** #716's scalar site; #876's reconcile; any `SqliteHasArrayType`.

**Separable concerns:** none found.

**Tasks:**

1. Emit the impl behind `pg_array`, set the flags, and **probe the coverage gate**
   before anything is built on it.
2. Delete `raw_ids`; bind `&[FeedEventId]` at its five call sites.
3. Add the Postgres string-newtype array test.

**Key risks / decisions:**

- **The coverage gate is the real unknown** (spec AC9). Two generated fn bodies land
  on every opted-in type across `common`; most never execute. Task 1 probes this
  before Tasks 2–3 exist, so a red gate is discovered in the cheapest place.
- **No `where` clause.** A trivial bound on a concrete impl is discharged at the
  definition — see the spec's correction log. The opt-in flag replaces it.
- **Seven sites, not six.** The spec says six; the real count is seven, plus two
  `BridgeSpec` literals inside `sqlx_bridge.rs`'s own `tests` module which also need
  the field.

## Global Constraints

- `macros/` never depends on sqlx; everything it emits is inside
  `#[cfg(feature = "sqlx")]`.
- Postgres-only tests use the `postgres_only` rstest template
  (`storage/src/test_support.rs:440`), not `backends` — a bare `#[tokio::test]` that
  should be parameterised fails the `test-backend-pattern` guard.
- No `Co-Authored-By` trailer.
- Run `cargo xtask check` before committing; the pre-commit hook runs it too.

---

### Task 1: Emit the impl behind an opt-in flag, and probe coverage

**Files:**

- Modify: `macros/src/sqlx_bridge.rs` — `BridgeSpec` (line 33-40), `bridge()`
  (43-104), module doc (1-23), `bridge()`'s doc (42), `tests` module (106-187).
- Modify: `macros/src/id_newtype.rs:102`, `macros/src/str_newtype.rs:338` and `:355`,
  `macros/src/text_enum.rs:221` — set `pg_array: true`.
- Modify: `macros/src/num_newtype.rs:110`,
  `macros/src/sqlx_bridge_derive.rs:38` and `:70` — set `pg_array: false`.

**Interfaces:**

- Produces: `BridgeSpec.pg_array: bool`; a `PgHasArrayType` impl on opted-in newtypes.
- Consumes: sqlx 0.8.6's `PgHasArrayType` (`array_type_info`, `array_compatible`).

---

- [ ] **Step 1: Add the field and the impl**

In `macros/src/sqlx_bridge.rs`, add to `BridgeSpec`:

```rust
    /// Emit `PgHasArrayType`, so a slice of this newtype binds as a Postgres array.
    ///
    /// Opt-in per caller rather than universal, and **deliberately not a `where`
    /// clause**: the impl is concrete, so `where #type_inner: PgHasArrayType` would be
    /// a trivial bound that rustc discharges at the definition (`E0277`) rather than
    /// deferring to use sites. sqlx implements `PgHasArrayType` for `i32`, `i64` and
    /// `String` only — so `NumNewtype` (whose inners include `u32`/`usize`) must stay
    /// off or `common` stops compiling.
    pub(crate) pg_array: bool,
```

Destructure it in `bridge()` and emit, inside the existing `const _: () = { … }`:

```rust
            #pg_array_impl
```

built as:

```rust
    let pg_array_impl = if *pg_array {
        quote! {
            #[automatically_derived]
            impl ::sqlx::postgres::PgHasArrayType for #name {
                fn array_type_info() -> ::sqlx::postgres::PgTypeInfo {
                    <#type_inner as ::sqlx::postgres::PgHasArrayType>::array_type_info()
                }
                fn array_compatible(ty: &::sqlx::postgres::PgTypeInfo) -> bool {
                    <#type_inner as ::sqlx::postgres::PgHasArrayType>::array_compatible(ty)
                }
            }
        }
    } else {
        TokenStream::new()
    };
```

- [ ] **Step 2: Set the flag at all seven sites**

`pg_array: true` — `id_newtype.rs:102` (inner `i64`), `str_newtype.rs:338` and `:355`
(both `String`), `text_enum.rs:221` (`String`).

`pg_array: false` — `num_newtype.rs:110`, `sqlx_bridge_derive.rs:38` and `:70`.

Add a one-line comment at `num_newtype.rs`'s site naming the reason (`u32`/`usize`
inners have no `PgHasArrayType`), since that is the non-obvious one.

Also add `pg_array` to the two `BridgeSpec` literals in the `tests` module —
`spec_for` (`:123`) with `false`, and the inline one at `:167`.

- [ ] **Step 3: Write the macro unit tests**

Add to `macros/src/sqlx_bridge.rs`'s `tests` module:

```rust
    #[test]
    fn pg_array_impl_delegates_to_type_inner_when_enabled() {
        let n = format_ident!("X");
        let out = norm(&bridge(&BridgeSpec {
            pg_array: true,
            ..spec_for(&n)
        }));
        assert!(out.contains("impl::sqlx::postgres::PgHasArrayTypeforX"));
        assert!(out.contains(
            "<::std::string::Stringas::sqlx::postgres::PgHasArrayType>::array_type_info()"
        ));
        assert!(out.contains(
            "<::std::string::Stringas::sqlx::postgres::PgHasArrayType>::array_compatible(ty)"
        ));
    }

    #[test]
    fn pg_array_impl_is_absent_when_disabled() {
        let n = format_ident!("X");
        let out = norm(&bridge(&spec_for(&n)));
        assert!(
            !out.contains("PgHasArrayType"),
            "NumNewtype-style callers must not get the impl: their inner may be u32/usize"
        );
    }
```

`..spec_for(&n)` compiles: `BridgeSpec<'a>`'s lifetime is inferred, and the base is an
owned temporary so its `TokenStream` fields move out cleanly. Use the update syntax —
writing the literal out by hand would let the two drift.

(Step 2 must have added `pg_array` to `spec_for`'s own literal first, or this does not
compile.)

- [ ] **Step 4: Split the `#[automatically_derived]` count test**

`output_is_feature_gated_and_marked_derived` (`:180-186`) asserts exactly 3. Do **not**
just change it to 4 — it must assert 3 for a disabled spec and 4 for an enabled one:

```rust
    #[test]
    fn output_is_feature_gated_and_marked_derived() {
        let n = format_ident!("X");
        let out = norm(&bridge(&spec_for(&n)));
        assert!(out.contains("#[cfg(feature=\"sqlx\")]"));
        assert_eq!(out.matches("#[automatically_derived]").count(), 3);

        // The array impl is the fourth, and only when opted in.
        let with_array = norm(&bridge(&BridgeSpec {
            pg_array: true,
            ..spec_for(&n)
        }));
        assert_eq!(with_array.matches("#[automatically_derived]").count(), 4);
    }
```

- [ ] **Step 5: Run the macro tests**

```bash
devtool run -- cargo nextest run -p macros
```

Expected: **PASS**, including the two new tests and the split count test.

- [ ] **Step 6: Prove the opt-in protects the non-arrayable newtypes (AC3)**

```bash
devtool run -- cargo check -p common --features sqlx
```

Expected: exit 0. This is the criterion: with `pg_array: true` on `num_newtype.rs`
instead, `PageSize` (`common/src/pagination.rs:22`, `u32`), `FeedMinItems` /
`FeedMinDays` (`common/src/feed/settings.rs:12`, `:23`) and the `usize` retention
newtype (`common/src/backup.rs:141`) each fail with `E0277`. Flip the flag once
locally to watch it fail, then flip it back — the cheapest proof the opt-in is
load-bearing rather than decorative.

`common` is not the only crate that expands these derives: `host/src/invite.rs:26`
derives `StrNewtype` with `host`'s `sqlx` feature on by default
(`host/Cargo.toml:31`), so it silently acquires the impl too. That one is covered by
the full gate in Step 9, not by this command.

**Note what this experiment does and does not leave behind.** Flipping the flag proves
the protection today, but nothing in the tree re-runs it — a future edit turning
`num_newtype`'s flag on would be caught by CI failing to compile `common`, which is
adequate, but there is no dedicated regression test. That is deliberate (a
compile-fail test harness is not worth standing up for this), and is recorded so the
absence reads as a decision rather than an oversight.

- [ ] **Step 7: PROBE THE COVERAGE GATE (AC9) — before Tasks 2–3**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-891-pg-array-newtype -- cargo xtask check
```

Read the `coverage` step's line. The question: do the two generated fn bodies, now
present on every opted-in newtype across `common`, appear as **uncovered lines**?

- **Green** → proceed to Task 2 unchanged.
- **Red** → stop and report before building Tasks 2–3 on it. Options to weigh then:
  a `cov:ignore` span around the emitted impl (the repo uses `cov:ignore-start/stop`,
  e.g. `storage/src/postgres/feed_events.rs:31`), or narrowing the opt-in further.
  **Do not** widen a coverage exemption without saying so — that is a gate-weakening
  change and needs explicit approval (`jaunder-commit`).

This step exists because it is much cheaper to discover a red coverage gate here than
after two more tasks depend on the design.

- [ ] **Step 8: Update the docs that count the impls**

Three sites count the impls, not two:

- `macros/src/sqlx_bridge.rs`'s module doc (`:1-23`) — "The three impls are pure
  delegation", plus the per-caller table.
- `bridge()`'s own doc (`:42`) — "The three sqlx bridge impls".
- `macros/src/lib.rs:873` — the `has_sqlx_bridge` helper's doc, same phrase.

All become four, and the table gains a `pg_array` column recording which callers opt
in and why `NumNewtype` cannot.

- [ ] **Step 9: Gate and commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-891-pg-array-newtype -- cargo xtask check
```

Expected: exit 0. Then:

```bash
git add macros/src/sqlx_bridge.rs macros/src/id_newtype.rs macros/src/str_newtype.rs macros/src/text_enum.rs macros/src/num_newtype.rs macros/src/sqlx_bridge_derive.rs
git commit -m "feat(macros): emit PgHasArrayType for opted-in newtypes (#891)"
```

---

### Task 2: Delete `raw_ids` and bind typed slices

**Files:**

- Modify: `storage/src/postgres/feed_events.rs` — delete the doc block (`:38-46`) and
  fn (`:47-49`); update call sites `:27`, `:91`, `:102`, `:117`, `:137`.

**Interfaces:**

- Consumes: `PgHasArrayType for FeedEventId`, from Task 1.

---

- [ ] **Step 1: Delete the helper and its doc block**

Remove lines 38-49 in full — the doc block narrates a defect that no longer exists, so
leaving it would be worse than leaving the function.

- [ ] **Step 2: Bind typed slices at all five call sites**

`:27` becomes:

```rust
    if let Err(e) = sqlx::query("DELETE FROM feed_events WHERE id = ANY($1)")
        .bind(ids)
```

The four `let raw = raw_ids(ids);` sites (`:91`, `:102`, `:117`, `:137`) drop the
local and bind `ids` directly:

```rust
        let now = Utc::now();
        sqlx::query("UPDATE feed_events SET regenerated_at = $1 WHERE id = ANY($2)")
            .bind(now)
            .bind(ids)
```

If `.bind(ids)` needs a reborrow (`ids` is already `&[FeedEventId]`), follow the
compiler — sqlx's `Encode for &[T]` is what makes this work.

- [ ] **Step 3: Run the feed-events tests**

```bash
devtool run -- devtool pg run -- cargo nextest run -p storage feed_events
```

Expected: **PASS**. These already exercise all five statements on Postgres, so they
are the regression cover for the bind change.

- [ ] **Step 4: Confirm the strip is gone**

```bash
rg -n 'raw_ids|i64::from' storage/src/postgres/feed_events.rs
```

Expected: no output.

- [ ] **Step 5: Gate and commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-891-pg-array-newtype -- cargo xtask check
```

Expected: exit 0, with `sqlx-newtype-bind` green and **no** allowlist edit.

```bash
git add storage/src/postgres/feed_events.rs
git commit -m "refactor(storage): bind FeedEventId slices without stripping to i64 (#891)"
```

---

### Task 3: Prove the string-newtype case

**Files:**

- Modify: `storage/src/postgres/mod.rs` — add to the existing `#[cfg(test)] mod tests`
  (starts `:321`).

**Interfaces:**

- Consumes: `PgHasArrayType for Tag`, from Task 1.

---

- [ ] **Step 1: Write the test**

`feed_events` covers the `i64` case; nothing covers `String`, and #876 needs it. Add:

```rust
    // reason: Postgres-only by nature, not by omission — `SQLite` has no array type,
    // so there is no parity behaviour to match. Per ADR-0053 a dialect feature with no
    // generic home lives with its dialect code. #891: a slice of a `StrNewtype` binds
    // as a `TEXT[]`, so a typed call site needs no strip. `feed_events` proves the
    // `i64` case; this is the `String` one, which #876's single-statement tag
    // reconcile depends on.
    #[apply(postgres_only)]
    #[tokio::test]
    async fn str_newtype_slices_bind_as_a_postgres_array(#[case] backend: Backend) {
        let env = backend.setup().await;
        let CloseablePool::Postgres(pool) = env.base.pool() else {
            unreachable!("postgres_only yields a Postgres pool")
        };

        for slug in ["alpha", "beta", "gamma"] {
            sqlx::query("INSERT INTO tags (tag_slug) VALUES ($1)")
                .bind(slug)
                .execute(pool)
                .await
                .expect("seed tag");
        }

        // The point of the test: `&[Tag]` binds directly, with no `Vec<String>` strip.
        let wanted = vec![parse_tag("alpha"), parse_tag("gamma")];

        let found = sqlx::query_scalar::<_, Tag>(
            "SELECT tag_slug FROM tags WHERE tag_slug = ANY($1) ORDER BY tag_slug",
        )
        .bind(&wanted)
        .fetch_all(pool)
        .await
        .expect("array bind");

        assert_eq!(found, wanted);
    }
```

**The `// reason:` line is load-bearing, not decorative.** `test-backend-pattern`
(`xtask/src/steps/test_pattern_check.rs:208-247`) requires a line that *trims to*
`// reason:` inside an `#[apply(postgres_only)]` cluster; a `///` doc comment does
**not** satisfy it and the gate fails. Every existing `postgres_only` test uses this
form — see `storage/src/postgres/backup.rs:365` and `:389`.

`parse_tag` is `common::test_support::parse_tag` — the repo's convention for building
a `Tag` in storage tests (used at `storage/src/posts.rs:3082`), rather than
`.parse::<Tag>()`. The `query_scalar::<_, Tag>` turbofish matches the existing style
at `storage/src/postgres/mod.rs:296`.

**This adds a new test flavour to that module, not just a test.** `mod tests` at
`storage/src/postgres/mod.rs:321` is currently pure-sync — `use super::*` plus
`common::test_support::with_env`, with no rstest, no tokio, no `test_support`. So the
imports to add are: `rstest::*`, `rstest_reuse::*`, `common::tag::Tag`,
`common::test_support::parse_tag`, and
`crate::test_support::{Backend, CloseablePool, postgres_only}`. Follow
`storage/src/postgres/backup.rs:361-363` for the shape.

- [ ] **Step 2: Run it**

```bash
devtool run -- devtool pg run -- cargo nextest run -p storage str_newtype_slices_bind
```

Expected: **PASS**, one case (`postgres`).

To confirm it is actually testing the new capability, temporarily set
`pg_array: false` on `str_newtype.rs`'s two sites and re-run: expected **compile
error** on `.bind(&wanted)`. Revert.

- [ ] **Step 3: Gate and commit**

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-891-pg-array-newtype -- cargo xtask check
```

Expected: exit 0.

```bash
git add storage/src/postgres/mod.rs
git commit -m "test(storage): pin that StrNewtype slices bind as Postgres arrays (#891)"
```

- [ ] **Step 4: Run the full local gate (AC10)**

`check` is the iterate-time gate; AC10 names `validate`. Run it here so the criterion
has an owner rather than being deferred to ship in prose:

```bash
devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-891-pg-array-newtype -- cargo xtask validate --no-e2e
```

Expected: exit 0. `--no-e2e` is right: the diff is `macros/` and `storage/` only, with
no web, server-fn, or browser surface. Use Bash background mode; if it exceeds the
harness's 10-minute cap, say so rather than splitting it into pieces — CI runs the
full matrix on the PR.

---

## Self-review

**Spec coverage:**

| Spec AC | Task                                                              |
| ------- | ----------------------------------------------------------------- |
| AC1     | T1 S1 (field + impl, no `where`), T1 S3 (both unit tests)         |
| AC2     | T1 S2 (seven sites — note: spec says six, real count is seven)    |
| AC3     | T1 S6 (`cargo check -p common --features sqlx`)                   |
| AC4     | T1 S4 (count test split, not bumped)                              |
| AC5     | T2 (all five call sites), T2 S4 (grep confirms)                   |
| AC6     | T3 (postgres_only, with the parity rationale in the doc comment)  |
| AC7     | T2 S5 (`sqlx-newtype-bind` green, no allowlist edit)              |
| AC8     | T1 S8 (three doc sites), T2 S1 (`raw_ids` doc block)              |
| AC9     | T1 S7 (the probe, deliberately before Tasks 2–3)                  |
| AC10    | **T3 S4** (`validate --no-e2e`); per-task `check` throughout      |

**Placeholders:** none — every step carries real Rust or a real command. Three steps
carry a "follow the compiler" fallback, which is a verifiable instruction rather than
a hole.

**Type consistency:** `pg_array: bool` is spelled identically in the field
definition, all seven production sites, both test literals, and the two new unit
tests. `PgHasArrayType`'s two methods match sqlx 0.8.6 exactly
(`array_type_info() -> PgTypeInfo`, `array_compatible(&PgTypeInfo) -> bool`), verified
in the spec's "Verified before writing".
