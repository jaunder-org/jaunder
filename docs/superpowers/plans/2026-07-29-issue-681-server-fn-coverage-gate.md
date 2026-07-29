# Trace-derived `#[server]` fn flow-coverage gate — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:**
`docs/superpowers/specs/2026-07-29-issue-681-server-fn-coverage-gate.md` — the
"what/why". This plan is the "how"; it does not restate the spec's analysis.
**Issue:** [#681](https://github.com/jaunder-org/jaunder/issues/681)

**Goal:** Prove empirically, from e2e traces, which `#[server]` fns a browser
session actually drives — and make an uncovered one fail the build.

**Architecture:** A shared syn enumerator supplies the inventory. A pure
extractor reads the e2e run's OTLP capture and derives fn → covering-tests via
an ancestor walk to the per-test span, using derived `#[tracing::instrument]`
span names as the primary signal and request `uri` as the complement. The result
is committed as a snapshot, checked cheaply in the static lane and regenerated
fail-on-drift in the `sqlite × chromium` e2e lane.

**Tech Stack:** Rust (xtask, `syn`, `serde_json`), TypeScript (Playwright
fixtures, OTLP), Nix e2e checks.

---

## Review header

**Scope — in:**

- Extract the `#[server]` enumerator from `server_fn_registrar_check.rs` into a
  shared module
- Derive the 11 hand-typed span names; guard against reintroduction
- Widen `traces::parse::Span` with `span_id` / `parent_span_id`
- Per-test traceparent propagation in the e2e harness (incl. throwaway contexts)
- Convert `atompub.spec.ts` and `feeds.spec.ts` onto the fixtures' `test`
- Pure coverage extractor + committed snapshot + hand-maintained allowlist
- Two-lane gate, fail-closed, with an in-repo bite test
- Seed the allowlist from a real run; file one issue per genuine gap

**Scope — out:** flow docs (#601) · instrumenting the other 44 fns (#511) ·
endpoint derivation (#698) · writing the missing e2e tests (filed per-gap in
Task 9) · union aggregation across combos.

**Tasks:**

1. Extract the shared `#[server]` enumerator, widened with ident/endpoint/module
2. Derive the 11 span names + guard against explicit `name =`
3. Widen `traces::parse::Span` with span/parent ids
4. Per-test traceparent propagation in the e2e harness
5. Convert the two non-fixture spec files
6. Pure coverage extractor (ancestor walk, both signals)
7. Snapshot + allowlist model, stable serialization, drift compare
8. Wire the gate into both lanes, with actionable failures and a bite test
9. Real run → seed allowlist, file per-gap issues, commit the snapshot
10. Documentation + CI note

**Key risks:** Task 4 touches `fixtures.ts`, which most tests run through — it
lands as its own commit and Task 9's real run is the proof. Task 9 is the only
task requiring a full e2e combo (~long); everything before it is host-testable.

## Global Constraints

- **No `Co-Authored-By` trailer** on any commit.
- Pre-commit runs the full `cargo xtask check`; run it first so it passes clean
  (**`jaunder-commit`**). Never edit files while a gated commit is running.
- xtask is excluded from the workspace — test it with
  `--manifest-path xtask/Cargo.toml`.
- `expect_used` is denied; mint known-valid values through a trusted door, not
  `.parse().expect()`.
- Artifacts live at `docs/coverage/*.json` — **never** under `end2end/`
  (unfiltered derivation source) and **never** `.toml` (crane's cargo-source
  filter can match it).
- Coverage is the **union** of the span-name and `uri` signals.
- The snapshot carries **no provenance fields** (no commit, no timestamp).
- A missing/empty/unparseable capture **fails closed**.
- `xtask/` is excluded from the coverage derivation's source (`flake.nix:1183`),
  so **none** of the new Rust code is coverage-gated — do not add `cov:ignore` /
  `crap:allow` markers to it. (The `test-backend-pattern` guard is likewise
  scoped to `server/tests` and `storage/src` and does not apply.)
- New struct fields have no non-test reader until Task 6, and `xtask-clippy`
  runs `--all-targets -- -D warnings` over the lib target at every commit gate —
  hence the `pub mod` changes in Tasks 1 and 3.

---

### Task 1: Extract the shared `#[server]` enumerator

**Files:**

- Create: `xtask/src/server_fns.rs`
- Modify: `xtask/src/steps/server_fn_registrar_check.rs:50-54` (`ServerFn`),
  `:60-71` (`server_fns_in`), `:73-99` (`ServerFnVisitor`), `:107-120`
  (`server_fn_default_named`), `:124-136` (`pascal_case`) — remove the moved
  items, import them; update the call site in `problems()` at `:199` and the
  test setup lines at `:316,332,338,343,349,355`
- Modify: `xtask/src/lib.rs` (add **`pub mod server_fns;`** — a private `mod`
  would leave the new fields dead-code in the lib target and redden
  `xtask-clippy`'s `-D warnings` at this task's commit gate, since nothing reads
  them until Task 6)
- Test: in-file `#[cfg(test)]` in `xtask/src/server_fns.rs`

**Interfaces:**

- Produces:

```rust
/// One `#[server]` fn discovered in a `web` source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerFn {
    /// PascalCase generated type name (`ListMyMedia`) — the registrar guard's key.
    pub name: String,
    /// The fn ident as written (`list_my_media`) — the coverage gate's key.
    pub ident: String,
    /// The declared `endpoint = "…"` value, leading slash stripped, if present.
    pub endpoint: Option<String>,
    /// Module path relative to the crate root, `::`-joined (`media::api`).
    pub module: String,
    /// 1-based line of the `#[server]` attribute.
    pub line: usize,
}

/// Every `#[server]` fn in one source file, or why the file could not be enumerated.
pub fn server_fns_in(src: &str, module: &str) -> Result<Vec<ServerFn>, String>;

/// `web/src`-relative source path → `::`-joined module path.
/// `lib.rs` → `""`; `site/api.rs` → `site::api`; `posts/mod.rs` → `posts`;
/// `posts/api/listing.rs` → `posts::api::listing`.
pub fn module_path_of(rel_path: &Path) -> String;
```

- Consumes: nothing (first task).

`server_fn_registrar_check` keeps asserting only on `.name`, `.line`, and
`pascal_case`, so its **assertions** are unchanged — but every `server_fns_in`
call site gains the `module` argument, so setup lines do change. `pascal_case`
must stay reachable from the registrar's test module (`:325` calls it directly).

`module_path_of` is the sole producer of the `module` argument, used by all
three consumers (`problems()`, Task 2's `check`, Task 8's static step) and
load-bearing for Task 6's `code.namespace` comparison — hence its own tests
below.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_ident_endpoint_and_module() {
        let src = r#"
            #[server(endpoint = "/list_my_media")]
            pub async fn list_my_media() -> Result<(), ServerFnError> { Ok(()) }
        "#;
        let fns = server_fns_in(src, "media::api").expect("enumerates");
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, "ListMyMedia");
        assert_eq!(fns[0].ident, "list_my_media");
        assert_eq!(fns[0].endpoint.as_deref(), Some("list_my_media"));
        assert_eq!(fns[0].module, "media::api");
    }

    #[test]
    fn bare_server_attr_has_no_endpoint() {
        let src = r#"
            #[server]
            pub async fn thing() -> Result<(), ServerFnError> { Ok(()) }
        "#;
        let fns = server_fns_in(src, "x").expect("enumerates");
        assert_eq!(fns[0].endpoint, None);
        assert_eq!(fns[0].ident, "thing");
    }

    #[test]
    fn endpoint_leading_slash_is_stripped() {
        let src = r#"
            #[server(input = MultipartFormData, endpoint = "/upload_media")]
            pub async fn upload_media() -> Result<(), ServerFnError> { Ok(()) }
        "#;
        let fns = server_fns_in(src, "media::api").expect("enumerates");
        assert_eq!(fns[0].endpoint.as_deref(), Some("upload_media"));
    }

    #[test]
    fn positional_rename_is_a_hard_error() {
        let src = r#"
            #[server(SomeName)]
            pub async fn thing() -> Result<(), ServerFnError> { Ok(()) }
        "#;
        assert!(server_fns_in(src, "x").is_err());
    }

    #[test]
    fn unparseable_source_is_an_error_not_an_empty_set() {
        assert!(server_fns_in("fn (((", "x").is_err());
    }

    #[test]
    fn non_server_fns_are_ignored() {
        let src = "pub async fn helper() {}";
        assert!(server_fns_in(src, "x").expect("enumerates").is_empty());
    }

    #[test]
    fn module_path_of_maps_source_paths_to_module_paths() {
        assert_eq!(module_path_of(Path::new("lib.rs")), "");
        assert_eq!(module_path_of(Path::new("site/api.rs")), "site::api");
        assert_eq!(module_path_of(Path::new("posts/mod.rs")), "posts");
        assert_eq!(
            module_path_of(Path::new("posts/api/listing.rs")),
            "posts::api::listing"
        );
    }
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fns`
Expected: FAIL — module `server_fns` does not exist.

- [ ] **Step 3: Implement against the tests**

Move `ServerFn`, `server_fns_in`, `ServerFnVisitor`, `server_fn_default_named`,
and `pascal_case` from `server_fn_registrar_check.rs` into
`xtask/src/server_fns.rs`, widening the struct and the visitor to record
`ident`, `endpoint`, and the caller-supplied `module`. Every branch above is
pinned by a test. Re-point `server_fn_registrar_check` at the new module; **its
existing test assertion bodies must not change** (relocation and `use`-line
edits only).

- [ ] **Step 4: Run the tests, verify they pass**

Run: `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: PASS — including every pre-existing `server_fn_registrar_check` test.

- [ ] **Step 5: Commit**

```bash
git add xtask/src/server_fns.rs xtask/src/lib.rs xtask/src/steps/server_fn_registrar_check.rs
git commit -m "refactor(xtask): extract shared #[server] enumerator (#681)"
```

---

### Task 2: Derive the span names, and guard against reintroduction

**Files:**

- Modify (**twelve** attributes, not eleven — eleven on `#[server]` fns plus
  `auth/server.rs:112`'s `require_auth`): `web/src/auth/api.rs:41,114,131`,
  `web/src/auth/server.rs:112`, `web/src/site/api.rs:15,28,55`,
  `web/src/backup/api.rs:17,30,43`, `web/src/registration/api.rs:41,53`.
  `backup/api.rs:43-46` is the multi-line one.
- Create: `xtask/src/steps/span_name_derived_check.rs`
- Modify: `xtask/src/lib.rs:296` (the `Check` arm) **and**
  `xtask/src/lib.rs:328` (the `Validate` arm) — in-process syn checks are
  registered at both sites, alongside `server_fn_registrar_check::run`. **Not**
  `static_checks.rs`: that returns shell-command `StepSpec`s and its ordering is
  pinned by `step_order_is_locked` (`static_checks.rs:297-317`), so adding there
  would both break that test and fail to run the check.
- Test: in-file `#[cfg(test)]` in the new check

**Interfaces:**

- Consumes: `server_fns::server_fns_in` (Task 1)
- Produces: `pub fn check(root: &Path) -> Result<Vec<String>, String>` — returns
  one human-readable violation per offending fn.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn explicit_name_on_a_server_fn_is_a_violation() {
    let src = r#"
        #[server(endpoint = "/get_site_identity")]
        #[tracing::instrument(name = "web.site.get_identity")]
        pub async fn get_site_identity() -> Result<(), ServerFnError> { Ok(()) }
    "#;
    let v = violations_in(src, "site::api").expect("scans");
    assert_eq!(v.len(), 1);
    assert!(v[0].contains("get_site_identity"), "names the fn: {}", v[0]);
}

#[test]
fn derived_instrument_is_clean() {
    let src = r#"
        #[server(endpoint = "/get_site_identity")]
        #[tracing::instrument]
        pub async fn get_site_identity() -> Result<(), ServerFnError> { Ok(()) }
    "#;
    assert!(violations_in(src, "site::api").expect("scans").is_empty());
}

#[test]
fn skip_args_are_allowed_without_a_name() {
    let src = r#"
        #[server(endpoint = "/login")]
        #[tracing::instrument(skip(password, label))]
        pub async fn login() -> Result<(), ServerFnError> { Ok(()) }
    "#;
    assert!(violations_in(src, "auth::api").expect("scans").is_empty());
}

#[test]
fn explicit_name_on_a_non_server_fn_is_not_this_gates_business() {
    let src = r#"
        #[tracing::instrument(name = "storage.sqlite.user.create_user")]
        pub async fn create_user() {}
    "#;
    assert!(violations_in(src, "x").expect("scans").is_empty());
}
```

- [ ] **Step 2: Run the tests, verify they fail**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml span_name`
Expected: FAIL — `violations_in` not defined.

- [ ] **Step 3: Remove all twelve `name = "…"` arguments**

Delete only the `name = "…"` argument from each `#[tracing::instrument]` listed
in **Files**, leaving every `skip(...)` argument untouched. An attribute reduced
to no arguments becomes bare `#[tracing::instrument]`. Include
`auth/server.rs:112` (`require_auth`) — it is not a `#[server]` fn, so the guard
will not police it, but leaving a `web.auth.*` name on a non-server fn is
exactly the phantom the spec calls out. Afterwards,
`rg 'instrument\(name' web/src` must return nothing.

- [ ] **Step 4: Implement the check**

Signature
`fn violations_in(src: &str, module: &str) -> Result<Vec<String>, String>`, plus
a `check(root)` that walks `web/src` and aggregates. Reuse `server_fns_in` for
the set of `#[server]` fns; for each, inspect sibling `#[tracing::instrument]`
attributes for a `name =` NameValue. All four branches above are pinned by
tests.

- [ ] **Step 5: Run the tests and the wasm clippy pass**

Run: `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: PASS Run:
`devtool run -- cargo clippy -p web --target wasm32-unknown-unknown -- -D warnings`
Expected: PASS — wasm clippy before committing web changes, or the slow gate
fails later.

- [ ] **Step 6: Commit**

```bash
git add web/src xtask/src/steps/span_name_derived_check.rs xtask/src/lib.rs
git commit -m "refactor(web): derive server-fn span names from the fn ident (#681)"
```

(Safe to register this check now — unlike Task 8's coverage step, it is green
the moment the twelve names are gone.)

---

### Task 3: Widen `traces::parse::Span` with span and parent ids

**Files:**

- Modify: `xtask/src/traces/parse.rs:15-32` (struct + doc comment), and the
  **single** construction site at `:172`
- Modify: `xtask/src/lib.rs:14` — `mod traces;` → `pub mod traces;`, for the
  same dead-code reason as Task 1: nothing reads the two new fields until Task
  6, and `xtask-clippy` runs `--all-targets -- -D warnings` over the lib target
  at this task's commit gate
- Test: in-file `#[cfg(test)]` in `parse.rs`

**Interfaces:**

- Produces: `Span` gains `pub span_id: String` and `pub parent_span_id: String`
  (empty string when absent — matches the existing string-typed fields'
  convention).

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn span_ids_are_parsed() {
    let jsonl = r#"{"resourceSpans":[{"scopeSpans":[{"spans":[
        {"traceId":"aa","spanId":"bb","parentSpanId":"cc","name":"request",
         "startTimeUnixNano":"0","endTimeUnixNano":"1000000",
         "attributes":[{"key":"uri","value":{"stringValue":"https://h/api/create_post"}}]}
    ]}]}]}"#;
    let spans = parse_spans(jsonl, &Filters::default(), "t").expect("parses");
    assert_eq!(spans[0].span_id, "bb");
    assert_eq!(spans[0].parent_span_id, "cc");
}

