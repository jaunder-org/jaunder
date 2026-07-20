# Plan — Issue #502: `RenderedHtml` trailer + write-side sqlx bridge

Spec:
[`docs/superpowers/specs/2026-07-20-issue-502-renderedhtml-trailer-sqlx-encode.md`](../specs/2026-07-20-issue-502-renderedhtml-trailer-sqlx-encode.md)
(the "what/why"; this is the "how" — read the spec's Decisions D1–D3 and
AC1–AC10).

## Review header

**Goal.** Give `RenderedHtml` the rest of the StrNewtype read-out trailer
(`From<Self> for String`, `Borrow<str>`, `PartialEq<str>`/`<&str>`) and a
**write-side** sqlx bridge (`Type` + `Encode`, no `Decode`), then convert the
three storage bind sites off the `.as_ref()` workaround and retire the `#502`
markers/allowlist that tracked it.

**Scope.**

- In: `common/src/render.rs` (trailer impls + tests; write-side sqlx bridge).
  Three storage bind sites (`storage/src/posts.rs`, `sqlite/posts.rs`,
  `postgres/posts.rs`). `xtask/src/steps/sqlx_newtype_bind_check.rs` (drop the
  `rendered_html` allowlist entry + its doc/test). `storage/src/helpers.rs`
  (`#502` comment cleanup).
- Out: any `Decode`/read-path change (the column stays `String` → `from_trusted`
  in `build_post_record`); feed serializer signatures; the
  `deserialize_rendered_html` wire door; the `rendered-html-from-trusted` gate
  (unchanged, must stay green).

**Tasks (one line each).**

1. Trailer impls + unit tests on `RenderedHtml` (AC1–AC4).
2. Write-side sqlx bridge — `Type::type_info` +
   `Encode::{encode_by_ref,size_hint}`, no `compatible`, no `Decode` (AC5).
3. Convert the three storage bind sites to `.bind(&input.rendered_html)`; drop
   their `#502` bind comments (AC6).
4. Retire the `rendered_html` entry from the `sqlx-newtype-bind` gate: allowlist
   entry, count/module doc comments, and its unit test (AC7).
5. Clean up remaining `#502` markers in `helpers.rs`; confirm the read path +
   from-trusted gate are untouched (AC8, AC9).
6. Full gate green + bridge-coverage check (AC5b, AC10).

**Key risks / decisions.**

- **No `Decode` — deliberate** (spec D2): a decode would launder untrusted
  columns into trusted unescaped HTML. `compatible()` is omitted
  (decode-path-only default) so the write-only impl has no unreachable line to
  cover.
- **Bridge coverage** (spec AC5b): encode-path lines are covered by existing
  `create_post`→read round-trips; Task 6 verifies and, only if a line is
  unreached, adds a direct storage-side unit test (never a `compatible`
  override).
