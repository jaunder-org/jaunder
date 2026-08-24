# Default Audience Implementation Outline

> Execute with `jaunder-iterate`, using `jaunder-dispatch` for task execution.
> This outline exists because the public cross-crate `SiteConfigStorage`
> contract changes from the wider per-Post audience target to the closed Default
> Audience.

Spec: `docs/superpowers/specs/2026-08-24-issue-843-default-audience.md`

## Scope

In:

- Closed Default Audience domain value and exact token behavior.
- Typed config registry row with no custom parser escape.
- Storage trait cutover, all production/test callers, dual-backend behavior, and
  affected architecture/domain documentation.

Out:

- Per-Post audience-target variants, persistence, resolution, or string grammar.
- Config migrations, protocol changes, and new dependency seams.

## Task outline

- [x] Task 1: Establish the closed Default Audience contract and registry
  - Contract: export `DefaultAudience::{Public, Subscribers, Private}` from the
    existing visibility module; standard closed-enum interfaces own exact token
    behavior; `From<DefaultAudience> for AudienceTarget` preserves each variant;
    `PostsDefaultAudience` declares `DefaultAudience` directly.
  - Verification: unit and registry tests prove all token round-trips, rejection
    of unknown and whitespace-padded tokens, and variant-preserving widening;
    the bespoke parse/format helpers, config validator, and custom-parser macro
    arm are absent; `cargo xtask precommit` passes before commit through
    `jaunder-commit`.
- [x] Task 2: Cut storage and creation paths over to Default Audience
  - Contract: `SiteConfigStorage::{get,set}_default_audience` expose
    `DefaultAudience`; missing and invalid rows return `Private`, database
    errors propagate; web and AtomPub widen only at their per-Post boundaries.
  - Verification: `#[apply(backends)]` storage tests cover unset, invalid, every
    value round-trip, and database-error propagation without fallback on
    SQLite/PostgreSQL; web coverage proves an absent setting starts Private;
    AtomPub coverage proves all three configured values become matching per-Post
    targets; architecture and glossary stay aligned; `cargo xtask precommit`
    passes before commit through `jaunder-commit`.

## Risk checks

- Migrate every getter/setter caller in one clean cutover; leave no
  compatibility alias or wider typed setter.
- Preserve `SiteConfigStorage` object safety and existing dependency-injection
  ownership.
- Preserve defensive config reads while changing only their fallback value;
  never convert database errors into `Private`.
- Preserve SQLite/PostgreSQL parity and avoid schema or migration changes.
- Keep the payload-aware per-Post audience-target row mapping unchanged.
- Update `docs/ARCHITECTURE.md` closed-enum adoption counts and
  visibility/config descriptions when the implementation changes those facts.