#[test]
fn missing_parent_span_id_is_empty_not_an_error() {
    let jsonl = r#"{"resourceSpans":[{"scopeSpans":[{"spans":[
        {"traceId":"aa","spanId":"bb","name":"request",
         "startTimeUnixNano":"0","endTimeUnixNano":"1000000","attributes":[]}
    ]}]}]}"#;
    let spans = parse_spans(jsonl, &Filters::default(), "t").expect("parses");
    assert_eq!(spans[0].parent_span_id, "");
}
```

- [ ] **Step 2: Run, verify fail**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml traces::parse`
Expected: FAIL — no field `span_id`.

- [ ] **Step 3: Implement**

Add the two fields, populate from `span["spanId"]` / `span["parentSpanId"]` as
strings, and update the struct doc comment — it currently states these are
deliberately omitted, which becomes false.

- [ ] **Step 4: Run, verify pass**

Run: `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: PASS — existing `traces` analyzer tests unaffected.

- [ ] **Step 5: Commit**

```bash
git add xtask/src/traces/parse.rs xtask/src/lib.rs
git commit -m "feat(xtask): retain span/parent ids when parsing traces (#681)"
```

---

### Task 4: Per-test traceparent propagation

**Files:**

- Modify: `end2end/tests/otel.ts:114-157` (`SpanInput`, `buildSpan`)
- Modify: `end2end/tests/fixtures.ts` — new `testSpanId` fixture + helper;
  consumers at `:282-295` (`user`), `:328-346` (`verifiedUser`), `:349`
  (`_autoPerfSpan`), `:788-796` (the `e2e.test` span)
- Modify: `end2end/tests/perf.ts:55-59`
- Create: `end2end/tests/otel.unit.ts` (**not** `.spec.ts` — see Step 1)

**Interfaces:**

- Produces:

```ts
// otel.ts — buildSpan accepts a caller-minted id instead of always generating one.
export interface SpanInput {
  // …existing fields…
  spanId?: string;
}

