# Issue #2116 — render `dev`'s tables with `tabulate`

## Outcome

Every table `dev` prints is drawn by one helper, `report.table`, on top of
`tabulate` (pinned by #2100). The hand-rolled padding in `dev trace summary` and
`dev wasm-audit report` is deleted. Columns, headings and cell formats stay the
same; only the drawing changes.

## Approach and tradeoffs

Audit of `tools/dev/dev` found three hand-rolled table sites; `boot_phases`'
per-population lines are labelled fields, not a table, and stay as they are.

| Command                             | Tables                           | Today                    |
| ----------------------------------- | -------------------------------- | ------------------------ |
| `dev trace summary`                 | slowest spans                    | `ljust`/`rjust` padding  |
| `dev trace summary --markdown`      | same, for `$GITHUB_STEP_SUMMARY` | hand-built pipe table    |
| `dev wasm-audit report`             | artifact / raw / gzip / brotli   | f-string widths (`:<16`) |
| `dev wasm-audit report --breakdown` | sections; crates                 | f-string widths (`:<18`) |

`report.table(headers, rows)` (used by `trace-analysis` and `boot-phases`) gains
two optional arguments:

- `align`: per-column `"left"`/`"right"` (tabulate's `colalign`). Numbers stay
  right-aligned, text left, as today. Omitted, output is unchanged, so the
  existing `trace-analysis`/`boot-phases` tables do not move.
- `markdown`: tabulate's `pipe` format instead of the box-drawn
  `simple_outline`, with `|` in cells escaped as `\|` (tabulate does not escape
  it; span names can contain one).

The terminal tables become box-drawn like `trace-analysis`' — one look for every
`dev` table. Accepted cost: wider output and different exact text; no consumer
parses it (CI only appends the Markdown form to step summaries, docs quote no
sample output).

**`pipe`, not `github`.** The issue names `github`, but tabulate 0.10's `github`
rule has no alignment colons, so GitHub would left-align the duration columns.
`pipe`'s rule ends in `:` for right-aligned columns (and starts with `:` for
left ones), so GitHub renders the table as today. The Markdown text itself
changes: `pipe` pads every cell to its column width.

The `Runner:` line and the blank line before a Markdown table stay in
`summary.render`. wasm-audit's title/path/total lines stay as prose above its
tables, and `--breakdown`'s blank line and "crate attribution …" caption stay as
prose between the sections and crates tables. The comment that Span is last "so
a long span name needs no padding" becomes false and is reworded (Span stays
last for readability).

## Acceptance

- `rg -n 'ljust|rjust|:[<>]\d' tools/dev/dev` finds no match.
- `test_tracing.py` (plain and Markdown) and `test_wasm_audit.py` assert the new
  exact output: numeric columns right-aligned, empty cells empty, Markdown rule
  ending in `:` for numeric columns, `|` in a span name escaped in Markdown.
- `test_report.py` covers `align` and `markdown` on `report.table`.
- `pytest tools/dev/tests` passes locally; the `dev-python` check (ruff,
  `mypy --strict`, 100% branch coverage) stays green.

## Boundaries

- No change to which rows/columns are shown, number formatting, or CLI flags.
- No change to the `trace-analysis`/`boot-phases` tables' output.
- No new dependency; `boot_phases`' labelled lines are not converted.
