# ADR-0050: Stateless coverage gate — `cov:ignore` + `#[component]` exemption + CRAP threshold

- Status: accepted
- Date: 2026-07-04
- Issue: [#231](https://github.com/jaunder-org/jaunder/issues/231)

## Context

The coverage gate was a **stateful ratchet**: it classified each uncovered line
by line number against a committed `coverage-baseline.json` (accepted-uncovered
gaps) and a committed `crap-manifest.json` (per-function CRAP scores), auto-heal
in Fix mode. To keep that state usable across line-shifting edits and concurrent
merges, a scaffold accreted around it: a **text-identity re-anchor** (ADR-0030),
`cargo xtask coverage reanchor` / `refresh-crap` candidate-promotion commands, a
`merge.coverage-keepours` git merge driver plus its self-install
(`ensure_merge_driver_installed()`), and a Fix-mode heal woven into the
single-pass pre-commit hook (ADR-0029).

That machinery was fragile in ways that could not be patched away:

- **#100 — stale-anchor divergence.** The verdict depended on _which_ baseline
  the gate loaded (working tree vs. anchor commit), so the same source could
  pass or fail depending on checkout depth and merge history.
- **#112 — text-identity is unsound as a classifier.** After a rebase the gap's
  move is invisible to the gate (the diff is empty), so text alone cannot tell
  "the accepted gap moved here" from "a different line regressed to the same
  text" (`}`, `Ok(())`, `.await?`). Promoting text identity from a diff-scoped
  safety net to the primary classifier was proved unsound and closed
  not-planned. The re-anchor (ADR-0030) survived _only_ as a diff-scoped excuse,
  which is exactly the complexity we no longer want to maintain.
- **General fragility.** The baseline/manifest/merge-driver/candidate-file
  surface was a large amount of code and operator ritual whose entire job was to
  manage two generated files. Most of the protection it bought was over
  **already-accepted** lines.

Meanwhile the thing the ratchet was protecting is mostly CSR UI.
`web/src/pages/*` `#[component]` bodies render `view!` trees whose correctness
is validated by the e2e matrix in the browser (WASM), never by native host tests
— so they are structurally 0% covered host-side and were carried as ~700
permanently-accepted baseline gaps. Stable Rust has no `#[coverage(off)]` to
express that intent in source.

## Decision

Replace the stateful ratchet with a **stateless gate** whose verdict is a pure
function of `(coverage report, source tree)`:

1. **Structural exemption for `#[component]`.** After each `cargo llvm-cov`
   build, the gate parses each source file with `syn` and drops the body-span
   lines of any `#[component]` function from the executable set. Recognition is
   **attribute-anchored** (`component` attribute path, incl.
   `#[component(...)]`) and **fail-closed**: an unrecognized component form
   leaves its body measured → the gate can FAIL, never silently exempts. The key
   is the **construct**, not a file or directory, so co-located
   `#[server]`/helper code stays measured.

2. **A1-guard tripwire.** The gate **fails** if any _covered_ report line falls
   inside a recognized `#[component]` span. This turns the design's load-bearing
   assumption — "native tests never render components, so exempting their bodies
   discards no coverage" — into an enforced invariant; if someone later adds
   native SSR render tests, the tripwire forces a deliberate decision instead of
   silently trusting the exemption. Proven green on a real instrumented build
   before any deletion.

3. **`cov:ignore` as the sole manual acceptance path.** An uncovered, non-exempt
   line fails unless it carries a `// cov:ignore` marker. The matcher is
   anchored to the line's real trailing `//` comment (a marker inside a string
   or doc comment does not suppress). A **block form** — `// cov:ignore-start` …
   `// cov:ignore-stop` — covers contiguous regions and lines that cannot take a
   trailing comment; nesting and unmatched/stray markers are **hard errors**.
   There is no baseline file; every marker is reviewable in the diff where it
   lives.

