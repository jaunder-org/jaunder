# Issue #88 — Surface + harden the baseline-reanchor command

**Status:** approved
**Issue:** [#88](https://github.com/jaunder-org/jaunder/issues/88) — _coverage: surface + harden the baseline-reanchor command (un-hide, --check vetting, recovery hint)_
**Milestone:** 1 — Verify-gate hardening
**Builds on:** #86 / ADR-0030 (text-identity re-anchor primitive `reanchor_is_safe`), #87 (gate prints `file:line: text`), #110 (re-heal loads the baseline from the anchor commit)

## Problem

The only way to re-anchor a drifted coverage baseline today is `cargo xtask __regen-baseline
--gcroot <path>`, which is:

- **hidden** (`#[command(hide = true)]`) and undocumented — undiscoverable from `--help` or
  CONTRIBUTING; and
- **accept-all** — `regen_baseline_inner` unconditionally rebuilds `coverage-baseline.json`
  from the current report and overwrites it (`lib.rs:177-198`), with **no** safety check. It
  will silently lower coverage if genuinely-newly-uncovered lines are present.

Separately, when the coverage gate fails it prints the offending `file:line: text` (#87) plus
prose guidance, but **no copy-paste recovery command** — the operator is told to "re-anchor"
without being told how.

## Decisions (settled in brainstorming)

1. **Reuse the sound, diff-based primitive — not a text-set check.** The no-lowering check is
   the existing `reanchor_is_safe(verdict, current, baseline) -> ReanchorSafety { safe,
   lowering }` (`xtask/src/coverage/reanchor.rs`), which keys on `appeared ⊆ structural` using
   the diff's removed-set as evidence (ADR-0030). The issue's original "uncovered-text-set
   unchanged/shrank" wording describes the *naive multiset* check that ADR-0030's #112
   supplement **rejected as unsound** (on collision-prone texts it can mask a real lowering).
   Reusing `reanchor_is_safe` makes the command refuse lowering exactly when the gate does, and
   its `.lowering: Vec<LineText>` is already the `file:line: text` list to print on refusal.
2. **Candidate-promotion only — no accept-all path anywhere.** The command either re-anchors a
   safe move (writes `coverage-baseline.json`, exit 0) or, on a genuine lowering, writes the
   would-be baseline to a side path and refuses (non-zero). Accepting an approved lowering is a
   **deliberate manual `cp`** of the candidate over the committed baseline — never a flag or
   command — so it always lands as a reviewable diff. `__regen-baseline` is **removed entirely**.
3. **Nested `cargo xtask coverage reanchor`** — the first nested subcommand group in xtask; the
   natural namespace, leaving room for future coverage subcommands.
4. **Supplement ADR-0030** with the reanchor command's safety model (reuse the primitive, refuse
   lowering, no accept-all). No `docs/README.md` row change.
5. **Gate prints the recovery command** for coverage lowerings only.

## Design

### `cargo xtask coverage reanchor [--gcroot DIR]`

Operates on an **existing** coverage report (default gcroot `.xtask/gcroots/coverage`, like
`__regen-baseline`); it does **not** rebuild coverage — that is `check`/`validate`. If the
report is missing, it errors telling the operator to run the gate first.

1. Parse `<gcroot>/coverage-report.txt` → current `Vec<FileCoverage>`
   (`coverage::report::parse_text_report`).
2. Load the baseline from the **anchor commit** (consistent with #110, not the working tree),
   and compute the line-classifier verdict + `safety = reanchor_is_safe(...)` — the same inputs
   the gate assembles.
3. Build the candidate = `Baseline::from_files(&current)` (the would-be new baseline). In both
   branches the *content* written is this candidate; only the destination and exit code differ.
4. **If `safety.safe`:** write the candidate to `coverage-baseline.json`; exit 0
   (`re-anchored N file(s)`).
5. **If not safe (genuine lowering):** write the candidate to the gitignored
   `.xtask/coverage-baseline.candidate.json` (confirmed ignored via `/.xtask/`,
   `.gitignore:39`), print each `safety.lowering` entry as `file:line: text`, and exit non-zero.
   The message explains: inspect with
   `git diff --no-index coverage-baseline.json .xtask/coverage-baseline.candidate.json`; if the
   lowering is genuinely approved (per the coverage-baseline policy), promote it by copying the
   candidate over the baseline and committing. There is **no** auto-accept flag.

**Scope: the coverage baseline only** (`coverage-baseline.json`). The CRAP manifest is untouched
(see Out of scope + Separable concern).

### Module boundary (testable seam)

The command logic lives in the `coverage` module as a testable entry point, e.g.
`coverage::reanchor_command(gcroot: &str) -> anyhow::Result<ReanchorOutcome>` returning
`Safe { candidate: Baseline }` / `Lowering { candidate: Baseline, lowering: Vec<LineText> }`. It
reuses the gate's existing anchor-load + classify + `reanchor_is_safe` steps — factor a shared
helper rather than duplicate the verdict computation that `run_inner` performs. `lib.rs` stays
thin: map the outcome to a file write + exit code, build the `CommandResult`.

### Gate recovery message (`failure_report`, `coverage/mod.rs:254-301`)

Today it prints the lowering `file:line: text`, CRAP regressions, and category-split prose
guidance, but no command. Append the exact recovery line **only when there are coverage
lowerings**:

```
  → run:  cargo xtask coverage reanchor
```

(The gate runs on the default gcroot, so the printed command needs no `--gcroot`.) A CRAP-only
failure keeps its existing manifest-refresh prose unchanged — `reanchor` does not apply to CRAP.

### Remove `__regen-baseline`

Delete the `RegenBaseline` enum variant + its `hide`/`--gcroot` (`lib.rs:47-54`), its
`command_name` arm (`lib.rs:79`), its `run` arm (`lib.rs:124-131`), and `regen_baseline` /
`regen_baseline_inner` (`lib.rs:168-198`). The reanchor path reuses the same building blocks
those functions used (`parse_text_report`, `Baseline::from_files`).

### Docs

- `coverage reanchor` appears in `cargo xtask --help` (un-hidden) with `after_help` examples.
- **CONTRIBUTING.md**: document `coverage reanchor` and the candidate-promotion flow; replace
  the existing references to the hidden `__regen-baseline` one-shot (the coverage / merge
  sections) with the new command.
- **ADR-0030 `## Supplement (#88)`**: the reanchor command reuses `reanchor_is_safe`, refuses
  lowering, and deliberately has no accept-all — approved lowering is a manual candidate
  promotion so it is always a reviewable diff. No `docs/README.md` row change (status stays
  `accepted`).

## Testing

- **Outcome logic** (the `coverage`-module seam): a safe report → `Safe { candidate }` whose
  candidate equals the re-anchored baseline; a lowering report → `Lowering { candidate,
  lowering }` with the correct `file:line: text` entries. Reuse the existing `reanchor.rs` test
  fixtures (`fc`, `baseline_with`, `fl`) and the `mod.rs` `safe()` helper.
- **Clap wiring**: `cargo xtask coverage reanchor` parses, and `--gcroot` is accepted — the
  first command-level parse test for this area (mirror the `validate_*_parses` pattern,
  `lib.rs:330-357`), exercising the nested-subcommand dispatch.
- **`failure_report`**: update the existing tests
  (`failure_report_lists_lines_crap_and_recovery`, `_guidance_is_category_conditional`,
  `_caps_long_lists`) to assert the `cargo xtask coverage reanchor` line is present for a
  coverage lowering and **absent** for a CRAP-only failure.
- Per-task gate `cargo xtask check --no-test`; final `cargo xtask validate`.

## Out of scope

- **The CRAP manifest.** `coverage reanchor` does not touch `crap-manifest.json`. `check`
  (Fix mode) already *regenerates* the CRAP manifest on a safe, regression-free run
  (`mod.rs:202-212`); a genuine CRAP regression's recovery stays the existing manual
  approved-refresh prose — see the separable concern below.
- `reanchor` rebuilding coverage itself (it consumes an existing report).
- Any change to the line-identity classifier or the `reanchor_is_safe` predicate.

## Separable concern (→ plan's first task)

The CRAP-regression recovery path is **prose-only** (`mod.rs:294-298`): it states the policy
("refresh crap-manifest.json with approval, only if stale drift") but gives no mechanism — no
command, no pointer to the fresh `crap-report.json`, no candidate-promotion analog — and the
manual refresh is fiddly (Fix-mode only regenerates the manifest when there are *no*
regressions, so an approved one must be hand-overwritten and re-run). This is the symmetric gap
to the one #88 closes for the baseline, and a "failure-artifact recoverability deficiency" to
file as its own milestone-1 dx/coverage issue (likely mirroring #88's candidate-promotion: a
`coverage refresh-crap` helper and/or the gate printing exact steps). **Keep #88
baseline-only.** Filing this issue is the implementation plan's first task.