// fixtures.ts — every context the fixtures create routes through this.
export function applyTestTraceparent(
  context: BrowserContext,
  traceId: string,
  testSpanId: string,
): Promise<void>;

// fixtures.ts — a worker-independent, per-test id that OTHER fixtures can destructure.
// `user`, `verifiedUser`, and `_autoPerfSpan` are independent fixtures and cannot read a
// value minted inside one another, so the id must itself be a fixture.
type JaunderFixtures = {
  testSpanId: string; // 16 lowercase hex chars
  // …existing fixtures…
};
```

The traceparent value is `00-${traceId}-${testSpanId}-01`.

- [ ] **Step 1: Write the failing tests**

`end2end/tests/otel.unit.ts` (new). **Named `.unit.ts`, not `.spec.ts`,
deliberately:** `testDir: "./tests"` with only an `admin-site|invite`
`testIgnore` (`playwright.config.ts:74-75`) means a `tests/otel.spec.ts` would
be collected by the `chromium`, `firefox`, and `webkit` projects and ship into
the e2e VM run — becoming a 20th spec file in all four combos and contradicting
spec D6's 19-file arithmetic. It would pass (it requests no
`page`/`context`/`browser` fixture, and Playwright instantiates fixtures
lazily), but it does not belong in the browser matrix. Run it explicitly by
filename instead:

```ts
import { test, expect } from "@playwright/test";
import { buildSpan } from "./otel";

