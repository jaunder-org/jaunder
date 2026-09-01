# ADR-0170: Use a typed site-config baseline for dual-backend tests

- Status: accepted
- Date: 2026-09-01
- Issue: [#841](https://github.com/jaunder-org/jaunder/issues/841)

## Context

The shared dual-backend harness deliberately owns database provisioning and
returns a migrated `TestEnv` from `Backend::setup()`. HTTP tests nevertheless
repeated raw site-config writes before exercising their subject. Open
registration and the same example base URL were common fixture assumptions,
while omission silently selected production's safe Closed registration policy
and produced misleading assertion failures.

The harness location and backend parity are already governed by the
[shared database test harness](0033-shared-db-test-harness-crate.md). Site
configuration remains a
[closed typed registry](0102-config-key-closed-registry.md), and production must
continue treating absent or invalid registration policy as Closed. The
unresolved decision is the default state of a test fixture and how tests request
deviations from it.

## Decision

`storage::test_support::Backend::setup()` returns an awaitable, typed setup
builder. Bare setup seeds Open registration and the canonical
`https://example.com/` base URL after migration, then returns the existing
`TestEnv`.

The builder accepts typed registration, optional base-URL, aggregate
backup-config, and aggregate media-limit overrides. It never accepts raw config
keys or unvalidated strings. All selected rows commit in one confirmed write
scope before the environment becomes observable.

Tests that require no site configuration call `.pristine()`. Pristine setup is
exact and mutually exclusive with overrides. An optional base-URL override can
omit only that row while retaining the other baseline. Duplicate declarations
and pristine/override combinations fail loudly rather than gaining
order-dependent precedence.

Production defaults do not change. In particular, absent or invalid registration
configuration remains Closed outside the explicit test baseline. Valid
configuration behavior continues through typed accessors such as identity and
backup setters; fixture construction does not replace those surfaces. Tests of
deliberately invalid legacy rows inject physical database state through test
support, preserving ADR-0102's rule that raw config primitives belong only
inside typed accessor bodies.

## Consequences

New HTTP tests inherit the common usable fixture without repeated policy or URL
writes. Deliberate InviteOnly, Closed, absent, backup, and media states become
visible at the setup call.

Existing bare `.setup().await` syntax and `TestEnv` ownership remain
source-compatible, but the public test-support return type changes from an
anonymous async-function future to an awaitable builder. Callers that require an
absent row must opt out explicitly, and the redundant server-only base-URL setup
helper is removed.

The builder adds one test-only future allocation unless Rust gains a suitable
stable named opaque associated future; this cost occurs only during isolated
test provisioning and avoids a bespoke manual future state machine.

The fixture contract must be proven on both SQLite and PostgreSQL. Configuration
seeding remains inside the feature-gated storage harness and follows the
existing confirmed write boundary; it does not become production dependency
injection or a second harness.
