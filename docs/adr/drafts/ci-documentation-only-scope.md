# ADR-DRAFT: CI runs only the documentation checks for a Markdown-only change

- Status: proposed
- Date: 2026-11-25
- Issue: [#1971](https://github.com/jaunder-org/jaunder/issues/1971)

## Context

Every CI run executed every validation lane and the whole E2E matrix, about 160
runner-minutes (promoter PR run 45503763934). A pull request that changes only
documentation, and the ADR-promoter pull request that follows every ADR-bearing
change, paid that cost once on the pull request and again in the merge queue. No
product lane reads Markdown: nothing includes it into a build, and the Nix
product checks filter `.md` out of their sources. A new commit on a pull request
also left the superseded run going to completion.

The precommit already had a Markdown-only test for its narrow route
([ADR-0029](../0029-git-enforced-verify-gate.md)). It also sent any delete or
rename broad, which no check in the narrow route needed.

## Decision

- **One Markdown-only rule.** A change is Markdown-only when it is non-empty and
  every record of its raw Git diff adds, modifies or deletes a case-sensitive
  `.md` path that is a regular file on each side that exists. It is one function
  in `dev` (`rawdiff.markdown_only`); the precommit applies it to the staged
  diff, and `dev.scope` to the diff between a base commit and the tested commit.
  The precommit keeps its own conditions about the working tree (nothing
  uncertain, untracked or unstaged). A Markdown delete, and so a Markdown
  rename, now takes the precommit's narrow route.
- **Documentation-only CI.** The `ubuntu-slim` `Merge-queue verdict` job runs
  `dev.scope` against the pull request's base or the merge-queue entry's base,
  with the image's own `python3` and `git`. A push has no base and is never
  documentation-only. For a documentation-only change only the `Validation docs`
  lane runs (`dev ci-validate-docs`: Markdown Prettier and the
  documentation-shape checks); every other validation lane, E2E producer and
  reconciliation is skipped. `Validate (no e2e)` then requires the verdict job
  and docs lane, and `e2e gate` requires the verdict job and passes with the E2E
  jobs skipped as not applicable. Anything `dev.scope` cannot establish — no or
  unfetchable base, a git error, a malformed record — runs everything. The merge
  queue uses the same rule (human decision, 2026-11-24, on #1971).
- **Superseded pull-request runs are cancelled.** The workflow's concurrency
  group is the pull request number for `pull_request` runs and the run id for
  every other run, with `cancel-in-progress`. Merge-queue and push runs are
  never cancelled. The aggregates and reconciliation use `!cancelled()` rather
  than `always()`, so a cancelled run starts no further jobs.
- **The rule runs in the verdict job, in Python.** A separate full-size scope
  job would make every lane wait for it to win a runner, the queueing the
  verdict job's matrix projection already removed; rewriting the rule in
  `github-script` would be a second implementation. So `dev.scope` and the rule
  (`dev.rawdiff`) import only the standard library and run on the slim image's
  own Python, as the matrix projection runs on its own `jq`; `dev`'s tests run
  the same modules under the pinned Python, one without site-packages (human
  decisions, 2026-11-25).

## Consequences

- A Markdown-only pull request and its merge-queue entry take a few
  runner-minutes instead of about 160. ADR-promoter pull requests qualify when
  they touch only Markdown; one that also rewrites a citation in code runs
  everything.
- `Validation host`, `main`'s kache writer, now waits for the seconds-long
  verdict job so it can skip a documentation-only change; it still runs for a
  merge-queue-tested push, and when the verdict job fails.
- `dev.scope` and `dev.rawdiff` must stay standard-library-only and run on the
  slim image's Python (3.12 as of this decision).
- A Markdown file that a product lane starts to read must change the rule, not
  be added silently. The source probes that used to dirty `README.md`
  ([ADR-0116](../0116-coverage-probe-dirty-tree-workaround.md)) now dirty their
  worktree with a new intent-to-add Markdown file of their own, so no tracked
  Markdown is load-bearing for them.
- Amends ADR-0029's narrow-class definition (Markdown deletes and renames are
  narrow; the `delete-or-rename` reason is retired) and ADR-0116's choice of
  dirtied file. Current state: `docs/ARCHITECTURE.md`.
