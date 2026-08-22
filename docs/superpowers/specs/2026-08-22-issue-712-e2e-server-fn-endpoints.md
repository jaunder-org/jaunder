# Issue #712 — guard e2e server-fn endpoint literals

## Outcome

`cargo xtask check` fails when a Playwright test hardcodes a
`/api/<vertical>/<op>` server-fn endpoint that is not in Jaunder's derived
server-fn endpoint inventory. A server-fn rename now produces a focused
static-check failure naming the stale e2e literal instead of a slow e2e
404/timeout.

## Load-bearing decisions

- Implement a read-only xtask gate, not a generated TypeScript endpoint module.
  The generated-module route would still duplicate xtask's endpoint derivation
  and would force broad test churn; a static gate is the smallest guard that
  closes this issue's drift detector gap.
- The gate's server-fn source of truth is the existing xtask server-fn inventory
  (`web/src/**` `#[macros::server]` enumeration and the ADR-0082
  `/api/<vertical>/<ident>` derivation), while
  `server/tests/web/server_fn_wire.rs` remains the independent test that the
  real macro-generated `ServerFn::PATH` values match that derivation.
- The checked population is TypeScript string/template literals under
  `end2end/tests/**/*.ts` that spell a concrete server-fn endpoint either as a
  full `/api/<vertical>/<op>` path or as a helper endpoint tail passed to
  `failServerFn` / `stallServerFn`.
- The gate parses TypeScript with an xtask Rust dependency
  (`oxc_parser`/`oxc_ast`) rather than searching raw text, so comments and
  dynamic templates are outside the population unless they are real literals.
- Dynamic helper templates such as `**/api/${endpoint}` are not server-fn
  endpoint claims and must not be checked as concrete endpoints.
- Non-server `/api` routes must be explicitly allowed by the gate with a written
  reason; current known example: `/api/client-telemetry`.
- The gate must fail closed on unreadable files, malformed literals it elects to
  parse, duplicate/ambiguous allowlist entries, and concrete `/api/...` literals
  that look like server-fn endpoints but are absent from the derived inventory.
- The gate should report file/line and the endpoint literal so the fix is local
  and does not require running e2e to discover the drift.
- No Playwright behavior needs to change for this issue; replacing literals with
  imported constants is out of scope unless a narrow helper is required to make
  the static population unambiguous.

## Acceptance

- `cargo xtask check --no-test` includes the new guard and fails on a fixture or
  unit test representing a stale e2e server-fn endpoint.
- The guard accepts the currently valid Playwright server-fn literals and helper
  endpoint-tail arguments.
- The guard explicitly permits `/api/client-telemetry` as a non-server endpoint
  and does not silently treat all `/api/*` paths as server fns.
- The guard's tests cover at least: valid full endpoint, stale full endpoint,
  valid helper endpoint tail, stale helper endpoint tail, allowed non-server
  endpoint, and dynamic helper template ignored.
- Existing e2e specs continue to run against the same URLs; no runtime semantics
  change.

## Boundaries

- No generated TypeScript endpoint module in this issue.
- No broad Playwright refactor or constant-import migration.
- No change to `#[macros::server]` endpoint derivation, server-fn placement
  rules, or the `ServerFn::PATH` wire tests.
- No e2e matrix run is required for this static guard unless implementation
  unexpectedly changes Playwright runtime behavior.
