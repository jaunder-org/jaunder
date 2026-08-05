# ADR-0103: Prefer the real harness over a fake that mirrors backend behaviour

- Status: accepted
- Date: 2026-08-05
- Issue: [#687](https://github.com/jaunder-org/jaunder/issues/687)

## Context

`InMemorySiteConfig` was a `Mutex<BTreeMap<String, String>>` standing in for
`SiteConfigStorage` in twelve unit tests. It was never a considered choice: the
commit that introduced it says it collapsed two byte-identical hand-rolled
doubles that already existed.

It had drifted into mirroring the real backend. To be useful to the SMTP tests
it re-implemented the sqlx bridge decode **by hand**, so that an empty stored
credential would fail the way the real store fails. Two consequences followed:

- `load_smtp_config_returns_err_for_invalid_port` proved that **the fake's**
  parser rejected a bad port. It said nothing about the real one.
- The hand-rolled decode needed three `sqlx-newtype-decode` allowlist entries of
  its own — the test double was itself a source of unchecked decodes.

ADR-0033's problem statement had already named this gap: storage's own unit
tests "hardcode `sqlite::memory:` and run SQLite-only, leaving Postgres
unexercised for backend-common contract behavior asserted inside `storage` (e.g.
`site_config` get/set semantics…)".

## Decision

**When a fake would have to reproduce backend behaviour to be useful, use the
real harness instead** — `#[apply(backends)]` with `backend.setup()`. A test
double is right when it stands in for a collaborator that is _not_ the thing
under test; it is wrong when the thing under test is the backend contract.

`InMemorySiteConfig` is deleted. Its tests run against the real store on both
backends.

`MockSiteConfigStorage` stays, and the distinction is the point: its call sites
assert **non-interaction** — a bare `::new()` with no expectations panics if
anything calls it, which is an assertion a real store cannot express, since it
would answer with defaults and the test would still pass.

## Consequences

- **Those tests became dual-backend**, per ADR-0019/0053. That is the intended
  cost: "it currently hardcodes SQLite" was never an accepted reason for
  `sqlite_only`.
- **There is now exactly one implementation of the trait's primitives**, so a
  bridge decode has one home and cannot drift between real and fake. Required
  trait methods stop carrying a duplication penalty, which is what made a
  required `get_smtp_config` affordable.
- **Three allowlist entries disappeared** simply because the code that needed
  them no longer exists — the burn-down was partly a consequence of deleting the
  fake, not of writing new gate exceptions.
- Tests pay a temp directory and a migration run each. Where a test genuinely
  touches no store, that cost is not paid: it keeps a `guard:no-backend` marker
  with an honest reason.
- **In-memory SQLite is not the cheaper middle ground.** Each _connection_ to
  `sqlite::memory:` gets its own database, so a multi-connection pool over it is
  not a coherent shared store, and WAL — which production and the harness both
  set — is unavailable for it. The harness uses a temp-file database for these
  reasons.
