# Issue #2166 — deterministic collector fault injection in the `e2e-local` tests

## Outcome

The `dev e2e-local` tests that make the fake collector sabotage the first
instance's setup fail the run with status 1 every time, regardless of process
scheduling.

## Approach and tradeoffs

`dev e2e-local` treats the collector as ready once both of its loopback ports
accept connections, then immediately starts instance 0: it creates
`<capture>/instance-0/` and then a temporary storage directory under `TMPDIR`.

The fake `otelcol-contrib` in `tools/dev/tests/test_e2e_local.py` currently
binds its two listeners first and only afterwards injects two faults:

| Behaviour       | Fault                                | Step it should fail     |
| --------------- | ------------------------------------ | ----------------------- |
| `instance-file` | creates `instance-0` as a plain file | `instance-0-server-log` |
| `readonly-tmp`  | makes `TMPDIR` read-only (`0o500`)   | `instance-0-tmpdir`     |

Once the listeners are up, `e2e-local` can win the race and create the directory
before the fault lands; the run then succeeds (observed once for `instance-file`
under a loaded `dev precommit`). `readonly-tmp` has the same ordering and the
same race, so it is fixed too.

The change moves both injections to just after the bind-collision early exit
(`collide`, `collide-once`) and before the listener binding. Readiness then
implies the fault is already in place, so the fixture's ordering guarantees it,
not timing. The capture directory exists before the fake is spawned, and
`TMPDIR` going read-only does not block writes into that existing subdirectory.
Nothing else in the fake depends on the order. The cost we accept: the order of
operations in the fake is now load-bearing, and only a comment and review
enforce it.

No production code changes; `e2e_local.py` already reports both failures
correctly when the fault is present.

## Acceptance

- The determinism proof is the ordering itself. A green run cannot show that a
  race is gone. In the fake collector script, the `instance-file` and
  `readonly-tmp` injections run after the collision exit and before any listener
  is bound. Check this in the diff.
- No regression: `traced-run -- pytest tools/dev/tests/test_e2e_local.py`
  passes.
- The parametrize table of `test_a_failing_step_is_reported_and_fails_the_run`
  is unchanged in the diff.

## Boundaries

- Test fixture only; no change to `tools/dev/dev/e2e_local.py` or its readiness
  semantics.
- No new synchronization machinery (files, signals, retries) in the fake.
