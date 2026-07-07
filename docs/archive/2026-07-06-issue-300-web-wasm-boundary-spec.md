# Spec — issue #300: module-level host/wasm boundary in `web`

**Issue:** jaunder-org/jaunder#300 **Cycle scope decision:** Direction **A**
only (module-level boundary, one `web` crate). Direction **B** (crate split) is
filed as a separate follow-on issue. **Server-fn arg restructuring** (the 2
`too_many_arguments` suppressions) is **deferred to #299**, not done here.

## Problem

`web` compiles for two targets (host + wasm32). The host build is load-bearing
and stays (server binary's projector uses `web::render`; `#[server]` fn bodies;
30+ host unit tests of pure page logic; the coverage gate compiles the client
feature on the host; host clippy). The smell is not the dual-target compile — it
is that the boundary is enforced **line by line** inside host-compiling
`#[component]` UI modules:

- **20 line-level gates** — `#[cfg(not(target_arch = "wasm32"))]` stub arms and
  `#[cfg_attr(not(target_arch = "wasm32"), allow(unused_variables))]` shims —
  across
  `web/src/pages/{posts,upload,audiences,signal_read,home,ui,cockpit}.rs`, each
  carrying `cov:ignore` noise.
- A **fake-value host stub**: `local_datetime_to_utc_rfc3339`
  (`web/src/pages/ui.rs`) returns `Some(trimmed.to_string())` on the host
  instead of the browser's real local→UTC conversion. There is **already a host
  test** (`ui.rs` ~line 1466) asserting that fake arm — the "test passes while
  verifying a branch that never ships" hazard, realized.

### Corrections to the issue body (survey re-run 2026-07-06, per the issue's own instruction)

- The `must_use_candidate` suppressions the issue describes as "~45 item-level
  `#[allow]`s, no crate-wide disable" **no longer exist**. #94 (merged after the
  issue was written) added a crate-wide `must_use_candidate = "allow"` to
  `web/Cargo.toml` and removed the item-level allows. **Out of scope here.**
- The only **boundary-relevant** suppressions left in `web` are the **2
  `#[allow(clippy::too_many_arguments)]`** on the `create_post`/`update_post`
  `#[server]` fns (`web/src/posts/mod.rs:222,402`). Those fns **do not move**
  under Direction A, and a dedicated issue (#299) owns the arg-list
  restructuring, which also carries the `input = Json` wire-shape constraint.
  **Deferred to #299; #300's acceptance is amended to remove them.** (`web` also
  carries conventional test-scoped `#![allow(clippy::unwrap_used, expect_used)]`
  in four test modules and one `#[expect(clippy::needless_pass_by_value)]` on a
  Leptos component prop at `ui.rs:299` — these are policy-compliant keepers,
  untouched here.)

## Approach (Direction A)

Relocate the pure logic stranded in `pages/` into host-compiled homes **first**,
then gate the whole `pages` module (and its `lib.rs` re-exports)
`#[cfg(target_arch = "wasm32")]`. Server fns and `render` stay put. Feasibility
is confirmed: the host build (`server` binary + `server` tests) references only
`web::render`, `web::posts`, `web::auth`, `web::tags`, `web::media` — **never
`web::pages`** — so gating `pages` wasm-only does not break the host compile.
The wasm bundle still compiles `pages` (its `target_arch` _is_ wasm32).

Note on the ADR-0040 projector: the server **never mounts the Leptos `App`** —
it rebuilds public markup from `web::render`. So `pages` components never
actually SSR, and host arms that read "runs on the server where nothing updates
it" (e.g. the `read_signal!` `.get_untracked()` arm at
`pages/signal_read.rs:16`) are **compile-appeasement for the dual-target
build**, not live server behavior; they are safe to drop once `pages` is gated
wasm-only.

### Relocation map

Two kinds of relocation — do not conflate them:

**(a) Pure logic → host home (keep host-tested + coverage-measured):**

| Item                                          | From             | To                                                                                                         | Tests                                                                |
| --------------------------------------------- | ---------------- | ---------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `is_valid_tag_slug`, `normalize_tag_token`    | `pages/ui.rs`    | `web::tags` (both-target domain module; keeps them out of `common` per the existing doc-comment rationale) | move the `is_valid_tag_slug` / `normalize_tag_token` tests with them |
| `format_bytes`                                | `pages/media.rs` | `web::render` (joins the other pure display formatters)                                                    | move its 4 tests                                                     |
| `avatar_parts` / `format_post_time` **tests** | `pages/ui.rs`    | `web::render` (the fns already live there)                                                                 | move the tests to sit with their subjects                            |
| `default_theme_is_nonempty` test              | `pages/mod.rs`   | `web::render` (`DEFAULT_THEME` lives there)                                                                | move the test                                                        |

**(b) Genuinely browser-only logic → wasm-only, delete the fake host arm and its
host test (verification story becomes the e2e matrix):**

- `local_datetime_to_utc_rfc3339` (`pages/ui.rs`): delete the
  `#[cfg(not(target_arch = "wasm32"))] { Some(trimmed.to_string()) }` fake arm;
  the fn becomes wasm-only (empty-guard + `js_sys::Date`). Its three host tests
  (`ui.rs:1455-1469`) all go: `local_datetime_host_arm_returns_trimmed_input`
  asserts the fake arm; and `local_datetime_empty_or_blank_is_none`, though it
  asserts the _real_ both-target empty-guard (`trimmed.is_empty() -> None`, not
  the fake arm), also goes because the whole fn is now wasm-only. The guard is a
  one-line early-return not worth extracting into a separate host helper; its
  verification folds into the wasm-only fn's e2e story. (This is the one
  legitimate pure test lost — called out so it isn't a silent coverage drop.)
