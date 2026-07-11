# Spec — issue #370: `#[client_only]` coverage exemption for client-only reactive helpers

**Issue:** jaunder-org/jaunder#370 · **Milestone:** Code quality ratchet ·
**Surfaced by:** #359

## Problem

The stateless coverage gate exempts `#[component]` bodies — client-only reactive
code that renders only in the browser and cannot run in a host test — via a
**syntactic** rule in `xtask/src/coverage/exempt.rs` (it matches the `component`
attribute path on a free function). But client-only reactive code also lives in
**methods that aren't components**: `web::reactive::Invalidator`'s `resource` (a
`server_resource` that panics host-side without the async executor), `action`
(whose gating `Effect` runs only in the browser), and their peers `patched` and
`sticky` (each builds a `server_resource` and/or an `Effect`). All four are
exercised by the audiences e2e, not host tests. Today they sit under a single
hand-written `// cov:ignore-start` / `// cov:ignore-stop` block
(`web/src/reactive.rs:45`–`152`) — a scattered comment suppression rather than a
principled, framework-recognized rule.

## Goal

Introduce a principled coverage exemption for client-only reactive helpers — a
real `#[client_only]` attribute the coverage framework recognizes — generalizing
the `#[component]` rule to methods. Mark all four `Invalidator` helpers with it
and delete the `cov:ignore` block. The gate stays green with no new suppression.

## Design decisions

### D1 — `#[client_only]` is a real attribute backed by a new `macros` proc-macro crate (not a `macro_rules!` block)

Stable Rust has no way to attach a custom inert attribute to a method for free,
so `#[client_only]` must be backed by a proc-macro, and a proc-macro crate
(`[lib] proc-macro = true`) can hold **only** proc-macros — no runtime code,
host-compiled, loaded by rustc at build time. So it lives in a new dedicated
workspace crate. It is named **`macros`** — a general home for the workspace's
proc-macros, following the terse, unprefixed local convention (`common`, `host`,
`web`), deliberately **not** narrowed to `web-`/`client-`/`coverage-`: the
attribute is build-time tooling, not client runtime nor coverage-specific, and
future workspace proc-macros (for any purpose) belong here too. `#[client_only]`
is its **first tenant** — a single **identity** attribute macro:

```rust
#[proc_macro_attribute]
pub fn client_only(_attr: TokenStream, item: TokenStream) -> TokenStream { item }
```

It expands to the annotated item **unchanged**; its only effect is to be a
syntactic marker the coverage framework reads from source (`syn` sees the
attribute before expansion) — a peer of the comment markers (`cov:ignore`,
`crap:allow`) that needs a real crate only because a _custom attribute_ must be
macro-backed. It accepts and ignores any argument tokens, so both
`#[client_only]` and `#[client_only(...)]` compile — mirroring how the framework
already matches both `#[component]` and `#[component(...)]`.

Rejected alternative: a `client_only! { … }` `macro_rules!` block (no new crate,
reuses the existing `visit_macro` recognition path). Rejected because rustfmt
does not format method bodies inside a macro invocation and it is a block
marker, not the per-method attribute the issue calls for.

Rejected names: `web-macros` (implies web-only ownership — a future ADR-0058
`client` crate depending on a `web-*` crate reads backwards); `client-macros` (a
proc-macro is host build-time tooling, not client runtime, so `client-`
mis-scopes it); `coverage-macros` (too narrow — the crate should welcome
non-coverage macros); `jaunder-macros` (no crate in this workspace carries the
`jaunder-` prefix).

### D1a — record the `macros` crate charter in an ADR

