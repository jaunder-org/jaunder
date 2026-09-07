# ADR-0181: Identity-verified local shutdown

- Status: accepted
- Date: 2026-09-06
- Issue: [#142](https://github.com/jaunder-org/jaunder/issues/142)

## Context

Jaunder publishes one storage-scoped runtime identity containing its PID and
process start time. Accepted graceful-shutdown behavior already uses SIGINT and
SIGTERM, drains admitted work, removes that identity, and releases the storage
lock. The original issue #142 proposal would add a secret and authenticated HTTP
administration route solely to request the same shutdown.

A shutdown-only network channel would duplicate the signal path, create
secret-storage and comparison requirements, and risk exposure through the
configurable application listener. Signaling a PID after separately checking its
start time is smaller but retains a race: the target can exit and the PID can be
reused between verification and signal delivery. Independent shutdown callers
can also send the existing second signal and accidentally select the forced-exit
path.

## Decision

`jaunder shut-down` is local process control, not an administration protocol. It
reads the canonical runtime identity for the selected storage directory,
acquires a stable operating-system process handle, validates that handle against
both the recorded PID and process start time, and sends SIGTERM through that
handle. It refuses absent, invalid, stale, mismatched, or port-zero startup
identities without signaling or changing runtime metadata.

The command waits for that exact process to exit and for the canonical runtime
identity to stop naming it. The wait has a 30-second default and a positive CLI
override; timeout reports failure without escalation. A newly published
different identity does not change the result for the captured process.

Signal escalation distinguishes automation from interactive intent. Additional
SIGTERM signals after graceful shutdown begins are non-escalating, allowing
concurrent commands to join the same drain. A further SIGINT retains the
existing interactive forced-exit behavior. The command has no force option.

No administration token, HTTP route, Unix socket, or other control channel is
added until a concrete non-signal administration operation justifies that
protocol and its security boundary.

## Consequences

Shutdown automation is race-free with respect to PID reuse and safe to retry
concurrently. Success means the captured instance exited and the runtime
identity no longer names it, rather than merely that a signal was accepted. An
external forced exit during the wait is intentionally indistinguishable; the
command itself never escalates.

The process-control implementation remains Linux-specific, consistent with the
existing `/proc` start-time identity and Linux/NixOS deployment contract. Server
signal handling must treat repeated SIGTERM differently from repeated SIGINT.
Startup cleanup remains the sole owner of stale runtime-file recovery.

A future administration operation such as reload or drain-without-exit requires
a new decision; this ADR does not pre-authorize a network or socket control
surface.

This decision narrows the deferred local-control direction in
[ADR-0035](0035-elisp-live-integration-harness.md) and preserves the canonical
storage-scoped ownership contract in
[ADR-0167](0167-bounded-transient-data-retention.md).
