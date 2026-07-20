# Spec — Issue #502: align `RenderedHtml` with the StrNewtype trailer + write-side sqlx bridge

## Context

`RenderedHtml` (`common/src/render.rs`) is a provenance newtype from #398: it
marks HTML that came out of `render()` and is emitted **unescaped** into the
DOM. It is deliberately **not** a `#[derive(StrNewtype)]` type — it has no
public string→newtype constructor (`From<String>`/`TryFrom`/`FromStr`), no
`Deserialize`, and only the trusted rebuild door `from_trusted` (pinned to an
allowlist by the `rendered-html-from-trusted` xtask gate). It is
`Serialize`-only; inbound wire values route through a `deserialize_with` helper
that calls `from_trusted`.

Because it was hand-rolled, it is missing two things every
`#[derive(StrNewtype)]` type has:

1. **Part of the read-out trailer:** `From<RenderedHtml> for String`,
   `Borrow<str>`, `PartialEq<str>`/`PartialEq<&str>`. It currently has only
   `Display`, `AsRef<str>`, `Deref<Target = str>`, `Serialize`.
2. **The sqlx storage bridge.** Two xtask gates and the storage code carry
   explicit `#502` markers deferring this:
   - `sqlx-newtype-bind` (forbids `.bind(newtype.as_ref())`) **allowlists**
     `input.rendered_html.as_ref()` with reason _"RenderedHtml has no sqlx
     bridge yet (#502)"_.
   - `storage/src/{posts.rs, sqlite/posts.rs, postgres/posts.rs}` each bind
     `input.rendered_html.as_ref()` with a `#502` comment.
   - `storage/src/helpers.rs` (`PostRecordParts`/`PostRow`) types the
     `rendered_html` column as `String` and notes it stays stringly per `#502`.

## Decisions

### D1 — Hand-add the trailer; no derive.

No `StrNewtype` derive variant expresses `RenderedHtml`'s shape. All three kinds
require a public string→newtype construction door and/or a `Deserialize` that
`RenderedHtml` forbids (`Default`: `TryFrom<String>`+`FromStr`+`Deserialize`;
`Infallible`: `From<String>`+`From<&str>`+`Deserialize`; `Secret`: redacted
`Debug`, no `Display`/`Deref`). Adding a fourth "serialize-only / trusted-door"
variant for a single type is not warranted. The four trailer impls are
hand-written next to the existing ones.

### D2 — sqlx bridge is **write-side only** (`Type` + `Encode`, no `Decode`).

`RenderedHtml` has **no validation** — it is provenance, not a validated value.
The #438 bridge's `Decode` for other newtypes is safe _because_ it validates via
`FromStr` and rejects a corrupt/wrong-column value. `RenderedHtml` has no
`FromStr`; a `Decode` could only route through `from_trusted`, which **asserts**
trust rather than checking it.

That makes `Decode` unsafe here: it would bless **any** text column decoded into
a `RenderedHtml` as trusted, unescaped HTML — including columns like `body` that
hold raw, un-rendered user input — and each such decode would be an implicit
trust assertion **invisible** to the `from_trusted` grep/allowlist gate.
`Encode` carries no such hazard (serializing a `RenderedHtml` to text is always
safe).

Therefore: implement `Type` + `Encode` (enough for `.bind(&x)`), **not**
`Decode`. The read path is unchanged — the `rendered_html` column still decodes
as `String` and is rebuilt by the single, gated, greppable `from_trusted` call
in `build_post_record`. The `Encode`-without-`Decode` asymmetry is the security
boundary made structural, and is documented at the impl site.

### D3 — Owned-extraction acceptance is vacuously satisfied; no borrow-site churn.

The feed serializers `render_rss`/`render_atom` take `items: &[FeedItem]` and
iterate by reference; `regenerate_feed` also borrows the same `Vec<FeedItem>`
for `feed_etag`. So no site **owns** a `RenderedHtml` from which the inner could
be _moved_ today — the two `.to_string()` calls (`common/src/feed/rss.rs`,
`common/src/feed/atom.rs`) operate on `&RenderedHtml` and are borrow-clones, not
owned extractions. `From<RenderedHtml> for String` is added for trailer
completeness and future owned moves; the acceptance criterion "owned extraction
uses the move" is satisfied because the set of owned extraction sites is empty.
The borrow sites are **not** rewritten (a `.clone().into()` would be no cheaper
than `.to_string()` and only adds noise).

## Scope

In scope: `common/src/render.rs` (trailer + write-side sqlx bridge), the three
storage bind sites, and the two xtask-gate/allowlist updates. Out of scope: any
`Decode`/read-path change, feed serializer signatures, and the
`deserialize_rendered_html` wire door.

## Acceptance criteria

Each is observable (compiles/tests/gates), so ship's conformance review can tell
delivered from not.

**Trailer (D1):**

- **AC1** `String::from(rendered)` moves the inner `String` out of a
  `RenderedHtml` (owned `From<RenderedHtml> for String`), verified by a unit
  test asserting the resulting `String` equals the original HTML.
- **AC2** `<RenderedHtml as Borrow<str>>::borrow(&h)` returns the inner `&str`,
  verified by a unit test (e.g. via a `Borrow<str>`-bound helper or explicit
  UFCS call).
- **AC3** `RenderedHtml == "<...>"` and `RenderedHtml == &"<...>"` compile and
  evaluate, with `PartialEq<str>` and `PartialEq<&str>` unit-tested for both an
  equal and an unequal case.
- **AC4** The trust boundary is unchanged: no public string-validating
  constructor and no blanket `Deserialize` are added. The two existing
  `compile_fail` doctests (private field; no `From`) still pass, and no
  `From<String>`/`TryFrom<String>`/`FromStr`/`Deserialize` for `RenderedHtml`
  exists.

**Write-side sqlx bridge (D2):**

- **AC5** `RenderedHtml` implements `sqlx::Type<DB>` and `sqlx::Encode<'q, DB>`
  under `#[cfg(feature = "sqlx")]` (generic over `DB: Database` where
  `String: Type/Encode`), delegating to the inner `String`, and does **not**
  implement `sqlx::Decode`. `Type` provides only the required `type_info()`; it
  **omits** the `compatible()` override (uses the trait default) because
  `compatible` is consulted only on the decode path, which RenderedHtml — having
  no `Decode` — never reaches. So there is no decode-only line to cover. (The
  no-`Decode` negative is structurally intended but not test-observable — a
  `from_trusted`-based `Decode` _could_ compile — so it is verified by reading
  the impl block, not by a test.)
- **AC5b** Every bridge line is executed under coverage: `encode_by_ref`,
  `size_hint`, and `type_info` lie on the bind/encode path and are exercised by
  the existing storage `create_post` → `get`/`list` round-trip tests (which bind
  `rendered_html` and read it back) — the same mechanism that covers every macro
  newtype's `Encode` today. If the coverage run flags any single bridge line as
  unreached by a bind, cover it with a direct `#[cfg(test)]` unit test in the
  **storage** crate (sqlx always enabled there) calling it by name (e.g.
  `<RenderedHtml as sqlx::Type<sqlx::Postgres>>::type_info()`); do **not**
  reintroduce a `compatible` override to satisfy coverage.
- **AC6** The three storage write sites bind the typed value directly —
  `.bind(&input.rendered_html)` (no `.as_ref()`) in `storage/src/posts.rs`,
  `storage/src/sqlite/posts.rs`, `storage/src/postgres/posts.rs`.
- **AC7** The `input.rendered_html.as_ref()` entry is removed from
  `sqlx_newtype_bind_check`'s `ALLOWLIST`, and its unit tests no longer assert
  `rendered_html.as_ref()` is exempt — the
  `allowlisted_title_and_rendered_html_are_clean` test is renamed/narrowed to
  `input.title.as_ref()` only (so the gate would now flag any `rendered_html`
  reintroduction). The now-stale doc comments are updated too: the module
  header's "All storage bind sites are already converted" wording, the "The two
  exempt bind-expressions" count comment (`:32`, now one), and the removed
  entry's rationale. The gate passes on the converted storage tree.
- **AC8** The `#502` markers that this change resolves are removed/updated: the
  bind-site comments in the three storage files and the
  `PostRecordParts`/`PostRow` notes in `storage/src/helpers.rs` no longer
  describe `rendered_html` as lacking a bridge (the read path's `String`
  typing + `from_trusted` rebuild remains, correctly explained by D2).

**Read path unchanged (D2):**

- **AC9** `build_post_record` still rebuilds `rendered_html` via
  `RenderedHtml::from_trusted` and remains the storage entry on the
  `rendered-html-from-trusted` gate's `ALLOWED_FNS`; that gate is unchanged and
  passes.

**Whole-gate green:**

- **AC10** `cargo xtask validate --no-e2e` is green (static + clippy + coverage,
  both backends). Coverage of the new lines: the **trailer** impls by the
  AC1–AC3 unit tests; the **`Encode`** path (`encode_by_ref`/`size_hint`) plus
  `Type::type_info` by the existing storage round-trip integration tests, with
  the AC5b storage-side fallback unit test for any line a bind doesn't reach. No
  `compatible` line exists (AC5).

## Verification

- Unit tests in `common/src/render.rs` for AC1–AC4 (move-out, borrow, equality
  both directions and both polarities, trust-boundary compile-fails).
- The two xtask gates (`sqlx-newtype-bind`, `rendered-html-from-trusted`)
  exercise AC7/AC9.
- Existing storage round-trip tests (SQLite + PostgreSQL) exercise the converted
  bind path (AC6) end to end; `validate --no-e2e` covers AC10.
