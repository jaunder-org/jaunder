# Media filesystem usage implementation outline

> Execute with `jaunder-iterate` and `jaunder-dispatch`. This outline exists
> because recursive filesystem I/O must preserve async-runtime isolation and
> all-or-nothing collection semantics.

## Scope

In:

- Periodically collect logical usage for `<storage_path>/media` and expose the
  distinct filesystem metric through the established saturation mechanism.
- Test success, drift, hard-link, failure, disabled-export, and scheduling
  contracts; document the metric's filesystem semantics.

Out:

- Changes to DB-declared media accounting, cleanup/repair behavior, allocated
  block reporting, or media-management UI.

## Task outline

- [x] Task 1: Define and verify filesystem-tree measurement
  - Contract: a path-rooted synchronous measurement returns the logical sum of
    regular directory entries or one failure; directories recurse, and symlinks,
    special entries, missing paths, and read/metadata errors fail the complete
    sample. Hard-linked directory entries are counted independently.
  - Verification: focused server tests use a temporary media tree to prove each
    success and failure classification, including no partial result.

- [x] Task 2: Integrate the filesystem source into saturation metrics
  - Contract: the server sampler runs the Task 1 measurement through Tokio
    blocking work, waits before the next sample, stores its value on success,
    clears a prior value on every failure, and reports failures as
    `server.metrics.media_filesystem_bytes`. The host facade registers
    `jaunder.media.filesystem_bytes` (`By`) as a synchronous snapshot observer.
  - Verification: focused metrics tests prove distinct metric/snapshot behavior,
    a success followed by failure emits no datapoint, blocking
    isolation/non-overlap, and no collector when OTLP export is absent.

- [x] Task 3: Publish the operational metric contract
  - Contract: observability and architecture documentation name the filesystem
    metric, its complete media-tree/logical-length meaning, its difference from
    DB-declared upload bytes, and its no-datapoint failure behavior.
  - Verification: documentation gates pass with the changed source.

## Risk checks

- Preserve the existing `jaunder.media.storage_bytes` identity and DB-only
  semantics exactly.
- Keep OpenTelemetry callbacks as synchronous snapshot reads; no recursive walk
  may enter an HTTP request path or run on an async runtime worker.
- Maintain OTLP gating and `PreparedSaturationMetrics` cancellation ownership.
- Run `devtool run -- cargo xtask precommit` before each commit through
  `jaunder-commit`; no lint suppression without explicit approval.