test("buildSpan uses a caller-supplied span id", () => {
  const span = buildSpan({
    traceContext: { traceId: "a".repeat(32) },
    name: "e2e.test",
    kind: "client",
    startMs: 0,
    endMs: 1,
    spanId: "0123456789abcdef",
  });
  expect(span.spanId).toBe("0123456789abcdef");
});

test("buildSpan still mints an id when none is supplied", () => {
  const span = buildSpan({
    traceContext: { traceId: "a".repeat(32) },
    name: "e2e.test",
    kind: "client",
    startMs: 0,
    endMs: 1,
  });
  expect(span.spanId).toMatch(/^[0-9a-f]{16}$/);
});
```

- [ ] **Step 2: Run, verify fail**

Run:
`devtool run -- playwright test --config end2end/playwright.config.ts otel.unit.ts`
Expected: FAIL — `spanId` not accepted / ignored.

(The bare `playwright` binary from the devShell PATH, per
`xtask/src/steps/e2e_local.rs:179-198`. **Not** `npx` — `end2end/node_modules`
carries type deps only (`flake.nix:1156`), so `npx` resolves nothing locally and
may reach for the network.)

- [ ] **Step 3: Implement**

Add the optional `spanId` to `SpanInput` and use `input.spanId ?? randomHex(8)`
in `buildSpan`. In `fixtures.ts`, add a **`testSpanId` fixture** that mints
`randomHex(8)` once per test, and have `_autoPerfSpan`, `user`, and
`verifiedUser` all destructure it — they are independent fixtures, so a value
minted inside one is not visible to the others. Call `applyTestTraceparent`
immediately after each `browser.newContext()` (`:283`, `:333`) and for the
test's own context, and pass `testSpanId` as `spanId` to the `e2e.test`
`buildSpan` call so the id the server saw is the id the span carries. Update
`perf.ts` to use the same fixture rather than reading the env traceparent
independently.

Note: `browser.newContext()` does **not** inherit config-level
`extraHTTPHeaders`, so those two contexts carry no traceparent at all today —
this is what fixes them.

- [ ] **Step 4: Run, verify pass, then smoke one real spec**

Run:
`devtool run -- playwright test --config end2end/playwright.config.ts otel.unit.ts`
Expected: PASS Run: `devtool run -- cargo xtask e2e-local auth.spec.ts`
Expected: PASS — the shared fixture still drives a real browser run.

- [ ] **Step 5: Commit** (own commit — this is R1's blast radius)

```bash
git add end2end/tests/otel.ts end2end/tests/otel.unit.ts end2end/tests/fixtures.ts end2end/tests/perf.ts
git commit -m "feat(e2e): propagate a per-test traceparent to every browser context (#681)"
```

---

### Task 5: Convert the two non-fixture spec files

**Files:**

- Modify: `end2end/tests/atompub.spec.ts:1`
- Modify: `end2end/tests/feeds.spec.ts:1`

**Interfaces:**

- Consumes: `fixtures.ts`'s exported `test` / `expect` (Task 4).

- [ ] **Step 1: Change the imports**

In both files replace `import { test, expect } from "@playwright/test";` with
`import { test, expect } from "./fixtures";`. Keep
`import type { Page } from "@playwright/test";` and every other import. Both
files already import helpers from `./fixtures`, so merge rather than duplicate
the specifier.

- [ ] **Step 2: Run both specs**

Run: `devtool run -- cargo xtask e2e-local feeds.spec.ts` Expected: PASS Run:
`devtool run -- cargo xtask e2e-local atompub.spec.ts` Expected: PASS

These 9 tests become subject to `_autoPerfSpan`, which **requires the `page`
fixture** and runs `warmupPageContext` — real added per-test work these specs
did not previously do. (Their existing `setTestBudget` import does _not_ mean
the auto fixture was already active: it calls `test.info()` and works under
either `test` object.) If either spec now exceeds its budget, raise that spec's
budget rather than reverting the import — the attribution depends on it.

- [ ] **Step 3: Commit**

```bash
git add end2end/tests/atompub.spec.ts end2end/tests/feeds.spec.ts
git commit -m "test(e2e): put atompub and feeds specs on the shared fixtures (#681)"
```

---

### Task 6: The pure coverage extractor

**Files:**

- Create: `xtask/src/server_fn_coverage/mod.rs`,
  `xtask/src/server_fn_coverage/extract.rs`
- Create: `xtask/src/server_fn_coverage/testdata/coverage-sample.jsonl`
- Test: in-file `#[cfg(test)]` in `extract.rs`

