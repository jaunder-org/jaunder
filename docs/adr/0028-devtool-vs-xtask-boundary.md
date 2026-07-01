# ADR-0028: The `devtool` / `xtask` Boundary — In-Sandbox Producer vs. Host Analyzer

- Status: accepted
- Deciders: mdorman, Claude
- Date: 2026-06-27

## Context and Problem Statement

The coverage-pipeline Rust migration
([archive/2026-06-24-coverage-pipeline-rust-migration-design.md](../archive/2026-06-24-coverage-pipeline-rust-migration-design.md))
introduced three workspace pieces and split the old coverage bash between them:

- **`tools/devtool`** (bin crate) — runs **inside** the Nix coverage/e2e build
  sandboxes, where `xtask` and `nix` themselves are unavailable.
  `devtool coverage emit` runs the instrumented suite and _produces_ artifacts
  (`status.json`, reports, CRAP, diagnostics) into `$out` for exfiltration. It
  is deliberately cache-eligible (kept out of `xtask/`'s cache-exclusion
  boundary).
- **`xtask`** (bin crate) — runs on the **host**. It invokes `nix build`, then
  _consumes and analyzes_ the exfiltrated `$out` (the gap-based gate, baseline
  heal, CRAP gate) — work that is inherently host-only because it needs
  committed baselines and git context the sandbox lacks. Stays excluded from the
  coverage cache so frequently-edited gate logic never busts the expensive
  in-sandbox build.
- **`tools/coverage`** (lib crate) — pure logic (parsing, path normalization,
  classify, the baseline model) shared by both sides.

That migration deferred five further bash scripts to follow-up `tooling` issues
(#29–#33), each titled "migrate `scripts/X` into `devtool`." But the title was
applied uniformly without re-checking that each script actually belongs
**in-sandbox**. `scripts/audit-wasm-bundle` (issue #31) is the counterexample:
it runs `nix build .#site` — which **cannot** run inside a Nix build sandbox —
and then does pure host-side analysis (gzip/brotli sizing, table rendering).
Placing it in `devtool` would put a tool that shells out to `nix build` into the
crate defined as "the thing that runs where `nix` is unavailable," eroding the
very boundary the migration established. We need that boundary written down so
future migrations (and readers) don't have to reverse-engineer it from the
coverage code.

## Decision Drivers

- The boundary already exists in the code and the coverage design doc; it just
  was never stated as a rule, so the follow-up issues drifted from it.
- A migration's _home_ should be decided by **where the code must execute**, not
  by the wording of the issue that filed it.
- Keep the in-sandbox crate (`devtool`) small and genuinely sandbox-shaped so
  its build stays cache-eligible and its purpose stays legible.

## Decision Outcome

**Place each tool by a single litmus test: _where must this code execute?_**

- **`devtool`** — code that must run **inside a Nix build sandbox**, where `nix`
  and `xtask` are unavailable. Its job is to _produce / collect_ artifacts for
  exfiltration via `$out`. Cache-eligible.
- **`xtask`** — code that runs on the **host**: it _invokes_ `nix build`, and
  _consumes / analyzes_ exfiltrated artifacts, gates, and reports. Carries the
  `CommandResult` envelope, the `.xtask/last-result.json` sidecar, and the
  `xtask-done:` sentinel.
- **`tools/coverage`** (and any future shared lib) — pure logic used by both
  sides, with no I/O policy of its own.

**Litmus:** _Does it need to run where `nix`/`xtask` are absent (inside a
derivation)?_ → `devtool`. _Does it run on the host — invoking `nix`, or
analyzing build outputs?_ → `xtask`. Pure helpers either side needs → a shared
lib crate.

**Classification of the five deferred migrations** (#29–#33):

| Script                          | Runs                                          | Home                               |
| ------------------------------- | --------------------------------------------- | ---------------------------------- |
| `with-ephemeral-postgres` (#29) | in-sandbox (coverage `emit`, e2e derivations) | `devtool`                          |
| `seed-e2e-fixtures.sh` (#30)    | in-sandbox (Nix e2e checks)                   | `devtool`                          |
| `audit-wasm-bundle` (#31)       | host (`nix build .#site` + size analysis)     | **`xtask`**                        |
| `analyze-otel-traces` (#32)     | host (analyzes exfiltrated e2e traces)        | **`xtask`** (confirm in its cycle) |
| `run-e2e-trace-analysis` (#33)  | host (orchestrates e2e + analysis)            | **`xtask`** (confirm in its cycle) |

This ADR **supersedes the "into `devtool`" wording** of issues #31–#33 where it
conflicts with the litmus test; each issue is placed by where its code executes.
The #29/#30 placements are unchanged.

## Consequences

- Good: the boundary is now a rule, not folklore — future tooling lands on the
  right side without re-deriving it from the coverage migration.
- Good: `devtool` stays genuinely sandbox-shaped and cache-eligible; host
  analysis accretes in `xtask`, where the result envelope and Nix-invocation
  machinery already live.
- Trade-off: the milestone is no longer "all five → `devtool`." Two scripts land
  in `devtool` and three in `xtask`. The #32/#33 rows are the best current
  reading and are reconfirmed against this litmus when those cycles run.
- Neutral: shared pure helpers may justify new lib crates over time (as
  `tools/coverage` did); the rule covers them — pure logic is library code,
  independent of which binary calls it.

## Supplement (#158): `devtool run`

`devtool` gains a `run` subcommand: a no-shell single-command runner. It runs
exactly one program via `exec` (never `sh -c`), parks stdout/stderr under
`.xtask/run/`, and returns a JSON result (`exit_code`, `ok`, `signal`,
`duration_ms`, per-stream `{path, bytes, lines}`); its own exit status equals
the child's, so a caller's pass/fail signal is honest without shell scaffolding
(`; echo $?`, `2>&1 | tail`, `| rg` — all of which silently overwrite the exit
status). It refuses shell re-entry (`bash -c`, `nix develop`, …), so an
allowlist entry for it is narrower than one for `bash`.

This fits the litmus on both sides: it is the in-sandbox process runner, and it
is also useful on the host as the gate-execution surface for humans and agents.
`devtoolBin` is therefore exposed in the **default devShell** (direnv) in
addition to the coverage sandbox's `nativeBuildInputs`. No git-revision build
stamp is added: `devtoolBin` is a build input to the coverage check, so stamping
it with the repo revision would bust the coverage cache on every commit;
`devtool --version` reports the crate version, and staleness while developing
the runner is handled by running it live via `cargo run -p devtool`.