4. **CRAP by per-function threshold, T = 30.** A function whose CRAP score
   exceeds T fails unless a `// crap:allow: <reason>` marker (non-empty reason
   required) sits within its line span. Replaces the `crap-manifest.json`
   regression check. Progressive tightening of T is the _Code quality
   improvement_ milestone's remit (#232+).

5. **Delete the subsystem.** Remove `coverage-baseline.json`,
   `crap-manifest.json`, the `baseline`/`classify`/`diffmap`/`reanchor` modules
   and their anchor/heal logic, the `coverage reanchor` / `refresh-crap` CLI
   subcommands, and the `merge.coverage-keepours` merge driver and its
   self-install. `Mode::Fix` is retained (it still drives formatting); only the
   coverage/CRAP heal branches keyed on it are removed. The pre-commit hook
   keeps its fail-and-restage on **formatting** changes.

6. **Bound the coverage source (#37).** The Nix coverage `src` is filtered to
   cargo sources (re-admitting the compile-time `include_str!` of
   `csr/index.html`), with a `drvPath` probe so untracked non-source junk cannot
   change the build while an untracked `.rs` still does. This gives the working
   tree a well-defined, junk-insensitive source contract.

## Consequences

- **#100 dissolves.** The verdict no longer depends on which baseline was loaded
  or on checkout depth; it is a pure function of report + source. No
  text-identity guessing (#112) survives.
- **Intent lives in source.** Exemptions are `#[component]` (structural) or an
  in-source, greppable, reviewable `cov:ignore` / `crap:allow` marker — no
  out-of-band generated files, no merge-driver, no candidate-file promotion
  ritual. A fresh clone needs no coverage-specific git config.
- **Accepted protection tradeoffs (equivalent-or-weaker, not stricter).** Stated
  plainly here and in `CONTRIBUTING.md`:
  1. **Component bodies: weaker.** A new uncovered line inside a component body
     is blanket-exempt and passes silently, where the ratchet forced sign-off.
     The A1-guard bounds this to _uncovered_ component code but does not restore
     sign-off.
  2. **CRAP: weaker below T.** A threshold is blind to a function that worsens
     but stays under T (5 → 29); this argues for keeping T tight.
  3. **`cov:ignore` is permanent.** A marked line that later becomes covered and
     then regresses is never re-flagged, unlike the covered-state ratchet. The
     migration bakes in ~700 permanent (but in-source, reviewable) blind spots.
     These are accepted because component UI is covered by e2e, all non-exempt
     code still fails on any uncovered line, and the deleted machinery's
     fragility outweighed the marginal ratchet protection on already-accepted
     lines.
- **Forward-compat to native `#[coverage(off)]`.** The exemption is centralized
  and construct-keyed. When `coverage_attribute` stabilizes (or the coverage
  build moves to nightly), the `#[component]` macro (leptos or a thin wrapper)
  can emit `#[cfg_attr(coverage_nightly, coverage(off))]` and the host-side
  `syn` recognition is deleted in one place. Inert `cfg_attr` markers are
  deliberately **not** added to source now — on stable they change nothing.

## Relationship to prior ADRs

- **Supersedes [ADR-0030](0030-coverage-reanchor-text-identity.md)** (coverage
  re-anchor by text identity). The baseline and its diff-scoped re-anchor no
  longer exist, so the concept ADR-0030 records is gone; its #112 supplement
  (why text identity cannot be a classifier) is preserved as history and
  reinforced by the move to a stateless verdict.
- **Amends [ADR-0029](0029-git-enforced-verify-gate.md)** (git-enforced verify
  gate). The single-pass pre-commit `cargo xtask check` and its
  fail-and-restage-on-change mechanism stand, but the Fix-mode **coverage/CRAP
  heal** and the `merge.coverage-keepours` merge-driver self-install described
  there are removed; the hook now restages only on **formatting** changes. The
  clean-tree gating and self-healing `core.hooksPath` install are unchanged.
