# Issue #2030 — E2E lane phases as child spans of the producer step

## Outcome

A `dev e2e-producer <lane>` trace shows the lane's VM phases (VM startup and
readiness, Playwright gate execution, result lift, post-gate checks) as child
spans of its `e2e-producer` step. Their durations equal those in that run's
`e2e-phase-<lane>.json`, and `dev trace summary` lists them with the rest of the
run.

## Approach and tradeoffs

Today the NixOS test driver in `nix/checks.nix` times each phase with
`time.monotonic()` and writes only `duration_ms`. A duration with no start
cannot be placed on the trace's timeline, and `dev` has nothing else to anchor
it to: the phases are separated by unmeasured gaps (package copy, DB seed,
census preflight), and the Nix build that hosts the VM runs before them.

1. **The manifest records when each measured phase started.** The driver also
   records the wall clock (`time.time_ns()`) when each measured phase starts,
   and writes it as a new `started_at_unix_ns` field on that phase. The field is
   additive, so `schema_version` stays 1. `duration_ms`, phase names, outcomes
   and the unavailable `nix-*` entries do not change. host-proof's serde ignores
   unknown fields, so `e2e-evidence` verify and reconcile keep their verdicts.
   The existing monotonic anchors stay exactly as they are; host-proof's VM
   contract test asserts them literally. The driver is host-side Python in the
   build sandbox, with no time namespace, so its wall clock is the clock the
   `dev` span records use, both locally and on CI runners. Changing the test
   script rebuilds every E2E check once.
2. **`dev` emits the children.** Once the producer step's child exits (success,
   failure or signal), and while the step's span is still open, `dev` reads the
   lifted `.xtask/diagnostics/e2e-<lane>/e2e-phase-<lane>.json`.
   `scripts/e2e-workflow` deletes the previous copy before every build, so this
   invocation lifted the file. It may still describe another run: a reused or
   substituted output, or a stale kept output on a failed build. Rule 3 decides
   that. For each phase that qualifies, `dev` emits a child span named after the
   phase. The span runs from `started_at_unix_ns` to that start plus
   `duration_ms`, so the length always comes from the monotonic measurement. A
   wall-clock step during the run can shift a span but never change its
   duration. The span's `jaunder.status` is `failed` when the phase's outcome is
   `failed`, otherwise `ok`. Explicit OpenTelemetry span start and end times go
   through the existing record writer, so the trace format and the merge stay as
   they are. The existing hook for recording the output source runs only on
   success, so this needs a small step hook that runs whatever the outcome.
3. **Honesty rules** (Milestone 33: absent or incomplete, never zero):
   - A phase that is `unavailable` or has no duration or start (the `nix-*`
     entries, or a manifest written before this change) gets no span.
   - A phase whose interval does not lie between the step's start and the time
     its child exited gets no span. This rejects any interval this step could
     not have produced: a reused or substituted output, or a stale kept output
     after a failed build. For a successful reused or substituted output, the
     step's `jaunder.cache` attribute shows why the children are missing. Two
     concurrent producers of the same lane share one diagnostics directory. That
     was already outside the cooperative assumption, and containment cannot tell
     their manifests apart.
   - A missing, unreadable or malformed manifest (a failed or timed-out lane,
     #2020) produces no children. A phase that never ran is not in the manifest,
     so it gets no span.
   - Reading the manifest never changes the step's status or exit code, and
     never adds a step to the gate.

The unmeasured gaps and the Nix fetch/build show up as the producer's
**Untraced** time in `dev trace summary`. That is honest: nothing measures them
today. **Rejected alternative:** lay the phases back to back, ending when the
step ends, with no Nix change. That needs no rebuild, but it invents placements
the run never observed. **Not done:** closing as not planned. The manifest
already exists, so the trace costs one field and one reader.

## Acceptance

- **Unit tests** (`tools/dev`, fake Nix): a lifted manifest whose phases fall
  inside the step gives `e2e-producer` one child per measured phase, at the
  manifest's start, with the manifest's duration, and `failed` status where the
  manifest says so. A reused output (phases outside the step), a missing
  manifest, a malformed manifest and an `unavailable` phase each give no child,
  and the step's status and exit code are unchanged. For the inside case, the
  fake `nix build` writes the manifest during the step, stamping each start with
  the current wall-clock time. The other cases can keep prebuilt store outputs.
- **CI proof (at ship):** on one PR CI E2E lane, `dev trace summary` for the
  producer run lists the phase children under `e2e-producer`. Their durations
  match that lane's uploaded `e2e-phase-<lane>.json` to the millisecond.
- **Unchanged verdicts:** existing `e2e-evidence` tests and the PR's CI E2E
  lane, reconciliation and server-function coverage verdicts all pass with no
  change to their code.
- **Docs:** `docs/observability.md` says what the producer's phase children are
  and when they are absent.

## Boundaries

- No new trace framework, exporter or span-record field. The manifest's only
  addition is `started_at_unix_ns`.
- No new phase boundaries (for example around seeding or the Nix build), and no
  change to which phases host-proof requires.
- Nothing changes the verdict of a lane, reconciliation or coverage check.
