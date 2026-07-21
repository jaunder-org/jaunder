# Plan — Issue #504: `SoftPath<T>` extractor

Spec:
[`docs/superpowers/specs/2026-07-20-issue-504-softpath-extractor.md`](../specs/2026-07-20-issue-504-softpath-extractor.md)
(decisions D1–D5, acceptance AC1–AC5). This is the "how".

## Review header

**Goal.** Add a `SoftPath<T: FromStr>(Option<T>)` axum extractor (parse-miss →
`None`, never a pre-handler 400) and route the ~10 deliberately-soft segment
parses through it, preserving the shell/soft-404 semantics.

**Scope.**

- In: new `server/src/soft_path.rs`; conversions in
  `server/src/feed/handlers.rs`, `server/src/projector/mod.rs`,
  `server/src/media.rs` (+ its fixtures).
- Out (spec D5): numeric date segments (`i32`/`u32`), the feed
  `ext`/`parse_format` segment, `media` `p1`/`p2`, and the strict `atompub`
  typed handlers.

**Tasks.**

1. Create `SoftPath<T>` (struct, `parse`, `value`, `Deserialize`, `From`
   unwrap) + unit tests.
2. Convert `feed/handlers.rs` (3 handlers).
3. Convert `projector/mod.rs` (4 handlers).
4. Convert `media.rs` (`ServeParams` +
   `validate_serve_params`/`resolve_media_path` + `serve_response` + fixtures).
5. Full gate + commit.

**Key risks / decisions.**

- **Behavior parity (AC3):** each conversion is a mechanical
  `let Ok(x) = s.parse() else` → `let Some(x) = soft.into() else` (or `.value()`
  for the `&ServeParams` site) — the miss branch (shell / 404) is unchanged. The
  existing handler tests + e2e are the parity check.
