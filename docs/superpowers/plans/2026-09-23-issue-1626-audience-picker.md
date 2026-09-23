# Lossless Audience Picker — Implementation Outline

> Execute with `jaunder-iterate`; use `jaunder-dispatch` only for a bounded task
> if useful. This outline exists because the web server-function audience
> request/response contract cannot represent Public + Subscribers together,
> although the underlying target set can.

## Scope

In:

- Web Post creation, editing, and retrieval of complete selected target sets;
  audience initialization and load safety; one accessible compact picker;
  affected integration and browser flows.

Out:

- AtomPub wire changes, new audience kinds, Named-audience management, or
  changes to viewer authorization and membership resolution. Storage/schema work
  only if round-trip proof shows it necessary.

## Task outline

- [x] Task 1: Replace the web audience DTO and picker as one buildable vertical
      slice.
  - Contract: Web `AudienceSelection` represents independent Public and
    Subscribers flags plus Named IDs; no flags/IDs denotes Private. Conversions
    and the picker preserve every valid target including Public + Subscribers.
    One disclosure has independent checkboxes and Clear all; its headline names
    Public, else Subscribers, else singular/plural Named count, else Private,
    while the panel reveals every checked target. Explicit empty selection means
    Private. An _absent web_ selection defers to an Org metadata audience if
    present, otherwise uses Public on both create and update; browser
    initialization separately loads the site Default Audience for new Posts.
    AtomPub omission semantics (create uses Default Audience, update preserves
    current) remain untouched.
  - Verification: capture before screenshots before any presentation mutation;
    focused host round-trip/headline tests; both-backend HTTP integration tests
    for create, update, and owner retrieval of Public + Subscribers + multiple
    Named, Named-only, and Private, plus unchanged viewer access and Public-only
    syndication; the updated picker and existing affected browser helpers
    compile and run together.
- [ ] Task 2: Make composer audience initialization and submission safe across
      asynchronous loads.
  - Contract: Default/current-Post selection and Named-audience data have
    distinct pending, ready, and failed outcomes. A placeholder is never
    submittable; errors are visible. An author choice made before a late default
    cannot be overwritten. Existing Post save never falls back to Public after a
    failed audience read.
  - Verification: focused host transition tests and browser flows for delayed
    default, failed default/current selection, and failed Named-audience load
    without a destructive write.
- [ ] Task 3: Prove the complete editor interaction and finish its visual
      presentation.
  - Contract: No new data model; use the picker and load guards from tasks 1–2
    across `/posts/new`, inline Home `/app`, and the existing-Post editor. Keep
    the closed headline compact and the open panel keyboard/assistive-technology
    legible.
  - Verification: Playwright save/reopen of Public + Subscribers + multiple
    Named and Named-only selections, remove/recheck Public without losing
    narrower choices, and Clear all → save/reopen Private; keyboard/disclosure
    checks. Compare before/after pairs for closed and open multi-target and
    Private `/posts/new` at 1280×800 and 390×844, and compact `/app` where its
    layout differs. Migrate remaining browser helpers for the removed base
    select.

## Risk checks

- Preserve ADR-0020/0207 union semantics and Private-only wire contract; never
  drop Subscribers because Public is present or drop Named because no built-in
  is checked.
- Audit every `AudienceSelection` producer/consumer and initialization path, not
  just the new picker; changing its server-fn representation is an API change.
  Keep web absent-selection and AtomPub omission policies distinct, and preserve
  both.
- Verify SQLite and PostgreSQL parity with `#[apply(backends)]` integration
  tests. Preserve the repository's endpoint integration plus Playwright coverage
  policy and the one-boot browser discipline.
- Each task receives focused proof and a staged, hook-gated commit through
  `jaunder-commit`; no lint suppressions without explicit approval. Broad
  diagnostics when needed use `devtool run -- cargo xtask check`; no
  `Co-Authored-By` trailer.
