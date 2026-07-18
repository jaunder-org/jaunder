# `client` crate scaffolding (#513) Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with
> **jaunder-iterate** (delegating an individual task to a subagent via
> **jaunder-dispatch** when useful). Steps use checkbox (`- [ ]`) syntax for
> tracking.

**Goal:** Scaffold an empty, wasm-only `client` workspace crate (the symmetric
peer of `host`) plus its build/gate wiring and a charter ADR, so Milestone 14's
follow-on issues have a home to relocate browser glue into.

**Architecture:** New leaf `rlib` crate `client/`, gated entirely behind
`#![cfg(target_arch = "wasm32")]` so it is an empty rlib on host (zero
coverage-measured lines) and active only on wasm. No code is relocated here and
`web` is untouched — the first inhabitant and the `web → client` edge land in
#514. The wasm-clippy static-check step (client's only meaningful compile gate)
is extended to lint `-p client`, mirrored in the flake and locked by a unit
test.

**Tech Stack:** Rust (cargo workspace, resolver v2), xtask static-check driver,
Nix/crane flake checks.

Spec:
[`2026-07-17-issue-513-client-crate-scaffolding.md`](../specs/2026-07-17-issue-513-client-crate-scaffolding.md).
Read it for the _what/why_; this plan is the _how_. Acceptance criteria are
referenced below as **AC1**–**AC10**.

## Global Constraints

- **Empty scaffolding only.** No primitive is relocated; `web/` source is
  unchanged; no `web → client` dependency edge (AC8). Nix source filters and
  cargo-leptos/csr wiring are untouched (auto-admit; nothing names `client`).
- **Crate mirrors `host`** (AC2): `name = "client"`, `version = "0.1.0"`,
  `edition = "2021"`, `license = "GPL-3.0-only"`, `[lints] workspace = true`, no
  `[lib]` block (default `rlib`), **empty `[dependencies]`** (no browser deps —
  added by follow-on issues when first used).
- **ADR draft already authored** at
  `docs/adr/0069-client-crate-wasm-only-home.md` (AC9), its content validated
  against AC9's six sub-points (0058 activation, 0056 reconciliation, 0055
  rules, charter, coverage position, gate wiring) in the spec soundness review.
  It is gitignored (draft-out-of-git flow) and is **not** committed by this plan
  — `cargo xtask adr promote` numbers it and rewrites its path-form references
  (incl. the one in `client/src/lib.rs`) at ship (**jaunder-ship**), which
  re-checks the content in its conformance review. No iterate task touches it.
- **Commit discipline:** run `cargo xtask check` **foreground** first
  (`timeout: 600000`) so it passes clean before committing (**jaunder-commit**);
  serialize edit → gate → commit (no edits mid-gate). **No `Co-Authored-By`
  trailer.** Adding a new member (no new external deps) may trigger a moderate
  workspace rebuild but **not** a full cold vendor rebuild.

---

## Review header — one line per task

- **Task 1 — Create `client` crate + workspace wiring.** New `client/Cargo.toml`
  - `client/src/lib.rs`; add `"client"` to root `members` and
    `[workspace.dependencies]`. Verify empty-on-host build + coverage gate
    (AC1–5).
- **Task 2 — Wire + lock the wasm-clippy gate.** Add `-p client` to the
  `wasm-clippy` step (xtask) with an arg-lock unit test, mirror in `flake.nix`
  (both positions), verify the invocation empirically, confirm the full local
  gate (AC6, AC7, AC10).

**Key risks/decisions:**

- The `cargo clippy -p web -p client --features csr` invocation is the one
  gate-breaking claim — resolver-v2 binds `csr` to `web` and leaves `client`
  (featureless) alone. **Task 2 runs it empirically**; fallback is
  `--features web/csr`.
- Flake mirror carries `-p web --features csr` in **two** spots
  (`cargoExtraArgs` L1120 _and_ `cargoClippyExtraArgs` L1126) — both get
  `-p client` in one commit to keep the deps-cache and clippy run in sync.
- No `wasm-clippy` arg-lock test exists today; Task 2 adds one so `-p client`
  can't silently drop.

---

## Task 1: Create the `client` crate and wire it into the workspace

**Files:**

- Create: `client/Cargo.toml`
- Create: `client/src/lib.rs`
- Modify: `Cargo.toml` (root) — `members` (lines 3–12) and
  `[workspace.dependencies]` (lines 26–29)

**Interfaces:**

- Consumes: nothing (leaf crate, empty deps).
- Produces: workspace member `client` (path `client`), consumable by later
  issues via `client.workspace = true`. No public Rust items (empty on both
  targets in this issue).

