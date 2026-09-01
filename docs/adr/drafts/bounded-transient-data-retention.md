# ADR-DRAFT: Bound retention for transient data

- Status: proposed
- Date: 2026-08-31
- Issue: [#393](https://github.com/jaunder-org/jaunder/issues/393)

## Context

Jaunder has durable product records and short-lived operational records with
materially different lifecycles. Leaving every transient row or file
indefinitely makes storage growth unbounded, while deleting too early can break
retries, credential use, or operator diagnosis. A retention policy must state
both the semantic cutoff that makes data unusable and the physical cleanup that
reclaims it.

[ADR-0136](../0136-local-post-lifecycle.md) deliberately retained local Posts,
revisions, tombstones, idempotency records, and child relationships indefinitely
until a future decision resolved a combined local-content purge policy. The
idempotency mapping is not durable local content, however: it is retry
coordination. This decision qualifies ADR-0136 by replacing its indefinite
retention of idempotency records with a bounded policy; it does not authorize
purging Posts, revisions, tombstones, child relationships, or referenced media.

Credential and feed-event rows also need distinct terminal-state handling.
Cleanup must be bounded, testable, observable, and safe to run repeatedly on
both supported databases. The existing OpenTelemetry discipline
([ADR-0011](../0011-unified-observability.md)) prohibits PII and secrets in
telemetry.

[ADR-0035](../0035-elisp-live-integration-harness.md) introduced the
runtime-info JSON file as both an ephemeral-port discovery handshake and a
startup mutex. [ADR-0144](../0144-process-configuration-cli-contract.md) later
made its path configurable. Runtime identity and discovery instead need one
storage-scoped path: storage already isolates an instance, and no repository
consumer needs a second path. Clearing `media/tmp` therefore requires
storage-directory ownership that exists before cleanup and survives until
shutdown.

## Decision

Jaunder adopts fixed product retention policies for four transient surfaces:

- **Idempotency mappings** expire semantically one hour after creation. At
  `cutoff <= now`, an expired mapping is no longer a replay mapping; a later
  request with that key is a new request.
- **Credentials with expiry** remain retained for 24 hours after their expiry so
  expiry can be diagnosed. A consumed credential is cleanup-eligible
  immediately, regardless of its nominal expiry.
- **Feed events** become cleanup-eligible immediately when completed. Exhausted
  events remain for seven days, then become cleanup-eligible. This policy owns
  retention of terminal rows only; it does not decide recovery or redrive policy
  for terminal events, which remains the concern tracked by #1052.
- **`media/tmp`** is cleared at server startup before uploads are accepted. Its
  cleanup failure is fatal because a dirty temporary upload area cannot safely
  be treated as ready.

To make temporary-upload cleanup exclusive, `serve` acquires an OS-backed
exclusive lock keyed only by the storage directory before inspecting or deleting
transient files, and retains it through shutdown. Before cleanup it publishes
the sole runtime identity at `<storage>/runtime.json`; failure to publish this
pre-bind reservation is fatal. If that file's JSON `pid` plus process start time
identifies a live process, startup refuses before cleanup. The pre-bind
reservation carries port zero; discovery consumers treat that value as not ready
and reread until a nonzero bound port is published. After the listener binds,
the address update is best-effort and must preserve the live identity on
failure. Graceful shutdown stops background admission and drains every admitted
job and active measurement before removing the canonical runtime file and
releasing the lock; forced process exit removes the file before the OS releases
the lock. This qualifies ADR-0035's JSON-as-mutex and best-effort initial-write
rules, and removes ADR-0144's `--runtime-file` and `JAUNDER_RUNTIME_FILE`
override: storage already provides instance isolation, and no repository
consumer needs a second path.

For database-backed transient data, semantic expiry is authoritative at
`cutoff <= now`; physical cleanup is not required to occur at that instant.
Cleanup runs once during startup and then daily. A run supplies one explicit
`now` and drains every eligible backlog through repeated fixed-size deletion
statements, releasing locks between batches, so lock occupancy and backlog
growth are both bounded. Each domain owns its cleanup predicate. A database
cleanup failure is reported and does not prevent later domains in the same run
from proceeding; the failed domain retries during the next scheduled run.

State transitions that establish expiry, consumption, completion, exhaustion, or
cleanup emit PII- and secret-free structured OpenTelemetry signals. Signals
contain only bounded classifications and stable, non-sensitive identifiers;
operators, not the application, own long-term telemetry retention.

The decision excludes durable Posts, Post Revisions, tombstones, and media
referenced by them; non-expiring sessions and App Passwords; `feed_cache`; and
external captures. It creates no generic retention framework: each domain owns
its semantics and bounded cleanup until proven commonality justifies extraction.

## Alternatives considered

- **Keep all transient data indefinitely.** This preserves every historical row,
  but leaves operational storage unbounded and retains retry mappings past their
  useful semantic window.
- **Delete data immediately at its semantic cutoff.** This conflates correctness
  with best-effort cleanup, makes cutoff behavior depend on scheduler timing,
  and loses useful short-lived credential and exhausted-event diagnostics.
- **Use one configurable retention framework.** This would prematurely unify
  surfaces with different state machines, failure handling, and operational
  requirements.
- **Treat feed retention as feed recovery policy.** Retention answers when
  terminal rows may be removed; it does not decide whether or how a failed
  delivery is retried or redriven.

## Consequences

Idempotency is explicitly a one-hour retry contract rather than indefinite
history, qualifying ADR-0136 without changing its durable local-content
retention. Callers and storage must use one authoritative cutoff comparison, and
cleanup remains an optimization rather than a correctness prerequisite.

Startup takes responsibility for a bounded cleanup pass before service begins,
and daily cleanup bounds later accumulation. Domain-owned cleanup interfaces
must accept time explicitly and limit each pass. Operators receive enough
structured state-transition telemetry to understand cleanup and terminal states
without receiving tokens, credentials, payloads, or other PII; they configure
and retain that telemetry outside Jaunder.

The storage-directory lock and canonical runtime file give startup one ownership
and discovery identity. Mandatory pre-cleanup publication, live pid/start-time
refusal, and port-zero-before-bind preserve the competing-live instance safety
and discovery handshake without a second runtime path.

The different outcomes are intentional: a failed `media/tmp` cleanup stops
startup, while a database cleanup failure is reported and later domain passes
continue. Durable content, permanent credentials, cache state, and external
captures remain outside this decision. A common retention abstraction is
premature until repeated implementation experience demonstrates a real shared
contract.
