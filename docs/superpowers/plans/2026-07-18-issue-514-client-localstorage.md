# `client` localStorage primitive — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with jaunder-iterate
> (delegating individual tasks to a subagent via jaunder-dispatch when useful).
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give `client` a generic `localStorage` primitive and rewire `web`'s
three raw `web_sys::Storage` touchpoints onto it, leaving the auth-marker codec
host-tested in `web`.

**Architecture:** New wasm-only `client::storage` module (`get`/`set`/`remove`
over `web_sys::Storage`). `web` depends on `client`; `auth/marker.rs` shrinks to
codec-only (cfg-free, host-tested); its `read`/`set`/`clear` browser glue moves
to a new wasm-only `web/src/auth/marker_storage.rs` that pairs `client::storage`
with the codec; the theme sites in `App` call `client::storage` directly.

**Tech Stack:** Rust, wasm32 target, `web-sys`, Leptos, `cargo xtask` gate,
Playwright e2e.

Spec:
[`docs/superpowers/specs/2026-07-18-issue-514-client-localstorage.md`](../specs/2026-07-18-issue-514-client-localstorage.md)
— see it for the "what/why," decisions D1–D7, and acceptance criteria AC1–AC8.
This plan is the "how."

## Global Constraints

_Copied verbatim from the spec; every task's requirements implicitly include
these._

- **ADR-0069 charter:** `client` is `#![cfg(target_arch = "wasm32")]` (empty
  rlib on host). New modules carry **no per-item `#[cfg]`** and **no domain
  types** — raw browser infrastructure only. `client` depends on no workspace
  crate except `common`(+`macros`); `web`/`csr` depend on `client`, never the
  reverse.
- **ADR-0044 sync invariant:** do **not** change `MARKER_KEY`
  (`"jaunder_auth"`), the `{"username":…}` JSON shape, or
  `render::PREPAINT_SCRIPT`. The codec stays host-tested in `web`.
- **No fake host substitutes**; pure/testable logic never moves to `client`.
- **Commits:** conventional-commit messages, **no `Co-Authored-By` trailer**.
  One clean commit per task. Run the per-commit gate (`cargo xtask check`) clean
  first (**jaunder-commit**).
- **No ADR:** this is a direct application of ADR-0069; no new decision to
  record.

---

## Review header

**Scope — in:** `client::storage` primitive; `web`→`client` dependency;
auth-marker codec/glue split; theme persistence rewire; `web-sys`
`Storage`-feature hygiene; done-when verification incl. e2e.

**Scope — out:** any change to `MARKER_KEY`/JSON/`PREPAINT_SCRIPT`; moving the
codec to `common`/`client`; `sessionStorage` or a typed/JSON layer; `web`'s
other `web_sys` usages (`Window`/`Document`/`Location`/upload/`Request…`) —
those are #516–#520; resolving the PR #508 overlap.

**Tasks:**

1. `client::storage` — the generic localStorage primitive (+ `web-sys` dep on
   `client`).
2. Split the auth marker: codec-only `marker.rs` + wasm-only
   `marker_storage.rs`; `web`→`client` dep; repoint 7 call sites.
3. Theme persistence via `client::storage`; drop `Storage` from `web`'s
   `web-sys` features.
4. Done-when verification: `rg` audits + full gate + e2e (marker + theme).

**Key risks / decisions:**

- `client::storage` is wasm-only → not host-unit-testable; behavior is
  e2e-verified (existing `authed-flash.spec.ts` + `theme.spec.ts`). The testable
  part (codec) stays host-tested in `web` (unchanged tests = regression guard).
- Marker `read/set/clear` **must stay wasm-gated in `web`** (they call
  `client::storage::…`, which only exists on wasm) — hence `marker_storage.rs`,
  not a move into `client`.
- Dropping `web`'s `Storage` feature is manifest hygiene, **not** a compile
  backstop (Cargo unifies `web-sys/Storage` in via `client`). Enforcement is the
  AC1 `rg` audit (Task 4).
- Each task commits with the host gate green; e2e confirmation is Task 4 (CI
  also runs the full matrix).

---

### Task 1: `client::storage` — generic localStorage primitive

**Files:**