No unit test: the crate is empty, so there is no behavior to pin — verification
is the build + coverage gate (the crate's contract here is "compiles empty on
host, compiles on wasm, does not fail the coverage gate").

- [ ] **Step 1: Create `client/Cargo.toml`** (mirror `host/Cargo.toml`, empty
      deps)

```toml
[package]
name = "client"
version = "0.1.0"
edition = "2021"
license = "GPL-3.0-only"

[dependencies]

[lints]
workspace = true
```

- [ ] **Step 2: Create `client/src/lib.rs`** (charter doc + crate-level cfg
      gate, no items)

```rust
//! `client` — strictly-client (wasm/browser) shared infrastructure.
//!
//! The symmetric wasm peer of `host`: holds only raw browser glue
//! (`web_sys` / `js_sys` / `wasm_bindgen` / wasm-side leptos plumbing) and
//! never our domain types. Depends on no workspace crate except `common`
//! (+ `macros`). `web`/`csr` depend on `client`, never the reverse.
//!
//! Wasm-only: the crate-level `#![cfg(target_arch = "wasm32")]` below makes it
//! an empty rlib on the host target (zero coverage-measured lines) and active
//! only on wasm. Every module relocated here inherits that gate, so it needs no
//! per-item `#[cfg]` and no `#[client_only]` marker.
//!
//! See docs/adr/0069-client-crate-wasm-only-home.md.
#![cfg(target_arch = "wasm32")]
```

- [ ] **Step 3: Add `client` to the workspace** in root `Cargo.toml`

In `[workspace] members` (line 3), add `"client"` as the **first** entry
(alphabetically `"client"` < `"common"`):

```toml
members = [
  "client",
  "common",
  "csr",
  "host",
  "macros",
  "server",
  "storage",
  "test-support",
  "web"
]
```

In `[workspace.dependencies]` (line 26), add the path dep before `common` (keeps
the path-deps alphabetical):

```toml
[workspace.dependencies]
client = { path = "client" }
common = { path = "common" }
host = { path = "host" }
storage = { path = "storage" }
```

- [ ] **Step 4: Verify the crate builds empty on both targets**

Run: `cargo build --workspace` Expected: PASS — `client` compiles as an empty
rlib on host (AC5).

Run: `cargo build -p client --target wasm32-unknown-unknown` Expected: PASS —
the cfg gate compiles clean on wasm (empty crate).

- [ ] **Step 5: Run the full check gate (foreground) to confirm coverage
      tolerates the empty crate**

Run (foreground, `timeout: 600000`): `cargo xtask check` Expected: PASS —
static + clippy + Nix coverage all green; `client` contributes **zero**
coverage-measured lines and does **not** fail the coverage gate (AC5). Inspect
`.xtask/last-result.json` `steps[]` if any step is `ok:false`.

- [ ] **Step 6: Commit**

```bash
git add client/Cargo.toml client/src/lib.rs Cargo.toml
git commit -m "feat(client): scaffold empty wasm-only client crate (#513)"
```

(The pre-commit hook re-runs `cargo xtask check`; Step 5 having passed, it lands
clean. AC1–AC5 satisfied.)

---

## Task 2: Extend and lock the wasm-clippy gate; mirror in the flake

**Files:**

- Modify: `xtask/src/steps/static_checks.rs` — the `wasm-clippy` `StepSpec`
  (lines 69–88) and a new arg-lock test in the `#[cfg(test)] mod tests` block
  (after line 207)
- Modify: `flake.nix` — the `wasm-clippy` derivation, `cargoExtraArgs`
  (line 1120) and `cargoClippyExtraArgs` (lines 1125–1127)

**Interfaces:**

- Consumes: the `client` member from Task 1 (so `-p client` resolves).
- Produces: the wasm-clippy gate lints `client` on the wasm target; the arg-lock
  test guarantees `-p client` stays in the step.

- [ ] **Step 1: Write the failing arg-lock test** in `static_checks.rs`
      `mod tests` (mirrors `xtask_clippy_denies_warnings_in_both_modes`,
      asserting the full arg vector so `-p client` and its position are pinned)

```rust
#[test]
fn wasm_clippy_lints_web_and_client() {
    for mode in [Mode::Check, Mode::Fix] {
        let s = specs(mode);
        let wasm_clippy = find(&s, "wasm-clippy");
        assert_eq!(wasm_clippy.program, "cargo");
        assert_eq!(
            wasm_clippy.args,
            [
                "clippy",
                "-p",
                "web",
                "-p",
                "client",
                "--features",
                "csr",
                "--target",
                "wasm32-unknown-unknown",
                "--",
                "-D",
                "warnings",
                "-A",
                "clippy::too_many_arguments",
                "-A",
                "unfulfilled_lint_expectations",
            ]
        );
    }
}
```

- [ ] **Step 2: Run the test, verify it fails**

Run:
`cargo nextest run --manifest-path xtask/Cargo.toml wasm_clippy_lints_web_and_client`
Expected: FAIL — the step still carries only `-p web` (assertion mismatch).

(xtask is excluded from the workspace — always use
`--manifest-path xtask/Cargo.toml`, not `-p xtask`.)

- [ ] **Step 3: Add `-p client` to the `wasm-clippy` `StepSpec`**

In the `args` vec (lines 72–87), insert `"-p", "client"` immediately after
`"-p", "web"`:

```rust
args: vec![
    "clippy",
    "-p",
    "web",
    "-p",
    "client",
    "--features",
    "csr",
    "--target",
    "wasm32-unknown-unknown",
    "--",
    "-D",
    "warnings",
    "-A",
    "clippy::too_many_arguments",
    "-A",
    "unfulfilled_lint_expectations",
],
```

- [ ] **Step 4: Run the test, verify it passes**

Run:
`cargo nextest run --manifest-path xtask/Cargo.toml wasm_clippy_lints_web_and_client`
Expected: PASS.

- [ ] **Step 5: Verify the wasm-clippy invocation empirically** (the one
      gate-breaking claim — proves resolver-v2 accepts `--features csr` across
      `-p web -p client` when only `web` has the feature)

Run:
`cargo clippy -p web -p client --features csr --target wasm32-unknown-unknown -- -D warnings -A clippy::too_many_arguments -A unfulfilled_lint_expectations`
Expected: PASS (client is empty → nothing to lint; web lints clean). AC6. If
cargo instead rejects the shared `--features csr`, fall back to
`--features web/csr` in both the StepSpec (Step 3) and the test (Step 1), and
note the deviation.

- [ ] **Step 6: Mirror the change in `flake.nix`** (both positions, one edit)

Line 1120 — `cargoExtraArgs`:

```nix
cargoExtraArgs = "-p web -p client --features csr";
```

Lines 1125–1127 — `cargoClippyExtraArgs`:

```nix
cargoClippyExtraArgs =
  "-p web -p client --features csr -- -D warnings "
  + "-A clippy::too_many_arguments -A unfulfilled_lint_expectations";
```

- [ ] **Step 7: Verify the flake `wasm-clippy` check builds** with the new arg

Run (background via Bash background mode or foreground `timeout: 600000` — a
wasm rebuild): `nix build .#checks.x86_64-linux.wasm-clippy -L` Expected: PASS —
the derivation evaluates and the wasm clippy check succeeds with
`-p web -p client` (AC7). (web's wasm build is cachix-warm; client adds an empty
crate.)

- [ ] **Step 8: Run the full check gate (foreground) then commit**

Run (foreground, `timeout: 600000`): `cargo xtask check` Expected: PASS — the
host `wasm-clippy` step now lints `client`, the new unit test passes, coverage
stays green.

```bash
git add xtask/src/steps/static_checks.rs flake.nix
git commit -m "build(client): lint client in the wasm-clippy gate (xtask + flake) (#513)"
```

- [ ] **Step 9: Confirm the full local gate** (AC10)

Run (foreground, `timeout: 600000`, or Bash background mode if long):
`cargo xtask validate --no-e2e` Expected: PASS — the pre-push-style gate
(static + clippy + coverage) is green on the branch. This is the final
acceptance for AC5 (coverage) and AC10.

---

## Self-review

- **Spec coverage:** AC1 (member) → T1 S3; AC2 (mirror host) → T1 S1; AC3 (cfg
  gate, no items) → T1 S2; AC4 (workspace dep) → T1 S3; AC5 (empty on host, no
  coverage failure) → T1 S4–5, T2 S9; AC6 (wasm-clippy lints client +
  empirical + arg-lock test) → T2 S1–5; AC7 (flake parity, both positions) → T2
  S6–7; AC8 (no scope creep — `web` unchanged) → Global Constraints, no task
  touches `web`; AC9 (ADR draft) → Global Constraints (authored, promoted at
  ship); AC10 (gate green) → T2 S9. All covered.
- **Placeholder scan:** none — every step carries real TOML/Rust/Nix/commands.
- **Type consistency:** step name `"wasm-clippy"`, package `client`, arg vector,
  and flake strings are consistent across T2 Steps 1/3/5/6 and the test.
