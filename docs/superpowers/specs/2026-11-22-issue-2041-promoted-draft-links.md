# Issue #2041: cross-draft ADR links broken by an earlier promotion

## Outcome

An author whose PR is ejected from the merge queue because a linked draft ADR
was promoted first can fix it without investigating. The drafts README explains
the race and how to fix it, and the `doc-links` failure names the numbered ADR
that the draft became.

## Approach and tradeoffs

The promoter rewrites `drafts/<slug>` citations only in the tree it promotes
(`main`). A draft on an unmerged branch that links another PR's draft
(`../drafts/aaa.md`) keeps the old path. When that other draft is promoted
first, the merge group's `doc-links` check finds the link dead and ejects the
PR. Branch CI can't catch this, because promotion can happen after branch CI has
passed.

- **Docs:** `docs/adr/drafts/README.md` rule 5 will say that a link to a draft
  from another unmerged PR breaks if that draft is promoted first. The remedy is
  to rebase onto `main` and link `../NNNN-<slug>.md`. Rule-3 path citations of
  that draft (in code or prose) go stale in the same race, and no gate catches
  them, so the author rewrites those too. The last paragraph of "Gate
  invisibility", which says `promote` always adjusts cross-draft links, gets the
  same qualification.
- **Remedy hint:** in `tools/repository-policy`'s `doc-links` gate, a dead link
  whose normalized target (fragment stripped) is `docs/adr/drafts/<slug>.md`,
  directly in that directory, gets a hint. The hint applies only when exactly
  one tracked file in `docs/adr/` is named `<digits>-<slug>.md`, with the whole
  part after the first `-` equal to `<slug>`. The `tracked` set the gate already
  reads is enough to find it. The hint follows the existing
  `(use … to survive ADR promotion)` convention: it gives the literal
  replacement link, relative to the source file and keeping the fragment, as
  `(promoted: use ../0007-aaa.md#sec)`. The hint works for any source file, not
  just drafts. All other dead links keep their current output.
- **Rejected: running branch CI against current `main`.** Promotion can still
  happen after that check passes, so it narrows the race but doesn't close it.
- **Rejected: preventing the ejection.** That would need two changes. The gate
  would have to accept `drafts/<slug>` as an alias for a numbered ADR, and every
  promoter run would have to rewrite those stale aliases. That adds an aliasing
  rule to both the Rust gate and the Python promoter. It would also leave alias
  links on `main` that never get rewritten when a run has no drafts to promote.
  For a race seen once (#2035), that costs more than a rebase.
- **Accepted cost:** the PR is still ejected, and the author must rebase and get
  a fresh merge approval.

## Acceptance

- Unit tests in `links.rs`, with each source file linking its own path form to a
  draft `aaa`, and the full expected line asserted in each case:
  - Source `docs/adr/drafts/b.md` links `../drafts/aaa.md#sec`, and
    `docs/adr/0007-aaa.md` is tracked: the hint is
    `(promoted: use ../0007-aaa.md#sec)`.
  - Source `docs/x.md` links `adr/drafts/aaa.md`: the hint is
    `(promoted: use adr/0007-aaa.md)`.
  - No numbered match: plain `<file>:<line> -> <target>` output, unchanged.
  - Two numbered matches for the slug (`0007-aaa.md` and `0300-aaa.md`): no
    hint.
  - Slug overlap: a link to `drafts/verdict.md` when only `0255-ci-verdict.md`
    exists gets no hint.
- The README rule 5 and "Gate invisibility" text describe the race and the
  remedy.
- `dev check` is green.

## Boundaries

- No change to the promoter, the CI workflows, or whether the PR is ejected.
- No change to existing `doc-links` messages other than adding this hint.
