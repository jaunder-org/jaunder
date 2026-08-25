# ADR-0151: Subscriber references are non-blank validated identities

- Status: accepted
- Date: 2026-08-24
- Issue: [#857](https://github.com/jaunder-org/jaunder/issues/857)

## Context

A subscriber is identified in storage by `(channel_id, subscriber_ref)`.
`SubscriberRef` is opaque outside its channel namespace: the seeded `local`
channel uses a decimal `UserId`, while a remote channel owns its own spelling.
The reference is identity, not display text.

`SubscriberRef` was introduced with `#[str_newtype(infallible)]`, which accepts
every `String`. That made a blank reference representable even though it
identifies nobody. Supported HTTP subscription writes derive a non-blank decimal
ID, but the public storage seam, manual SQL, and raw backup restore could still
store an empty value. One subscriber-summary query also projected the raw column
directly to display text without typed decoding.

[ADR-0063](0063-domain-value-newtype-convention.md) defines a validating newtype
whenever a string has a rejectable value.
[ADR-0101](0101-infallible-kind-is-invariant-first.md) retained a separate
infallible macro kind for values with no rejecting rule, but its last original
adopters later became validating. `SubscriberRef` was the only new adopter;
applying the invariant-first question shows that it was misdeclared. Keeping the
mode would preserve codegen and public macro surface for no legitimate
production use.

Storage adds two constraints.
[ADR-0020](0020-content-visibility-and-subscription-model.md) places practical
subscription integrity rules in the database, and raw restore bypasses Rust
domain decoding. But Rust's Unicode `trim().is_empty()` predicate has no exact
portable SQLite expression. Existing empty rows also have no honest backfill: a
sentinel invents identity, while automatic deletion can silently remove
dependent audience membership.

## Decision

`SubscriberRef` rejects every Unicode-blank string and preserves every accepted
value byte-for-byte. It remains meaningful only as the reference half of a
`SubscriberIdentity` paired with `ChannelId`. Remote-channel grammar stays
opaque. The local decimal-`UserId` conversion is an infallible typed-proof door;
untyped strings use the validating boundary.

Serde and SQLx decoding use that same validation. Stored rows get no trusted
bypass, and projections that expose a subscriber reference decode the typed
value before producing display text.

Both storage backends add the strongest portable schema constraint:
`subscriber_ref` remains `NOT NULL` and additionally rejects the zero-length
string. Rust owns the stronger Unicode-blank rule. The paired migration is
strict and atomic: an existing empty row aborts upgrade for explicit operator
repair. It neither synthesizes a reference nor deletes subscriptions or audience
membership. Durable guidance identifies affected rows and dependent membership.

Raw backup restore re-enters through the live schema. Constraint failure rolls
back, leaves the target unmodified, and surfaces as
`BackupError::ConstraintViolation` on SQLite and PostgreSQL.

The separate `#[str_newtype(infallible)]` mode is removed. String-backed domain
newtypes use one validating generated trailer; a type for which no input is
invalid can express that with `FromStr::Err = Infallible` without a second macro
mode. This amends ADR-0063's generated-trailer mechanism and retires the kind
retained by ADR-0101; their invariant-first question remains authoritative.

## Upgrade recovery

Migration 0026 fails atomically when an existing database contains a zero-length
reference. The database remains at its pre-upgrade schema and data. Find every
affected subscription and the audience membership that depends on it before
retrying (`audience_id` is `NULL` when none does):

```sql
SELECT s.subscription_id, s.author_user_id, s.channel_id, am.audience_id
FROM subscriptions AS s
LEFT JOIN audience_members AS am
  ON am.subscription_id = s.subscription_id
 AND am.author_user_id = s.author_user_id
WHERE s.subscriber_ref = '';
```

For each row, either repair `subscriber_ref` to the real identifier in that
row's channel namespace or explicitly delete the invalid identity. Deletion must
remove dependent audience membership first, using the same
`(subscription_id, author_user_id)` pair, and only then remove the subscription:

```sql
DELETE FROM audience_members
WHERE subscription_id = <subscription_id>
  AND author_user_id = <author_user_id>;

DELETE FROM subscriptions
WHERE subscription_id = <subscription_id>
  AND author_user_id = <author_user_id>;
```

Do not substitute a sentinel: it invents a channel identity and may later match
a real subscriber. After every diagnostic row is repaired or explicitly deleted,
retry the upgrade normally.

## Consequences

- Blank subscriber references become unrepresentable through Rust boundaries;
  zero-length values also become unrepresentable in either live schema.
- Accepted remote references are not normalized, so channel-owned identity does
  not change.
- A manually corrupted or out-of-tree database may require operator action
  before upgrade. Failure is preferred to silent identity invention or data
  loss.
- A whitespace-only value inserted by manual SQL can remain schema-legal because
  portable SQL enforces only zero length; typed reads still reject it. This is
  an explicit limit, not permission to bypass `SubscriberRef`.
- Subscriber summary reads and restore behavior gain invariant coverage that raw
  string projection previously bypassed.
- The macro loses its infallible parser option, generated trait branches,
  documentation, and mode-specific tests. Ordinary validation, `no_ord`, serde,
  and SQLx support remain.
- `docs/ARCHITECTURE.md` becomes the current projection. `CONTEXT.md` does not
  change because this is code/storage vocabulary rather than a user-facing
  domain-language addition.