- Modify: `client/Cargo.toml` (add `web-sys` dep)
- Modify: `client/src/lib.rs` (declare `pub mod storage;`)
- Create: `client/src/storage.rs`

**Interfaces:**

- Consumes: nothing (leaf primitive).
- Produces (relied on by Tasks 2 & 3):
  - `client::storage::get(key: &str) -> Option<String>`
  - `client::storage::set(key: &str, value: &str)`
  - `client::storage::remove(key: &str)`

**Testability note:** `client` is `#![cfg(target_arch = "wasm32")]`, so this
module compiles **only** under the wasm target and cannot carry a host unit
test. Its verification is the wasm-clippy compile/lint step (run via
`cargo xtask check`) plus the e2e in Task 4. No host test is written for it
(writing a fake host stub is forbidden by the charter).

- [ ] **Step 1: Add the `web-sys` dependency to `client`.**

In `client/Cargo.toml`, under `[dependencies]` (currently empty), add:

```toml
web-sys = { workspace = true, features = ["Window", "Storage"] }
```

(`Window` for `web_sys::window()`, `Storage` for
`local_storage()`/`get_item`/`set_item`/`remove_item`. No
`js-sys`/`wasm-bindgen`/`common` — the primitive constructs no `JsValue`.)

- [ ] **Step 2: Declare the module in `client/src/lib.rs`.**

After the existing crate doc + `#![cfg(target_arch = "wasm32")]` line, append:

```rust
/// Generic browser `localStorage` key/value primitive (#514). Raw string KV, no
/// domain types — the `web`/`csr` home for what were scattered `web_sys::Storage`
/// call sites.
pub mod storage;
```

- [ ] **Step 3: Write `client/src/storage.rs`.**

Signature is fixed by the Interfaces block; there is no branch a host test could
pin (wasm-only), so the body is written out in full:

```rust
//! Generic browser `localStorage` key/value access — raw browser infrastructure
//! per the `client` charter (ADR-0069), no domain types. Best-effort: every
//! operation silently no-ops when `window`/`localStorage` is unavailable or the
//! browser rejects the access (private-mode quota, storage disabled), matching the
//! swallow-the-error behavior the migrated `web` call sites have always had.

/// The window's `localStorage`, or `None` when unavailable.
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Read the string stored under `key`, or `None` if absent/unavailable.
#[must_use]
pub fn get(key: &str) -> Option<String> {
    local_storage()?.get_item(key).ok().flatten()
}

/// Store `value` under `key` (best-effort; ignores failure).
pub fn set(key: &str, value: &str) {
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(key, value);
    }
}

/// Remove any value stored under `key` (best-effort; ignores failure).
pub fn remove(key: &str) {
    if let Some(storage) = local_storage() {
        let _ = storage.remove_item(key);
    }
}
```

- [ ] **Step 4: Verify it compiles + lints on wasm.**

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS — the
wasm-clippy step compiles `-p client` for `wasm32-unknown-unknown` and lints
`client::storage` clean; the host build treats `client` as an empty rlib. (If
iterating faster: `cargo clippy -p client --target wasm32-unknown-unknown`
directly.)

- [ ] **Step 5: Commit.**

```bash
git add client/Cargo.toml client/src/lib.rs client/src/storage.rs
git commit -m "feat(client): add localStorage key/value primitive (#514)"
```

Run `cargo xtask check` (full) first so the commit is green
(**jaunder-commit**). Also stage the spec + this plan in this first commit (they
belong on the branch for jaunder-ship to archive):

```bash
git add docs/superpowers/specs/2026-07-18-issue-514-client-localstorage.md \
        docs/superpowers/plans/2026-07-18-issue-514-client-localstorage.md
```

---

### Task 2: Split the auth marker — codec-only `marker.rs` + wasm-only `marker_storage.rs`

**Files:**

- Modify: `web/Cargo.toml` (add `client` dep)
- Modify: `web/src/auth/marker.rs` (strip glue → codec-only)
- Create: `web/src/auth/marker_storage.rs`
- Modify: `web/src/auth/mod.rs` (declare `marker_storage`; fix doc comment)
- Modify: `web/src/pages/auth.rs:50,152,223` (repoint set/set/clear)
- Modify: `web/src/pages/ui.rs:323,993,999,1030` (repoint read/set/clear/read)
- Test: existing `#[cfg(test)] mod tests` in `web/src/auth/marker.rs` (unchanged
  — regression guard)

