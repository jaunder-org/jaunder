# Issue #857 — Subscriber reference invariant and infallible newtype removal

## Outcome

`SubscriberRef` becomes a validating domain value: a subscriber identity always
contains a non-blank, channel-scoped reference, while accepted spellings remain
opaque and byte-preserved. The now-unused infallible string-newtype codegen mode
is removed rather than retained without a legitimate production adopter.

Both storage backends reject zero-length references and upgrades fail safely
instead of inventing or silently deleting identity. Typed storage and restore
paths preserve the same externally observable failure contract across SQLite and
PostgreSQL.

## Load-bearing decisions

1. A `SubscriberRef` is the opaque reference half of a `SubscriberIdentity` and
   has meaning only inside its paired `ChannelId` namespace. The seeded `local`
   channel spells it as the decimal `UserId`; remote channels own their own
   identifier syntax.
2. A Unicode-blank string is not a subscriber reference. Rust validation rejects
   values for which `trim().is_empty()` is true.
3. Accepted values are stored verbatim. Validation does not trim, case-fold,
   normalize Unicode, or interpret remote-channel syntax.
4. The local `UserId` conversion remains an infallible typed-proof door because
   decimal ID spelling is necessarily non-blank. Untyped external strings use
   the validating boundary.
5. Serde and SQLx decoding route through the same validation. There is no
   trusted stored-row bypass for a blank reference.
6. Database enforcement covers the strongest portable subset of the invariant:
   both SQLite and PostgreSQL reject the zero-length string while retaining
   `NOT NULL`. Unicode whitespace semantics remain owned by the Rust type
   because SQLite cannot express Rust's predicate portably.
7. Applied migrations are immutable. A new, matching migration is added to both
   backend sequences; it does not rewrite the original subscriptions migration.
8. The migration is strict and atomic. An existing zero-length reference aborts
   the upgrade for operator repair. It is never replaced with a sentinel and is
   never automatically deleted, because no intended identity can be recovered
   and dependent audience membership would otherwise be lost silently.
9. Operator guidance identifies affected subscription rows and their dependent
   audience membership so repair or explicit deletion can precede a retry.
10. Every domain/application read that exposes a subscriber reference or a
    projection of it first decodes the typed value. SQL label projection must
    not bypass the invariant by coalescing a raw reference directly into a
    `String`. Schema-driven backup export remains an exact-value snapshot and is
    not a domain read.
11. Raw backup restore re-enters through the live schema. A zero-length
    reference rolls back the restore, leaves the target unmodified, and surfaces
    as `BackupError::ConstraintViolation` on both backends.
12. `#[str_newtype(infallible)]` is removed after `SubscriberRef`, its sole
    production adopter, becomes validating. Its parser option, generated trait
    paths, active macro API/rustdoc, and mode-specific tests leave together;
    historical decision/spec/removal records remain, as do independently useful
    `no_ord`, SQLx, serde, and ordinary validating behavior.
13. The architectural decision records amend ADR-0063 and ADR-0101:
    string-backed domain values now have one validating generated trailer,
    including the `Infallible` error type when no input is rejected. A separate
    macro mode is not retained speculatively.
14. `CONTEXT.md` remains unchanged. `SubscriberRef` is precise code and storage
    vocabulary, not a new user-facing ubiquitous-language concept.

## Acceptance

- Constructing or deserializing a blank `SubscriberRef` fails with a typed
  domain error; representative non-blank opaque values round-trip
  byte-identically.
- Local subscriber identity construction remains infallible and preserves the
  decimal `UserId` spelling.
- SQLite and PostgreSQL both reject a zero-length `subscriptions.subscriber_ref`
  at the schema boundary.
- With an existing zero-length row, each backend's upgrade fails atomically and
  leaves the pre-upgrade database intact; no subscription or audience membership
  is silently changed.
- Durable operator guidance gives a diagnostic query and explains
  repair/deletion ordering for dependent audience membership.
- Subscriber listing and summary behavior cannot expose a raw blank reference by
  bypassing typed decode.
- A backup containing a zero-length reference is rejected as
  `BackupError::ConstraintViolation` on both backends; rollback leaves the
  target unmodified.
- No production usage, test fixture, parser option, generated trait path, or
  active macro API/rustdoc surface for `str_newtype(infallible)` remains;
  historical decision and removal records remain intact.
- Existing validating string newtypes, `no_ord`, serde, SQLx, and compile-fail
  contracts continue to pass their observable tests.
- The new ADR is projected into `docs/ARCHITECTURE.md`; stale claims that no
  production type used the mode are replaced by the final invariant and removal
  decision.
- The repository's full SQLite/PostgreSQL verification and migration parity
  gates pass.

## Boundaries

- No remote-channel identifier grammar is introduced.
- No accepted subscriber reference is normalized or rewritten.
- No automatic repair, deletion, or sentinel identity is added for corrupt
  stored rows.
- The schema does not attempt to duplicate Rust's Unicode whitespace predicate.
- No subscription admission, visibility, audience, or wire-protocol semantics
  change beyond rejecting blank identity.
- No general audit of unrelated newtypes or database constraints is included.
- No backward restore of a pre-migration backup into a newer schema is added;
  existing exact-schema-version restore policy remains.
