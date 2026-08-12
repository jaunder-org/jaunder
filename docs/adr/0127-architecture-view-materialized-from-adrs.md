# ADR-0127: ADRs are immutable decision events; ARCHITECTURE.md is the materialized view

- Status: accepted
- Deciders: mdorman, Claude
- Date: 2026-08-12
- Issue: [#927](https://github.com/jaunder-org/jaunder/issues/927)

## Context

A full-corpus triage of ADRs 0000–0064 (July 2026) found the decision log
structurally sound — no ADR's Decision had been refuted by the code — but the
corpus was failing readers in two ways:

- **Addendum accretion.** Seventeen ADRs carried present-tense
  Addendum/Amendment/Supplement sections ("and now X is true") stapled onto
  their original text — mini-supersessions in place. Reading an ADR to learn
  current state meant mentally replaying its history, and the worst case
  (ADR-0050) had amendments woven through its Decision and Consequences.
- **No designated home for current truth.** "What is true now" was smeared
  across a fossilized `docs/ARCHITECTURE.md` (predating the CSR cutover and the
  `storage` crate split), the architectural half of `CONTRIBUTING.md`, and the
  addenda themselves. None was authoritative and none was updated when a new ADR
  landed — six ADRs (0059–0064) shipped in the week before the triage with zero
  view updates.

The tempting fix — rewriting each ADR so it "accurately reflects the current
codebase" — would destroy the log's value: an ADR is useful precisely because it
is pinned to the forces of its moment. The tension dissolves under an
event-sourcing model: ADRs are the immutable event log; current state is a
**materialized view** folded from them, rebuilt as events arrive, so nobody
replays history to learn the present.

## Decision

**ADRs are append-only decision events.**

- An ADR's Decision text is never edited to track the present. When a decision
  changes, a new ADR supersedes it (status `superseded` + reciprocal pointers),
  exactly as ADR-0030→0050 and 0055→0056 already did.
- In-place edits are limited to metadata and navigation: status-line changes,
  broken/moved pointers, and short **past-tense** annotations ("(since fixed:
  …)", "(shipped as `…`)").
- New addenda are written in past tense from birth — "as of <date>, Y held;
  current state: see ARCHITECTURE.md §Z" — never as present-tense patches
  (ADR-0033's `## History` is the exemplar). `docs/adr/template.md` carries the
  convention.

**`docs/ARCHITECTURE.md` is the materialized view.**

- It is the single authoritative statement of current architecture, organized by
  subsystem, and every claim cites the ADR(s) that established it.
- Where the view asserts something no ADR justifies, that claim is **listed
  explicitly** in a final `Un-ADR'd reality` section and tracked as an issue.
  The view does not silently carry unattributed claims, and it does not delete
  true ones to satisfy the citation rule: deleting a true statement loses
  content exactly as surely as asserting a false one. Each entry is then either
  ADR'd or judged not to warrant one, and the list is where the periodic replay
  audit starts so it does not re-discover the same gaps.
- It distinguishes **current reality** from **committed direction**:
  aspirational decisions (e.g. ADR-0005/0006/0009/0010 ingestion and federation)
  appear under an explicit direction heading, so unbuilt subsystems never read
  as built.
- `CONTRIBUTING.md` keeps process — how to set up, verify, and land work — and
  cross-links the view instead of restating structure. `CONTEXT.md` remains the
  domain glossary; both are projections in the same sense.

**The view is updated by two disciplines.**

1. **Online projection:** shipping an ADR includes updating `ARCHITECTURE.md`
   (and `CONTEXT.md` when the ubiquitous language changes) in the same change.
   The convention is stated in `docs/adr/template.md`; mechanical enforcement at
   `cargo xtask adr promote` — mirroring how promote already owns the README
   table projection — is committed follow-up work, tracked as an issue at ship.
2. **Periodic replay:** an occasional audit re-derives the view from the log
   plus the code — catching drift the online path cannot see (code that changed
   with no ADR). The July 2026 triage was the first such replay.

## Consequences

- Present-tense addenda are rewritten in past tense, so an addendum reads as a
  dated record rather than as a competing claim about now. **Only the tense
  changes.** Content is _not_ moved into the view: those addenda hold genuine
  rationale, and rationale is what an ADR is for. Compressing one into a pointer
  would destroy the thing the log exists to keep. Where the view describes the
  same subject, a pointer may be appended — an addition, never a replacement.
- Reading burden shifts favorably: newcomers read the view; only someone asking
  "why is it this way?" opens the cited ADRs.
- The promote-time gate adds friction to shipping an ADR — deliberately: an
  architectural decision that cannot say what it changed about the current
  architecture is not done.
- The view can be wrong in a way ADRs cannot (it makes claims about the
  present); the replay audit is the standing correction mechanism, and the
  citation discipline keeps every claim checkable.
- Amends ADR-0000 (documentation strategy): `ARCHITECTURE.md` is promoted from
  "a document that exists" to a generated-by-discipline projection with a
  defined update rule.
