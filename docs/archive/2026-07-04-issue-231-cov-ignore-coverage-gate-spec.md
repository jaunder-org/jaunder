# Spec — Coverage gate: in-source `cov:ignore` + `#[component]` structural exemption + CRAP threshold

**Issue:** Closes #231 (supersedes #100, folds in #37). Milestone: _Verify-gate
hardening_.

> Problem/motivation and evidence live in **#231** — not restated here. This
> spec is the _what/why-of-the-design_; the plan will be the _how_.

## Goal

Replace the stateful out-of-source coverage ratchet (`coverage-baseline.json` +
anchor + git-diff line-map + re-anchor, ~2,000+ LOC) with a **stateless** gate
whose only inputs are the current llvm-cov report and the source tree.
Acceptance of "OK uncovered" moves _into the source_, so the gate yields an
identical verdict in any checkout — dissolving #100 — and the fragile subsystem
(and its #107/#110/#112/#113 hardening lineage) is deleted.

## The contract

A line reported uncovered by `cargo llvm-cov` **fails the gate** unless it is
one of:

1. **Structurally exempt** — inside a `#[component]` function body (and `view!`
   markup), i.e. CSR UI exercised by the e2e matrix, not native unit tests.
   `#[server]` and plain helper functions are **not** exempt (they stay measured
   — evidence: 0% of `#[server]` body lines are uncovered today).
2. **Explicitly ignored** — carries a `cov:ignore` marker (line form, already in
   `report.rs`; new **block** form for contiguous regions).

CRAP is gated by a **per-function threshold** (with an override for
legitimately-complex functions), not a committed manifest.

## Requirements

### A — Structural exemption for CSR constructs

- **A1.** The gate recognizes `#[component]` function bodies (host-side; stable
  Rust has no `#[coverage(off)]`) and drops their lines from the executable set
  — the same treatment `report.rs` already applies to `cov:ignore` lines.
  Recognition **must use `syn`** (add it to `xtask/Cargo.toml` — present
  transitively; xtask is excluded from every Nix derivation at `flake.nix:1211`,
  so this touches no derivation) and be **attribute-anchored** on the
  `component` attribute path — never a brace-scanner. The failure mode must be
  **closed**: an unrecognized component form (`#[component(transparent)]`,
  aliased/re-exported macro, macro-generated) leaves its body _un_-exempt → its
  uncovered lines surface → the gate **FAILS** (safe/annoying), never silently
  exempts (unsafe). Tests must cover real forms incl. `#[component(...)]` with
  args (~57 `#[component]` attrs, ~224 `view!` blocks exist today).
- **A1-guard (invariant tripwire — load-bearing).** After each coverage build,
  the gate **fails** if any _covered_ report line falls inside a recognized
  `#[component]` span. This converts the assumption the whole design rests on —
  "native tests never render components, so component bodies are exactly 0%
  covered, so exempting them discards no coverage" — into an _enforced_
  invariant. If someone later adds native component rendering (SSR render
  tests), the tripwire fires and forces a deliberate decision (measure those
  components instead of blanket-exempting). The plan must run one real
  `cargo llvm-cov` to confirm the guard starts green.
- **A2.** The exemption is keyed on the **construct**, not files or directories,
  so `#[server]`/helper code co-located in `web/src/pages/*` remains measured
  (those files are only 30–61% uncovered — a directory exclusion would blind
  real logic).
- **A3. Forward-compat (design constraint, not built now).** Keep the exemption
  centralized and construct-keyed so it can later be replaced by the native
  attribute: when `coverage_attribute` stabilizes (or we move the coverage build
  to nightly), the `#[component]` macro (leptos, or a thin wrapper) emits
  `#[cfg_attr(coverage_nightly, coverage(off))]` and we **delete** the host-side
  recognition in one place. Do **not** put inert `cfg_attr` markers in source
  now — on stable they are decoration and change nothing.

### B — `cov:ignore` as the sole manual acceptance

- **B1.** Keep line-level `// cov:ignore`, but **tighten the matcher**. Today
  `report.rs:37-40` does a bare `text.contains("// cov:ignore")` on the rendered
  source line — so the marker appearing inside a _string literal_ or a _doc
  comment_ falsely suppresses (the module's own test fixture, `report.rs:59-66`,
  embeds the marker in a string). Making this the primary mechanism for ~700
  lines magnifies that into a standing soundness hole. Anchor the match to a
  **real trailing line comment** (the marker must be the line's actual
  `//`-comment, not any substring).
- **B2.** Add a **block form** — `// cov:ignore-start` … `// cov:ignore-stop` —
  for contiguous uncovered regions the line form can't annotate (mid-expression
  lines, inside multi-line/raw string literals). The block form is
  **load-bearing, not optional**: the ~700-line migration includes lines that
  cannot carry a trailing comment. Design it coherently with the fact that
  `"// cov:ignore-start".contains("// cov:ignore")` is true (don't let a
  start/stop marker be double-handled by the line path). **Unmatched or
  overlapping markers must be a hard error**, never a silent no-op (an
  unterminated `-start` would otherwise create an open-ended blind spot).
