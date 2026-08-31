# Bounded Transient Data Retention

## Outcome

Jaunder bounds four transient-data surfaces that currently grow indefinitely:
AtomPub Idempotency Keys, expired or consumed credentials, terminal Syndication
Feed events, and crash-orphaned media temporary files. Expiry changes behavior
at its policy cutoff; bounded cleanup reclaims the physical storage
independently.

Successful and terminal transitions emit structured, secret-free telemetry for
operator-retained traceability rather than preserving transient rows as an
unbounded audit log.

## Load-bearing decisions

- Retention horizons are fixed product policy, not configuration.
- A policy cutoff is inclusive (`cutoff <= now`) and authoritative even before
  physical cleanup.
- Restoring a backup never reactivates state already outside its policy window.
- Database maintenance runs once during startup and then once every 24 hours.
- Every maintenance run supplies one explicit current instant to every policy
  evaluation, making the eligible set finite and deterministic.
- A run drains each eligible backlog by repeating fixed-size deletion batches
  until no eligible rows remain. Each statement and lock hold is bounded, and
  locks are released between batches.
- SQLite and PostgreSQL expose the same retention behavior while retaining
  backend-appropriate bounded deletion statements and transaction discipline.
- Failure in one domain's database cleanup is reported as a structured transient
  operational failure, does not stop serving, does not prevent later domains
  from running in the same maintenance run, and is retried by the next scheduled
  run.
- Each domain owns its retention predicate and deletion invariants. No generic
  retention framework is introduced; shared machinery is extracted only after
  two completed surfaces demonstrate the same interface.

### AtomPub Idempotency Keys

- A mapping expires one hour after its creation time.
- Before the cutoff, a retry by the same User and key replays the original Post
  under the existing AtomPub contract.
- At and after the cutoff, the mapping cannot cause replay. The same User may
  reuse the key to create a new Post.
- Concurrent reuse at the cutoff is atomic: one fresh mapping wins and the
  unique key constraint cannot make an expired mapping extend replay lifetime.
- Expiring a mapping does not delete or otherwise alter its Post, Post
  Revisions, deletion tombstone, or referenced media.
- This deliberately narrows ADR-0136's indefinite retention rule for idempotency
  records; the durable local Post lifecycle remains unchanged.

### Expiring Credentials

- Email-verification, password-reset, and invite rows remain unusable at their
  existing validity cutoff.
- An unused expired row becomes cleanup-eligible 24 hours after `expires_at`.
- A consumed row becomes cleanup-eligible immediately and may be removed by the
  next maintenance pass.
- Removal may change a later rejection from expired or already-used to not
  found, but can never make a credential valid again. Sessions and App Passwords
  remain non-expiring and explicitly revoked.

### Syndication Feed Events

- A completed event becomes cleanup-eligible immediately and may be removed by
  the next maintenance pass.
- An exhausted event becomes cleanup-eligible seven days after entering terminal
  failure.
- The seven-day cutoff is the retention boundary that any inspection or redrive
  behavior must respect, but this work adds no operator inspection or redrive
  surface.
- Pending, claimed, and retryable events are never retention candidates.
- This work owns terminal-row retention only. Issue #1052 retains ownership of
  retry classification, independent regeneration/publication budgets,
  dead-letter inspection, and redrive behavior.

### Media Temporary Files

- Before the server can accept uploads, startup establishes an empty `media/tmp`
  directory by removing every leftover temporary upload artifact.
- The single-instance runtime guard means no valid upload is active during this
  startup cleanup.
- Failure to establish the clean directory is a fatal startup error rather than
  a best-effort maintenance warning.
- Finalized media, media metadata, reference-guarded files, and Emacs Local
  Media Copies are never affected.

### Traceability

- Successful credential consumption and feed-event terminal transitions emit a
  structured `INFO` event or span at the transition itself, not during later
  pruning.
- Idempotency creation/replay/expiry and every cleanup pass expose bounded
  operational metrics appropriate to their existing metric families.
- Telemetry contains no tokens, Idempotency Keys, email addresses, Post bodies,
  or other secrets or PII. Stable non-sensitive identifiers and bounded outcome
  enums are permitted under ADR-0011.
- Jaunder guarantees OpenTelemetry emission; the operator owns exporter
  configuration and long-term retention. No persistent audit table is added.

## Acceptance

- On both SQLite and PostgreSQL, a retry before the one-hour Idempotency Key
  cutoff replays the original Post, while reuse at the cutoff creates exactly
  one new Post and establishes a fresh replay window.
- Idempotency expiry leaves the original active or Deleted Post and all durable
  history unchanged.
- On both backends, a maintenance run drains the rows eligible at its supplied
  instant by repeating bounded deletion statements: consumed credential rows,
  credential rows expired for at least 24 hours, completed feed events, and feed
  events exhausted for at least seven days. Newer and non-terminal rows remain.
- Credential expiry changes at its exact cutoff even when deletion is deferred;
  the seven-day exhausted-event cutoff is exposed as the retention predicate
  that #1052's future inspection and redrive behavior must respect.
- A backup restored after a cutoff does not reactivate expired Idempotency Key
  replay or credential validity and does not make an old terminal event
  retention-ineligible; subsequent maintenance reclaims eligible rows.
- Startup and daily database maintenance release locks between bounded
  statements, drain eligible backlogs, expose success counts and structured
  failures, continue later domain cleanup after one domain fails, and do not
  turn transient database errors into server failure.
- Startup removes stale media temporary files before uploads are accepted, and
  an injected cleanup failure prevents startup with a specific operational
  error.
- Structured telemetry proves credential consumption plus feed-event completion
  and exhaustion, and bounded metrics distinguish Idempotency Key creation,
  replay, and expiry. None exposes secrets, PII, or unbounded metric attributes.
- Existing backend-parametric storage and protocol tests continue to prove
  SQLite/PostgreSQL parity.

## Boundaries

- No purge of Posts, Post Revisions, deletion tombstones, referenced media, or
  other durable domain history.
- No expiry for sessions or App Passwords.
- No retention policy for `feed_cache` or E2E/CI diagnostic artifacts.
- No operator-facing settings, UI, CLI flags, environment variables, or stored
  configuration keys.
- No expansion of #1052's Syndication Feed recovery policy and no generic
  retention framework before concrete implementations prove a shared interface.
