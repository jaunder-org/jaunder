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
