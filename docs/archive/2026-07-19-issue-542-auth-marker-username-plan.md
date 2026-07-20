# Plan — #542: type the wasm auth marker as `Username`

- Spec:
  [2026-07-19-issue-542-auth-marker-username.md](../specs/2026-07-19-issue-542-auth-marker-username.md)
- Issue: [#542](https://github.com/jaunder-org/jaunder/issues/542)
- For agentic workers: drive with **jaunder-iterate**.

## Review header

**Goal.** Retype the wasm auth marker's Rust API (`marker` codec +
`marker_storage` binding) and its `pages/ui.rs` read/write sites to speak
`Username` instead of raw `String`/`&str`, collapsing the
stringify→read→re-parse round-trip (and dissolving #503's sidebar
`parse::<Username>()`). The localStorage JSON is unchanged.

**Scope.**

- **In:** `web/src/auth/marker.rs` (codec + host tests),
  `web/src/auth/marker_storage.rs` (get/set), `web/src/pages/ui.rs` (`owner`,
  `marker_username_on_boot`, `authed_sidebar`, `marker_matches`, reconcile
  Effect), and the two component SET sites `web/src/auth/component.rs`,
  `web/src/registration/component.rs`.
- **Out:** `client::storage` (ADR-0069, stays generic); the `<head>` pre-paint
  script + `csr/index.html`; marker semantics.

**Tasks (one line each).**

1. Retype the codec → binding → `ui.rs` sites → SET sites to `Username`; update
   the codec host tests. (Atomic — one commit; the codec retype cascades to all
   callers, which must compile together.)

**Key risks / decisions.**

- **JSON contract is byte-stable.** `encode_marker` must emit the exact same
  `{"username":"…"}` for a given username as today — only the parameter type
  changes (`&str` → `&Username`, formatted via `Display`/`AsRef`). Guarded by
  the codec round-trip test and the pre-paint-script drift-guard test.
- **`decode_marker` parse is the one malformed→`None` chokepoint.** Malformed
  JSON **or** an invalid-username string → `None` (via `Username::from_str`).
  The old `.is_empty()` guard is subsumed. Add a host test for an
  invalid-username payload.
- **Everything but the codec is wasm-only.** `marker.rs` (cfg-free) compiles on
  host and is where the tests live; `marker_storage.rs` + `ui.rs` sites are
  wasm-only — verify with the gate's `--all-features --all-targets`
  (wasm-clippy) build.
- **Coverage.** `marker.rs` is host-testable (pure) → its new branches are
  coverage-measured; cover round-trip + both malformed paths. The wasm-only
  binding/`ui.rs` sites are `#[component]`/wasm and not host-coverage-measured.

## Global constraints

- Rust; structured edits. No `Co-Authored-By`. Gate: `cargo xtask check` clean
  before commit. Review base `wt-base-issue-542`; diff `git diff main...HEAD`.

---

## Task 1 — retype marker codec + binding + sites

**Files / interfaces** (exact edits derived at implementation from current
bytes; signatures per the spec's Decision):

- `web/src/auth/marker.rs`
  - `encode_marker(username: &Username) -> String` (same JSON body; format the
    newtype's `&str`).
  - `decode_marker(raw: &str) -> Option<Username>` — parse the JSON's `username`
    field via `Username::from_str(...).ok()`; drop the `.is_empty()` guard.
  - Host tests: build `Username` via `common::test_support::parse_username(...)`
    (or the crate's convention); assert
    `decode_marker(&encode_marker(&u)) == Some(u)`, JSON-shape stability,
    malformed-JSON → `None`, and invalid-username → `None`.
- `web/src/auth/marker_storage.rs`
  - `get() -> Option<Username>` (delegates to `decode_marker`).
  - `set(username: &Username)` (delegates to `encode_marker`).
- `web/src/pages/ui.rs`
  - `owner: RwSignal<Option<Username>>`;
    `marker_username_on_boot() -> Option<Username>`.
  - `authed_sidebar(active_key: &str, username: &Username, is_operator: bool)`:
    drop the `let username = username.to_string();` + `parse::<Username>()`;
    footer avatar `<Avatar name=username.clone() size=28 />`; label `{username}`
    → render via `Username` `Display` (e.g. `{username.to_string()}` only if the
    view needs an owned string, else `AsRef`/`Display` as the codebase does
    elsewhere for a newtype label).
  - `marker_matches(author: &Username) -> bool` —
    `get().as_ref() == Some(author)` (or `get() == Some(author.clone())`); call
    site `marker_matches(&post.username)`.
  - Reconcile Effect: `owner.set(Some(u.clone()))` / compare `Username`
    directly; `marker_storage::set(&u)` (drop `.to_string()`).
- `web/src/auth/component.rs`, `web/src/registration/component.rs`:
  `marker_storage::set(&input.username)` (drop `.as_ref()`).

**Verify**

- `cargo xtask check` — expect PASS (compiles wasm + host
  `--all-features --all-targets`; clippy clean; coverage clean).
- `cargo nextest run -p web marker` — codec round-trip + malformed tests PASS.
- Grep the diff: no `.parse::<Username>()` / `.as_ref()` / `.to_string()` on the
  marker value; `get`/`set`/codec signatures name `Username`.

**Commit** (after gate green — **jaunder-commit**; `git status` clean of
hook-restaged files first).

## Self-review checklist

- [ ] Every spec AC (1–6) maps to Task 1.
- [ ] localStorage JSON byte-stable (round-trip + drift-guard tests green).
- [ ] No marker stringly round-trip left; #503 sidebar `parse` gone.
- [ ] `cargo xtask check` green; `git status` clean.
