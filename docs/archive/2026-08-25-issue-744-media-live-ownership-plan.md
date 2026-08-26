# Live Media URL Ownership — Implementation Outline

> Execute task-by-task with `jaunder-iterate`; delegate bounded slices through
> `jaunder-dispatch`. Check boxes before each commit gate.

**Goal:** Decide whether a Post's media URL is served by this instance through a
live UUID-bearing response, then preserve exact evidence in one atomic deletion
decision.

**Trigger:** Persistent identity/schema, cross-crate interfaces, and PostgreSQL
concurrency require an outline.

**Spec:**
[`2026-08-25-issue-744-media-live-ownership-spec.md`](2026-08-25-issue-744-media-live-ownership-spec.md)

## Review header

**Scope — in:** persistent instance UUID; global response header; exact URL-form
reference persistence/backfill; bounded live HEAD resolver; exact foreign
evidence; shared Post-write/delete locks; web and AtomPub wiring.

**Scope — out:** ETag ownership, nonce/HMAC identity, request-derived proxy
scheme, browser UI, cached probes, and background ownership sweeps.

**Key interfaces:**

- `MediaReference` exposes `media()`, `kind()`, and `reference_form()`; no
  origin candidate collection.
- `InstanceId` is a validated canonical UUID loaded once at startup.
- `MediaReferenceOwnershipResolver::resolve(all_global_references, instance_id, base_url)`
  returns resolver-constructed `ProvenForeignReference` values keyed by exact
  Post ID + media triple + kind + complete form + expected InstanceId.
- Storage deletion/report/reclamation consumes one evidence set and keeps one
  conditional delete statement.
- Network work completes before transactions/advisory locks.

## Tasks

### Task 1: Replace the rejected configured-origin design

- [x] Rewrite spec, ADR, architecture projection, and outline around live
      instance ownership. Remove origin-candidate/configured-origin terminology.
- [x] Replace migration 0027 with exact reference kind/form persistence and the
      singleton identity table; retain `legacy` only for copied rows.

### Task 2: Persist and serve instance identity

- [x] Add validated `InstanceId`; atomically insert-if-absent/read-back on both
      backends and carry the immutable value through
      `OpenedDatabase → StartupDatabase → create_router`, not `AppState`.
- [x] Treat identity-only state as pristine. Clear-then-load adopts exactly one
      valid backup UUID; specify legacy missing identity, reject current
      missing/duplicate/malformed identity, and prove rollback and clone rules.
- [x] Add outer middleware that replaces inner values with exactly one
      `X-Jaunder-Instance` on success, errors, fallbacks, method-not-allowed,
      and protocol routes.

### Task 3: Persist exact reference forms

- [x] Parse local, absolute HTTP(S), and scheme-relative forms; reject userinfo,
      retain query, remove fragment, and preserve media-path validation.
- [x] Make extraction sorted/deduplicated by complete reference.
- [x] Write/backfill exact `(media, kind, form)` rows on both backends, with
      upgrade, rollback/retry, overlap, and restore tests.

### Task 4: Resolve ownership within one budget

- [x] Add the deep `MediaReferenceOwnershipResolver` seam outside storage and a
      live adapter using HEAD. Resolve scheme-relative forms by inheriting only
      the canonical base scheme.
- [x] Use one shared ordinary reqwest client with a request timeout. Let reqwest
      own DNS, configured proxies, redirects, TLS, and pooling; do not add IP
      policy, manual resolution, socket pinning, or redirect handling.
- [x] Keep eight-probe concurrency and a 10 s operation deadline; deduplicate
      identical targets and make unstarted/unfinished targets unknown.
- [x] Parse exactly one canonical response UUID: matching owned, true absence or
      one different foreign, ambiguous/malformed/request failure unknown. Test
      the header matrix, a real local HEAD request, and the budget bounds.

### Task 5: Keep deletion atomic under concurrent writes

- [x] Replace configured-origin interfaces with exact evidence shared by owner
      refusal, global rowless protection, reclamation, and distinct reporting.
      Load/resolve all Users' rows; derive owner reporting as a subset.
- [x] Build dynamic bound evidence SQL comparing every exact key field and
      expected InstanceId. Near-match, unknown, and new rows refuse.
- [x] PostgreSQL uses `pg_advisory_xact_lock`: create proposed keys, update
      sorted old/new union, delete/reclaim target before SQL. Define one stable
      key namespace/order and test insert race, A→B union, opposite orders,
      rollback release, and reclaim. SQLite proves parity under writer locking.
- [x] Fetch startup InstanceId and current SiteIdentity once, run one global
      resolution, and pass one evidence snapshot through web/AtomPub deletion,
      reclamation, and web reporting.

### Task 6: Verify, review, and return to the human merge gate

- [x] Run focused parser, identity, migration/backfill, resolver, manager, web,
      and AtomPub lanes on both backends; run `cargo xtask check`.
- [x] Run fresh Standards/Spec/security review and resolve every finding.
- [ ] Run `cargo xtask validate --no-e2e`, rebase, archive artifacts, rewrite
      the branch coherently, force-with-lease PR #1192, and watch to
      ready-to-land.
- [ ] Stop before `cargo xtask pr land`.

## Verification

- `devtool run -- cargo xtask test-local -- -p common parse_media_url`
- `devtool run -- cargo xtask test-local -- -p storage instance_identity`
- `devtool run -- cargo xtask test-local -- -p storage post_media`
- `devtool run -- cargo xtask test-local -- -p storage media`
- `devtool run -- cargo xtask test-local -- -p jaunder -E 'test(/^(media_ownership|storage::media|web::media|atompub::media)/)'`
- `devtool run -- cargo xtask check`
- `devtool run -- cargo xtask validate --no-e2e`
