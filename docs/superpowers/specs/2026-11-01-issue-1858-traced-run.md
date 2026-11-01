# Standalone `traced-run` for command capture and process spans

## Outcome and approval boundary

Recommend extracting the existing `devtool run` behavior into one standalone
executable (`traced-run`) on the development-shell PATH, and adding opt-in named
process tracing using Perfetto-compatible Chrome Trace Event JSON. This is the
simplest supported format: Perfetto imports it directly, no collector/exporter
or new tracing framework is required, and one run produces one self-contained
artifact. This is a recommendation for human approval, not authorization to
implement or alter interfaces.

The runner remains a direct single-program executor, never invokes a shell,
preserves its existing capture/result/exit contract, and writes under
`.xtask/run/`. Tracing is opt-in. Proposed CLI:

- `traced-run [--name LABEL] [--timeout DURATION] -- PROGRAM [ARG ...]`: normal
  captured mode; print the existing JSON result on the runner's own stdout and
  park child stdout/stderr in files.
- `traced-run --launcher --name LABEL -- PROGRAM [ARG ...]`: launcher-only mode;
  connect child stdout/stderr directly to the runner's inherited stdout/stderr,
  print no JSON or wrapper text, create no stdout/stderr capture files or
  summary JSON, and return the exact established child/signal/timeout outcome.
  Trace artifacts, when tracing is active, are distinct from output capture.
  Reject shell and `nix develop` invocations in either mode as today.

A `--launcher` invocation is a root owner unless it receives valid private
inherited correlation context from a parent traced-run. Direct `cargo xtask …`
is therefore covered before checkout discovery or Nix. If the caller already
used `traced-run cargo xtask …`, the launcher participates in that same run as a
named child, not another root; do not wrap the launcher again. Normal captured
mode around a launcher is not the supported launcher path: an external captured
runner necessarily parks the launcher's output, so interactive streaming cannot
be promised on that path.

## Proposed process protocol and ownership

Use one root runner per invocation tree. It creates a unique opaque run ID and
trace artifact path, records the root span, and owns the final merged trace.
Pass correlation to participating descendants through private environment
variables carrying run ID, trace directory, and parent span ID. Each
participating runner process allocates a unique span ID, records its explicit
bounded name, and atomically appends a complete event record to a
per-process/per-span temporary JSONL file in the run directory. The root merges
those records into one Chrome Trace Event JSON file after its child tree
settles; per-writer files avoid shared-file locking and partial-write races. No
collector, daemon, or arbitrary-process injection.

Only explicit `traced-run` invocations and the instrumented
`scripts/cargo-xtask` launcher participate. Environment inheritance alone does
not make arbitrary children traceable: unmodified programs are opaque time
inside their nearest participating parent span. The launcher uses a private
argv-only inner entry, separate from trace correlation context. On first entry
it validates Cargo's leading `xtask` token and invokes the already-built PATH
`traced-run --launcher --name cargo-xtask -- <absolute-script-path> --inner xtask <original-args>`
before checkout discovery or Nix. The inner launcher phase consumes `--inner`
and validates/removes the Cargo token, performs checkout discovery, then execs
Nix; it does not invoke the runner again. The runner rejects shell commands as
before; the supervised executable is the approved launcher script itself (with
its shebang), not arbitrary `sh -c`/`bash -c`.

The private argv entry means only “this launcher process is already inside its
runner-supervised inner phase”; it is cooperative reentry, not authentication.
There is no launcher environment guard and no runner special case for a
`cargo-xtask` executable basename. Later independent launcher invocations carry
no stale reentry state. `JAUNDER_TRACED_RUN_CONTEXT` remains independently
propagated and determines whether the runner joins an existing trace root or
creates one. Thus direct launcher execution creates a new root before
discovery/Nix, while an externally captured `traced-run cargo xtask …` remains
the root and parks all output; the inner launcher joins its trace, adds no JSON
and creates no output capture files.

Use a single monotonic clock domain for all spans in the process tree: root
exports a monotonic origin (nanoseconds) and descendants timestamp against the
same host monotonic clock, emitting complete duration events in microseconds
relative to that origin. Do not compare unrelated process-local origins or
wall-clock timestamps. Span records identify parent/child and bounded names
only; no argv-derived names, paths, or unbounded attributes. Keep determinant
fields on the narrowest owning span and honor ADR-0011 privacy/cardinality
limits and ADR-0147.

### Launcher simplification approval (2026-11-02)

The Manager explicitly designated the argv-only inner entry as a bounded
continuing correction after the accepted flow review: remove
`JAUNDER_CARGO_XTASK_INNER` and generic runner basename coupling, retain
independent recursive trace context, and preserve pre-discovery/Nix supervision
and existing execution behavior. This supersedes the earlier two-part guard
mechanism, not the public CLI or single-root contract; no antiforgery machinery
is intended. It is not merge, external-routing cutover, or packaging authority.

### Perfetto relationship-encoding clarification (2026-11-02)

