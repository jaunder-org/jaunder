# ADR-0088: Promotion is the acceptance event

- Status: accepted
- Date: 2026-07-31
- Issue: [#741](https://github.com/jaunder-org/jaunder/issues/741)

## Context

ADR-0048 (#219) moved ADR numbering out of authoring: a decision is drafted
numberless in `docs/adr/drafts/`, and `cargo xtask adr promote` assigns the
number at ship. The drafts pen exists so an ADR's number is never chosen by hand
and never races another branch.

`docs/adr/template.md` seeds `- Status: proposed`. Nothing ever changes it.
`run_promote` (`xtask/src/adr.rs`) rewrites the draft's heading token to the
assigned number and leaves the status line alone; `file_format_problems`
(`xtask/src/adr_readme.rs`) checks only that the token is in `STATUS_VOCAB`. So
`proposed` is a default state with no transition out of it and nothing that
notices.

At the time of this decision, 11 of the 88 numbered ADRs carried `proposed`.
Every one was audited against the tree and every one is in force — the decision
is implemented, and in most cases gate-enforced by the very check the ADR
mandates (0063, 0065, 0066, 0068, 0072, 0073, 0081, 0082, 0084, 0085, 0086).
None is superseded. ADR-0086 is the clearest instance: the commit that promoted
it is titled "promote ADR-0086" and the file still reads `proposed`.

This is not a backlog of open proposals. It is one mechanical omission, 11 times
— which is what makes it worth fixing mechanically rather than by remembering.

The `docs/README.md` table's Status column is generated from those files, so the
index is stale in exactly the same 11 places. `adr-readme-parity` stays green
throughout, because it faithfully mirrors a wrong file. Parity is not
correctness, and a gate that can only compare two artifacts cannot tell you they
are both wrong.

## Decision

**`drafts/` is the `proposed` state. Promotion is the acceptance event, and a
numbered ADR is therefore never `proposed`.**

Three consequences follow directly.

1. **`promote` writes the status.** A draft marked `proposed` graduates as
   `accepted`, in the same Pass B rewrite that replaces the heading token.

2. **Every other token passes through untouched.** `superseded`, `rejected`, and
   `deprecated` on a draft are deliberate — an ADR written to record a reversal,
   or to document a decision already dead — and promotion must not overwrite an
   author's explicit statement. Only the template's default is rewritten.

3. **`proposed` on a numbered ADR is an `adr-format` problem.** The gate is what
   makes the property hold; the rewrite alone would leave a hand-created ADR
   free to rot exactly as these 11 did. The status vocabulary therefore splits
   in two: the full set a _draft_ may carry, and the subset a _numbered_ ADR may
   carry. Keeping one flat vocabulary would leave the out-of-vocabulary error
   advertising `proposed` as legal while a sibling rule rejected it.

The reasoning is that an ADR is out-of-git and numberless _exactly while_ the
decision is under consideration. Numbering is not bookkeeping that happens to
accompany acceptance — it **is** acceptance. The status line should say so
rather than requiring a human to remember to say it.

## Consequences

**The status column becomes trustworthy.** Today a reader cannot distinguish
"this was proposed and never decided" from "nobody edited a line", so the column
carries no information. After this, `proposed` cannot appear on a numbered ADR
at all, and the remaining tokens all mean something a human chose.

**The status line gets one parse.** It was previously parsed twice, differently
— `status_token` tolerated indentation and a bare `Status:`,
`file_format_problems` did not. A rewrite that disagreed with the gate about
which line is the status line would emit a promoted file that immediately fails
`adr-format`, so the two parses collapse into one shared helper that the
rewrite, the gate, and the table renderer all consume.

**No escape hatch, initially.** A genuinely-under-consideration decision belongs
in `drafts/`, which is what the pen is for. If a real need for a
numbered-but-proposed ADR appears, it can be added then — but building the
exemption before the case exists would reintroduce the state that rotted.

**The existing 11 are corrected by hand, not by the promoter.** `promote` has no
knowledge of ADRs it is not promoting, and a blanket sweep would silently assert
that 11 decisions are in force. They are audited once, in this issue's change;
the gate holds the line afterwards.

**`docs/adr/template.md` keeps seeding `proposed`.** It is the honest state of a
draft, it keeps a draft a well-formed ADR minus its number (which `jaunder-adr`
step 2 documents), and it is gate-exempt because it carries no leading number.

**This is independent of where promotion happens.** #742 proposes relocating
promotion from ship to merge, in a promoter PR, to remove the ADR-number and
README-table conflict class. That changes _when_ the acceptance event occurs,
not that promotion is it — so this decision stands unchanged under either flow,
and the promoter PR performs exactly this rewrite.
