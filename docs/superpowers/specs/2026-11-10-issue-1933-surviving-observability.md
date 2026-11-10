# Surviving workflow observations — issue #1933

## Outcome

Ordinary `just tsc` and the native CI Validation hermetic lane explain their
observed request duration and outcome without per-run instrumentation by their
user. Available source/tool context and capture gaps accompany the observation;
missing evidence never proves success or absence of work. This is finite initial
coverage, not complete Milestone33 workflow observability.

## Approach and tradeoffs

Reuse the standalone surviving trace writer, existing recipe ownership, and
native Actions timing, logs and artifacts. At the local tsc recipe, add only a
passive record-only interface to the existing writer: start an observation, run
the existing request directly, then publish its observed status. The recipe
retains command ordering and immediately saves the workload status; every
observation operation is best effort and cannot prevent execution or replace
that saved status. Neither recorder executes, waits for, signals, times out or
cleans up the workload. No traps or competing lifecycle owner are introduced.
Reuse existing trace storage/publication rather than create a second receipt
protocol or reporting framework. A missing completion remains partial/unknown;
an available start is useful evidence, not a claim about signal delivery or
cleanup. Keep labels and metadata bounded: operation/mode, elapsed request time,
observed result and failing boundary, plus actually observed source/tool
identities or explicit unknown values. Do not copy arbitrary argv, environment,
payloads or secrets into new records. Existing command output is not permission
for additional telemetry capture.

Native CI remains the owner of job/step timing, status and source context; use
those facilities rather than wrap the lane in a launcher. Publish available
optional diagnostic records, including incomplete captures, through native
artifacts with 14-day retention. Missing/upload-failed diagnostics are visible
incidents; they do not change the workload verdict or weaken required
producer-proof integrity. Do not add a local collector, retention service,
scheduler or artifact-completeness certification.

Stop count-pruning checkout-local runner evidence so it remains available for
the worktree lifetime. Document explicit consumer-selected export and
byte/identity verification before separately authorized removal; failed export
is an incident, not removal or rerun authority. Accept growing local evidence
storage during that lifetime; report capture/storage failures without
substituting a successful workload result.

The simplest alternatives are native Just timing alone or the existing execution
wrappers. Just timing lacks durable contextual results and excludes dependency
timing; wrappers either alter stream handling or introduce
signal/descendant-cleanup authority. A narrow passive writer addition is
justified only for the missing local observation; native CI does not need a
replacement runner. Request duration is not compiler execution time or additive
CPU/human effort. Source HEAD is not dirty-input identity; request latency does
not prove cache freshness or Nix builder execution.

## Accepted output boundary

Local durable records retain operation/timing/failing phase/status/context.
Original compiler diagnostics remain live on inherited stdout/stderr and are
explicitly marked `raw output not archived`; no automatic later retrieval is
promised. Human answer “Accept explicit raw-output gap”, relayed by Manager from
background 5e6b5dd4-c58e-4053-afe6-97578afbbd01 under
`1933-boundaries-r1-output`, accepts this scope limitation only, not
tee/capture/wrapper or implementation authority. CI logs retain their native
14-day policy.

## Acceptance

- Ordinary `just tsc` automatically retains a useful observation without flags;
  preserve verify/fix behavior, output streams, workload status and existing
  cleanup ownership.
- Explicitly name the selected observed envelope; setup, Just evaluation or
  other work outside that envelope is unknown rather than retrospectively timed.
- Demonstrate a real successful workflow and a real failure through selected
  surviving entrypoints; identify failing boundary and preserve useful available
  failure evidence, within the accepted raw-output boundary above.
- Focused tests cover recorder startup/publication failure, malformed/missing
  context, interrupted/missing completion, concurrent publication and
  no-overwrite/privacy semantics. Capture failure never prevents the workload or
  replaces its status.
- Existing required signal/cleanup behavior remains owned and unchanged; no
  wrapper supervision is added as observation machinery.
- Repeated invocations beyond former pruning thresholds retain checkout-local
  evidence, including partial records. Export proof verifies selected
  source/export identities and bytes without deleting source.
- CI config and applicable native-run evidence show 14-day retention of
  available complete/incomplete diagnostics and visible upload incidents;
  required validation status and producer/verdict joins remain authoritative.
- Available identities and unknown provenance are distinguishable; overlapping
  durations are not summed and cache/substitution is not labeled newly executed
  work.
- Commit maintained behavior/tests/current documentation. Keep demonstrations,
  logs and review packets in ignored storage, not source.

## Boundaries and authority

Human confirmed initial pair under `1933-boundaries-r1`, relayed by Manager from
background e71177b1-ca7c-42bb-937b-5ad097b71b00: “Yes, start with this pair”.
This confirms coverage only, not implementation or this spec. Assigned checkout:
`/home/mdorman/src/jaunder/issue-1933-surviving-observability`; initial clean
head `dab5bf398b2102f43ca797133d4c6a4fb3e62994`. No retiring xtask/devtool
instrumentation, PR1932 adoption, Just patch/PATH interception, global timing
default, generic runner, scheduler or supervisor. Do not instrument E2E
mandatory-proof semantics or change Performance retention; those are outside the
selected pair. #1898 remains paused and untouched. #1864 is independent;
coordinate concrete documentation-region overlap only. A short outline is
required after spec approval for the cross-process writer interface,
partial/concurrent publication and retention/export invariants; obtain its
separate approval before implementation. Spec approval, outline approval, normal
hook-backed commits and exact-head human merge approval remain distinct gates.
Preserve branch and checkout for Manager-owned lifecycle. This design grants no
cleanup, removal, artifact disposal or merge authority.
