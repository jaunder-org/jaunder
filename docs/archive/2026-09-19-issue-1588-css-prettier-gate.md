# CSS Prettier Gate

## Outcome

Every tracked CSS file in the repository that is not excluded by
`.prettierignore` is checked and routinely reformatted by Prettier through
Jaunder's normal verification ladder.

## Load-bearing decisions

- The formatting population is repository-wide `**/*.css`, subject to the
  repository's existing `.prettierignore` rules.
- Server assets, end-to-end styles, and theme test fixtures belong to the same
  formatting population; directories are not admitted individually.
- CSS formatting is a dedicated `prettier-css` check with both verify and fix
  modes.
- In the ordered source-consistency formatter sequence, `prettier-css` runs
  immediately after `prettier-end2end` and before `elisp-fmt`.
- The check belongs to the broad code/static surface, including its hermetic Nix
  execution.
- The check is not eligible for the staged-Markdown-only pre-commit route, so a
  Markdown-only commit cannot rewrite unrelated CSS.
- The host and hermetic source populations must expose the same tracked CSS
  files to Prettier.

## Acceptance

- Broad fix-mode verification reformats an intentionally misformatted tracked
  CSS file anywhere in the repository that is not excluded by `.prettierignore`.
- Check-mode verification rejects that same misformatting without mutating it.
- All currently tracked CSS files are included, including files under `server/`,
  `end2end/`, and `testdata/`.
- The hermetic static-code check receives the repository-wide CSS population and
  runs the same `prettier-css` contract as the host check.
- The staged-Markdown-only catalog remains unchanged by the new CSS check.
- Catalog and command-construction tests lock the check's name, population,
  ordering, and check/fix arguments.

## Boundaries

- This work does not change CSS style rules or introduce project-specific
  Prettier configuration.
- It does not reclassify CSS as documentation or change Markdown/end-to-end
  formatting populations.
- It does not add new CSS files or alter rendered presentation beyond canonical
  formatting of existing files.
