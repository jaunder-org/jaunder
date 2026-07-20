# Spec — #542: type the wasm auth marker as `Username`

- Issue: [#542](https://github.com/jaunder-org/jaunder/issues/542)
- Milestone: Domain-value type safety (newtypes)
- Governing ADRs:
  [ADR-0044](../../adr/0044-authenticated-owner-flash-free-enhancement.md) (the
  pre-paint auth marker),
  [ADR-0063](../../adr/0063-domain-value-newtype-convention.md) (pervasive
  newtype use), [ADR-0069](../../adr/0069-client-crate-wasm-only-home.md)
  (`client::storage` stays generic)
- Origin: follow-up from #503
- Date: 2026-07-19

## Problem

The wasm auth marker is stored/read as a raw `String`/`&str` at every hop even
though every write already holds a validated `Username` and every meaningful
read wants one. A `Username` is stringified on write, read back as a `String`,
and re-`parse::<Username>()`'d on the sidebar read (the stopgap added in #503).
See the issue for the per-site inventory.

## Decision

Type the marker's **Rust** API as `Username` end-to-end. The localStorage JSON
shape is a **cross-boundary contract** (the blocking `<head>` pre-paint script
reads the same `jaunder_auth` key + `.username` field), so it stays
**byte-identical** — the parse/format happen at the codec edge, not on the wire.

- **Codec** (`web/src/auth/marker.rs`, cfg-free, host-tested):
  - `encode_marker(username: &Username) -> String` — same JSON, formats the
    newtype's string.
  - `decode_marker(raw: &str) -> Option<Username>` — parse the extracted
    username via `Username::from_str`; `None` on malformed JSON **or** an
    invalid username. This is the **single** malformed→`None` chokepoint (the
    old `.is_empty()` guard is subsumed — `Username` cannot be empty).
- **Storage binding** (`web/src/auth/marker_storage.rs`, wasm-only):
  `get() -> Option<Username>`; `set(username: &Username)`; `remove()` unchanged.
- **`web/src/pages/ui.rs`:**
  - `owner: RwSignal<Option<Username>>`;
    `marker_username_on_boot() -> Option<Username>`.
  - `authed_sidebar(active_key: &str, username: &Username, is_operator: bool)` —
    **drop the #503 `parse::<Username>()`**; footer avatar is
    `<Avatar name=username.clone() size=28 />`, label renders the `Username`'s
    `Display`.
  - `marker_matches(author: &Username) -> bool` — compare via `Username`'s
    `PartialEq` (`get() == Some(author.clone())` or equivalent), called with
    `&post.username`.
  - Reconcile Effect: compare `current_user()`'s `Username` against `owner`
    directly (drop the `.to_string()`/`as_str()`).
- **SET sites** — `auth/component.rs`, `registration/component.rs`, the
  reconcile Effect in `ui.rs`: pass the `&Username` directly (drop
  `.as_ref()`/`.to_string()`).

`client::storage` stays generic `&str`/`String` (ADR-0069) — untouched.

### No new ADR

Within existing conventions (ADR-0063 pervasive newtype use); the JSON contract
and marker semantics of ADR-0044 are preserved, not changed.

## Acceptance criteria (observable)

1. **The codec is typed** — `encode_marker` takes `&Username`, `decode_marker`
   returns `Option<Username>` (`web/src/auth/marker.rs`); its host tests build
   `Username` values and assert round-trip + malformed→`None`.
2. **The storage binding is typed** —
   `marker_storage::get() -> Option<Username>`, `set(username: &Username)`
   (`web/src/auth/marker_storage.rs`).
3. **No stringly round-trip of the marker value** — no `.parse::<Username>()`,
   `.as_ref()`, or `.to_string()` on the marker/username in `marker.rs`,
   `marker_storage.rs`, or the marker read/write sites in `pages/ui.rs`
   (`owner`, `marker_username_on_boot`, `authed_sidebar`, `marker_matches`, the
   reconcile Effect, the three SET sites). The `#503` sidebar
   `username.parse::<Username>().ok()` is gone.
4. **The localStorage JSON is byte-unchanged** — key `jaunder_auth`, shape
   `{"username":"…"}`. The pre-paint-script drift-guard test (asserting
   `csr/index.html` / projector head contains the script) still passes, and the
   codec still round-trips through the exact same JSON.
5. **Marker semantics preserved** — written on login/register, cleared on
   logout, corrected on reconcile mismatch (ADR-0044 §3); a malformed/absent
   marker still degrades to the anon default.
6. **Gate green** — `cargo xtask check` passes (static + clippy + coverage,
   `--all-features --all-targets`).

## Out of scope

- The `client::storage` primitive (stays generic per ADR-0069).
- The pre-paint `<head>` script / `csr/index.html` (JSON contract unchanged).
- Any change to marker _semantics_ (staleness, expiry — ADR-0044 defers these).