**Interfaces:**

- Consumes: `server_fns::ServerFn` (Task 1), `traces::parse::Span` (Task 3)
- Produces:

```rust
/// Which tests exercised each server fn, plus hits attributable to no test.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    /// fn ident → sorted set of `e2e.test` titles that drove it.
    pub covered: BTreeMap<String, BTreeSet<String>>,
    /// fn ident → number of hits that resolved to no test.
    pub orphans: BTreeMap<String, usize>,
}

pub fn extract(spans: &[Span], inventory: &[ServerFn]) -> Coverage;
```

- [ ] **Step 1: Write the failing tests**

Hand-author `coverage-sample.jsonl` containing: an `e2e.test` span (`spanId`
`t1`, attribute `e2e.test` = `"creates a post"`); a `request` span (`spanId`
`r1`, `parentSpanId` `t1`, `uri` **`/api/create_post`** — origin form); an
instrument span (`spanId` `i1`, `parentSpanId` `r1`, name `update_post`,
attribute **`code.namespace`** = `web::posts::api`); a `request` span with `uri`
`/api/get_post?id=7` under `t1`; a `request` span with the **absolute** form
`https://h/api/list_tags` under `t1`; a `request` span for `/api/register` with
a parent that exists in no test; and a static-asset `GET` for
`/pkg/jaunder.wasm`.

Two details the fixture must get right, both of which would otherwise pass here
and fail only after Task 9's expensive combo:

- **`code.namespace`, not `target`.** `tracing-opentelemetry` records a span's
  module as `code.namespace` (`layer.rs:905`); `target` is attached only to
  _events_ (`:1044`). Its value is **crate-prefixed** (`web::posts::api`), while
  `ServerFn.module` is crate-relative (`posts::api`), so the comparison must
  strip the `web::` prefix.
- **`uri` is origin-form.** `make_request_span` records `uri = %request.uri()`
  (`server/src/observability.rs:496`), which for a normal server request is
  `/api/get_post?id=7`, **not** an absolute URL. An implementation built on
  `Url::parse` would find zero hits against a real capture, so both forms are in
  the fixture and both are asserted.

```rust
#[test]
fn uri_hit_attributes_to_its_test() {
    let c = extract(&sample_spans(), &sample_inventory());
    assert_eq!(c.covered["create_post"], set(["creates a post"]));
}

#[test]
fn instrument_span_attributes_through_its_request_parent() {
    // Two hops: instrument span -> request span -> test span.
    let c = extract(&sample_spans(), &sample_inventory());
    assert_eq!(c.covered["update_post"], set(["creates a post"]));
}

#[test]
fn query_string_is_stripped() {
    let c = extract(&sample_spans(), &sample_inventory());
    assert!(c.covered.contains_key("get_post"));
}

#[test]
fn both_origin_form_and_absolute_uris_resolve() {
    // Real captures carry origin-form (`/api/…`); the analyzer's older fixtures
    // carry absolute URLs. Both must work or Task 9 finds zero hits.
    let c = extract(&sample_spans(), &sample_inventory());
    assert!(c.covered.contains_key("create_post"), "origin form");
    assert!(c.covered.contains_key("list_tags"), "absolute form");
}

#[test]
fn unattributable_hit_lands_in_orphans_not_covered() {
    let c = extract(&sample_spans(), &sample_inventory());
    assert!(!c.covered.contains_key("register"));
    assert_eq!(c.orphans["register"], 1);
}

#[test]
fn non_api_traffic_is_ignored() {
    let c = extract(&sample_spans(), &sample_inventory());
    assert!(!c.covered.keys().any(|k| k.contains("wasm")));
    assert!(c.orphans.is_empty() || !c.orphans.contains_key("jaunder.wasm"));
}

#[test]
fn span_name_in_the_wrong_module_is_not_counted() {
    // A span named `update_post` whose code.namespace is `web::storage::posts`
    // is a different fn.
    let spans = spans_with_code_namespace("update_post", "web::storage::posts");
    let c = extract(&spans, &sample_inventory());
    assert!(!c.covered.contains_key("update_post"));
}

#[test]
fn code_namespace_crate_prefix_is_stripped_before_comparing() {
    // code.namespace is `web::posts::api`; ServerFn.module is `posts::api`.
    // Comparing them raw would reject every span-name hit — silently, since
    // `uri` would still carry the fn.
    let spans = spans_with_code_namespace("update_post", "web::posts::api");
    let c = extract(&spans, &sample_inventory());
    assert!(c.covered.contains_key("update_post"));
}

#[test]
fn a_span_name_hit_with_no_code_namespace_is_not_counted() {
    let spans = spans_with_code_namespace("update_post", "");
    let c = extract(&spans, &sample_inventory());
    assert!(!c.covered.contains_key("update_post"));
}

#[test]
fn a_fn_hit_by_both_signals_is_counted_once_per_test() {
    let c = extract(&sample_spans(), &sample_inventory());
    assert_eq!(c.covered["create_post"].len(), 1);
}
```