- **Ordering is load-bearing:** Task 2 (Encode) must precede Task 3 (the typed
  bind won't compile without it); Task 4 (drop allowlist) must follow Task 3
  (else the still-present `.as_ref()` bind is flagged).

**For agentic workers.** Drive with `jaunder-iterate`; delegate a task to a
subagent via `jaunder-dispatch` if useful. Tick checkboxes live.

## Global constraints

- No `Co-Authored-By` trailer on commits. Do not commit without the gate green
  (`jaunder-commit`): the pre-commit hook runs `cargo xtask check`; run it
  first.
- Backend parity (`CONTRIBUTING.md`): the storage bind conversion touches both
  the sqlite and postgres `update_post` and the backend-agnostic `create_post` —
  all three.
- xtask tests run via `--manifest-path xtask/Cargo.toml` (not `-p xtask`); xtask
  `.rs` is not coverage-measured.
- The trailer/bridge live next to the existing `RenderedHtml` impls in
  `common/src/render.rs` (after the `Serialize` impl, ~line 117). Keep
  `render`/`from_trusted`/carve-out doc comments intact.

---

## Task 1 — Trailer impls + unit tests

**Files:** `common/src/render.rs` (impls after the `Serialize` block; tests in
the existing `#[cfg(test)] mod tests`).

**Add these four impls** (hand-written, matching the macro's `default_trailer`
bodies):

```rust
impl std::borrow::Borrow<str> for RenderedHtml {
    fn borrow(&self) -> &str {
        &self.0
    }
}

// Move the inner `String` out — a free move, unlike `.to_string()` (a clone +
// format machinery). Mirrors every StrNewtype's `From<Self> for String`.
impl From<RenderedHtml> for String {
    fn from(v: RenderedHtml) -> Self {
        v.0
    }
}

impl PartialEq<str> for RenderedHtml {
    fn eq(&self, other: &str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<&str> for RenderedHtml {
    fn eq(&self, other: &&str) -> bool {
        self.0 == **other
    }
}
```

**Tests** (append to `mod tests`):

```rust
#[test]
fn rendered_html_into_string_moves_inner() {
    let h = RenderedHtml::from_trusted("<p>move me</p>");
    let s: String = h.into();
    assert_eq!(s, "<p>move me</p>");
}

#[test]
fn rendered_html_borrows_as_str() {
    fn takes_borrow<T: std::borrow::Borrow<str>>(t: &T) -> &str {
        t.borrow()
    }
    let h = RenderedHtml::from_trusted("<p>b</p>");
    assert_eq!(takes_borrow(&h), "<p>b</p>");
}

#[test]
fn rendered_html_partial_eq_str_and_ref() {
    let h = RenderedHtml::from_trusted("<p>x</p>");
    assert!(h == "<p>x</p>"); // PartialEq<&str>
    assert!(h == *"<p>x</p>"); // PartialEq<str>
    assert!(h != "<p>y</p>");
}
```

**Interfaces:** purely additive; no existing signature changes. AC4's
trust-boundary compile-fail doctests already exist in the `RenderedHtml` doc
comment — leave them; adding `From<Self> for String` / `Borrow` /
`PartialEq<str>` does **not** add any string→newtype constructor, so they still
fail to compile as intended.

**Run:** `cargo nextest run -p common render::tests::rendered_html` — expect the
three new tests PASS (and the existing `rendered_html_*` tests still PASS).

**Commit** (after Task 2, or standalone once `cargo xtask check --no-test` is
clean):
`types: complete the RenderedHtml read-out trailer (From/Borrow/PartialEq) (#502)`.

## Task 2 — Write-side sqlx bridge (`Type` + `Encode`, no `Decode`)

**Files:** `common/src/render.rs` (after Task 1's impls). `common` already has
the optional `sqlx` dep + `sqlx` feature (Cargo.toml); no manifest change.

**Add** (feature-gated; mirrors the macro's `sqlx_impls_inner` **minus**
`compatible` and the whole `Decode` impl):

```rust
// Write-side sqlx bridge (#502): `RenderedHtml` is a first-class TEXT bind
// parameter, delegating to the inner `String` — so storage binds it directly
// (`.bind(&rendered_html)`), not via an `.as_ref()` str-strip.
//
// Deliberately NO `Decode`: a decode could only route through `from_trusted`
// (RenderedHtml has no validating `FromStr`), which would bless ANY text column
// decoded into it — e.g. a raw, un-rendered `body` — as trusted, unescaped HTML,
// invisible to the `rendered-html-from-trusted` gate. Reads stay explicit: the
// `rendered_html` column decodes as `String` and is rebuilt via the gated
// `from_trusted` in `build_post_record`. `Type::compatible` is omitted (its trait
// default is fine) because it is consulted only on that absent decode path.
#[cfg(feature = "sqlx")]
const _: () = {
    impl<DB: sqlx::Database> sqlx::Type<DB> for RenderedHtml
    where
        String: sqlx::Type<DB>,
    {
        fn type_info() -> <DB as sqlx::Database>::TypeInfo {
            <String as sqlx::Type<DB>>::type_info()
        }
    }

    impl<'q, DB: sqlx::Database> sqlx::Encode<'q, DB> for RenderedHtml
    where
        String: sqlx::Encode<'q, DB>,
    {
        fn encode_by_ref(
            &self,
            buf: &mut <DB as sqlx::Database>::ArgumentBuffer<'q>,
        ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
            <String as sqlx::Encode<'q, DB>>::encode_by_ref(&self.0, buf)
        }
        fn size_hint(&self) -> usize {
            <String as sqlx::Encode<'q, DB>>::size_hint(&self.0)
        }
    }
};
```

**Interfaces:** additive; no `Decode` (verified by reading the block — AC5). The
wasm guard in `common/src/lib.rs` already forbids `sqlx` on wasm32, so no new
cfg needed.

**Run:** `cargo check -p common --features sqlx` and `cargo check -p storage` —
expect both compile (storage enables `common/sqlx`).

**Commit:** fold with Task 1, or
`types: add write-side sqlx bridge (Type+Encode) to RenderedHtml (#502)`.

## Task 3 — Convert the three storage bind sites

**Files:** `storage/src/posts.rs` (`create_post` INSERT, ~1896–1898),
`storage/src/sqlite/posts.rs` (`update_post`, ~95–97),
`storage/src/postgres/posts.rs` (`update_post`, ~97–99).

At each site, delete the two-line `// RenderedHtml is hand-rolled … (#502) …`
comment and replace `.bind(input.rendered_html.as_ref())` with
`.bind(&input.rendered_html)` (mirrors the neighboring `.bind(&input.slug)` /
`.bind(&input.body)`).

**Run:** `cargo check -p storage` — expect compile (needs Task 2's `Encode`).
The `sqlx-newtype-bind` gate is still green here (the `.as_ref()` substring is
gone; the allowlist entry is now dead but harmless until Task 4).

**Commit:**
`storage: bind RenderedHtml as a typed sqlx param, not .as_ref() (#502)`.

## Task 4 — Retire the `rendered_html` `sqlx-newtype-bind` allowlist entry

**Files:** `xtask/src/steps/sqlx_newtype_bind_check.rs`.

- Remove the `Allowed { needle: "input.rendered_html.as_ref()", … }` entry (the
  `ALLOWLIST` now has one entry — `input.title.as_ref()`).
- Update the stale prose: the module header's "All storage bind sites are
  already converted" is still true; the `ALLOWLIST` doc "The two exempt
  bind-expressions … Each appears in `posts.rs` + `sqlite/posts.rs` +
  `postgres/posts.rs`" → **one** exempt expression (`input.title.as_ref()` — a
  typed `Option<PostTitle>` bind).
- Narrow the unit test `allowlisted_title_and_rendered_html_are_clean` (line
  ~189): drop the `rendered_html` line and rename to
  `allowlisted_title_is_clean`. (Optionally add a regression test asserting
  `.bind(input.rendered_html.as_ref())` is now _flagged_, proving the gate bites
  — a one-liner:
  `assert_eq!(violations(".bind(input.rendered_html.as_ref())\n"), vec![1]);`.)

**Run:** `cargo nextest run --manifest-path xtask/Cargo.toml sqlx_newtype_bind`
— expect the updated tests PASS. Then
`cargo run --manifest-path xtask/Cargo.toml -- check --no-test` (or
`devtool run -- cargo xtask check --no-test`) to confirm the live gate is green
on the converted tree.

**Commit:** `xtask: retire the RenderedHtml sqlx-bind allowlist entry (#502)`.

## Task 5 — Retire remaining `#502` markers; confirm read path unchanged

**Files:** `storage/src/helpers.rs`.

- `PostRecordParts` note (lines ~123–126) and `PostRow` note (lines ~276–278):
  they call out `rendered_html` as _"hand-rolled — #502 … not string newtypes,
  so they stay `String` and keep their existing … `from_trusted` handling."_ The
  read path genuinely **does** stay `String` → `from_trusted` (spec D2), so keep
  that fact but drop the `#502` deferral framing — reword to state it as the
  deliberate design (no `Decode`, so the column decodes as `String` and is
  rebuilt via the gated `from_trusted`), not as pending work. `format` remains a
  `String`-that-parses; keep it in the note.
- **Do not touch** `build_post_record`'s
  `RenderedHtml::from_trusted(rendered_html)` (line ~212) or its `ALLOWED_FNS`
  membership — AC9.

**Run:** `cargo check -p storage`; `rg -n '#502' storage/ common/ xtask/` should
return **nothing** (every marker resolved).

**Commit:**
`storage: document RenderedHtml's String read path as deliberate, drop #502 markers (#502)`.

## Task 6 — Full gate + bridge-coverage verification

**Run:** `devtool run -- cargo xtask check` (static + clippy + Nix coverage,
both backends; foreground, `timeout: 600000` per the coverage-rebuild note).
Then read the coverage sidecar for any `common/src/render.rs` bridge line
reported uncovered.

- **Expected:** green. The trailer impls are covered by Task 1's tests;
  `encode_by_ref` / `size_hint` / `type_info` by the existing
  `create_post`→`get`/`list` round-trip tests (they bind and read back
  `rendered_html`).
- **If** a bridge line is flagged uncovered (AC5b fallback): add one
  `#[cfg(test)]` unit test in the **storage** crate (sqlx always enabled)
  calling it directly, e.g.
  `assert!(!<RenderedHtml as sqlx::Type<sqlx::Postgres>>::type_info().to_string().is_empty());`
  — guard:no-backend, no DB. Do **not** add a `compatible` override.

Then the pre-push gate: `devtool run -- cargo xtask validate --no-e2e` — expect
green (AC10).

**Commit:** any coverage-driven test only; otherwise nothing new (Task 6 is
verification).

## Self-review

- Every spec AC maps to a task: AC1–AC4→T1; AC5→T2; AC6→T3; AC7→T4; AC8→T3+T5;
  AC9→T5; AC5b+AC10→T6.
- No task smuggles a `Decode`/read-path change (out of scope). No separable
  concerns surfaced (single-issue, no first-task issue filing needed).
- Each task is independently checkable (a `cargo check`/`nextest`/gate run),
  ordered so each compiles on the previous.
