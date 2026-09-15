# Issue 1503: Describe the hook-backed commit gate

## Outcome

Contributors can identify the repository's real commit boundary without running a redundant broad check by default. The focused-test guidance points them to the enforced pre-commit hook and preserves explicit broad checks as diagnostic tools.

## Load-bearing decisions

- Staging the final intended tree and invoking `git commit` is the documented commit boundary.
- The enforced hook's `cargo xtask precommit` run is the ordinary commit gate.
- A hook failure or formatter mutation requires inspection, re-staging, and retrying the commit.
- `cargo xtask check --no-test` and `cargo xtask check` remain available as explicit broad diagnostic or integration commands.
- Focused `test-local`, pre-push, and CI guidance retain their existing roles.
- This change documents existing policy; it does not alter verification behavior.

## Acceptance

- `CONTRIBUTING.md` names the pre-commit hook as the commit gate at the focused-test escalation boundary.
- The guidance states what to do after hook failure or mutation.
- Broad `check` commands are not described as mandatory precursors to every implementation commit.
- The surrounding focused-test, pre-push, and CI guidance remains coherent.
- Repository-local development skills are audited for the same commit-boundary policy, with any contradictory wording corrected in the same change.
- The resulting branch is documentation-only and passes the hook-backed documentation gates.

## Boundaries

- No changes to hooks, xtask, tests, CI, or verification policy.
- No skill edits where the existing guidance already matches the policy.
- No reorganization of the broader testing documentation.
- No new architectural or domain decision.