**Interfaces:**

- Consumes: `client::storage::{get,set,remove}` (Task 1).
- Produces (relied on by call sites + Task 4):
  - `web::auth::marker` keeps: `MARKER_KEY: &str`,
    `encode_marker(&str) -> String`, `decode_marker(&str) -> Option<String>` —
    **no** `read`/`set`/`clear`, **no** `target_arch` cfg.
  - `web::auth::marker_storage` (wasm-only) gains: `read() -> Option<String>`,
    `set(username: &str)`, `clear()`.

- [ ] **Step 1: Confirm the regression guard fails to move (baseline).**

The three codec tests in `marker.rs` (`round_trips_username`,
`decode_rejects_malformed`, `encode_escapes_json`) are the contract that the
codec is unchanged. Run them now to confirm green baseline:

Run: `cargo nextest run -p web marker` Expected: PASS (3 tests) — establishes
the codec behavior we must preserve.

- [ ] **Step 2: Add the `client` dependency to `web`.**

In `web/Cargo.toml`, under `[dependencies]`, add (alphabetically near
`cfg-if`/`common`):

```toml
client = { path = "../client" }
```

Unconditional — `client` is an empty rlib on host, so the host build is
unaffected.

- [ ] **Step 3: Shrink `web/src/auth/marker.rs` to codec-only.**

Delete the `storage()`, `read()`, `set()`, `clear()` fns (lines 35-62) **and**
their `#[cfg(target_arch = "wasm32")]` attributes. The file's top module doc
(lines 1-6, about the marker concept + `PREPAINT_SCRIPT` sync) stays. Final file
body:

```rust
//! The client-side **auth marker** (#181, ADR-0044): a JS-readable localStorage
//! value advertising "probably the owner" for pre-paint chrome adjustment. It is
//! ADVISORY, not a credential — the real session stays the HTTP-only cookie, and
//! the server authorizes every mutation. The pre-paint `<head>` script
//! (`render::PREPAINT_SCRIPT`) reads the SAME key + `.username` field, so the
//! encode/decode shape here and that script must stay in sync.
//!
//! Pure codec only: the wasm-only `localStorage` binding lives in
//! [`super::marker_storage`] (#514).

use serde::{Deserialize, Serialize};

/// The localStorage key holding the marker. Kept in sync with the pre-paint script.
pub const MARKER_KEY: &str = "jaunder_auth";

#[derive(Serialize)]
struct Marker<'a> {
    username: &'a str,
}

/// The localStorage value for `username` (JSON `{"username":"…"}`).
#[must_use]
pub fn encode_marker(username: &str) -> String {
    serde_json::to_string(&Marker { username }).unwrap_or_default()
}

/// Parse a marker value back to its username, `None` when malformed/empty.
#[must_use]
pub fn decode_marker(raw: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Owned {
        username: String,
    }
    let m: Owned = serde_json::from_str(raw).ok()?;
    (!m.username.is_empty()).then_some(m.username)
}

#[cfg(test)]
mod tests {
    use super::{decode_marker, encode_marker};

    #[test]
    fn round_trips_username() {
        let raw = encode_marker("alice");
        assert_eq!(raw, r#"{"username":"alice"}"#);
        assert_eq!(decode_marker(&raw), Some("alice".to_string()));
    }

    #[test]
    fn decode_rejects_malformed() {
        assert_eq!(decode_marker("not json"), None);
        assert_eq!(decode_marker("{}"), None);
    }

    #[test]
    fn encode_escapes_json() {
        // A quote in a username must not break the JSON the pre-paint script parses.
        assert_eq!(
            decode_marker(&encode_marker(r#"a"b"#)),
            Some(r#"a"b"#.into())
        );
    }
}
```

- [ ] **Step 4: Create `web/src/auth/marker_storage.rs`.**

