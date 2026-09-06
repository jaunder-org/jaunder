# Identity-verified local shut-down implementation outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` where a task benefits
> from delegation. This outline exists because issue #142 changes a public
> CLI/process-control contract and concurrent signal semantics.

## Scope

In:

- A storage-scoped `jaunder shut-down` command with a 30-second default and
  positive timeout override.
- Race-free Linux process identity, signal delivery, exit waiting, and
  runtime-identity relinquishment checks.
- Non-escalating repeated SIGTERM with second-SIGINT interactive escalation.
- Refusal-category diagnostics, focused behavioral coverage, the proposed ADR,
  and its architecture projection.

Out:

- Network or socket administration channels, secrets, authentication, additional
  administration operations, stale cleanup, startup cancellation, and a force
  flag.
- Database behavior, HTTP behavior, browser behavior, or cross-platform process
  control.

## Task outline

- [x] Task 1: Make automated graceful-shutdown requests safely repeatable.
  - Contract: the shutdown supervisor starts the graceful drain on the first
    SIGINT or SIGTERM; later SIGTERM is non-escalating; a later SIGINT retains
    forced exit and runtime-identity removal.
  - Verification: focused lifecycle unit tests prove each signal-state
    transition and preserve the existing real-signal graceful-shutdown coverage.
- [x] Task 2: Deliver identity-verified `jaunder shut-down` end to end.
  - Contract: expose one deep command interface to dispatch, accepting the
    existing `StorageArgs` and a positive duration. It owns strict
    runtime-identity classification, stable process-handle acquisition and
    PID/start-time validation, SIGTERM delivery, bounded exact-target exit
    waiting, and the final same-identity check. Errors distinguish missing,
    malformed, dead, start-time mismatch, port-zero startup, timeout, and
    unchanged identity for stderr context.
  - Contract: `Commands::ShutDown` flattens the existing storage arguments,
    defaults to 30 seconds, rejects non-positive timeout input, and remains a
    non-serve command. Linux pidfd support is a direct declared dependency
    rather than PID-only signaling or local unsafe syscall code.
  - Verification: in-process CLI parsing tests pin command spelling, shared
    storage behavior, timeout default/override, invalid timeout rejection, and
    rendered help. Deterministic process-operations seam tests force identity
    changes before, during, and after stable-handle acquisition and prove
    delivery either refuses or remains bound to the captured process. Command
    tests cover every refusal category, timeout, replacement identity,
    completion, byte-preserving non-mutation, and no sentinel mis-signal; a real
    dedicated child proves pidfd delivery and exact-target exit waiting. The
    existing `WorkTracker` admission test, a blocking saturation-measurement
    shutdown test, the real-SIGTERM lifecycle test, and the pidfd command test
    compose the drain proof without adding test-only constructors to production
    worker ownership. Ship verification smoke-tests the production binary's help
    and refusal exit/status because the feature-unified test binary deliberately
    fails closed before Clap when cheap KDF is linked.
  - Documentation: update `README.md` with the operator invocation and
    semantics; keep the issue scope, proposed ADR, architecture projection, CLI
    help, and approved spec consistent. Verify rendered
    `jaunder shut-down --help` names storage targeting, full-completion success,
    timeout/no escalation, and refusal cases. `CONTEXT.md` remains unchanged
    because no ubiquitous-language term is added.

## Ordering and interfaces

- Task 1's signal contract precedes Task 2's real-process and compositional
  shutdown proof.
- Keep the OS/process complexity behind the command interface. Dispatch must not
  learn runtime JSON fields, pidfd operations, polling, or refusal
  classification.
- Reuse the canonical runtime-path and process-start-time ownership logic
  without reusing startup's intentionally permissive stale-file classification.
- Tests cross the same command interface as dispatch; any internal seam exists
  only to make OS transitions deterministic, not as a second public abstraction.

## Risk checks

- A PID cannot be signaled after a check-then-reuse race; signal delivery must
  target the validated stable process handle.
- Opening the process handle and validating start time must be ordered so PID
  reuse before, during, or after acquisition cannot redirect SIGTERM.
- Port-zero startup reservations are never signaled by this command.
- Concurrent SIGTERM senders cannot trigger the forced-exit path; timeout never
  sends another signal.
- Completion accepts absent or different runtime identity only after the
  captured process exits; the old same identity is an error.
- Refusal and timeout paths do not mutate runtime files.
- Behavioral proof must cross the executable command interface for refusal
  status/diagnostics and the server lifecycle for admitted-work draining; module
  tests alone are insufficient for those contracts.
- No backend matrix or e2e coverage is required: the behavior is process-local
  and does not touch storage dialects, HTTP routes, or browser surfaces.
- New production dependency features must pass repository dependency policy; no
  lint or coverage suppression is introduced without explicit approval.
