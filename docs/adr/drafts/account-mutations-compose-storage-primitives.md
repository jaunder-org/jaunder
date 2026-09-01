# ADR-DRAFT: Account Mutations Compose Storage Primitives

- Status: proposed
- Date: 2026-08-31
- Issue: [#238](https://github.com/jaunder-org/jaunder/issues/238)

## Context

`AtomicOps` was introduced when multi-table account operations also owned their
database transactions. SQLite and PostgreSQL therefore had separate injected
implementations for registration with an invite and password reset, even though
their SQL and orchestration were effectively identical.

ADR-0164 moved transaction ownership to the caller. Every audited mutation now
receives a sealed `&mut WriteTransaction` minted by `WriteScope`, and the
backend marker adapts that capability to each concrete connection. `AtomicOps`
no longer owns atomicity: it is an object-safe holder for two routines that
execute inside someone else's scope. It duplicates token, user, and session
operations that have natural owning storage traits, adds a fourteenth handle to
`AppState`, and forces otherwise-identical SQLite and PostgreSQL composition
roots to select separate concrete implementations.

The account operations still require a shared orchestration boundary. Invite
consumption must roll back if user creation fails; reset-token consumption,
password replacement, and whole-user session revocation must succeed or roll
back together. Moving that orchestration to the web layer would put database
invariants above `storage`, while replacing `AtomicOps` with another service
object would preserve the same unnecessary holder under a new name.

## Decision

Cross-store account mutations are public functions owned by `storage`. Each
receives the caller's `&mut WriteTransaction` and only the object-safe storage
traits it composes; no function receives `AppState`, a pool, a general executor,
or a replacement dependency bundle.

Primitive mutations live on the trait that owns their rows. `InviteStorage` owns
a read-only validity precheck and one conditional claim that records the created
`UserId`; `SessionStorage` owns revocation of every session for one user.
Registration prechecks before Argon2, creates the user, then conditionally
claims the invite. A PostgreSQL race loser is classified as already used and the
caller-owned scope rolls its inserted user back. Password reset composes
`PasswordResetStorage::use_password_reset`, `UserStorage::set_password`, and
whole-user session revocation.

`AtomicOps` and both concrete implementations are removed, as are the `AppState`
field and web context that carried them. The audited ADR-0164
application-mutation census remains closed at 48 methods: its two `AtomicOps`
declarations move one-for-one to `InviteStorage` and `SessionStorage`.

The existing high-entropy capability ordering remains explicit. Registration
validates before Argon2 and claims after user insertion so `used_by` is
preserved; the conditional claim rechecks validity. Password reset claims before
Argon2. Any later password-preparation, uniqueness, race, or storage error rolls
the surrounding transaction back.

Invalid registration capabilities retain their classifications. Password-reset
callsites use the existing typed conversion whose missing, expired, and
already-used variants are client validation errors; this deliberately corrects
the prior web callsite's blanket storage-error mapping.

Public `Backend` remains the generic-store marker and owns only the sealed
transaction-connection adapter. Crate-private `AppStateBackend: Backend` owns
the pool-to-`WriteScope` factory and is implemented only for SQLite and
PostgreSQL. Generic `make_app_state<DB>(Pool<DB>)`, its sole consumer, is bound
by that private trait. Downstream code can run a factory-minted scope but cannot
name the trait or construct one from a pool, preserving ADR-0164's
downstream-construction invariant without superseding ADR-0019's public
`Backend` marker surface.

The builder states one `XStore<DB>: XStorage` coercion bound per handle; each
store remains the sole owner of its detailed sqlx bounds, and neither `Backend`
nor `AppStateBackend` carries them. ADR-0019's per-consumer bound discipline
therefore remains intact.

## Consequences

The two database openers share one composition implementation and cannot drift
field by field. `AppState` returns to thirteen storage handles plus
`WriteScope`, and web callsites declare the exact storage dependencies of each
account mutation.

Dual-backend storage tests become the direct proof of the shared orchestration,
persisted invite attribution, token-state boundaries, concurrent-claim rollback,
password-error source chains, and whole-user session revocation. The structural
mutation gate remains fail-closed and changes category ownership without
changing its count.

PostgreSQL registration now rejects concurrent reuse of one invite instead of
allowing two users and overwriting `used_by`. Invalid password-reset
capabilities now surface through the intended client-validation mapping instead
of as storage/server failures.

This does not implement ADR-0016 Phase B's on-demand handle factory: production
entry points still receive the existing full `AppState`. It does not expose
transaction internals, alter SQLite scope acquisition or PostgreSQL row-locking
requirements, or broaden the storage traits beyond the read/claim/revocation
primitives needed to retire `AtomicOps`.

<!--
Shipping an ADR includes updating docs/ARCHITECTURE.md (and CONTEXT.md when
the ubiquitous language changes) in the same change — the view is the home
of current truth. Later addenda to a shipped ADR are written in past tense
("as of <date>, Y held; current state: ARCHITECTURE.md §Z"), never as
present-tense patches: an ADR is an immutable event. See
docs/adr/0127-architecture-view-materialized-from-adrs.md.
-->