```rust
//! Browser (`localStorage`) binding of the auth marker (#181, ADR-0044). The pure
//! codec + `MARKER_KEY` live in [`super::marker`] (host-tested); this wasm-only
//! module pairs them with the generic [`client::storage`] primitive. Split out of
//! `marker.rs` (#514) so that codec file stays cfg-free and host-tested.

use super::marker::{decode_marker, encode_marker, MARKER_KEY};

/// Read + decode the marker from localStorage, `None` when absent/malformed.
#[must_use]
pub fn read() -> Option<String> {
    decode_marker(&client::storage::get(MARKER_KEY)?)
}

/// Write the marker for `username`.
pub fn set(username: &str) {
    client::storage::set(MARKER_KEY, &encode_marker(username));
}

/// Remove the marker.
pub fn clear() {
    client::storage::remove(MARKER_KEY);
}
```

- [ ] **Step 5: Wire the module + fix the stale doc in `web/src/auth/mod.rs`.**

Replace the current lines 9-11:

```rust
/// The client-side advisory auth marker (#181, ADR-0044). Pure encode/decode are
/// host-testable; `read`/`set`/`clear` are wasm-only (localStorage).
pub mod marker;
```

with:

```rust
/// The client-side advisory auth marker (#181, ADR-0044). Pure encode/decode +
/// `MARKER_KEY` are host-testable here; the wasm-only `localStorage` binding
/// (`read`/`set`/`clear`) lives in [`marker_storage`].
pub mod marker;

/// Browser `localStorage` binding of the auth marker (wasm-only): `read`/`set`/
/// `clear` over [`client::storage`] + the [`marker`] codec (#514).
#[cfg(target_arch = "wasm32")]
pub mod marker_storage;
```

- [ ] **Step 6: Repoint the 7 call sites** from
      `crate::auth::marker::{read,set,clear}` to
      `crate::auth::marker_storage::{read,set,clear}` (the `marker::` codec path
      is unaffected — no call site touches `encode`/`decode`/`MARKER_KEY`):

- `web/src/pages/auth.rs:50` — `crate::auth::marker::set(` →
  `crate::auth::marker_storage::set(`
- `web/src/pages/auth.rs:152` — same `set` repoint
- `web/src/pages/auth.rs:223` — `crate::auth::marker::clear()` →
  `crate::auth::marker_storage::clear()`
- `web/src/pages/ui.rs:323` — `crate::auth::marker::read()` →
  `crate::auth::marker_storage::read()`
- `web/src/pages/ui.rs:993` — `crate::auth::marker::set(&u)` →
  `crate::auth::marker_storage::set(&u)`
- `web/src/pages/ui.rs:999` — `crate::auth::marker::clear()` →
  `crate::auth::marker_storage::clear()`
- `web/src/pages/ui.rs:1030` — `crate::auth::marker::read()` →
  `crate::auth::marker_storage::read()`

- [ ] **Step 7: Run the codec regression guard + full gate.**

Run: `cargo nextest run -p web marker` Expected: PASS (3 tests) — codec
unchanged.

Run: `devtool run -- cargo xtask check` Expected: PASS — host build (marker.rs
codec compiles/tests; `pages`+`marker_storage` are wasm-only, not host-compiled)
and wasm-clippy (compiles `marker_storage` + repointed wasm call sites against
`client::storage`) both clean.

- [ ] **Step 8: Commit.**

```bash
git add web/Cargo.toml web/src/auth/marker.rs web/src/auth/marker_storage.rs \
        web/src/auth/mod.rs web/src/pages/auth.rs web/src/pages/ui.rs
git commit -m "refactor(web): split auth-marker codec from its localStorage binding (#514)"
```

---

### Task 3: Theme persistence via `client::storage`; drop `web`'s `Storage` feature

**Files:**

- Modify: `web/src/pages/mod.rs:90-109` (`App` theme read + write)
- Modify: `web/Cargo.toml` (remove `"Storage"` from `web-sys` features)

**Interfaces:**

- Consumes: `client::storage::{get,set}` (Task 1).
- Produces: nothing new (internal to `App`).

**Testability note:** `App` is in the wasm-only `pages` module
(`web/src/lib.rs:31`), so it is not host-compiled and carries no host test. This
is a 1:1 plumbing swap (identical semantics: `get_item`→`get`, `set_item`→`set`,
same non-empty guard); behavior is covered by `theme.spec.ts` in Task 4. No new
test.