- [ ] **Step 2: Run, verify fail**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_coverage`
Expected: FAIL — module not defined.

- [ ] **Step 3: Implement `extract`**

Build a `span_id → &Span` map and an `e2e.test` span-id → title map. For each
span, identify a fn by (a) span name present in the inventory **and**
`code.namespace` — with the leading `web::` stripped — equal to that fn's
module, or (b) `uri` whose **path** begins `/api/` (handling both origin-form
and absolute values, query stripped), matched against the inventory's declared
`endpoint` — never against `"/api/" + ident`. Then walk `parent_span_id` upward
until the id is a known test span id; attribute on success, increment `orphans`
on failure. Every branch — both signals, the crate-prefix strip, the module
mismatch, the missing-namespace case, both URI forms, the query strip, the
orphan path, dedupe — is pinned above.

- [ ] **Step 4: Run, verify pass**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_coverage`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add xtask/src/server_fn_coverage xtask/src/lib.rs
git commit -m "feat(xtask): derive server-fn coverage from e2e trace spans (#681)"
```

---

### Task 7: Snapshot and allowlist model

**Files:**

- Create: `xtask/src/server_fn_coverage/snapshot.rs`
- Test: in-file `#[cfg(test)]`

**Interfaces:**

- Consumes: `Coverage` (Task 6), `ServerFn` (Task 1)
- Produces:

```rust
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct Snapshot {
    /// fn ident → sorted covering test titles. BTreeMap ⇒ stable key order.
    pub covered: BTreeMap<String, Vec<String>>,
    /// fn ident → orphan hit count.
    pub orphans: BTreeMap<String, usize>,
}

#[derive(Serialize, Deserialize)]
pub struct AllowlistEntry {
    pub server_fn: String,
    pub reason: String,
    pub issue: String,
}

pub fn render(snapshot: &Snapshot) -> String;          // stable, newline-terminated JSON
pub fn verdict(
    inventory: &[ServerFn],
    snapshot: &Snapshot,
    allowlist: &[AllowlistEntry],
) -> Vec<String>;                                       // one message per violation
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn render_is_byte_stable_across_equal_snapshots() {
    assert_eq!(render(&sample()), render(&sample()));
}

#[test]
fn render_sorts_keys_regardless_of_insertion_order() {
    let mut a = Snapshot::default();
    a.covered.insert("z_fn".into(), vec!["t".into()]);
    a.covered.insert("a_fn".into(), vec!["t".into()]);
    let out = render(&a);
    assert!(out.find("a_fn").unwrap() < out.find("z_fn").unwrap());
}

#[test]
fn uncovered_and_unallowlisted_fn_is_a_violation() {
    let v = verdict(&inv(["delete_media"]), &Snapshot::default(), &[]);
    assert_eq!(v.len(), 1);
    assert!(v[0].contains("delete_media"));
    assert!(v[0].contains("server-fn-coverage regenerate"), "names the remedy: {}", v[0]);
    assert!(v[0].contains("allowlist"), "names the other remedy: {}", v[0]);
}

#[test]
fn allowlisted_uncovered_fn_passes() {
    let al = vec![AllowlistEntry {
        server_fn: "delete_media".into(),
        reason: "covered by server integration tests, no browser flow".into(),
        issue: "https://github.com/jaunder-org/jaunder/issues/700".into(),
    }];
    assert!(verdict(&inv(["delete_media"]), &Snapshot::default(), &al).is_empty());
}

#[test]
fn allowlist_entry_without_reason_or_issue_is_rejected() {
    let al = vec![AllowlistEntry {
        server_fn: "delete_media".into(),
        reason: "  ".into(),
        issue: String::new(),
    }];
    let v = verdict(&inv(["delete_media"]), &Snapshot::default(), &al);
    assert!(!v.is_empty(), "a hollow allowlist entry must not satisfy the gate");
}

#[test]
fn allowlist_entry_for_a_covered_fn_is_a_violation() {
    // The ratchet must not loosen: stale entries are removed, not accumulated.
    let mut snap = Snapshot::default();
    snap.covered.insert("delete_media".into(), vec!["deletes media".into()]);
    let al = vec![AllowlistEntry {
        server_fn: "delete_media".into(),
        reason: "r".into(),
        issue: "i".into(),
    }];
    let v = verdict(&inv(["delete_media"]), &snap, &al);
    assert_eq!(v.len(), 1);
    assert!(v[0].contains("no longer needed"), "{}", v[0]);
}

#[test]
fn endpoint_not_matching_fn_name_is_a_violation() {
    let v = verdict(&inv_with_endpoint("get_post", Some("fetch_post")), &covered_all(), &[]);
    assert!(v.iter().any(|m| m.contains("get_post")));
}

#[test]
fn bare_server_attr_without_endpoint_is_drift() {
    let v = verdict(&inv_with_endpoint("thing", None), &covered_all(), &[]);
    assert!(v.iter().any(|m| m.contains("thing")));
}
```

- [ ] **Step 2: Run, verify fail**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml snapshot`
Expected: FAIL — `snapshot` module not defined.

- [ ] **Step 3: Implement**

`render` uses `serde_json::to_string_pretty` over `BTreeMap`s plus a trailing
newline. `verdict` aggregates, in order: endpoint/fn-name drift (including
`None`), uncovered and unallowlisted, hollow allowlist entries, and stale
allowlist entries. Messages name the fn and both remedies verbatim (AC14).

- [ ] **Step 4: Run, verify pass**

Run: `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add xtask/src/server_fn_coverage/snapshot.rs
git commit -m "feat(xtask): server-fn coverage snapshot model and verdict (#681)"
```

---

### Task 8: Wire the gate into both lanes

**Files:**

- Modify: `xtask/src/lib.rs` (add `Command::ServerFnCoverage { … }` with
  `Regenerate` / `Verify`, and the `command_name()` arm)
- Create: `xtask/src/steps/server_fn_coverage_check.rs` (the static-lane step)
- Modify: `xtask/src/traces/run.rs:97` — bump `extract_trace` to `pub(crate)`.
  It already does exactly what is needed (`tar` + `flate2`, no shelling out),
  but is private.
- Modify: `xtask/src/steps/nix.rs` (after a passing `sqlite × chromium` combo,
  regenerate and compare)
- Test: in-file `#[cfg(test)]`; CLI-parse tests alongside the existing ones in
  `lib.rs:584+`