- **Media wrinkle:** `serve_response` uses `params.filename` (a `String`) for
  content-type detection/disposition (#495). Once it's `SoftPath<Filename>`,
  thread the **parsed** `Filename` out of `resolve_media_path` and use that
  (`&Filename` deref-coerces to `&str`) — don't read `params.filename`
  post-conversion.
- **Accessor convention:** owned unwrap is `From<SoftPath<T>> for Option<T>`
  (`soft.into()`); borrow is `value()` — matching ADR-0063 (`value()` +
  `From<Self> for inner`), not `get()`.
- **Lands as one commit** — small, coherent; `SoftPath` + its uses together (its
  `Deserialize`/`From`/`value` lines are covered by AC1's unit test + the
  handler tests).

**For agentic workers.** Drive with `jaunder-iterate`; the four edit tasks are
one compile unit — `cargo check -p jaunder` after each, one commit at task 5.

## Global constraints

- No `Co-Authored-By`. Commit on a green gate (`jaunder-commit`); the hook runs
  full `cargo xtask check`.
- `unwrap_used`/`expect_used` are `deny` in production (`.ok()`/`let-else` only;
  `unwrap`/ `expect` fine in `#[cfg(test)]`).
- `server`'s package is `jaunder` (`-p jaunder`); its integration tests are
  `--test integration`.

---

## Task 1 — `SoftPath<T>` module + unit tests

**Files:** new `server/src/soft_path.rs`; register `mod soft_path;` in
`server/src/lib.rs` (or `main.rs` — wherever the module list lives) with
`pub(crate) use soft_path::SoftPath;` if that matches the crate's re-export
style.

```rust
//! `SoftPath<T>` — a route-segment extractor that soft-parses a segment into `T` without
//! axum's pre-handler 400 (#504). A parse miss reaches the handler as `None`, which the
//! handler renders as the SPA shell / a soft 404 (the deliberate projector-vs-atompub
//! boundary, ADR-0063 §4).

use std::str::FromStr;

use serde::{Deserialize, Deserializer};

/// A path segment soft-parsed into `T`: `Some(t)` on success, `None` on a parse miss.
/// Being `Deserialize`-based (via `String::deserialize`), it composes anywhere `Path<String>`
/// does — a bare segment, a tuple element (incl. mixed with `i32`/`u32`), or a named-struct
/// field — and its deserialization **never errors**, so a malformed segment is `None`, not a 400.
pub struct SoftPath<T>(Option<T>);

impl<T: FromStr> SoftPath<T> {
    /// Soft-parse `s`: `Some` on success, `None` on a parse miss. The one soft-parse
    /// chokepoint — `Deserialize` and test fixtures both route through it.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        SoftPath(s.parse::<T>().ok())
    }
}

impl<T> SoftPath<T> {
    /// The parsed value by reference, or `None` on a miss (the ADR-0063 `value()` accessor).
    #[must_use]
    pub fn value(&self) -> Option<&T> {
        self.0.as_ref()
    }
}

/// Owned unwrap to the inner `Option<T>` — the ADR-0063 `From<Self> for <inner>` convention
/// (here the inner is `Option<T>`). Invoked as `soft.into()`.
impl<T> From<SoftPath<T>> for Option<T> {
    fn from(soft: SoftPath<T>) -> Self {
        soft.0
    }
}

impl<'de, T: FromStr> Deserialize<'de> for SoftPath<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // Deserialize as a plain String (axum's path deserializer drives this exactly like
        // `Path<String>`), then soft-parse — a miss is `None`, NOT an error, so no 400.
        let s = String::deserialize(d)?;
        Ok(SoftPath::parse(&s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::username::Username;

    #[test]
    fn parse_valid_is_some_and_unwraps() {
        let sp = SoftPath::<Username>::parse("alice");
        assert_eq!(sp.value().map(AsRef::as_ref), Some("alice"));
        let owned: Option<Username> = sp.into();
        assert_eq!(owned, Some("alice".parse().unwrap()));
    }

    #[test]
    fn parse_invalid_is_none_not_error() {
        let sp = SoftPath::<Username>::parse("not a valid name!!");
        assert!(sp.value().is_none());
        let owned: Option<Username> = sp.into();
        assert!(owned.is_none());
    }

    #[test]
    fn deserialize_never_errors_valid_or_invalid() {
        // The load-bearing property: deserialize SUCCEEDS for both, storing Some/None.
        let ok: SoftPath<Username> = serde_json::from_str("\"alice\"").unwrap();
        assert!(ok.value().is_some());
        let miss: SoftPath<Username> = serde_json::from_str("\"bad name!\"").unwrap();
        assert!(miss.value().is_none());
    }
}
```

(`Username::from_str` rejects any char outside `[a-z0-9_-]`, so
`"not a valid name!!"` / `"bad name!"` both → `None`. `serde_json` is a regular
`server` dependency — the third test compiles as-is.)

**Run:** `cargo nextest run -p jaunder soft_path` — expect the 3 tests PASS.

## Task 2 — `feed/handlers.rs` (3 handlers)

**Files:** `server/src/feed/handlers.rs` (`use crate::soft_path::SoftPath;`).
Each is the same mechanical change; the trailing `ext: String` and its
`parse_format` stay.

- `feed_site_tag` (~:120): `Path<(String, String)>` →
  `Path<(SoftPath<Tag>, String)>`; the
  `let Ok(tag) = tag.parse::<Tag>() else { … }` →
  `let Some(tag) = tag.into() else { … }`.
- `feed_user` (~:144): `Path<(String, String)>` →
  `Path<(SoftPath<Username>, String)>`;
  `let Some(username) = username.into() else { … }`.
- `feed_user_tag` (~:168): `Path<(String, String, String)>` →
  `Path<(SoftPath<Username>, SoftPath<Tag>, String)>`;
  `let (Some(username), Some(tag)) = (username.into(), tag.into()) else { … }`.

The `else { … }` (404) bodies are unchanged.

**Run:** `cargo check -p jaunder`.

## Task 3 — `projector/mod.rs` (4 handlers)

**Files:** `server/src/projector/mod.rs` (`use crate::soft_path::SoftPath;`,
**and add `use common::slug::Slug;`** — `Slug` is only referenced in a comment
today, so `permalink`'s `SoftPath<Slug>` needs the import). The `String`-only
rationale comments (`:138-143` etc.) can shorten to a one-line pointer, since
the type now carries the intent — but leave the ADR-0063 §4 reference.

- `permalink` (~:144): `Path<(String, i32, u32, u32, String)>` →
  `Path<(SoftPath<Username>, i32, u32, u32, SoftPath<Slug>)>`;
  `let (Some(username), Some(slug)) = (username.into(), slug.into()) else { return shell_response(&shell); };`
- `profile` (~:234): `Path<String>` → `Path<SoftPath<Username>>`; the `match`
  scrutinee becomes
  `match <Option<Username>>::from(username) { Some(username) => …, None => None }`
  (explicit turbofish on the match scrutinee to avoid relying on inference
  there).
- `site_tag` (~:269): `Path<String>` → `Path<SoftPath<Tag>>`;
  `let Some(tag) = tag.into() else { return shell_response(&shell); };`
- `user_tag` (~:300): `Path<(String, String)>` →
  `Path<(SoftPath<Username>, SoftPath<Tag>)>`;
  `let (Some(username), Some(tag)) = (username.into(), tag.into()) else { return shell_response(&shell); };`

**Run:** `cargo check -p jaunder`.

## Task 4 — `media.rs` (`ServeParams` + resolve + fixtures)

**Files:** `server/src/media.rs` (`use crate::soft_path::SoftPath;`).

- `ServeParams` (~:91): `source`/`hash`/`filename`: `String` →
  `SoftPath<MediaSource>` / `SoftPath<ContentHash>` / `SoftPath<Filename>`.
  `p1`/`p2` stay `String`. The `#[derive(Deserialize)]` is unchanged
  (`SoftPath: Deserialize`).
- `validate_serve_params` (~:245) — `.value()` + clone at the by-value return,
  preserving the source→hash→p1/p2→filename order (all misses → the same
  `NOT_FOUND`):

```rust
fn validate_serve_params(params: &ServeParams)
    -> Result<(MediaSource, ContentHash, Filename), StatusCode> {
    let Some(source) = params.source.value() else { return Err(StatusCode::NOT_FOUND) };
    let Some(hash) = params.hash.value() else { return Err(StatusCode::NOT_FOUND) };
    if !hash.starts_with(&params.p1) || !hash[2..].starts_with(&params.p2) {
        return Err(StatusCode::NOT_FOUND);
    }
    let Some(filename) = params.filename.value() else { return Err(StatusCode::NOT_FOUND) };
    Ok((source.clone(), hash.clone(), filename.clone()))
}
```

(`hash`/`filename` are `&ContentHash`/`&Filename`; `Deref<str>` makes
`starts_with`/`[2..]` work. `MediaSource` derives `Clone`.)

- `resolve_media_path` (~:273): thread the parsed `Filename` out — return
  `(MediaSource, ContentHash, Filename, PathBuf)` (it already has `filename`
  from `validate_serve_params`; add it to the tuple). **Forced test edit:** its
  `Ok`-case test `resolve_media_path_builds_path_for_valid_params` (~:377)
  destructures the 3-tuple — add the 4th binding
  (`let (_source, _hash, _filename, path) = …`). The six `Err(…)`-comparing
  tests are unaffected.
- `serve_response` (~:141):
  `let (source, hash, filename, file_path) = resolve_media_path(…)?;` then use
  the parsed `filename` (not `params.filename`) at ~:160/:162 —
  `detect_content_type(&filename)` and
  `content_disposition(&content_type, &filename)` (`&Filename` deref-coerces to
  `&str`).
- Fixtures — the `params(source, p1, p2, hash, filename)` helper (~:366) and any
  direct `ServeParams { … }` literals: `source: SoftPath::parse(source)`,
  `hash: SoftPath::parse(hash)`, `filename: SoftPath::parse(filename)` (this
  handles the existing bad-hash/bad-filename 404 tests uniformly — an invalid
  `&str` → a `miss`). `p1`/`p2` stay `.to_string()`.

**Run:** `cargo check -p jaunder`; then
`cargo nextest run -p jaunder --test integration web_media` (or the media
handler tests) — expect the existing serve/404 tests PASS unmodified.

## Task 5 — Full gate + commit

**Run:** `devtool run -- cargo xtask check` (foreground, `timeout: 600000`).
Expect green; `SoftPath`'s lines are covered by Task 1's tests + the handler
tests exercising Some/None. Grep to confirm AC2: no
`parse::<Username>()`/`parse::<Tag>()`/`parse::<Slug>()`/
`parse::<ContentHash>()`/`parse::<Filename>()`/`parse::<MediaSource>()` for a
path segment remains in the three files (only the out-of-scope `ext`/numeric
parses).

Then `devtool run -- cargo xtask validate --no-e2e` (AC5). Commit once
(`jaunder-commit`):
`server: SoftPath<T> extractor for deliberately-soft route-segment parsing (#504)`.

## Self-review

- Every AC maps: AC1→T1; AC2→T2–T4 (+ T5 grep); AC3→T2–T4 (existing tests
  unmodified); AC4→scope (numeric/ext/p1/p2/atompub untouched); AC5→T5.
- No separable concern surfaced → no first-task issue filing.
- The four edit tasks are one compile unit; verify by `cargo check` after each,
  one commit at T5.