Complete `ph:"X"` spans remain on their actual process/thread tracks. Each
participating parent/child relation additionally emits a separate Chrome Trace
Event flow pair: `ph:"s"` on the parent track and `ph:"f"` on the child track,
paired by one unique unsigned 64-bit ID serialized as hexadecimal text. The
finish event uses `bp:"e"` to bind to the child's enclosing complete span. This
creates an inspectable causal link, not cross-process slice nesting or a fake
call stack. Concurrent siblings retain distinct flow IDs, actual process
timelines, and overlapping intervals.

The representation was verified in Perfetto UI v59.1-79997ada9: importing a real
nested-parent/child/grandchild trace plus overlapping sibling invocations
produced three rows in `flow`, each joining the intended source/destination
slices; import errors and trace errors were both zero. `parent_span_id` remains
useful correlation metadata but is not itself the viewer relationship.

The root runner owns trace merging, final trace publication, and (in normal
captured mode) output-result JSON and capture files. In direct launcher mode
there are no output capture files or summary JSON; the root trace owner may be
the outer runner, while per-invocation output ownership remains with the outer
captured runner when one exists, otherwise output flows directly to inherited
descriptors. The inner launcher never adds output files or JSON. The root owns
pruning. On ordinary exit, nonzero exit, handled signal, timeout, spawn/launcher
error, it collects finished records, closes its own span, and atomically
publishes one trace; trace-finalization warnings must not change the established
command result. A hard kill, power loss, or root termination that prevents
finalization cannot honestly promise a complete trace; preserve partial
per-writer records if possible and do not publish them as complete. Descendants
that outlive the root are outside the supported lifecycle; document them as
uncollected rather than claiming completeness. Concurrency is supported through
unique run directories and independent append files, without shared global
state.

## Scope and compatibility

- Move only `devtool run` responsibility and its behavioral tests. Keep
  unrelated `devtool` responsibilities and their devshell/Nix consumers.
- Package `traced-run` on the development-shell PATH. Keep host-only `xtask`
  orchestration and analysis; do not rewrite `cargo xtask` or migrate to `just`.
- Preserve single-program/no-shell behavior, optional timeout, stdout/stderr
  paths and byte/line summaries, result fields (`command`, `exit_code`, `ok`,
  `signal`, `duration_ms`, optional `timed_out`, `stdout`, `stderr`),
  runner-error `{error, kind}` with status 64, child exit propagation, signal
  mapping, timeout status 124, and newest-50-file history pruning.
  Cleanup/summary/trace warnings do not falsify child success. Existing
  non-tracing invocations retain existing output behavior.
- External/global policy and skill routing is Manager-owned. **Removing or
  switching the canonical old `devtool run` route is blocked until the Manager
  confirms exact external migration readiness.** Prefer coordinated atomic
  cutover: migrate references and replace the old route together, leaving one
  final interface. If sequencing makes that impossible, allow only a documented,
  explicitly bounded temporary forwarding entry point sharing the exact same
  implementation; remove it at the confirmed cutover. No permanent dual
  interface.

## ADRs, migration, and proof plan

Amend ADR-0028's `devtool run` supplement to document standalone host/devshell
placement and launcher availability while retaining the in-sandbox `devtool`
versus host `xtask` boundary. Keep trace capture distinct from attribution and
lifecycle-bounded per ADR-0096; do not redefine `xtask.command`. Update
in-repository command-routing docs and examples as part of the authorized
migration; do not edit external policy/skills before Manager coordination.

Tests/proofs must establish: existing result schema and paths; normal output
capture and launcher inherited-stream passthrough;
output-before-child-completion is required only for direct launcher mode (normal
captured mode parks output until observed separately); externally captured
launcher output remains parked by the outer runner and is not promised
interactive; child status/signal/timeout behavior; spawn and early checkout/Nix
failure behavior; refusal/malformed CLI; nested runner and grandchild parentage;
recognized flow endpoint consistency, unique IDs and complete lifecycle;
arbitrary children are not misrepresented as instrumented; direct launcher root
versus externally wrapped launcher has exactly one root and one final trace;
concurrent invocation isolation; shared-clock alignment; normal and failure
finalization; and explicit partial/incomplete honesty after abrupt termination
where testable. Then run one representative filtered workflow,
`cargo xtask test-local -- -p storage site_config_primitives_round_trip`, and
inspect its end-to-end trace in Perfetto, recording useful spans and residual
unattributed time. This is not a performance claim.

PR #1857 is experimental evidence only, remains unmerged and is neither a
dependency nor authority. Its implementation is
`35f2df21c1fc9ca210e5739e8a826c730dfcbaad` (implementation commit
`72fa31560984d646cba9703b71dc6527ecb31af9`). Adapt selectively only if
consistent with the approved design; no blind cherry-pick or merge.

## Exclusions

No whole-command migration campaign, arbitrary-child instrumentation, general
orchestration/tracing framework, permanent duplicate runner, xtask rewrite,
`just` migration, external policy/skill edit, or claim that the representative
workflow is faster. #1857 remains unmerged.
