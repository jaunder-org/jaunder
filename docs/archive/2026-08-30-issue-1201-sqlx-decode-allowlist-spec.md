# Allowlist-free SQLx decode approval

## Outcome

Every structurally readable SQLx decode target under `storage/src` is valid by
declaration rather than by an exact-site exemption. The decode gate remains
structurally total and fail-closed over that population, so a green run means
that every enumerated leaf has an explicit type contract; no central or
distributed allowlist can preserve a bare primitive inside the gate's stated
honesty boundary.

## Load-bearing decisions

- ADR-0085 remains authoritative: the gate enumerates the same structural
  population, denies unknown leaves, fails on unreadable input, and never
  inspects SQL text or receiver names to infer safety.
- Approval is types-only. A leaf passes because it is a declaration-backed
  bridge type, an approved foreign type, or part of a composite whose leaves are
  independently policed. Field attributes, marker traits, site annotations, and
  replacement exemption registries are not approval paths.
- The current 53 allowlist entries are migrated as one clean cutover. `Allowed`,
  `Category`, `ALLOWLIST`, exact-site matching, multiplicity accounting,
  rationale rendering, and allowlist self-policing are deleted rather than
  retained empty.
- Type vocabulary is hybrid and private to storage unless it represents an
  already-shared domain concept:
  - row cardinality and existence use shared storage-private semantic types;
  - catalog object names, column names, database type names, SQL definitions,
    nullability tokens, and migration versions use distinct metadata-role types;
  - feed attempts, email verification, operator status, config keys and values,
    persisted diagnostics, serialized payloads, physical row identities, and
    other lossless values use concern-owned types;
  - no generic `RawText`, `OpaqueText`, `SqlString`, `SqlInteger`, or equivalent
    representation wrapper may become a primitive escape hatch.
- Existence has a Rust `bool` representation. PostgreSQL and SQLite use portable
  `EXISTS` expressions and one declaration-backed existence type; statements and
  helpers are shared where their complete SQL contract is genuinely identical.
  Backend-specific catalog predicates remain separate where their SQL actually
  differs.
- Real invariants are enforced at the decode boundary. Counts and feed attempts
  are nonnegative; existence is boolean; closed metadata tokens and structured
  payloads reject invalid values. The existing out-of-range feed-attempt failure
  remains a column-decode error, and negative counts or attempts no longer pass
  or normalize to zero.
- Lossless persistence contracts remain lossless. Unknown or orphan site-config
  keys remain visible to faithful export and deletion, invalid stored session
  labels still reach the existing repair-on-read policy, and arbitrary catalog
  definitions, diagnostics, and config values are not given invented grammar.
  Each such representation has its own role-specific persisted type and is
  converted or repaired at the existing policy boundary.
- Backend-common semantics use the same type on SQLite and PostgreSQL. A dialect
  difference may change SQL spelling or physical representation, but not expose
  different Rust meanings to shared storage code.
- Custom row policy is separated from decoding. Feed-event claim queries decode
  a fully policed intermediate row and then convert to the public claim result,
  preserving diversion of feed-URL parse failures while propagating every other
  decode failure. The post row decoder may remain handwritten under the gate's
  existing strict proof, but its serialized-tags leaf becomes declaration
  backed.
- Non-SQL calls already included by the scanner's honest structural boundary are
  not hidden by narrowing the population or recognizing receiver spellings.
  Their written target types must also be declaration backed.
- The declaration model remains self-policing: an unknown bridge-emitting macro,
  an unparsed declaration root, an unpoliced composite, or a novel bare
  primitive decode fails loudly.

## Acceptance

- The decode gate contains no `Allowed`, `Category`, `ALLOWLIST`, allowlist
  matching, allowlist count/rationale report, deferred-newtype rule, or
  allowlist-focused self-test. No equivalent central, per-site, marker, or
  attribute registry replaces them.
- Every decode occurrence represented by the 53-entry starting inventory passes
  through a declaration-backed leaf type or a composite whose leaves the gate
  separately proves.
- A focused synthetic fixture adds a novel bare primitive decode under an
  unrecognized spelling and the gate rejects it without any anticipated-site or
  anticipated-SQL rule.
- Focused gate fixtures prove that declaration-backed scalar, optional, tuple,
  derived-row, and strictly proven handwritten-row targets pass, while unknown
  leaves, unreadable targets, unpoliced composites, and incomplete macro models
  fail.
- PostgreSQL and SQLite existence paths return the same Rust boolean type.
  Identical `EXISTS` statements no longer differ only to decode `bool` on one
  backend and `i64` on the other; the subscription existence queries no longer
  need a BIGINT `CASE` encoding.
- Focused boundary tests prove that row counts reject negative values and accept
  valid boundary values; count query paths return typed counts without a silent
  fallback to zero.
- Focused storage tests prove that negative and out-of-range feed attempts fail
  decoding without deleting or mutating the affected row.
- Focused storage tests preserve faithful export of unknown site-config keys,
  repair-on-read of invalid stored session labels, lossless opaque/config text,
  and backend parity for backup, feed-event, subscription, and test-support
  decode paths.
- Feed-event claim tests prove that only feed-URL parse failures take the
  diversion path; status, attempts, timestamps, and every other decode failure
  propagate and leave the row untouched.
- The four one-line #728 revert proofs still fail with diagnostics naming the
  affected decode target or unreadable field position: the posts scalar target,
  the feed-event status field, the post-list tuple leaf, and the missing
  `ColumnInfo` field type.
- ADR-0085 and `docs/ARCHITECTURE.md` describe the allowlist-free declaration
  model, role-specific persisted types, policed intermediate rows, and the
  gate's unchanged structural limits.
- Applicable focused tests and `cargo xtask validate --no-e2e` pass on the final
  staged tree.

## Boundaries

- No database schema migration, wire/API change, or product-visible behavior is
  introduced. Rejecting corrupt negative counts or attempts is enforcement of an
  existing semantic invariant, not a new supported state transition.
- This issue does not shrink the scanner population, add receiver or SQL
  heuristics, introduce region/marker exemptions, or collapse distinct sites
  behind broad multiplicities.
- This issue does not redesign `sqlx-newtype-bind`, broaden primitive approval,
  or change #1200's private module ownership.
- Storage-mechanical types stay private. This work does not promote catalog,
  persistence-corruption, or test-fixture vocabulary into the public domain
  model.
- SQL consolidation is limited to statements with the same semantics and
  portable complete shape; genuine PostgreSQL/SQLite dialect differences remain
  explicit.
- The gate still does not prove SQL column-to-field correspondence or resolve
  types written only by later use. Its documented honesty boundary remains
  explicit and fail-closed where the AST population is readable.