- [ ] **Step 1: Rewrite the theme read-on-boot** (`web/src/pages/mod.rs`,
      currently lines 92-99):

```rust
    // On WASM: restore theme from localStorage on startup.
    if let Some(val) = client::storage::get("jaunder_theme") {
        if !val.is_empty() {
            theme.set(val);
        }
    }
```

- [ ] **Step 2: Rewrite the theme persist-Effect** (currently lines 103-109):

```rust
    // On WASM: persist theme to localStorage whenever it changes.
    Effect::new(move |_| {
        client::storage::set("jaunder_theme", &theme.get());
    });
```

- [ ] **Step 3: Drop the now-unused `Storage` feature from `web`.**

In `web/Cargo.toml`, remove the `"Storage",` line from the `web-sys` `features`
array (leaving `Window`, `Document`, `Element`, `Location`, `File`, `FileList`,
`FormData`, `HtmlInputElement`, `Request`, `RequestInit`, `RequestMode`,
`Response`). Manifest hygiene — `web` no longer references `web_sys::Storage`
directly.

- [ ] **Step 4: Run the full gate.**

Run: `devtool run -- cargo xtask check` Expected: PASS — host + wasm-clippy
clean. (`web` still compiles even without its own `Storage` feature: Cargo
unifies it in via `client`.)

- [ ] **Step 5: Commit.**

```bash
git add web/src/pages/mod.rs web/Cargo.toml
git commit -m "refactor(web): persist theme via client::storage (#514)"
```

---

### Task 4: Done-when verification (audits + gate + e2e)

**Files:** none (verification only — no commit unless an audit surfaces a fix).

**Interfaces:** none.

- [ ] **Step 1: AC1 — no raw storage glue remains in `web`.**

Run: `rg 'web_sys::Storage|local_storage\(' web/src` Expected: **zero** matches.

- [ ] **Step 2: AC2 — `marker.rs` is cfg-free.**

Run: `rg 'target_arch|web_sys' web/src/auth/marker.rs` Expected: **zero**
matches.

- [ ] **Step 3: AC3/AC7 — codec + drift-guard tests host-side.**

Run: `cargo nextest run -p web marker` (AC3: 3 codec tests) and confirm the
`render` drift-guard test still passes within the full suite. Expected: PASS.

- [ ] **Step 4: AC8 — full host gate.**

Run (foreground, `timeout: 600000` — a coverage rebuild):
`devtool run -- cargo xtask check` Expected: PASS — static + clippy +
wasm-clippy + Nix coverage; no coverage-gate regression.

- [ ] **Step 5: AC5/AC6 — behavior e2e.**

Run (foreground, long timeout; CI also runs the full
`{sqlite,postgres}×{chromium,firefox}` matrix):
`devtool run -- cargo xtask e2e sqlite chromium` Expected: PASS — including
`authed-flash.spec.ts` (marker set→read round-trip: `html.authed` + `data-user`)
and `theme.spec.ts` (`.j-root` real `data-theme` after hydration; theme read
path via `client::storage`).

- [ ] **Step 6: Hand off to jaunder-ship** once all audits + gate + e2e are
      green. (No commit in this task.)

## Self-Review

- **Spec coverage:** AC1→Task 4 Step 1 (+ D4 in Task 3); AC2→Task 4 Step 2
  (marker.rs shrunk in Task 2); AC3→Tasks 2/4 (codec tests preserved); AC4→Task
  1 (`client::storage` + `Cargo.toml`); AC5/AC6→Task 4 Step 5; AC7→Task 4 Step
  3; AC8→Task 4 Step 4. D1/D2→Task 1; D3/D5/D6→Task 2; D4/D7→Task 3. All
  covered.
- **Placeholders:** none — every step has concrete file paths, full code, and
  exact commands with expected results.
- **Type consistency:**
  `client::storage::{get(&str)->Option<String>, set(&str,&str), remove(&str)}`
  defined in Task 1 and consumed identically in Tasks 2 (`marker_storage`) and 3
  (`App`); `marker_storage::{read,set,clear}` produced in Task 2 and repointed
  at all 7 call sites in the same task.
