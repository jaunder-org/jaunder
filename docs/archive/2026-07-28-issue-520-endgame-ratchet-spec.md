# Issue #520 — web endgame ratchet

Lock in milestone #14's end state with compile-error enforcement, and retire the
interim exemption machinery.

## Current state (verified 2026-07-28, not assumed from the issue text)

The issue predates the vertical migrations (#315–#329, #526, #527, #530), so
much of what it asks for is already true:

- `js-sys` and `wasm-bindgen` are **already absent** from `web/Cargo.toml`.
  `web/src/media/component.rs:50,137` reach `JsCast` via `leptos::wasm_bindgen`.
- **Zero live `#[client_only]` uses.** The sole `rg` hit in `web/src`
  (`forms/field.rs:198-200`) is prose explaining why `Field<T>` is host-tested
  _instead of_ exempted.
- **Zero ungated `#[component]` functions.** 26 `component.rs` leaves carry
  `#[component]`, every one behind a
  `#[cfg(target_arch = "wasm32")] mod component;`. Four further files match
  `#[component]` in doc-comment prose only (`posts/mod.rs`, `media/mod.rs`,
  `tags/input_state.rs`, `tags/input_logic.rs`).
- `target_arch` in `web/src` is clean **except** `web/src/reactive.rs:64` and
  `:81`.
- `web-sys` remains a direct `web` dependency with **two** live consumers, both
  in `media/component.rs`: the file-picker glue (`:57-76`) and the
  click-to-select handler in `uploaded_url_view` (`:136-142`). Four of its eight
  declared features (`Window`, `Document`, `Element`, `Location`) have no
  consumer left.
- `web`'s `macros` dependency (`web/Cargo.toml:27`) is **already dead** —
  `rg 'macros' web/src` returns nothing. It survives only as the vestige of
  `#[client_only]`.
- Two inert `cov:ignore` markers survive at `web/src/backup/component.rs:50,184`
  — inert because a wasm-only file is never host-compiled.

## Decisions

### D1 — `web` drops both `web-sys` and `macros`

Both `web_sys` call sites move to `client` primitives, so `web` names no
`web_sys` type and the dependency is removed. A removed dependency makes
regression a compile error, which is the strongest available gate and what the
issue is after. Two primitives are needed, not one:

1. A file-picker primitive returning `server_fn::codec::MultipartData`
   (replacing `media/component.rs:57-76`).
2. A "select the clicked input's text" primitive over a leptos `MouseEvent`
   (replacing `media/component.rs:136-142`).

`macros` goes too: it is already unreferenced by `web/src`, and deleting
`#[client_only]` removes its last reason to exist there.

`MultipartData` is **not** a new dependency for `client`: `web` already reaches
it via `leptos::server_fn::codec` (`web/src/media/api.rs:14`), and `client`
already depends on `leptos` behind `csr`, so the path resolves today. What
`client` gains is an explicit `server_fn` manifest entry enabling the
`multipart` feature — required because `MultipartData` sits behind
`#[cfg(feature = "multipart")]` (`server_fn-0.8/src/codec/mod.rs:38-41`). That
feature is currently lit only by `web`'s manifest via cargo feature unification;
making it explicit removes a load-bearing accident.

**Acknowledged tension.** ADR-0070:119-124 charters `client` as the home for
**cross-vertical** browser primitives, and both new primitives serve only the
media vertical. We accept this and clarify the ADR rather than contort the
design: the operative test in ADR-0069 is _domain-freedom_ (raw browser glue, no
domain types), and "cross-vertical" describes the typical case rather than an
admission gate. A single-vertical primitive that is genuinely raw browser glue
belongs in `client`; the alternative — keeping `web-sys` alive in `web` for
twenty lines — forfeits the compile-error gate this issue exists to install. E2
records the clarification.

Rejected: keeping `web-sys` with a pruned feature list (achieves no gate); a
primitive returning `web_sys::FormData` (`web` would still have to name it for
the `.into()`).

**Two dependency-honesty fixes ride along**, both one-liners in files this issue
already edits:

- `web/Cargo.toml:66` lists `"leptos/ssr"` in the `server` feature. We do no
  server-side rendering (#487) — and the line is **redundant anyway**:
  `leptos_axum` hard-requires `leptos/ssr`
  (`leptos_axum-0.8.10/Cargo.toml:74-79`), so feature unification supplies it
  regardless. Removing it is behaviour-neutral and stops the manifest implying
  an SSR mode we don't have. Actually shedding the SSR stack means replacing
  `leptos_axum` — filed as **#677**, deliberately not folded in here.
- `client/Cargo.toml:20-23` justifies its optional `leptos` dep with the claim
  that leptos **forbids** `csr` alongside `ssr`. That could not be
  substantiated: leptos 0.8.20 has no `compile_error!` for the pair, and
  `leptos_macro`'s `csr = []` / `ssr = ["server_fn_macro/ssr"]` are not mutually
  exclusive at the Cargo level. The comment is rewritten to the reason we _can_
  evidence — `leptos_axum` requires `leptos/ssr`, so an unconditional `leptos`
  dep in `client` risks unifying `csr` into the server build. #677 carries the
  open question of whether the conflict is real.

### D2 — the `invalidator_scope!` macro moves to a gated leaf file

`web/src/reactive.rs` becomes `web/src/reactive/mod.rs` +
`web/src/reactive/scope.rs`. The macro and its own `pub(crate) use` move into
`scope.rs`, which then carries **no cfg of its own at all**. `mod.rs` declares
`#[cfg(any(target_arch = "wasm32", test))] mod scope;` and carries the paired
re-export `pub(crate) use scope::invalidator_scope;` under the **same** gate.

Putting the re-export in `mod.rs` is deliberate: under D3 a gated `use` in a
`mod.rs` is a permitted form, so `crate::reactive::invalidator_scope` keeps
working and the sole consumer needs no edit. The `test` arm of the gate is what
keeps the generated newtype host-tested and coverage-measured rather than
exempted.

**Both macro tests live in `mod.rs`, not `scope.rs`** — forced by
`unused_imports` (denied). A test inside `scope.rs` reaches the macro by
_textual_ scope, so it never consumes `scope.rs`'s `pub(crate) use`, leaving
that import unused on the host test build; gating it wasm-only would put a
`target_arch` cfg inside a leaf file, precisely what D3 forbids. Importing via
`super::invalidator_scope` from `mod.rs`'s tests instead keeps the whole
re-export chain consumed on host and on wasm, which is why both gate lines share
one gate rather than being asymmetric.

Rejected: `#[allow(unused_macros)]` with no cfg (trades a precise gate for lint
suppressions; `#[expect]` is unusable since the macro _is_ used on wasm);
deleting the macro for its single call site (pushes the newtype into wasm-only
code, regressing coverage against `reactive.rs:113-114`'s stated reasoning);
moving it to `client` (the macro expands to `$crate::reactive::Invalidator`, a
`web` type, which `client` may not name — and `client` is invisible to host
coverage).

### D3 — the placement check enforces three permitted forms

The issue's literal wording ("`target_arch` only on `mod` declarations") is not
achievable: ~15 verticals also gate the paired re-export, e.g.
`auth/mod.rs:43-44`
`#[cfg(target_arch = "wasm32")] pub use component::{LoginPage, LogoutPage};`.
Those are structurally required, not drift. The enforced rule is:

1. **Inner attribute on the file** (`#![cfg(... target_arch ...)]`) — allowed
   iff the file is `lib.rs`. This is the whole-crate gate `client` and `csr`
   use.
2. **Outer attribute on an item** — allowed iff the file is `mod.rs` or `lib.rs`
   **and** the item is `Item::Mod` or `Item::Use` (any visibility).
3. **Everything else** is a violation: an attribute on a `fn`/`struct`/`impl`/
   `macro_rules!`, or on a statement or expression inside a body.

Both halves of rule 2 are load-bearing. File-scope alone would permit a gated
`fn` in a `mod.rs`; item-scope alone would have passed `reactive.rs:81`
(`#[cfg(target_arch = "wasm32")] pub(crate) use invalidator_scope;` — a
legitimate-looking gated re-export in a leaf file).

Implemented with **syn**, not a line scan: the invariant is structural, and a
line scan cannot distinguish an attribute on an item from one inside a function
body. Doc comments that merely mention `target_arch` are excluded by anchoring
recognition on the attribute's **path** (`cfg` / `cfg_attr`) — syn models `//!`
and `///` as `#[doc = "…"]` attributes, so a token-text scan alone would flag
the eight module docs in `web/src` that quote the gate (`auth/mod.rs:9`,
`registration/mod.rs:12`, `profile/mod.rs:8`, `sessions/mod.rs:9`, and four
`component.rs` headers).

Policed roots: `web/src`, `client/src`, `csr/src`. `web` is the only crate whose
host/wasm boundary runs through it; `client`/`csr` pass immediately under form 1
and are policed to hold that line.

### D4 — no new ADR; four amended, plus CONTRIBUTING

The check enforces an existing decision rather than making a new one, matching
how `no_full_reload_check` enforces #592.

Retiring the `#[component]` exemption reaches further than the placement check
does. It is **Decision 1 of ADR-0050**, which is titled for it, and it is
documented as live policy in two `CONTRIBUTING.md` sections. This is an
amendment rather than a superseding ADR: ADR-0050's architecture — a stateless,
marker-based gate with a CRAP threshold — is unchanged; one exemption mechanism
became _unnecessary_ because components no longer host-compile at all.
ADR-0070:129-131 already foreshadows exactly this ("component lines leave the
host denominator entirely — not-compiled beats measured-but-exempt"), so the
amendment completes a transition the tree already made rather than reversing a
standing decision.

## Acceptance criteria

### A. Dependency removal

- **A1** `web/Cargo.toml` contains no `web-sys` and no `macros` dependency
  entry.
- **A2** `rg 'web_sys::' web/src` returns zero matches — no `web` code names a
  `web_sys` type. Remaining prose mentions of `web_sys` must be _accurate_ after
  the change: the "irreducible `web_sys` event touch stays inline in the
  component" claims at `tags/component.rs:7` and `tags/input_state.rs:5` are
  false once no such touch exists and must be corrected. (`lib.rs:46`'s "no
  `web_sys`" remains true and may stand.)
- **A3** `client` exposes **two** primitives — one producing `MultipartData`
  from the picked file, one selecting the text of a clicked input — and neither
  signature requires `web` to name a `web_sys` type.
- **A4** `client/Cargo.toml` declares `server_fn` with the `multipart` feature,
  gated to `csr`, rather than relying on `web`'s manifest via feature
  unification.
- **A5** `cargo clippy -p web --features server --all-targets -- -D warnings` is
  clean, so the server-gated `web` paths the issue's "Done when" names are
  actually linted. The standard ladder lints neither: `static_checks.rs:56`
  passes `--all-targets` with **default features only**, so everything behind
  `feature = "server"` is never compiled by it (#678).

  The issue's literal wording asks for a workspace `--all-features` build. We
  substitute per-crate feature selection **because it is the more precise tool**,
  not because `--all-features` is known to fail: it targets exactly the
  feature-gated code the criterion cares about, whereas `--all-features` would
  also light `leptos/csr` and `leptos/ssr` together across the workspace.
  Whether that combination actually breaks is **an open question** (#677) — A7
  demotes `client/Cargo.toml:20-23`'s "leptos forbids it" claim to unsubstantiated,
  so it would be incoherent to lean on it here. Either way the per-crate run
  satisfies the intent, and #678 owns settling the general case.
- **A6** `cargo xtask validate --no-e2e` is green, and the media-upload e2e
  passes on at least one `{backend}×{browser}` combo locally — it covers both
  relocated primitives. The full matrix is CI's job (ADR-0034).
- **A7** `"leptos/ssr"` is absent from `web/Cargo.toml`'s `server` feature, and
  the server build still compiles — proving the line was redundant
  (`leptos_axum` supplies it). `client/Cargo.toml:20-23`'s comment states the
  substantiated reason and no longer asserts that leptos forbids `csr` + `ssr`.

### B. `reactive` leaf split

- **B1** `web/src/reactive/` is a directory containing `mod.rs` and `scope.rs`.
- **B2** `scope.rs` contains no `target_arch` cfg attribute (a
  `#[cfg(test)] mod tests` is expected and fine); the gate lives on `mod.rs`'s
  `mod scope;` declaration as `#[cfg(any(target_arch = "wasm32", test))]`.
- **B3** `scope_newtype_derefs_to_its_invalidator` — the one test that exercises
  the macro — still runs on the host and still covers the generated newtype; no
  new exemption or `cov:ignore` is introduced.
  `notify_changes_the_tracked_revision` tests `Invalidator` itself and stays
  with it in `mod.rs`.
- **B4** `web/src/audiences/component.rs:13` is **unchanged** —
  `crate::reactive:: invalidator_scope` still resolves, via the gated re-export
  in `mod.rs`.

### C. The placement check

- **C1** A new xtask step exists and runs as part of the static ladder, i.e.
  `cargo xtask check --no-test` fails on a violation.
- **C2** It implements exactly the three forms of D3.
- **C3** Policed roots are `web/src`, `client/src`, `csr/src`; a missing/renamed
  root is a hard failure, so a moved tree cannot silently disable the guard
  (matching `no_full_reload_check::run`,
  `xtask/src/steps/no_full_reload_check.rs:74-80`).
- **C4** The pure helpers are unit-tested, including negatives that must **not**
  flag (a doc comment mentioning `target_arch`; an inner attribute in `lib.rs`;
  a gated `mod` and a gated `pub use` in a `mod.rs`) and positives that **must**
  flag (a gated `fn`; a gated `use` in a leaf file; an inner attribute in a
  non-`lib.rs` file; a cfg on a statement inside a function body).
- **C5** It demonstrably bites: a unit test asserts the _pre-fix_ `reactive.rs`
  shape (gated `macro_rules!` + gated `pub(crate) use` in a leaf file) is
  reported.
- **C6** It passes on the tree as shipped.

### D. Retirement

- **D1** `macros::client_only` and `macros/tests/identity.rs` are deleted.
- **D2** No live recognition of either attribute survives:
  `rg 'is_ident\("client_only"\)|is_ident\("component"\)' xtask/src` is empty,
  and `rg client_only web/src macros/src client/src` is empty.

  > **Criterion amended during execution.** As originally written this required
  > `rg client_only … xtask/src` and `rg '#\[component\]' xtask/src/coverage/`
  > to be literally empty. Both now match inside `xtask/src/coverage/exempt.rs`
  > — in the dated retirement note, and in the fixture of a new test
  > (`does_not_exempt_component_or_client_only_marked_fns`) that feeds both
  > attributes to `exempt_lines` and asserts nothing is exempted. That test
  > serves the criterion's _intent_ — proving the machinery is gone — better
  > than its absence would, since it fails loudly if anyone reintroduces the
  > attribute rule. The check is therefore restated against recognition rather
  > than the bare string.

- **D3** The A1 guard retains its `unreachable!` arm; its diagnostics no longer
  name `#[component]`/`#[client_only]`.
- **D4** The two inert markers at `web/src/backup/component.rs:50,184` are
  removed and `rg 'cov:ignore' web/src` is empty.
- **D5** The coverage gate is green after the exemption is deleted — the
  empirical proof that it was dead machinery rather than a load-bearing
  exemption.
- **D6** `web/src/forms/field.rs:198-200` no longer refers to `#[client_only]`.

### E. Documentation

- **E1** ADR-0062 records the `#[client_only]` tenant's retirement.
- **E2** ADR-0070 records the machine enforcement, updates its §6 "rather than
  hiding behind the `#[component]` exemption" wording, and clarifies that
  `client`'s "cross-vertical" framing describes typical use rather than an
  admission test (per D1's acknowledged tension).
- **E3** ADR-0069 clarifies that "never our domain types" means _ours_,
  permitting framework transport such as `MultipartData`.
- **E4** ADR-0050 is amended: its title, Decision 1 (`:51-60`), the A1-guard
  text (`:78`), and the consequences at `:123`, `:130-139`, `:160` no longer
  describe a live `#[component]`/`#[client_only]` exemption, and record why it
  retired.
- **E5** `CONTRIBUTING.md:422-443` (structural exemption) and `:508-525`
  (component bodies are weaker / invariant tripwire) are updated to match. No
  section of `CONTRIBUTING.md` describes removed machinery.

## Sequencing constraint

The retirements (D) and their doc amendments (E4/E5) land **last**, after the
placement check and the `reactive` split. Deleting the exemption while any
component is still ungated would fail the coverage gate; our audit says none is,
but the ordering costs nothing and keeps each commit independently green.

## Out of scope

- Making `--all-features` a permanent part of the gate ladder. A5 verifies it
  once for this change; closing the standing gap is a separate concern.
- The stale unarchived planning docs in `docs/superpowers/{specs,plans}/` for
  shipped issues (#315, #400, #433) — a housekeeping miss from earlier ship
  runs.
- `.claude/skills/jaunder-issues/claim-status.md`'s `--limit 200` recipe, which
  now silently returns nothing because the board holds >300 items. That file is
  untracked local agent config, not part of this repository's tree.

## Risk

**Leptos macro expansion could require `web_sys` in scope.** If `view!` or
`NodeRef` expand to unqualified `web_sys::` paths, removing the dependency
breaks the wasm build. Evidence against: `csr/Cargo.toml` declares no `web-sys`
yet `csr/src/lib.rs` uses leptos view machinery, so expansions resolve through
leptos's own re-exports. The wasm build settles it. If this proves wrong, the
fallback is to keep `web-sys` with only the four live features and reopen D1 — a
decision to surface, not to take silently.
