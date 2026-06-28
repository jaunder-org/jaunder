# 0030. Coverage re-anchor by text identity

Status: accepted

## Context

The coverage gate classifies each uncovered line by **line number** against the
committed `coverage-baseline.json`. A line-shifting change whose unified diff
models an accepted-uncovered gap as deleted-then-reappeared produces a *phantom*
regression/new-uncovered: the line did not change coverage, it only moved. This
blocked the Fix-mode auto-heal and forced manual regeneration (#51/#52/#53
refactors, #63 sweep).

The naive fix — "current uncovered text multiset ⊆ baseline accepted text
multiset" — is unsound: covering one `}` while a different identical-text `}`
regresses leaves the multiset unchanged and would mask the regression.

## Decision

The heal's safety condition is **text-identity re-anchor**, keyed on what the
diff *removed* vs. what *appeared*:

- `structural_texts(file)` = texts of accepted gaps the diff removed (the
  classifier's `structural` bucket).
- `appeared_texts(file)` = texts of newly-flagged uncovered lines
  (`regressions` ∪ `new_uncovered`).
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
  *excuses* line failures the diff explains as moves.
- The predicate is a single primitive (`reanchor_is_safe`) reused by the gate
  and, later, the explicit reanchor command (#88).

## Supplement (2026-06-28) — text identity is a safety net, not a classifier (#112)

This ADR uses text identity *only* to excuse line failures the diff already
explains as moves (the `appeared ⊆ structural` check above). A later attempt
(#112) to promote it to the **primary** classifier — keying pass/fail on
uncovered-line text instead of mapping lines through the diff, to make the gate
robust to a pre-PR rebase — was **rejected as unsound for a ratchet**, and the
classifier deliberately stays line-identity.

Why it cannot work: after a rebase the gap's move is *invisible* to the gate. The
working tree equals the anchor commit's tree, so `git diff <anchor>..worktree` is
empty, and the committed baseline holds only a stale line number plus the text.
With no diff to say what was *removed*, text alone **cannot distinguish "the
accepted gap moved here" from "a different line independently regressed to the
same text."** A text-primary classifier therefore silently masks real
regressions on collision-prone texts (`}`, `Ok(())`, `.await?`), which are
exactly the lines most often both uncovered and duplicated. This is strictly
weaker than the line-identity classifier it would replace; uniqueness-, count-,
and deletion-based patches all fail because the distinguishing information was
destroyed by the rebase. (A strong review proved it with concrete
counterexamples; #112 was closed not-planned.)

The crucial difference from this ADR's safe use: here, the diff supplies the
`structural` (removed) set, so a same-text appearance is matched against a gap we
*know* was removed — a verifiable move. Without that removed-set evidence
(the rebase case) the match is a guess, and a ratchet must not guess.

**Therefore:** rebase-robustness comes from a sound **re-heal**, not the
classifier — after a rebase, `cargo xtask check` regenerates the baseline from
*actual* coverage (no guessing), and #110 made that re-heal consistent (load the
baseline from the anchor commit, not the working tree). The classifier remains
line-identity with this text check as its diff-visible safety net.
