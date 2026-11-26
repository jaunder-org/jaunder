# Issue #2115: e2e capture spans in the `dev` Perfetto trace

## Outcome

When someone opens a `dev` run's `.xtask/run/<run_id>.trace.json` in Perfetto,
each `e2e-producer` step's `gate-execution` phase now contains that lane's
Playwright run on the same timeline. The run shows each test's lifecycle, test,
page, context-mint and teardown spans, the server `request` spans those tests
caused, and the server and storage spans under them. This needs no flag. The
same spans appear in `dev trace summary`.

## Approach and tradeoffs

**Where the spans come from.** `scripts/e2e-workflow producer` already lifts the
lane's `capture-<lane>.tar.gz` into `.xtask/diagnostics/e2e-<lane>/`, beside the
phase manifest the step reads for its VM phases
([ADR-0263](../../adr/0263-dev-traces-producer-timed-phases.md)). A capture is
lifted even from a failed build's kept output. When the producer's child exits,
the step runs its existing post-child hook. That hook writes the phase spans and
then reads the capture through the trace core (`dev.traces`, #2100). The phase
writer returns the spans it wrote, so the capture can go under the
`gate-execution` span. The selected capture spans are written as finished
after-the-fact spans through the run's normal tracer and record writer, parents
before children. They use the same record format, golden file and merge as every
other span. There is no new trace format or exporter.

**Which spans: the lane's whole test tree.** The VM driver injects a per-lane
`JAUNDER_E2E_TRACEPARENT` into Playwright. The host step never sees that value,
so `dev` takes it from the capture. Every `e2e.test.lifecycle` span names the
same parent span, and that parent is not itself in the capture. If the lifecycle
spans name zero or several such parents, the capture is malformed. The tree is
every span whose parent chain reaches that injected parent. A lifecycle span, or
any other direct child of the injected parent, becomes a child of
`gate-execution`. Below that, each span keeps its capture parent.

On the measured CI lane (run 45888881602, `sqlite-chromium-2-of-3`), the tree is
14.8k of 23.5k spans. The rest are background roots that no test owns (feed
pollers, `write_scope`, server startup) and about 130 spans whose parent was
lost. Those are left out.

_Rejected:_ e2e spans only, with an opt-in flag for requests (the issue's
alternative). That adds a flag and a second path, and 15k spans per lane are
cheap for Perfetto once tracks are reused (below).

**Span content.** Each span keeps its capture name and its start and end times
in Unix nanoseconds. Its OTLP attributes are flattened to plain values:

- string and bool values stay as they are;
- `intValue`, a string in OTLP JSON, becomes an int;
- `doubleValue` becomes a float;
- any other kind of value is dropped, so a new kind never drops a lane.

The SDK's default limit of 128 attributes per span applies. The OTLP status is
kept as the attribute `otel.status_code`. A capture span gets no
`jaunder.status`, because that attribute is a `dev` step's outcome. That keeps
`dev trace summary` from counting a green lane's expected server errors as
failed rows; the measured green lane had 13.

**Clock window: all or nothing per lane.** The VM's spans carry the guest's
clock. The NixOS test driver times `gate-execution` on the host. On the measured
lane, the test tree began 1.8 s after `gate-execution` began and ended 0.39 s
before it ended. Following ADR-0263, which leaves out phases it cannot place,
`dev` checks only that every lifecycle span lies inside `gate-execution`. A
server span may outlive its test, and the track fix places such a span
correctly.

| Situation, checked in this order                                                 | Result                               |
| -------------------------------------------------------------------------------- | ------------------------------------ |
| No `gate-execution` span this run (reused output; a timed-out gate records none) | no capture spans, silent             |
| No capture file (a failed lane lifted nothing)                                   | no capture spans, silent             |
| Capture unreadable or malformed                                                  | no capture spans, one `dev:` warning |
| Any lifecycle span outside `gate-execution`                                      | no capture spans, one `dev:` warning |
| Otherwise                                                                        | the whole test tree                  |

The capture never changes the step's status or exit code, or the child's output.
It adds at most one `dev:` warning. The cost this accepts:

- A skew within the measured margin (about 1.8 s at the start and 0.4 s at the
  end) shifts the lane's spans by that skew.
- A larger skew drops the whole lane.
- A Playwright timeout records no `gate-execution`, so a timed-out lane shows no
  capture spans. A lane that failed and still recorded `gate-execution` shows
  them.

**Track layout fix (shared).** Today `chrome.tracks` never reuses a track: each
overlapping sibling opens a new track for good. On the measured lane, that makes
106 tracks for the e2e spans alone and 1,020 for the whole tree. With the fix,
each track records the latest end of any slice placed on it, nested slices
included. A span that cannot nest on its parent's track takes the first track
whose slices all end at or before its start. It opens a new track only when no
track is free. Track 0 stays reserved for the whole run, because top-level spans
nest on it. So no two slices on one track can partly overlap, which Perfetto's
nesting requires. A prototype on the same lane needs 26 tracks for the whole
tree. The final rule's count is re-measured in acceptance. The fix applies to
every `dev` trace.

**Cost accepted.**

- The capture read is not new cost. `e2e-evidence` already parses the whole
  capture (about 38 MB of OTLP JSON) in the same process.
- Writing about 15k spans and merging them is new cost, and so is a larger
  trace. `dev e2e` puts all 16 lanes in one run, about 240k capture spans held
  in memory at merge.
- The 20 slowest rows of `dev trace summary` may now include test-lifecycle
  spans.
- Acceptance 4 measures these costs.

**Decision record.** A numberless ADR draft extends ADR-0263 to a lifted OTLP
capture:

- It widens ADR-0263's amendment of ADR-0254's "attributes only" rule.
- It records the selection, the attribute and status mapping and the
  clock-window rule.
- It keeps ADR-0263's "nothing is exported over OTLP".

The track fix needs no ADR. The change is projected into
`docs/ARCHITECTURE.md`'s dev trace paragraph and CONTRIBUTING's "Observation and
traces". `docs/observability.md`'s `dev` trace section is corrected, because it
says `dev` spans are not request attribution.

## Acceptance

Proof to execute:

1. Hermetic `dev` tests (`tools/dev/tests/`), using the existing fake producer.
   It writes the capture's `otel-traces.jsonl` from a template on its own clock
   (`@NOW@` plus offsets), as it already does for the phase manifest, and tars
   it. The tests check that:
   - lifecycle spans are children of `gate-execution`, and requests are children
     of their test;
   - a direct child of the injected parent that is not a lifecycle span is
     included;
   - background roots and orphan spans are absent;
   - attributes are flattened, including dropped kinds;
   - `otel.status_code` is kept, and no `jaunder.status` is set.
2. Tests for each row of the clock-window table, in its order: the spans written
   or not, the warning present or absent, and the step's status and exit code
   unchanged. The existing `capture_only()` tests stay green.
3. `chrome.tracks` tests:
   - finished overlapping siblings free their track for a later span;
   - a child outside its parent claims a track, and a later sibling or orphan
     root cannot nest onto it;
   - over the merged events, no track holds two slices that partly overlap;
   - the existing trace-merge tests stay green.
4. One real local `dev e2e-producer <lane>`. Report:
   - the number of capture spans under `gate-execution`, and confirm that the
     clock window dropped none;
   - the track count;
   - the added step time, the merge time and the `trace.json` size;
   - an estimate for all 16 lanes.
5. `dev check` green.

Deliverables to commit: the code and tests, the ADR draft, and the
ARCHITECTURE.md, CONTRIBUTING.md and observability.md updates.

## Boundaries

- Only `e2e-producer` lanes. `dev perf` captures and other producers are out of
  scope.
- No change to the capture format, the VM driver, Playwright instrumentation, or
  any e2e verdict. Rendering is diagnostic and never fails a step.
- No CLI flag, no span-name filter, no clock correction.
- `dev trace summary`'s code does not change.
- Track names stay "the first span on the track".
