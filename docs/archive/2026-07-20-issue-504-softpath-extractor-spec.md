# Spec — Issue #504: `SoftPath<T>` extractor for soft route-segment parsing

## Context

Several public handlers deliberately extract route segments as `Path<String>`
(or `Path<(String, …)>`) and parse them into newtypes _inside the handler body_,
so a malformed segment yields the SPA shell / soft-404 (and stays DoS-safe)
instead of axum's pre-handler **400**. The rationale is documented in-code
(`projector/mod.rs:138–143,263–268,296–299`; `media.rs:242–266`) — the
deliberate projector-vs-atompub boundary split (ADR-0063 §4). The policy is
correct; it leaves ~10 open-coded `parse::<T>()` sites. This introduces a
`SoftPath<T>` extractor so the parse is expressed by the type while preserving
the exact miss semantics.

## Decisions

### D1 — `SoftPath<T>(Option<T>)`, parse-in-`Deserialize`.

`SoftPath<T>` wraps `Option<T>`. Its `Deserialize` reuses `String::deserialize`
then `FromStr`, storing `Some(t)` on success and **`None` on parse failure** —
extraction **never errors**, so no pre-handler 400. Because it is
`Deserialize`-based (not a standalone `FromRequestParts` extractor), it composes
as a tuple element (`Path<(SoftPath<Username>, SoftPath<Tag>)>`), inside a mixed
tuple with numeric segments (`permalink`), and as a named struct field
(`ServeParams`).

```rust
pub struct SoftPath<T>(Option<T>);

impl<'de, T: FromStr> Deserialize<'de> for SoftPath<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;      // axum path-deserializer supported (Path<String> works)
        Ok(SoftPath(s.parse::<T>().ok()))     // parse failure → None, NOT a deserialize error
    }
}
```

### D2 — Accessors + a test constructor.

- **Owned unwrap:** `From<SoftPath<T>> for Option<T>`, invoked as `soft.into()`
  — mirroring ADR-0063's owned-unwrap convention (`From<Self> for <inner>`);
  `SoftPath`'s inner is `Option<T>`. Used at the projector/feed move-into-seed
  sites (`let Some(x) = soft.into() else { … }` — infers cleanly, single `From`
  impl). No inherent `into_inner()` — the `From` impl is the one owned door, for
  convention consistency.
- **Borrow:** `value(&self) -> Option<&T>`, for the `&ServeParams` site. Named
  `value()` — **not** `get()` — to match the domain-newtype accessor convention
  (ADR-0063; NumNewtype emits `value()`).
- **Construction door:** `SoftPath::parse(s: &str) -> Self` (=
  `SoftPath(s.parse::<T>().ok())`) — the soft-parse-from-string constructor that
  `Deserialize` itself calls, and that fixtures use to seed a segment from a
  `&str` (valid → parsed, invalid → miss), covering both the hit and the 404
  test cases uniformly. Not a `From`, to avoid a confusing
  bidirectional-`Option` pair with the unwrap `From`.

### D3 — Store `Option<T>`, discard the parse error.

Every current site does `let Ok(x) = s.parse() else { miss }` — none use the
error. So `Option<T>` maps directly (`let Some(x) = soft.into() else { miss }`);
no error type is threaded (avoids speculative generality).

### D4 — Location.

A new `server/src/` module (e.g. `server/src/soft_path.rs`), `pub` within the
crate. Not `web`/`common` — it is an axum-server-side extractor.

### D5 — Scope: FromStr-newtype segments only.

Convert only the deliberately-soft `parse::<T>()` sites for `FromStr` newtypes
(`Username`, `Tag`, `Slug`, `ContentHash`, `Filename`, `MediaSource`). **Out of
scope, unchanged:**

- Numeric date segments in `permalink` (`i32`/`u32` year/month/day) — already
  typed; a non-numeric date already 400s and the issue does not flag it.
- The feed `ext`/format segment — parsed by `parse_format` (not `FromStr`), a
  separate helper.
- Strict typed handlers (`atompub/posts.rs` `Path<…>`) — correctly 400 already.

## Surfaces (the ~10 sites)

- **`server/src/feed/handlers.rs`:** `feed_site_tag` (`tag`), `feed_user`
  (`username`), `feed_user_tag` (`username`, `tag`) — the `Path<(String, …)>`
  tuples gain `SoftPath<…>` elements; the trailing `ext` stays `String`. Miss →
  the existing 404.
- **`server/src/projector/mod.rs`:** `permalink` (`username`, `slug` — mixed
  tuple with `i32/u32`), `profile` (`username`), `site_tag` (`tag`), `user_tag`
  (`username`, `tag`). Miss → `shell_response`.
- **`server/src/media.rs`:** `ServeParams.source`/`hash`/`filename` (`String` →
  `SoftPath<MediaSource>` / `SoftPath<ContentHash>` / `SoftPath<Filename>`),
  read via `.value()` in `resolve_paths`; miss → `NOT_FOUND`. (`resolve_paths`
  returns the parsed `hash` by value, so with `.value() -> Option<&T>` behind
  `&ServeParams` it `.clone()`s the borrowed value — a plan detail.) The
  `p1`/`p2` fields stay `String` — they are literal hex path components, not
  newtype-parsed.

## Acceptance criteria

- **AC1** A `SoftPath<T: FromStr>(Option<T>)` extractor exists with
  `Deserialize` (parse failure → `None`, never a deserialize error), a
  `value(&self) -> Option<&T>` accessor, a `From<SoftPath<T>> for Option<T>`
  owned unwrap (`.into()`), and a `parse(&str)` soft-parse constructor (which
  `Deserialize` reuses). A unit test asserts: a valid segment deserializes to
  `Some`, an invalid one to `None` (**not** an `Err`).
- **AC2** No open-coded `parse::<T>()` for a route-segment newtype remains in
  `feed/handlers.rs`, `projector/mod.rs`, or `media.rs` — each goes through
  `SoftPath<T>`. (`grep` shows the converted signatures; the only remaining
  segment parses are the out-of-scope `ext`/numeric ones.)
- **AC3** Behavior is unchanged: a malformed soft segment still yields the SPA
  shell (projector) / 404 (feed, media), **not** a 400. Verified by the existing
  handler unit tests and e2e (which already cover the miss paths) continuing to
  pass unmodified.
- **AC4** The out-of-scope surfaces are untouched: numeric date segments, the
  feed `ext`/format segment, and the strict `atompub` typed handlers.
- **AC5** `cargo xtask validate --no-e2e` clean — no coverage/CRAP regression;
  the extractor's `Deserialize`/accessors are covered by AC1's unit test + the
  handler tests exercising the Some/None paths.

## Verification

- Unit test on `SoftPath` `Deserialize` (Some/None, no error) + accessors (AC1).
- Existing `feed/handlers.rs`, `projector/mod.rs`, `media.rs` handler tests
  exercise the parsed and missed paths unchanged (AC3).
- `validate --no-e2e` (AC5); CI e2e covers the projector shell / media 404
  end-to-end.
