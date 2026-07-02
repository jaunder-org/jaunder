# Spec — Generate the `docs/README.md` ADR table from `docs/adr/` (#196)

**Issue:** jaunder-org/jaunder#196
**Date:** 2026-07-02
**ADR:** addendum to **ADR-0036 (Identifier-Collision Policy)** — the README ADR
table becomes a generated projection of the ADR directory. No new ADR number.

This spec is a *record* of what the design interview resolved, not a task list
(that is the plan). It captures the decisions and the reasons, including the ones
that departed from the issue's literal proposal.

## Problem (recap)

`docs/README.md` carries a hand-maintained ADR index table. Every new ADR must add
a row `| [NNNN](adr/NNNN-slug.md) | Title | status |`. `cargo xtask adr renumber`
already rewrites path-form and bare `ADR-NNNN` references when it bumps a number,
but it does **not** touch the table's visible bare `[NNNN]` number cell — so under
the planned always-`0000` authoring flow (write `docs/adr/0000-<slug>.md`, let
`renumber` assign the number) that cell would be wrong on every author. The table
is also an unchecked mirror: a wrong status, a stale title, or a missing row drifts
silently.

## What the interview changed vs. the issue's literal proposal

The issue proposed generating the **whole** row, with the **title taken verbatim
from the ADR `#` heading**. Grounding that against the real data killed it:

- **13 of 42 README titles are intentionally curated** — short, Title-Cased index
  labels — while the `#` headings are long, sentence-case, sometimes
  parenthetical/backticked prose (e.g. 0023 heading ends `(format media types,
  \`j:slug\`, capability discovery)`; the index says `AtomPub Jaunder Wire
  Extensions`). Generating titles from headings would overwrite all 13 with uglier
  long-form text.
- **Heading format is `# ADR-NNNN: Title`**, not `# NNNN. Title` as the issue body
  states — and it isn't uniform: **12 of 42** files use the old `# NNNN. Title`
  form (0019, 0021, 0022, 0026, 0030, 0032, 0033, 0034, 0037, 0039, 0040, 0041).
- **Status lines aren't uniform**: 0015 trails prose after the token; 0029 and 0030
  use a bare `Status:` line (no `- ` list marker).

So the resolved design does two things:

1. **Narrows the generator to the mechanical cells** and **keeps titles
   hand-owned**, making the drift gate robust — it never fights a curated title,
   and never fights prettier's table formatting.
2. **Makes ADR file style a real invariant** rather than something the parser
   tolerates: fix every heading/status outlier now, and add a conformance gate so
   the corpus stays uniform. This lets the generator parse **one** heading form and
   a **fixed** status vocabulary, not a fuzzy union.

## Resolved design

### D1 — The generator owns number + link + status; titles stay hand-curated

`cargo xtask adr sync-readme` regenerates, for each ADR file in `docs/adr/`, only:

- **number cell** — `[NNNN]`, from the filename's leading four digits;
- **link target** — `adr/NNNN-slug.md`, the filename;
- **status cell** — the ADR's canonical status token (see D4).

The **title cell is preserved verbatim** from the existing row. It is never
overwritten once a row exists.

Row set reconciliation (all keyed by ADR number):

- **Existing row** → rewrite its number/link/status cells in place, keep its title.
- **ADR with no row (new)** → **create** the row, seeding the title cell from the
  ADR `#` heading with the `ADR-NNNN:` / `NNNN.` prefix stripped. Thereafter the
  title is hand-owned and never touched again. This keeps "no table row touched by
  hand" true for the mechanical authoring path while giving a sensible default.
- **Row with no ADR (orphan)** → **remove** it.
- Rows are emitted **sorted by number ascending**.

The command is **idempotent** and touches **only the delimited table block** (D2).
It emits simple single-space-padded cells; column alignment is prettier's job and
the parity gate compares semantically (D5) — sync-readme does not try to reproduce
prettier's padding.

### D2 — The table block is delimited by HTML-comment markers

The table lives between:

```
<!-- adr-table:begin -->
… table …
<!-- adr-table:end -->
```

sync-readme replaces only what is between the markers; the rest of `docs/README.md`
is untouched. Adding these markers to `docs/README.md` is part of this work.
Prettier preserves HTML comments, so the markers survive the markdown gate.
If the markers are absent, sync-readme and the gate check both fail with a clear
message (they are always present post-merge).

### D3 — Fold `sync-readme` into `adr renumber`