Introducing a new shared workspace crate with a charter ("the home for workspace
proc-macros") is the same kind of structural decision ADR-0058 recorded for
`host`. A short ADR (drafted via the `jaunder-adr` flow, numbered at ship)
records it, situated in the crate-layering family (ADR-0055/0056/0058): `macros`
is an orthogonal, target-agnostic _build-time_ crate, distinct from the
`common`/`host`/`client` _runtime_ trio.

### D2 — the exemption recognizes `client_only` on both free functions and methods

`exempt.rs` currently exempts only `#[component]` free functions
(`visit_item_fn`). Components are always free functions, but the client-only
helpers are **methods** (`ImplItemFn`). The visitor therefore gains an
`ImplItemFn` arm, and the attribute predicate generalizes from "is `component`"
to "is `component` **or** `client_only`", applied uniformly in **both** the
free-function and method arms. A `#[client_only]` marker exempts the whole item
— signature **and** body spans — exactly as `#[component]` does (the signature
lines of a client-only method are equally un-exercised host-side; over-exempting
non-executable signature lines is harmless).

### D3 — the A1 guard generalizes for free; no gate change

`gate::evaluate` computes exemption set-membership per line and flags any
_covered_ line inside an exempt span as an A1 `guard_violation`, independent of
_which_ rule produced the exemption. So the guard's self-enforcing safety — if a
host test ever actually exercises a `#[client_only]` helper, the gate fails,
signalling the exemption's premise is now false — applies to `#[client_only]`
spans with **no change to `gate.rs`**. (This is the same
interim-until-wasm-bindgen-test property the issue notes for these helpers.)

### D4 — no new coverage or CRAP surface from the proc-macro crate

`devtool coverage emit` runs `cargo llvm-cov nextest` over the whole workspace.
A proc-macro crate is a separate `.so` loaded by rustc at **build** time and is
not linked into the instrumented **test** binaries, so it is expected to
contribute no gate-failing line: `macros` most likely does not appear in
`coverage-report.txt` at all, and if it does its identity fn ran at build time
(merged as _covered_, benign) rather than as a 0-count line. This is **not
assumed** — A5 verifies the gate outcome empirically, and the documented
fallback (a single `cov:ignore` on the identity fn) handles the unlikely 0-count
case.

### D5 — user-facing gate messaging names the new rule

`exempt.rs`'s module docs and the failure-report guidance in `coverage/mod.rs`
that today say "#[component]" are generalized to name `#[client_only]` too, so a
gate failure's "uncovered / revisit-the-exemption" guidance is accurate.
Existing asserted substrings in tests are preserved.

### D6 — mark all four helpers

`resource`, `action`, `patched`, and `sticky` all carry `#[client_only]`; the
`cov:ignore-start`/`cov:ignore-stop` pair and its explanatory comment are
removed. All four are genuinely client-only, matching exactly what the existing
block covered — nothing regresses to measured-but-uncovered.

## Acceptance criteria

- **A1.** A new `macros` workspace crate (added to root `Cargo.toml` `members`;
  `[lib] proc-macro = true`; **no external deps** — only the built-in
  `proc_macro` crate) exports a `#[client_only]` identity proc-macro attribute;
  annotating an item with it is a no-op — the item compiles and behaves
  identically to unannotated (verified transitively by compilation + the
  audiences e2e in A6, since the helpers are client-only and have no host-test
  instantiation). `web/Cargo.toml` gains a `macros = { path = "../macros" }`
  dependency.
- **A1a.** A draft ADR (via `jaunder-adr`, numbered at ship) records the
  `macros` crate's charter — the workspace home for proc-macros, a
  target-agnostic build-time crate distinct from the `common`/`host`/`client`
  runtime trio — situated in the crate-layering family (ADR-0055/0056/0058).
- **A2.** `xtask/src/coverage/exempt.rs` exempts an item (signature + body)
  carrying `#[client_only]` on **both** a free function and a method, and
  continues to exempt `#[component]`; a plain (unmarked) method stays measured.
  Recognition is path-anchored (`#[my::client_only_thing]` does not match) and
  fail-closed (unparseable source → nothing exempt), consistent with the
  existing rules. New unit tests in `exempt.rs` assert: exempts a
  `#[client_only]` method body, exempts a `#[client_only]` free fn, does not
  exempt an unmarked method, and (path-anchoring) does not match a non-ident
  path. The `exempt.rs` module docs and the `coverage/mod.rs`
  `failure_report`/`CoverageReport` guidance that today name only `#[component]`
  are generalized to name `#[client_only]` too (D5), preserving the substrings
  the `mod.rs` tests assert (`cov:ignore`, `crap:allow`,
  `revisit the exemption`).
- **A3.** `web/src/reactive.rs` imports the attribute
  (`use macros::client_only;`) and its `resource`, `action`, `patched`, and
  `sticky` methods each carry the **bare-ident** `#[client_only]` (path-anchored
  recognition requires the bare ident — a `#[macros::client_only]` path would
  compile but not be recognized, leaving the bodies measured-but-uncovered →
  gate failure); the `// cov:ignore-start` / `// cov:ignore-stop` block and its
  comment are deleted. No `cov:ignore` marker remains in `web/src/reactive.rs`.
- **A4.** `macros` is admitted by the Nix flake source (its `.rs` + `Cargo.toml`
  are cargo sources; new files are `git add`ed so the flake sees them). No
  `flake.nix` edit is required for admission; if the full gate reveals
  otherwise, that edit is in scope.
- **A5.** No `macros` line **fails** the coverage gate and no `macros` **CRAP
  failure** is reported (verified by running the gate — the check is the gate
  _outcome_, not mere presence in the report: a build-time-covered line or an
  under-threshold CRAP entry is benign). If a 0-count `macros` line does fail
  the gate, the fallback is a single infra-level exclusion (a `cov:ignore` on
  the identity fn, or a coverage-source exclusion), documented as such; the
  reactive-helper suppressions are removed either way.
- **A6.** The full local gate — `cargo xtask validate` (static + clippy on host
  and wasm + coverage + e2e) — is green. In particular the audiences e2e (which
  exercises the four helpers) still passes, and the coverage gate reports 0
  failures and 0 guard violations.

## Out of scope

- Covering these helpers for real via `wasm-bindgen-test` in a headless browser
  (the Test-infra epic); `#[client_only]` is the interim exemption until then.
- Adopting `#[client_only]` on any client-only code beyond the four
  `Invalidator` helpers.
- Any change to `gate.rs`, the CRAP threshold, or the `unreachable!` rule.
