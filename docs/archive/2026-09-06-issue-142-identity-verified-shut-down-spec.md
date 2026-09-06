# Issue #142: identity-verified local shut-down

## Outcome

`jaunder shut-down` safely requests graceful shutdown of the live Jaunder
instance that owns the selected storage directory and waits for that exact
instance to finish. It uses the canonical local runtime identity rather than
adding a network administration channel or shared secret.

## Load-bearing decisions

- The public command is spelled `jaunder shut-down` and uses the existing
  storage-selection contract.
- `<storage>/runtime.json` remains the sole published runtime identity and the
  source of the target PID and process start time.
- Before signaling, the command acquires a stable operating-system process
  handle and verifies that it refers to the same PID and start time recorded in
  `runtime.json`.
- The command must not signal by PID alone or retain a race in which PID reuse
  can redirect the signal to another process.
- A missing, malformed, dead, or PID-reused runtime identity is an error. The
  command neither repairs nor removes runtime metadata.
- A runtime identity whose port is zero represents an instance that is still
  starting. The command refuses it without signaling or mutation.
- A normal request sends one SIGTERM to the verified process. There is no HTTP
  route, administration token, token file mode, Unix socket, or other control
  channel.
- Repeated SIGTERM after graceful shutdown has started is non-escalating.
  Concurrent or repeated `shut-down` callers may join the same graceful shutdown
  safely.
- The existing interactive escape remains: after shutdown starts, a further
  SIGINT forces process exit. SIGTERM never takes that forced-exit role.
- The command waits for the captured process handle to exit and then verifies
  that `runtime.json` no longer names the captured identity. A replacement
  instance with a different identity is allowed.
- The default wait limit is 30 seconds. A CLI option allows a positive custom
  timeout.
- Timeout is an error and never sends another signal or otherwise escalates
  shutdown.
- Success means the captured instance has exited and `runtime.json` no longer
  names its identity; merely delivering SIGTERM is not success. Because an
  external actor may force the process to exit while the command waits, this
  observable result does not claim an unknowable exit cause.
- No `--force` option is introduced.

## Acceptance

- Given a ready live instance and no external escalation, `jaunder shut-down`
  targets that exact instance, returns success only after its graceful drain
  completes, and the old identity no longer owns `runtime.json`.
- Existing admitted background work and active measurement draining remain
  intact on the command-driven shutdown path.
- Two overlapping `jaunder shut-down` invocations do not trigger forced exit;
  both can observe completion of the same target.
- Repeated SIGTERM during an in-progress drain does not force exit, while a
  second SIGINT still does.
- If the target exits and a new instance publishes a different runtime identity
  before the command returns, the completed command still succeeds.
- Missing, malformed, dead, start-time-mismatched, and port-zero runtime
  identities each return nonzero, identify the applicable refusal category on
  stderr, and do not change runtime files or signal a process. Distinct exit
  codes are not required.
- A PID-reuse race cannot cause an unrelated process to receive the signal.
- The command defaults to a 30-second wait and honors a positive custom timeout.
- On timeout, the command returns nonzero while the server continues its
  existing graceful drain; no escalation signal is sent.
- CLI help and operator-facing documentation describe the storage target,
  full-completion semantics, timeout behavior, and refusal cases.

## Boundaries

- No general local administration protocol is created.
- No reload, drain-without-exit, status, or other administration operation is
  added.
- No authentication or authorization mechanism beyond access to the selected
  storage directory and its canonical runtime identity is introduced.
- The command does not clean stale runtime files or take over startup recovery;
  `serve` retains that ownership under `runtime.lock`.
- The command does not stop a port-zero startup reservation or make server
  preparation cancellable.
- Service-manager restart policy is outside the command: a replacement process
  does not invalidate successful shutdown of the captured target.
- An external forced exit during the wait can satisfy the observable completion
  predicate; no new durable graceful-completion protocol is introduced.
- Cross-platform process control is outside scope; Jaunder's existing
  Linux/NixOS runtime identity and signal model remains the platform contract.
