# Issue #2126: the measured-phases producer test stops flaking

## Outcome

`test_a_failed_producer_shows_only_this_runs_measured_phases[measured]` passes
on every run, whatever the timing between the fake producer exiting and `dev`
reading its phase manifest.

## Approach and tradeoffs

`trace.record_phases` keeps a phase only when it lies inside the producer step
so far: `started_ns <= phase.start_ns <= phase.end_ns <= ended_ns`, where
`ended_ns` is taken when `dev` reads the manifest. The test's fake producer
stamps both phases with its own "now" and writes them at once. The phases (3 ms
and 2 ms long) therefore end in the future, and `dev` drops them if it reads the
manifest within 3 ms.

The fix changes only the fake, in `tools/dev/tests/test_workflows.py`. When it
writes phases, it takes its start time, sleeps for the longest phase, and only
then writes the manifest. That is the order a real producer follows: it measures
a phase, then reports it. Both bounds then hold by construction, on the same
`time.time_ns()` clock:

- the step's start is taken before the fake launches;
- `time.sleep` sleeps at least as long as asked, so the phases have ended before
  `dev` reads them.

The longest duration becomes one constant that `MEASURED` and the fake share, so
the sleep cannot fall out of step with the phases.

The issue suggested backdating each phase to `now - duration` instead. That
holds only as long as interpreter startup outlasts the phase, rather than by
construction. The product filter in `trace.record_phases` is correct and stays
unchanged.

## Acceptance

- One-off red/green proof, not committed: with the shared duration temporarily
  raised to 1000 ms, the current fake fails the `measured` case every time, and
  the fixed fake passes it.
- The `missing`, `not-a-list` and `malformed` cases, and the out-of-window
  `earlier-run` exclusion, are unchanged and pass.

## Boundaries

Test fixture only; no change to `dev`'s phase handling.
