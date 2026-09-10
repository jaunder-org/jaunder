# Emit confirmed expired Idempotency Key telemetry

Issue: #1445

## Outcome

Production AtomPub Post creation emits the existing bounded `expired`
idempotency metric when a confirmed creation retires an expired Idempotency Key
mapping. Production and test-support creation paths apply the same post-commit
rule.

## Load-bearing decisions

- The metric remains `jaunder.atompub.idempotency_keys{event="expired"}`; this
  work does not add a new metric or label.
- Post creation emits one `expired` event only when the committed storage result
  reports that it retired an expired mapping.
- A confirmed creation that did not retire an expired mapping emits no `expired`
  event.
- A commit-indeterminate result emits no `expired` event. The storage result may
  describe work attempted inside the transaction, but telemetry must not report
  that work as confirmed after the commit outcome becomes indeterminate.
- Metric emission happens after the write scope reports a confirmed commit, not
  from within the transaction.
- Production and test/test-utils creation paths use one shared post-commit
  outcome-handling authority. That authority emits any earned metric and maps
  the storage creation result to the returned Post record, preventing parallel
  implementations from drifting.
- Existing Idempotency Key expiry, replay, retirement, and replacement semantics
  remain authoritative and unchanged.
- Telemetry remains bounded and PII- and secret-free. It never includes the
  client-supplied Idempotency Key, Post content, or another unbounded value.
- SQLite and PostgreSQL must exhibit the same behavior through the shared
  storage harness.

## Acceptance

- A production AtomPub create that confirms retirement of an expired mapping
  increments the `expired` counter exactly once.
- A production AtomPub create that confirms no expired mapping was retired does
  not increment the `expired` counter.
- A commit-indeterminate create does not increment the `expired` counter,
  including when its underlying storage result reports an expired mapping.
- Production and test/test-utils creation paths both obtain their metric and
  returned-record behavior from the shared post-commit authority.
- Existing successful creation and Idempotency Key replay behavior remains
  unchanged.
- The shared storage behavior is demonstrated for both SQLite and PostgreSQL.

## Boundaries

- No change to Idempotency Key lifetime, cutoff calculation, replay,
  replacement, or response semantics.
- No general telemetry refactor and no new metric dimensions.
- No schema, migration, protocol, Post-content, media-ownership, or Syndication
  Feed behavior change.
- No telemetry event is used to imply success when commit outcome is
  indeterminate.
