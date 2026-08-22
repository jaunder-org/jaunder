# Issue 1119: Focused `e2e-local` guidance

## Outcome

Developers and agents have a verified, copy-pastable focused browser-flow
validation path for changed web behavior. The docs make
`cargo xtask e2e-local <spec-or-file:line>` the local surface proof before broad
e2e gates when one Playwright spec or line-scoped test answers the question.

## Load-bearing decisions

- `CONTRIBUTING.md` owns the human-facing e2e validation rule: browser behavior
  changes need actual surface proof, and focused `e2e-local` is the default
  local proof when a single spec or `file:line` can cover the changed flow.
- Agent-facing workflow guidance must point agents to focused `e2e-local` for
  changed browser flows before broad e2e gates, while preserving
  `cargo xtask check` as the task/commit gate and PR CI as the authoritative
  boundary gate.
- The documented command surface must match xtask's real interface. Today
  `e2e-local` accepts one optional Playwright positional filter
  (`<spec-or-file:line>`) and `--update-visual-snapshots`; it does not expose
  arbitrary Playwright flags such as title `--grep` unless implementation proves
  otherwise and documents that support.
- Backend/browser defaults stay explicit: focused `e2e-local` owns its own
  server and temporary SQLite DB, runs the Chromium local lifecycle, and scopes
  both visual-prerequisite and ordinary/admin invocations to the positional
  filter. Full `{sqlite,postgres}×{chromium,firefox}` confidence remains
  CI/`cargo xtask validate` or `cargo xtask e2e <backend> <browser>`.
- The change is documentation/process guidance unless verification finds
  argument forwarding broken enough to require an xtask fix. It must not weaken
  CI, hook, visual snapshot, accessibility, or zero-panic policy.

## Acceptance

- `CONTRIBUTING.md` documents focused
  `cargo xtask e2e-local <spec-or-file:line>` usage for local browser-flow
  proof, including one spec-file example and one `file:line` example.
- The docs explicitly state whether title/grep selection is supported; if
  unsupported, they say to use Playwright's file or line targeting instead of
  showing fake `--grep` examples.
- The docs state the default backend/browser behavior and when to escalate to
  `cargo xtask e2e <backend> <browser>`, CI, or full `cargo xtask validate`.
- Agent-facing workflow guidance in the skill source tree under
  `~/src/agent-configuration` tells agents to use focused `e2e-local` for
  changed browser flows before broad e2e gates.
- The documented examples are validated against the actual command surface and
  avoid placeholder syntax that looks executable but is not.
- `devtool run -- cargo xtask check --no-test` passes after the docs change,
  plus one focused `e2e-local` smoke proves the documented path.

## Boundaries

- Do not change Playwright test behavior, fixtures, browser wait discipline,
  screenshot baselines, or accessibility policy.
- Do not replace the PR e2e matrix or ship gate with focused local proof.
- Do not add new validation policy tables or change hook behavior.
