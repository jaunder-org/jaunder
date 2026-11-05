# ADR sibling-link promotion prevention — issue #1900

**Status:** Approved spec; implementation not started.

## Outcome

Ensure the four observed cross-draft links survive promotion and prevent another
bare relative link to a relocating sibling draft from reaching `main`. Keep the
documented `../drafts/<slug>.md` form as the contract, preserve valid unrelated
relative links, and retain the existing promoter warning and hard `doc-links`
gate behavior.

## Evidence and context

Promoter run 41966201735/job 127043657757 failed its pre-push `doc-links` gate
on four generated ADR targets. Their bare sibling links resolve while drafts
share `docs/adr/drafts/`; `doc-links` checks tracked-target existence relative
to the current file and does not model relocation
(`xtask/src/doc_links.rs:196-214`). Promotion moves each draft and strips a
leading `../` (`xtask/src/adr/promote.rs:160-165`,
`xtask/src/adr/rewrite.rs:18-50`); Pass C rewrites `drafts/<slug>` citations
(`promote.rs:180-192`). Documented cross-draft links use `../drafts/<slug>.md`
(`docs/adr/drafts/README.md:35-41`) and are already covered before/after by
`tracked_draft_links_and_citations_resolve_before_and_after_promotion`
(`promote.rs:380-402`). Push CI and merge-group passed; #1872 changed no
ADR/promoter paths. Run 41923596652 was observed successful, so this is not a
claim that promotion generally fails.

## Approach and tradeoffs

1. Normalize the four affected draft links to the existing documented
   `../drafts/<slug>.md` form. This is the smallest direct repair for known bad
   inputs.
2. Extend existing pre-merge `doc-links` handling with this exact predicate:
   apply the new rule only when both the source and resolved sibling target are
   immediate `docs/adr/drafts/*.md` files eligible for relocation under
   `promote`'s current selection (`xtask/src/adr/promote.rs:86-94`), excluding
   `README.md`. For such a source-target pair, reject target spellings
   `sibling.md` and `./sibling.md`; classify after removing any fragment, and
   preserve that fragment in the suggested `../drafts/sibling.md#fragment`
   remedy. Other sources and targets retain current existence-only validation.
   Reuse existing scanner/resolver and gate; no new framework or universal
   portability policy.
3. Keep `promote` post-staging unresolved-link warnings intentional and
   unchanged (`promote.rs:217-238`, tested at `:349-365`). The existing hard
   pre-push `doc-links` gate remains the publication barrier.

The prevention check adds only this focused source-time invariant rather than
teaching promotion another link syntax. It makes no comprehensive portability
claim.

## Acceptance

- All four affected draft links use the canonical form and resolve both before
  and after promotion.
- Existing canonical before/after test remains green.
- In ordinary `doc-links`, reject `sibling.md`, `./sibling.md`,
  `sibling.md#fragment`, and `./sibling.md#fragment` when both source and
  resolved target are immediate relocating draft files; diagnostics suggest
  `../drafts/sibling.md`, retaining `#fragment` when present.
- Accept canonical `../drafts/sibling.md#fragment`; exclude target
  `docs/adr/drafts/README.md` and exclude nonrelocating source
  `docs/adr/drafts/README.md` from the new rule.
- Preserve ordinary existence validation for unrelated links, including valid
  non-draft relative links.
- Existing promotion before/after test remains green. Promoter warning semantics
  remain successful-warning behavior; unresolved generated links still fail the
  ordinary hard `doc-links` gate.

## Review resolution and boundaries

Cold review finding: the original draft left source eligibility and `bare`
ambiguous. Resolved by constraining the rule to a relocating source and
relocating immediate sibling target under `promote` selection, excluding
`README.md`; matching both `sibling.md` and `./sibling.md` after fragment
removal; and retaining fragments in the canonical remedy. Explicit acceptance
cases cover both spellings with and without fragments, canonical fragment form,
README source/target exclusions, and unchanged ordinary existence validation.
Existing scanner and promotion fixtures are sufficient; no broader review or
research loop is needed.

No bare-link rewriting in promotion, syntax/framework expansion,
universal/all-Markdown portability guarantee, warning-to-error change, ADR
numbering, controller retry, manual promoter branch mutation, or #1897/#1859
replacement work. Do not close a parent or assert a native dependency absent an
actual prerequisite. Future implementation must use a semantic branch
`issue-1900-...` from `origin/main`; preserve the current diagnosis
checkout/branch.

## Approval provenance

Human approval for the bounded issue classification/scope under
`adr-sibling-links-prevention-v1`: “agreed, and sure, put it under milestone33”.
Explicit spec approval: human approval `c32ec14c-47ba-466d-9b82-274254be1b48`,
request `issue1900-adr-links-spec-v1`, exact response **“Approve spec and
begin”**. The revised proposal received a cold soundness review with no findings
(`result:cold-spec-review#3`).

Live issue #1900 was verified open as Bug/P1 with labels `tooling` and `dx`,
milestone #33 (“Streamline testing without reducing protection”), in Jaunder
Backlog; its current status is In Progress. This is the approved current spec
status and supersedes only the proposal's historical pending-approval wording;
no implementation has begun.