- `marker_matches` (`ui.rs:368`), `marker_username_on_boot` (`ui.rs`), and the
  other `#[cfg(not(target_arch = "wasm32"))]` host-`false`/`None` stubs in
  `pages/` become wasm-only; their host stub-tests (`marker_matches_*`,
  `marker_username_on_boot_is_none_off_wasm` at `ui.rs:1479`) are deleted (they
  assert non-shipping host values). The pure marker encode/decode already lives
  host-tested in `web::auth::marker` and is untouched.

### Measured-line audit (before gating `pages`)

The relocation map above is driven by _tests_, but constraint 4 is about
_measured lines_. `pages/` also holds currently-host-compiled-and-measured pure
helpers that have **no** unit test (e.g. `TimelineState::adopt` /
`TimelineState::default` at `pages/timeline.rs`), which would silently leave the
coverage-measured set when `pages` goes wasm-only. So the work includes an
explicit audit: enumerate every `pages/` line that is host-measured today, and
for each either (a) relocate it to a host home if it is pure non-glue logic, or
(b) record it as genuinely wasm-only reactive/UI glue whose verification is the
e2e matrix. The audit's output is a short written list (in the plan or PR
description) of measured lines leaving the set and why each is glue — this is
the concrete evidence backing AC#5, since "genuinely wasm-only glue" is
otherwise a judgment call with no mechanical before/after check.

### Boundary move

- `web/src/lib.rs`: `#[cfg(target_arch = "wasm32")] pub mod pages;` and gate the
  `pub use pages::{…}` / `pub use pages::App` re-exports the same way (only
  `mount_csr`'s wasm body consumes them).
- Once `pages` is wasm-only, every `#[cfg(not(target_arch = "wasm32"))]` arm and
  `cfg_attr(not(target_arch = …), allow(unused_variables))` shim **inside**
  `pages/` is deleted — the module-level gate subsumes them.

### Wasm-target clippy in the gate

Host clippy no longer sees `pages/`, so add a wasm-target clippy pass
(`cargo clippy --target wasm32-unknown-unknown` over the client feature) to
`cargo xtask` (`check` + `validate`) and the corresponding Nix check, so
wasm-only code is still linted. The wasm32 target must be available in the gate
toolchain (cargo-leptos already builds wasm, so it should be present — confirm
in the plan).

## Acceptance criteria (observable)

1. `rg 'cfg\(not\(target_arch = "wasm32"\)\)|cfg_attr\(not\(target_arch' web/src/pages`
   returns **no matches** — no host-stub arms or `unused_variables` shims remain
   in the UI modules.
2. No fake-value host stub remains in `web`: `local_datetime_to_utc_rfc3339` has
   no `#[cfg(not(target_arch = "wasm32"))]` arm returning a substitute value; no
   other fn in `web` returns a divergent host placeholder.
3. `web/src/lib.rs` declares `pages` as `#[cfg(target_arch = "wasm32")]`
   (module + re-exports), and the host build (`cargo build -p web`, default +
   `server` features) and `cargo build -p server` both compile.
4. Every pre-existing **pure** host test survives in its new location and
   passes: the `is_valid_tag_slug`, `normalize_tag_token` tests (now under
   `web::tags`), the `format_bytes` tests (now under `web::render`), the
   `avatar_parts` / `format_post_time` / `DEFAULT_THEME` tests (now under
   `web::render`). The only host tests deleted are those tied to a wasm-only fn:
   the `local_datetime_*` trio (both the fake-arm test and the empty-guard test,
   since the whole fn is now wasm-only — see the map) and the `marker_*`
   host-stub tests.
5. The coverage-measured line set does not shrink except for lines that are
   genuinely wasm-only UI glue. Evidenced two ways: (a) the measured-line audit
   above produces a written list of every `pages/` measured line leaving the set
   with a one-line glue/relocation disposition for each, and (b)
   `cargo xtask validate`'s coverage gate stays green.
6. A wasm-target clippy pass is part of `cargo xtask check` and
   `cargo xtask validate` (and the Nix gate), and passes clean.
7. No new lint suppressions are introduced. The 2 `too_many_arguments` allows
   are left in place (their removal is #299's).
8. `cargo xtask validate` is green (static + clippy + coverage + e2e).
9. An ADR records the module-level-boundary decision and the deferral of the
   crate split (Direction B) to its follow-on issue.
10. Issue tracker reconciled: #300's acceptance amended to scope out the
    `too_many_arguments` work; #299 noted as the owner of it; Direction B filed
    as a new follow-on issue (blocked-by / relates as appropriate).

## Out of scope

- The crate split (Direction B) — filed separately.
- The `#[server]` arg-list restructuring / `too_many_arguments` removal — #299.
- The `must_use_candidate` suppressions — already resolved by #94.
- Any behavior change to the shipping wasm client or the server; this is a
  boundary/relocation refactor, verified behavior-preserving by the existing
  host tests + the e2e matrix.
