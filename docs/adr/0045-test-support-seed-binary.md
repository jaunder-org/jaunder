# ADR-0045: A `test-support` binary that links `storage` for out-of-process e2e seeding

- Status: proposed
- Date: 2026-07-02
- Issue: [#210](https://github.com/jaunder-org/jaunder/issues/210)

## Context

The live-server e2e suite (Playwright, HTTP-only, no DB handle) populates
timeline fixtures by looping `POST /api/create_post` — one HTTP round-trip per
post, ~50 per heavy test. That sequential setup, not the behaviour under test,
is the long pole for the slow timeline tests (#155/#152 trace analysis) and
balloons under CPU contention.

The Rust side already has the fast path — `storage::test_support::seed_posts`
calls `create_rendered_post` in-process, bypassing axum/`server_fn` — but a
TypeScript test driving a live server can't call it. We need a way to reach that
storage code path from an out-of-process e2e run. Three obvious bridges were
considered and rejected:

- **A `jaunder seed-posts` CLI subcommand** — ships a seed command in the
  _production_ binary; absurd in a real install.
- **A `POST /api/seed_posts` server-fn** — puts a seed-arbitrary-posts surface
  in / near the release artifact. True compile-time gating forces a second build
  of the whole server (the e2e VMs run the same shared `jaunderBin`), and a
  runtime env gate still ships the code into prod.
- **Raw per-backend SQL (`sqlite3`/`psql`)** — re-implements post creation as
  two hand-maintained SQL dialects. A timeline-visible post is not one row: it
  needs a `posts` row _and_ a `post_audiences` row (`target_kind_id = 1` =
  public) or it is private and invisible (`resolution_where`), plus a NOT-NULL
  `rendered_html`. This is precisely the backend-parity divergence that the
  storage layer, ADR-0019 per-backend dialect files, and the
  `test-backend-pattern` guard exist to prevent.

## Decision

Test-only tooling that must reach jaunder's internals from **outside** the
server process lives in a **dedicated workspace binary crate, `test-support`**,
that links the real crates (`storage`, `common`, …) and drives the genuine code
paths — never a production CLI subcommand, a production HTTP surface, or
hand-written per-backend SQL.

For #210 its one subcommand is
`test-support seed-posts --db <StorageArgs> --username <name> --count <N> [--published]`,
which builds storage from the same `StorageArgs` the server uses and seeds via
`storage::test_support::seed_posts`.

Invariants that make this a decision, not a convenience:

- The `jaunder` production binary and the `services.jaunder` NixOS module
  **never** reference `test-support`. Its absence from prod is structural, not
  gated — there is no flag to accidentally flip.
- It is **not `xtask`**: xtask is host-only and must never run inside a Nix
  derivation; `test-support` is a normal crane-built binary placed on the e2e VM
  PATH so it runs _inside_ the VM.
- It **reuses** the real code paths (`storage::test_support::seed_posts` /
  `create_rendered_post`); it does not fork a parallel seeding implementation.
  Backend parity, audience rows, and rendered HTML come from the one storage
  code path.

## Consequences

- A new small crane build derivation (no leptos/wasm/web deps; shares
  `cargoArtifacts`; Cachix-cached). A test tool being its own artifact is the
  correct shape, not a cost to apologise for.
- A principled home now exists for _all_ out-of-process test/e2e state
  manipulation. Immediate follow-up: replace `scripts/seed-e2e-fixtures.sh` in
  its entirety with `test-support` subcommands (fixture users,
  `site.registration_policy` — today's _"no CLI for that yet"_ raw-SQL hack —
  mail-capture reset).
- Opens the door to migrating `storage::test_support::seed_posts` out of
  `storage` into `test-support` once the binary exists. **Not committed by this
  ADR** — noted as future work.
- Rules out, going forward, adding seed/test affordances to the production CLI
  or HTTP surface, or hand-rolled per-backend SQL fixtures, when the storage
  code path can be linked instead.