**Registration is deferred to Task 9.** Do **not** add
`steps::server_fn_coverage_check::run` to `lib.rs:296`/`:328` in this task — see
Step 5.

**Interfaces:**

- Consumes: `verdict`, `render`, `extract`, `server_fns_in`
- Produces: `cargo xtask server-fn-coverage regenerate|verify`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn missing_capture_fails_closed() {
    let err = coverage_from_capture(Path::new("/nonexistent.tar.gz")).unwrap_err();
    assert!(err.to_string().contains("capture"), "{err}");
}

#[test]
fn empty_capture_fails_closed_rather_than_reporting_full_coverage() {
    let err = coverage_from_jsonl("").unwrap_err();
    assert!(err.to_string().contains("no spans"), "{err}");
}

#[test]
fn unparseable_capture_fails_closed() {
    assert!(coverage_from_jsonl("{not json").is_err());
}

#[test]
fn gate_bites_on_an_uncovered_fn() {
    // AC12 — the enforcement proof lives in the repo, not in PR prose.
    let inventory = inv(["create_post", "brand_new_uncovered_fn"]);
    let mut snap = Snapshot::default();
    snap.covered.insert("create_post".into(), vec!["creates a post".into()]);
    let v = verdict(&inventory, &snap, &[]);
    assert!(v.iter().any(|m| m.contains("brand_new_uncovered_fn")));
}

#[test]
fn cli_parses_regenerate_and_verify() {
    assert!(Cli::try_parse_from(["xtask", "server-fn-coverage", "regenerate"]).is_ok());
    assert!(Cli::try_parse_from(["xtask", "server-fn-coverage", "verify"]).is_ok());
}
```

- [ ] **Step 2: Run, verify fail**

Run: `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: FAIL — subcommand and helpers not defined.

- [ ] **Step 3: Implement**

`regenerate` reads
`.xtask/diagnostics/e2e-sqlite-chromium/capture-sqlite.tar.gz`, extracts
`capture/otel-traces.jsonl` (reuse `traces::run`'s existing extraction helper
rather than shelling to `tar`), runs `extract`, and writes
`docs/coverage/server-fns.json` via `render`. `verify` does the same and
compares to the committed file, failing on any difference.

The **static-lane step** reads only the committed snapshot + allowlist + syn
inventory and calls `verdict` — no capture, so it runs in `validate --no-e2e`.
In `nix.rs`, after a passing `sqlite × chromium` combo, run the `verify` path;
per D8 do **not** run it from the aggregate `checks.e2e` join, where both sqlite
combos' captures collide.

- [ ] **Step 4: Run, verify pass**

