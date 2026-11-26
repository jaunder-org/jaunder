# ADR-DRAFT: Dev traces show an e2e lane's capture spans

- Status: proposed
- Date: 2026-11-26
- Issue: [#2115](https://github.com/jaunder-org/jaunder/issues/2115)

## Context

[ADR-0263](../0263-dev-traces-producer-timed-phases.md) let a `dev` step write
after-the-fact child spans for the phases its sandboxed producer timed itself.
In doing so it amended the "attributes only" rule of
[ADR-0254](../0254-dev-trace-step-accounting.md). An e2e lane's `e2e-producer`
step therefore shows the lane's VM phases, `gate-execution` among them. The rest
of the picture sat in the lane's lifted OTLP capture
(`capture/otel-traces.jsonl`) and was not on the `dev` timeline. That part is
each test's `e2e.*` spans and the server `request`, server and storage spans
under them.

One measured CI lane (`sqlite-chromium-2-of-3`, run 45888881602) held 23.5k
capture spans:

- 14.8k descended from the parent span the VM driver injects into Playwright;
- the rest were background work no test owns, such as feed pollers, write scopes
  and server startup.

Every test span fell inside `gate-execution`, although the capture is on the
guest's clock and the phase on the host's. The merge's track layout never reused
a track, so the test tree alone needed 1,020 Perfetto tracks.

## Decision

A `dev` step may also write a producer's lifted OTLP capture as after-the-fact
spans. This widens ADR-0263's amendment of ADR-0254 from self-timed phases to a
foreign span tree. The spans still go through the same tracer, record writer,
golden record format and merge, and nothing is exported over OTLP.

- **What is written.** For an `e2e-producer` step, the lane's whole test tree
  goes under the `gate-execution` span this run wrote. The tree is every capture
  span below the injected Playwright parent. That parent is the one every
  `e2e.test.lifecycle` span names, and the capture never holds it. Each span
  keeps its capture parent below the top of the tree. Background roots and spans
  whose parent was lost are not written.
- **What a span carries.** Each span keeps its name, its capture start and end
  times, and its OTLP attributes flattened to plain values: strings, bools,
  int64s and finite doubles, with other kinds left out. Its OTLP status becomes
  `otel.status_code` (`OK` or `ERROR`). It gets no `jaunder.status`, which
  remains a `dev` step's outcome. So a green lane's expected server errors never
  become failed rows in `dev trace summary`.
- **When it is written.** The whole tree is written, or none of it is. Every
  top-level tree span must lie inside `gate-execution`; otherwise the tree is
  left out, with one `dev:` warning. A run that wrote no `gate-execution` writes
  nothing, silently: a reused output, or a timed-out gate that recorded no
  phase. So does a lane that lifted no capture. An unreadable or malformed
  capture writes nothing and gives one warning. None of this changes the step's
  status, exit code or the child's output. There is no flag.
- **Track layout.** The merge reuses a track once every slice on it has ended.
  Top-level spans keep track 0, which is never handed out. The measured CI
  lane's test tree then needs 27 tracks.

## Consequences

- Perfetto shows each lane's tests, requests and server work inside its
  `gate-execution` phase, and `dev trace summary`'s slowest rows can be test
  spans.
- The window check trusts the guest's clock to the host's within the measured
  margin. A skew inside it shifts the lane's spans by the skew, and a larger one
  drops the lane. Nothing corrects the clock.
- Each lane adds work after its VM run: reading the capture and writing about
  15k spans. One local lane (`sqlite-chromium-1-of-3`, 14,244 spans) measured:
  - at most 5.4 s between its last phase and the end of its step;
  - about 3 s more of merge;
  - a 9.7 MB `trace.json`;
  - 11 tracks.

  A full local `dev e2e` puts every lane in one run.

- Another producer's capture would need only its own selection rule, under the
  phase that ran it.
