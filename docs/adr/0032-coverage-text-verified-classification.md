# 0032. Coverage classification verified by uncovered-line text

Status: accepted

## Context

The coverage classifier maps each accepted-uncovered baseline gap to its current
line through a `git diff <anchor>..worktree` line-map, where `anchor` is the last
commit that touched `coverage-baseline.json` (ADR-0030). That map can only see
shifts *between* the anchor and the working tree.

A **rebase** breaks this. It replays the baseline commit's *content* (pre-rebase
line numbers) onto `origin/main`'s *code*, so `origin/main`'s line-shifts are
baked **into** the anchor commit's tree and are invisible to the anchor→worktree
diff. After a clean pre-PR rebase the committed baseline references stale lines,
the line-map maps gaps to the wrong current lines, and the gate reports phantom
drift — fixable only by a manual `cargo xtask check` re-heal + commit. The
text-multiset re-anchor safety net (ADR-0030) can't rescue it, because with an
empty diff there is no `structural` (removed-gap) bucket to match against.

## Decision

Resolve each gap **text-verified**: map the gap's line through the diff, but
accept the mapped line only when its *text* matches the gap. When it does not — a
move the diff can't see — find the gap's text among the current lines (nearest to
the diff hint, to disambiguate duplicate texts), preferring an unclaimed
uncovered line. Line numbers are a hint for disambiguation, not the key.

Soundness is preserved by **confirming the mapped line first**: a gap whose own
line is now covered resolves to an *improvement* before any same-text line
elsewhere can claim it, so a genuinely new uncovered line of the same text is
still flagged (the net-zero-swap case ADR-0030 was built to catch).

## Consequences

- Line-shifts the anchor diff can't see — most importantly an upstream rebase —
  no longer desync the baseline. `validate` finds the moved gaps by text and
  passes on the rebased tree with **no manual re-heal**; the "rebase just before
  the PR" step stops being a source of phantom failures.
- The line-identity re-anchor machinery (ADR-0030 `reanchor_is_safe`) is now
  subsumed by the classifier for the common cases; it remains as a consistent
  secondary safety check.
- Residual ambiguity is the same bounded duplicate-identical-text case ADR-0030
  documented: when several lines share a text and one is removed while an
  unrelated same-text line regresses, proximity disambiguation can mis-pair them.
  Bounded, and the confirm-mapped-line-first rule keeps the common swap sound.
- The committed baseline's line numbers are now advisory (a hint); the gate's
  pass/fail no longer depends on them being current.