Run: `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: PASS — the step's behavior is proven by its unit tests, since it is
not yet wired into either lane.

- [ ] **Step 5: Commit — code only, not the registration**

```bash
git add xtask/src
git commit -m "feat(xtask): server-fn flow-coverage check and regenerate/verify commands (#681)"
```

**Why the registration waits.** Between this task and Task 9 the snapshot does
not exist yet, so a registered step would see 55 inventory fns with neither
snapshot nor allowlist and fail — and pre-commit runs the full
`cargo xtask check`, so this very commit could not land. The `lib.rs:296`/`:328`
registration therefore ships in **Task 9's** commit, atomically with the seeded
artifacts that make it green. Verify before committing that
`cargo xtask check --no-test` is still green (it will be — nothing calls the new
step yet).

---

### Task 9: Seed from a real run, file the gaps, commit the snapshot

**Files:**

- Create: `docs/coverage/server-fns.json`,
  `docs/coverage/server-fns-allowlist.json`
- Create: `xtask/src/server_fn_coverage/testdata/otel-traces-seed.jsonl` (the
  AC2/AC11 fixture)
- Modify: `xtask/src/lib.rs:296` and `:328` — register
  `steps::server_fn_coverage_check::run` in both arms (deferred from Task 8; it
  becomes green only once this task's artifacts exist)

- [ ] **Step 1: Run the authoritative combo**

Run: `devtool run -- cargo xtask e2e sqlite chromium` Expected: PASS. Run in the
**foreground** with a long timeout — a backgrounded slow gate gets killed. If
the host is loaded, run nothing else concurrently.

- [ ] **Step 2: Regenerate and inspect**

Run: `devtool run -- cargo xtask server-fn-coverage regenerate` Then read
`docs/coverage/server-fns.json` and record: how many of the 55 are covered, and
the exact uncovered list. **This list — not the preliminary guess — is the
seed.** Confirm the orphan bucket contains only outside-any-test traffic; a
per-test orphan means Task 4 or 5 is incomplete, and that must be fixed before
proceeding rather than allowlisted around.

- [ ] **Step 3: Commit the seeding capture, and assert the allowlist against
      it**

Copy the extracted `otel-traces.jsonl` to
`xtask/src/server_fn_coverage/testdata/otel-traces-seed.jsonl`. Then add the
test that actually enforces AC11 — without it, an evidence-seeded allowlist and
a guessed one are byte-identical and nothing in the repo can tell them apart:

```rust
#[test]
fn every_allowlist_entry_is_absent_from_the_seed_captures_hit_set() {
    let spans = read_spans(Path::new(SEED_FIXTURE), &Filters::default()).expect("fixture");
    let inventory = inventory_from_web_src().expect("enumerates");
    let coverage = extract(&spans, &inventory);
    for entry in load_allowlist().expect("allowlist") {
        assert!(
            !coverage.covered.contains_key(&entry.server_fn),
            "{} is allowlisted but the seed capture shows it covered — \
             the allowlist was not derived from evidence",
            entry.server_fn
        );
    }
}
```

Also assert the seed fixture reproduces the committed snapshot, which pins the
extractor against real data rather than only hand-authored spans:

```rust
#[test]
fn seed_fixture_reproduces_the_committed_snapshot() {
    let spans = read_spans(Path::new(SEED_FIXTURE), &Filters::default()).expect("fixture");
    let inventory = inventory_from_web_src().expect("enumerates");
    let regenerated = render(&Snapshot::from(extract(&spans, &inventory)));
    let committed = std::fs::read_to_string("docs/coverage/server-fns.json").expect("snapshot");
    assert_eq!(regenerated, committed);
}
```

- [ ] **Step 4: File one issue per genuine gap**

For each uncovered fn, file via **`jaunder-issues`** (`--type Task`, milestone
_Test infrastructure & E2E_, blocked-by nothing) describing the missing browser
flow. Where a fn has server integration coverage but no browser flow, say so —
that is the allowlist reason the spec sanctions, not an excuse. Write each issue
URL into the allowlist entry.

- [ ] **Step 5: Write the allowlist, register the check, verify the gate is
      green**

Add `steps::server_fn_coverage_check::run(&mut result);` to
`xtask/src/lib.rs:296` (Check) and `:328` (Validate), beside
`server_fn_registrar_check::run`.

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS — every
inventory fn is now covered or allowlisted with reason + issue, so the step is
green the moment it is wired in.

- [ ] **Step 6: Observe AC16 — the artifacts do not invalidate the e2e
      derivations**

Before staging, capture the baseline; after staging, compare:

```bash
nix eval --raw .#checks.x86_64-linux.e2e-sqlite-chromium.drvPath
```

Expected: **identical** before and after the artifact commit. (Nix ignores
untracked files, so `git add` first, then evaluate.) This is the only step that
actually observes AC16 — `flake.nix:278-297` is an allowlist filter admitting
only `.sql`, `.css`, `csr/index.html`, `scripts/*`, and cargo sources, and
`e2ePackage` is `src = ./end2end` (`:546`), so `.json` under `docs/` is in
neither. Note the `static-checks` derivation's filter (`flake.nix:1134-1139`) is
exclusion-only, so its cache _does_ bust — cheap (a `runCommand`), and outside
AC16's claim, but expect it.

- [ ] **Step 7: Commit**

```bash
git add docs/coverage xtask/src/server_fn_coverage/testdata xtask/src/lib.rs
git commit -m "test(xtask): seed server-fn coverage snapshot and allowlist from a real run (#681)"
```

---

### Task 10: Documentation and CI note

**Files:**

- Modify: `docs/observability.md` (a short section: what the snapshot is, how to
  regenerate)
- Modify: `CONTRIBUTING.md` (coverage policy — the new obligation when adding a
  `#[server]` fn)
- Modify: `docs/adr/drafts/empirical-server-fn-flow-coverage.md` (only if
  implementation diverged from the draft; the ADR is numbered at ship by
  `cargo xtask adr promote`)

- [ ] **Step 1: Document the developer obligation**

In `CONTRIBUTING.md`, alongside the existing coverage policy: adding a
`#[server]` fn requires either an e2e flow plus
`cargo xtask server-fn-coverage regenerate`, or an allowlist entry with a reason
and a filed issue. Name both commands exactly. Keep it to what the author needs
at the moment the gate reddens.

- [ ] **Step 2: Note the CI shape**

In `docs/observability.md`, record that the snapshot is regenerated only by the
`sqlite × chromium` combo (D8's `symlinkJoin` collision is the reason — state
it, or someone will "fix" it by moving the check to the aggregate).

- [ ] **Step 3: Run the full local gate**

Run: `devtool run -- cargo xtask validate --no-e2e` Expected: PASS. Then
`git status --porcelain` — `check` auto-fixes formatting without committing, so
confirm the tree is clean before the final commit.

- [ ] **Step 4: Commit**

```bash
git add docs CONTRIBUTING.md
git commit -m "docs: record the server-fn flow-coverage gate and its workflow (#681)"
```

---

## Self-review

**Spec coverage.** AC1→T1 · AC2→T6+T9 Step 3 · AC3→T2 · AC4→T7 · AC5→T6+T9 ·
AC6→T5 · AC7→T8+T9 · AC8→T8+T9 Step 5 · AC9→T7 · AC10→T7 · AC11→**T9 Step 3's
`every_allowlist_entry_is_absent_from_the_seed_captures_hit_set`** · AC12→T8 ·
AC13→T4/T5/T9 · AC14→T7 · AC15→T8 · AC16→**T9 Step 6's `drvPath` comparison**.

AC11 and AC16 previously had no enforcing step — each was asserted in this table
but nothing in the repo or the task list would have checked it. Both now have
one.

**Ordering.** T1–T3 are pure host work. T4–T5 change the harness and are proven
by `e2e-local` before the expensive full combo. T6–T8 build on T1/T3 and are
unit-tested. T9 is the single task requiring a full combo, and is the only one
that can surface an attribution defect — which is why T4's orphan invariant is
checked there explicitly rather than assumed.

**Every task's commit gate is green at that task.** The one hazard is a check
registered before the data it needs exists: T8 builds the coverage step but T9
registers it, so no commit is ever blocked by a gate for artifacts that do not
yet exist. T2's span-name check has no such dependency — it is green as soon as
the twelve names are gone — so it registers in its own task.

**Naming consistency.** `ServerFn.ident` is the coverage key throughout (T1, T6,
T7); `ServerFn.name` stays the PascalCase registrar key and is used only by
`server_fn_registrar_check`. `extract` → `Coverage`; `render`/`verdict` →
`Snapshot`. No task references a type another task does not define.