- **B3.** `cov:ignore` is the **only** manual escape hatch; there is no baseline
  file to edit. Every marker is visible and reviewable in the diff where it
  lives.

### C — CRAP by threshold

- **C1.** Replace the `crap-manifest.json` regression check with a threshold:
  fail if any function's CRAP exceeds **T**. **Recommended T = 30** — flags
  exactly one function today (`test-support/src/main.rs::main`, 0% cov).
  (Distribution: median 2, p99 15, max 156.) That one offender is tracked
  separately in **#232** (milestone _Code quality improvement_) — so turning on
  `T = 30` depends on #232 being resolved (covered/refactored) or an override
  applied. Progressive tightening of `T` is the _Code quality improvement_
  milestone's remit, not this cycle's.
- **C2.** Provide an override for legitimately-complex-and-tested functions
  (e.g. a `crap:allow` marker or a small config list) — note a fully-covered
  function's CRAP equals its cyclomatic complexity, so high-cyclomatic functions
  can legitimately sit near T. Max cyclomatic today ≈ 25.
- **C3.** Delete `crap-manifest.json`.

### D — Delete the subsystem + fold in #37

- **D1.** Delete `coverage-baseline.json`, and
  `xtask/src/coverage/{baseline,classify, diffmap,reanchor}.rs` plus the
  anchor/heal/re-anchor logic in `mod.rs`. Keep and extend `report.rs`;
  keep/trim `crap.rs` for the threshold check. **Also remove the now-dead
  support surface the baseline/manifest existed for** (the cold review found
  these — they don't crash if left, but become dead code / stale docs):
  - The **`merge.coverage-keepours` merge-driver subsystem** that exists only to
    manage the two generated files: `ensure_merge_driver_installed()`
    (`xtask/src/main.rs:9`, called every run), `xtask/src/lib.rs:404-459` + its
    tests (`lib.rs:740-804`), and the `.gitattributes:6-7` entries. This is part
    of **ADR-0029**, so reconcile there, not just ADR-0030.
  - The **`coverage reanchor`** and **`coverage refresh-crap`** CLI
    subcommands + parse tests (`xtask/src/lib.rs:187-317`, `526-565`).
  - Rewrite **`.githooks/pre-commit`** — its documented rationale (auto-heal the
    baseline/CRAP manifest, fail-and-restage per #113) evaporates; the generic
    clean-tree/`porcelain` check survives but the comments and heal-path
    narrative must be updated.
  - `crap-manifest.json` (also C3). CI/Nix are unaffected: `ci.yml:42-50` only
    uploads `status.json`+diagnostics, and the Nix `coverage-gate`
    (`flake.nix:1264`) checks only `status.category`, never the baseline.
- **D2. #37 fold-in.** Bound the Nix coverage `src` to cargo sources (no
  dependence on untracked junk), **re-admitting `csr/index.html`** (a
  compile-time `include_str!`), plus a `drvPath` probe so the filter can't
  silently drift. (This is the salvageable part of the parked #37 work.)

### E — Docs

- **E1.** Rewrite the `CONTRIBUTING.md` coverage section to the stateless model
  (structural exemption + `cov:ignore` + CRAP threshold; no
  baseline/anchor/regen ritual).
- **E2. ADRs:** supersede **ADR-0030** (text-identity re-anchor — the concept is
  gone); amend **ADR-0029** (the Fix-mode heal / single-pass story); new draft
  ADR recording the stateless-gate decision.

## Decisions to confirm (my recommended defaults — flag any you'd change)

1. **Remainder policy — the ~700 currently-accepted non-exempt lines** (444
   page-helpers
   - 268 scattered backend). **Recommended: faithful translation, not a testing
     sprint** — migrate them to `cov:ignore` (preserving today's acceptance; the
     baseline already accepts them, so this is _not_ new debt), and treat
     burn-down as incremental follow-up now that each is visible in-source.
     _Alternative:_ test the genuinely-testable backend lines
     (server/storage/common) in this cycle. This is the main scope dial.
2. **Exemption key = `#[component]` (+ `view!`), not `#[server]`.** Recommended
   as above (server stays measured).
3. **CRAP threshold T = 30** with a `crap:allow`-style override. Recommended
   (C1/C2).
4. **Recognition = host-side parse** (this cycle), swappable to native later
   (A3).

## Protection tradeoffs (stated honestly — not "stricter")

The stateless gate genuinely dissolves #100 (identical verdict at any checkout
depth; no #112-style text-identity guessing — the verdict is a pure function of
`(report, source)`). But it is **equivalent-or-weaker** than today's ratchet on
three axes, by deliberate design. CONTRIBUTING.md and the plan must say so
plainly rather than sell it as stricter:

1. **Component bodies: weaker.** Today a _new_ uncovered line inside a component
   classifies as `new_uncovered` → fails → forces conscious baseline sign-off.
   After this, any uncovered line in any component body is blanket-exempt →
   passes silently. New untested component logic that today needs sign-off
   becomes invisible. (The A1-guard bounds this to _uncovered_ component code;
   it does not re-introduce sign-off.)
2. **CRAP: weaker below T.** A per-function threshold is blind to a function
   that worsens but stays under `T` (e.g. 5 → 29). The manifest regression-check
   caught that. This argues for keeping `T` tight and for the _Code quality
   improvement_ grind-down.
3. **`cov:ignore` is permanent.** Unlike the ratchet (which tracked
   covered-state and re-flagged covered→uncovered), a `cov:ignore`'d line that
   later becomes covered and regresses is never re-flagged. The ~700-line
   migration bakes in ~700 permanent (but in-source, greppable, reviewable)
   blind spots.

These are accepted because: component UI is covered by e2e, non-exempt code
still fails on any uncovered line, and the deleted machinery's fragility (#100
and its lineage) outweighs the marginal ratchet protection on already-accepted
lines.

## Non-goals

- Moving the coverage build to nightly (the only way to activate
  `#[coverage(off)]` now) — out of scope; A3 keeps the door open.
- A testing sprint to _raise_ coverage on the remainder (unless decision #1 says
  otherwise).
- Changing e2e or the coverage _report_ generation, beyond #37's source filter.

## Risks / open questions

- **Recognition soundness (A1).** A brace-scanner mis-parses generics/strings;
  prefer `syn`. The plan must pin this with tests over real page files.
- **Exemption over-reach.** Exempting whole `#[component]` bodies hides genuine
  logic bugs in components — accepted, because e2e covers UI; call it out in
  CONTRIBUTING.
- **Block-marker discipline (B2).** Unmatched/overlapping markers must be a hard
  error, or they become silent blind spots.
- **CRAP override abuse (C2).** Overrides must be visible/reviewable (in-source
  marker preferred over a hidden config).
