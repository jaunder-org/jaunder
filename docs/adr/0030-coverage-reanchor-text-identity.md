# 0030. Coverage re-anchor by text identity

Status: accepted

## Context

The coverage gate classifies each uncovered line by **line number** against the
committed `coverage-baseline.json`. A line-shifting change whose unified diff
models an accepted-uncovered gap as deleted-then-reappeared produces a _phantom_
regression/new-uncovered: the line did not change coverage, it only moved. This
blocked the Fix-mode auto-heal and forced manual regeneration (#51/#52/#53
refactors, #63 sweep).

The naive fix — "current uncovered text multiset ⊆ baseline accepted text
multiset" — is unsound: covering one `}` while a different identical-text `}`
regresses leaves the multiset unchanged and would mask the regression.

## Decision

The heal's safety condition is **text-identity re-anchor**, keyed on what the
diff _removed_ vs. what _appeared_:

- `structural_texts(file)` = texts of accepted gaps the diff removed (the
  classifier's `structural` bucket).
- `appeared_texts(file)` = texts of newly-flagged uncovered lines (`regressions`
  ∪ `new_uncovered`).
- **Safe re-anchor iff, per file, `appeared_texts` ⊆ `structural_texts`** as a
  multiset — every newly-flagged uncovered line is explained by an accepted gap
  of identical text that the diff removed (the line genuinely moved).

When safe, `cargo xtask check` (Fix) re-anchors the baseline and passes;
`validate` (Check) passes without mutating. When an appeared text has no removed
counterpart (genuine lowering), the gate still fails.

## Consequences

- Benign line-shifts (including those introduced by concurrently-merged
  branches) self-heal instead of forcing manual regeneration.
- Residual ambiguity: two identical-text lines in one file, where one is removed
  as an accepted gap and an unrelated identical-text line regresses in the same
  change, can be conflated as a safe move. Bounded and accepted; the
  line-identity classifier remains the primary signal — text-identity only
  _excuses_ line failures the diff explains as moves.
- The predicate is a single primitive (`reanchor_is_safe`) reused by the gate
  and, later, the explicit reanchor command (#88).

## Supplement (2026-06-28) — text identity is a safety net, not a classifier (#112)

This ADR uses text identity _only_ to excuse line failures the diff already
explains as moves (the `appeared ⊆ structural` check above). A later attempt
(#112) to promote it to the **primary** classifier — keying pass/fail on
uncovered-line text instead of mapping lines through the diff, to make the gate
robust to a pre-PR rebase — was **rejected as unsound for a ratchet**, and the
classifier deliberately stays line-identity.

Why it cannot work: after a rebase the gap's move is _invisible_ to the gate.
The working tree equals the anchor commit's tree, so
`git diff <anchor>..worktree` is empty, and the committed baseline holds only a
stale line number plus the text. With no diff to say what was _removed_, text
alone **cannot distinguish "the accepted gap moved here" from "a different line
independently regressed to the same text."** A text-primary classifier therefore
silently masks real regressions on collision-prone texts (`}`, `Ok(())`,
`.await?`), which are exactly the lines most often both uncovered and
duplicated. This is strictly weaker than the line-identity classifier it would
replace; uniqueness-, count-, and deletion-based patches all fail because the
distinguishing information was destroyed by the rebase. (A strong review proved
it with concrete counterexamples; #112 was closed not-planned.)

The crucial difference from this ADR's safe use: here, the diff supplies the
`structural` (removed) set, so a same-text appearance is matched against a gap
we _know_ was removed — a verifiable move. Without that removed-set evidence
(the rebase case) the match is a guess, and a ratchet must not guess.

**Therefore:** rebase-robustness comes from a sound **re-heal**, not the
classifier — after a rebase, `cargo xtask check` regenerates the baseline from
_actual_ coverage (no guessing), and #110 made that re-heal consistent (load the
baseline from the anchor commit, not the working tree). The classifier remains
line-identity with this text check as its diff-visible safety net.

## Supplement (#88) — the explicit reanchor command

ADR-0030 anticipated "the explicit reanchor command (#88)". It lands as
`cargo xtask coverage reanchor`, and it does **not** introduce a second, weaker
safety notion: it reuses this ADR's `reanchor_is_safe` predicate (diff-based,
`appeared ⊆ structural`), so it refuses a genuine lowering exactly when the gate
does. The issue's original "uncovered-text-set unchanged/shrank" wording — a
naive multiset check — is **not** what shipped; that is the text-primary
approach the #112 supplement above rejected as unsound.

The command is **candidate-promotion only**: a safe move re-anchors
`coverage-baseline.json` in place; a genuine lowering is refused (non-zero exit)
and the would-be baseline is written to
`.xtask/coverage-baseline.candidate.json` (under the gitignored `/.xtask/`),
never the committed file. There is deliberately **no accept-all path** — the
removed `__regen-baseline` was exactly that footgun. Accepting an approved
lowering is a manual `cp` of the candidate, so it always lands as a reviewable
diff under the coverage-baseline policy. The failing coverage gate prints
`cargo xtask coverage reanchor` as the recovery for a lowering; the symmetric
CRAP-manifest refresh path is tracked separately (the #88 CRAP follow-on, #131).

## Supplement (#131) — the symmetric CRAP refresh path

The #88 supplement noted "the symmetric CRAP-manifest refresh path is tracked
separately (#131)". It lands as `cargo xtask coverage refresh-crap`, mirroring
the baseline `reanchor` model exactly: a no-regression refresh rewrites
`crap-manifest.json` in place (a no-op on a pure line-shift, keyed on the same
line-independent canonical form the Fix-mode heal uses); a CRAP **regression**
is refused (non-zero exit) with the would-be manifest written to
`.xtask/crap-manifest.candidate.json` — never the committed file. There is **no
accept-all path**; promoting approved drift is a manual `cp` of the candidate,
so it always lands as a reviewable diff. The failing coverage gate now prints
`cargo xtask coverage refresh-crap` as the CRAP recovery, the category-split
companion to the lowering branch's `cargo xtask coverage reanchor`.
