# Issue 1118: Focused `test-local` guidance

## Outcome

Developers and agents can find and use focused
`cargo xtask test-local -- <nextest args>` runs for product Rust red/green work
before escalating to broader Jaunder gates. The guidance keeps
`cargo xtask check` as the per-task certification gate and makes broad green
gates a boundary proof, not the default debugging tool.

## Load-bearing decisions

- `CONTRIBUTING.md` owns the human-facing rule: use focused `test-local` when
  editing product Rust behavior and the desired proof is one Rust test, module,
  package, or subsystem.
- Agent-facing workflow guidance must point agents to the same focused lane for
  changed product Rust behavior, without replacing the required
  `cargo xtask check` commit gate.
- Examples must use real nextest argument shapes that Jaunder supports through
  `cargo xtask test-local --`, including a single test name, at least one
  package/module filter form, and rerunning the same focused command after a
  targeted fix.
- Escalation remains explicit: focused `test-local` is for red/green diagnosis;
  `cargo xtask check --no-test`, `cargo xtask check`, `cargo xtask prepush`, and
  CI/validation remain boundary or confidence gates according to their
  documented surfaces.
- The change is documentation/process guidance only. It does not change xtask
  behavior, gate coverage, hook behavior, or CI policy.

## Acceptance

- `CONTRIBUTING.md` states the focused Rust validation pattern before the broad
  gate ladder, not as an obscure aside after broad commands.
- Agent-facing workflow guidance tells agents to use
  `cargo xtask test-local -- <nextest args>` for changed product Rust behavior
  when a focused red/green loop is appropriate.
- The documented examples are validated against the actual command surface and
  avoid placeholder syntax that looks executable but is not.
- The guidance says broad green gates certify task or branch boundaries; they
  are not the default inner-loop debugging tool while a focused Rust test can
  answer the narrower question.
- `devtool run -- cargo xtask check --no-test` passes after the docs change.

## Boundaries

- Do not alter xtask command semantics, nextest configuration, hook behavior, CI
  jobs, or coverage policy.
- Do not weaken the requirement to run `cargo xtask check` before committing
  implementation work.
- Do not introduce new gate names or parallel validation policy tables; this
  issue documents the existing lane.
