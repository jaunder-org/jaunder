# Issue #2029: server spans lost in the server's export pipeline

## Outcome

Losing server spans can no longer show up as an orphan diff in the
server-function coverage snapshot. When the Jaunder server's OpenTelemetry SDK
reports losing spans, server-function coverage fails with a named "capture
incomplete" error that gives the lane and the reported loss. The E2E VM's server
also gets a larger span queue, so ordinary host load does not overflow it.

## Root cause

- **Instance.** PR #2021's local run captured one orphan,
  `timeline::list_home_timeline` with reason `unknown-parent:<id>`. That capture
  no longer exists, so the parent id cannot be checked.
  - The cause is inferred to be span loss in the server's export pipeline, most
    likely a full queue.
  - That fits the host's state at the time: the 15-minute load average was
    about 15.
- **Mechanism.** The server exports spans through the SDK's
  `BatchSpanProcessor`.
  - It loses spans when its queue is full; the default queue holds 2048.
  - It also loses spans when an export fails (a whole batch of up to 512) and
    when flushing at shutdown times out.
  - Suppose the lost span is a `request` span and its child span for the server
    function survives. The extractor walks up from the child, reaches a parent
    id that is not in the capture, and reports
    `unknown-parent:<request span id>`. That is exactly #2029's signature.
- **Demonstration (queue-full path).** One lane
  (`sqlite-chromium-serial-special`) ran with `OTEL_BSP_MAX_QUEUE_SIZE=16`, as a
  probe that was not committed.
  - The server dropped 280 spans, which produced 55 orphan hits. Every one was
    `unknown-parent:<missing request span id>`.
  - The SDK recorded the loss in the capture's `capture/diag.log`: it wrote
    `BatchSpanProcessor.SpanDroppingStarted`, then
    `BatchSpanProcessor.SpansDropped` with `dropped_span_count`.
  - The diag logs from a normal three-lane run contain no `opentelemetry_sdk`
    record. Normal peak load is about 960 server spans per 5 s.
- **Ruled out.** Eight CI captures, about 14k server-function hits each, contain
  no orphans and no dangling parents, including runs where a test was cut off by
  a timeout. Two other ways a parent can be missing were checked:
  - A request carrying the run-wide traceparent. No `/api/` request does.
  - A test whose `e2e.test` span was never exported. Interrupted tests still
    export theirs.

## Approach and tradeoffs

1. **Detect (class fix).** Server-function coverage reads each lane's
   `capture-*.tar.gz`. It will now also read `capture/diag.log` from it.
   - **Archive reading.** It uses the existing shared archive-member reader,
     generalised from the trace member to a path parameter, not a second tar
     reader.
   - **Refusal.** The capture is refused if any JSON line has target
     `opentelemetry_sdk`, level WARN or ERROR, and a `fields.name` starting with
     `BatchSpanProcessor.`. That covers queue-full drops, export errors and
     shutdown timeouts.
   - **Error message.** It names the archive and the record names. When a
     `dropped_span_count` is present it reports "at least N spans": an export
     error or a shutdown timeout loses spans that this count does not include.
   - **Empty or malformed log.** An empty `diag.log` passes; the log holds only
     WARN and above, so a clean run can leave it empty. Lines that are not JSON
     are ignored, as the panic gate does.
   - **Missing log.** A missing `diag.log` is refused: without it, loss cannot
     be ruled out. The server can run without the file if opening it fails. The
     panic gate treats the file as optional, and this check deliberately does
     not.
   - **Both paths.** `verify` and `regenerate` both go through this check, so a
     truncated capture is never written into the snapshot either.
   - **Placement.** It lives in coverage, not in the VM's `post-gate-checks`
     panic scan, because only coverage consumes server spans. A check there
     would fail every lane, and could not be checked on preserved artifacts.
2. **Prevent (queue-full path).** The E2E VM's `jaunder` service sets
   `OTEL_BSP_MAX_QUEUE_SIZE=16384`, eight times the default.
   - The SDK already reads this variable.
   - The SDK allocates its bounded channel up front, so the cost is about 16k
     queue slots committed at startup. Measured: `size_of::<SpanData>()` is 352
     bytes, so about 5.8 MB against VMs with 2–4 GB.
   - A larger queue would make flushing a stalled backlog at shutdown more
     likely to time out, so the size stays moderate.
   - The larger queue does not prevent export errors. Detection still names
     those.
3. **Unchanged.** The extractor, the orphan reasons, the snapshot, the SDK
   default in production, and the collector's own `batch` processor.

## Acceptance

- Unit tests in `host-proof` build capture archives. They check that:
  - a `SpansDropped` line fails, and the error reports at least N;
  - an `ExportError` line fails;
  - an empty `diag.log` passes, and a clean non-empty one passes;
  - a missing `diag.log` fails.
- Before/after check on a preserved root: run 1's two ordinary lanes plus the
  queue-16 probe lane. The result is recorded in the PR.
  - Before: `dev e2e-reconcile sqlite chromium <root>` fails with "snapshot out
    of date" (already observed).
  - After: the same command fails with the capture-incomplete error.
- A test in `nix/checks.nix` asserts that the E2E server's environment sets the
  queue size.
- One final `dev e2e-regenerate-coverage` on the finished tree passes, and
  `docs/coverage/server-fns.json` does not change.
- One sentence each in `docs/observability.md`'s flow-coverage section and in
  CONTRIBUTING's description of the coverage CLI: captures with SDK-reported
  span loss are refused. CONTRIBUTING's disclaimer about arbitrary hidden
  omissions stays.

## Boundaries

- No change to how orphans are classified, and no tolerance for orphans.
- Spans the JS harness exports (`e2e.test`) go straight to the collector without
  a batch queue, so they are out of scope.
- The `e2e-local` host server keeps the SDK default. Its captures do not feed
  coverage.
- Other code that reads captures does not use server spans, so it does not get
  this check.
- The stale doc comment on `OrphanReason` (about a benign warmup orphan) is
  outside this issue.