After `adr renumber` assigns a number (moves the file, rewrites references), it runs
the same `sync-readme` regeneration in the same invocation, so number assignment and
the table stay in lockstep — a collision-bump refreshes the table automatically.

### D4 — Canonical ADR file style (heading + status), enforced; fix all outliers now

**Canonical heading:** `# ADR-NNNN: <title>` on line 1, where `NNNN` matches the
filename's leading number. Rewrite the **12** files still on `# NNNN. <title>` to
this form (title text preserved verbatim).

*Why `# ADR-NNNN:` and not `# NNNN.`:* it is the 30-file majority, it matches the
`ADR-NNNN` bare-reference token used in prose and by `renumber`, and — critically —
the heading then **contains** an `ADR-NNNN` token, so `renumber`'s existing
`rewrite_bare` fixes the file's own heading number when it bumps a collision. Under
always-`0000`, writing `# ADR-0000: …` and running `renumber` rewrites the heading
to `# ADR-0043: …` for free. The `# NNNN.` form's `0000.` is not an `ADR-` token
and would silently keep the stale number. (Confirm `rewrite_bare` covers the
just-`git mv`'d file during implementation.)

**Canonical status line:** exactly `- Status: <token>`, where `<token>` is one of
the fixed vocabulary `{proposed, accepted, superseded, deprecated, rejected}` and
the line carries nothing after the token. Normalize the 3 non-conforming files (no
information lost):

- **0015** → `- Status: accepted`; move the "content-type token scheme superseded
  by [ADR-0023]…; the separate-serializers principle stands" note to a separate
  line (e.g. a `- Note:` bullet) so the supersession context is retained. Its
  README status stays `accepted`.
- **0029** → `Status: accepted` → `- Status: accepted` (add the list marker).
- **0030** → `Status: accepted` → `- Status: accepted` (and its heading is one of
  the 12 rewritten above).

Full outlier set fixed by this issue: **12 headings** + **3 status lines** (0030
appears in both) — 14 distinct files.

### D5 — Two read-only gate steps: format conformance, then table parity

Both are read-only siblings of `identifier-collisions` (run wherever
`sequence_check::run` runs today — `check` and `validate`), each with its own step
name and recovery hint.

**`adr-format`** — every `docs/adr/NNNN-*.md` must have a canonical heading
(`# ADR-NNNN:` with `NNNN` matching the filename) and a canonical status line
(`- Status: <token>` with `<token>` in the fixed vocabulary). **Fails** listing
each offending file and what's wrong (bad heading form, mismatched number, missing
/ malformed status, out-of-vocabulary token). This is logically upstream of parity:
a malformed ADR can't be projected into a table row, so format is checked first.
Recovery is a guided manual fix (there is no auto-fixer command in this issue).

**`adr-readme-parity`** — the committed table has not drifted from `docs/adr/`.
- **Semantic comparison, not byte-exact.** Parse the committed table's rows into
  `(number, link-target, status)` tuples and compare against what `docs/adr/`
  implies — plus **row presence** (every ADR has a row; no orphan rows) and
  **ordering** (ascending by number). Title cells are **not** compared (not owned).
  Prettier's whitespace/padding never causes false drift.
- **Recovery hint:** `recovery: cargo xtask adr sync-readme`.
- Robust to a transient duplicate number (the always-`0000` sentinel before
  `renumber`): it must not panic; `identifier-collisions` is the check that speaks
  to the duplicate, and its `adr renumber` recovery also re-syncs the table.

## Non-goals

- Not changing the curated title text of existing rows.
- No ADR-format **auto-fixer** command (e.g. `cargo xtask adr fmt`) — the one-time
  outlier cleanup is done in this issue; the ongoing gate is read-only with guided
  recovery. An auto-fixer could be a later follow-up.
- Not touching the existing `identifier-collisions` check behavior.
- Not enforcing the rest of the metadata block (Deciders/Date) — only heading and
  status are in scope.

## Outcome

ADR authoring becomes: write `docs/adr/0000-<slug>.md` with a canonical
`# ADR-0000:` heading and `- Status:` line, run `cargo xtask adr renumber`, stage,
commit — no number picked (renumber assigns it and rewrites the heading, refs, and
table together), no table row touched by hand (a new row is auto-created with a
heading-seeded title the author may later curate). The gate refuses any ADR whose
heading/status has drifted from canonical form (`adr-format`) and any table whose
number/link/status cells or row set have drifted from the directory
(`adr-readme-parity`). Unblocks the held `jaunder-adr` skill.
