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
8. The e2e lane — `regenerate`/`verify` a snapshot from a real capture
   - 8b. Finish Task 4's sweep: trace all 15 spec-level contexts, and gate it
9. The static lane + registration, seeded by a real run, with the gaps filed
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

- [x] **Step 6: Commit** — `8383fba9`

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

- [x] **Step 1: Write the failing test**

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

- [x] **Step 2: Run, verify fail**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml traces::parse`
Expected: FAIL — no field `span_id`.

- [x] **Step 3: Implement**

Add the two fields, populate from `span["spanId"]` / `span["parentSpanId"]` as
strings, and update the struct doc comment — it currently states these are
deliberately omitted, which becomes false.

- [x] **Step 4: Run, verify pass**

Run: `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: PASS — **actual: 296 tests, 296 passed**; the single construction site
meant no existing analyzer test needed touching.

- [x] **Step 5: Commit** — `7951d431`

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

- [x] **Step 2: Run, verify fail** — **not done; the unit spec was dropped.**

**Deviation, with reasoning.** `.unit.ts` does not match Playwright's default
`testMatch` (`**/*.@(spec|test).?(c|m)[jt]s?(x)`), so the file would never be
collected — passing it as a CLI filter only filters _collected_ files. The
alternatives were all worse than the thing being tested: name it `.spec.ts` and
it joins the browser matrix in all four combos (the problem this step was
written to avoid), or stand up a second test runner for what is literally
`input.spanId ?? randomHex(8)`.

Dropped instead, because the invariant is checked where it actually matters: if
the override silently failed, `buildSpan` would mint a _different_ id from the
one propagated in the traceparent, every server request span's `parentSpanId`
would point at an id no `e2e.test` span carries, and **every hit would land in
the orphan bucket** — which Task 9 Step 3 asserts must contain only
outside-any-test traffic. That is a stronger, end-to-end check than the unit
test would have been. `tsc` covers the type surface.

- [x] **Step 3: Implement**

Add the optional `spanId` to `SpanInput` and use `input.spanId ?? randomHex(8)`
in `buildSpan`. In `fixtures.ts`, add a **`testSpanId` fixture** that mints
`randomHex(8)` once per test, and have `_autoPerfSpan`, `user`, and
`verifiedUser` all destructure it — they are independent fixtures, so a value
minted inside one is not visible to the others. Call `applyTestTraceparent`
immediately after each `browser.newContext()` (`:283`, `:333`) and for the
test's own context, and pass `testSpanId` as `spanId` to the `e2e.test`
`buildSpan` call so the id the server saw is the id the span carries. ~~Update
`perf.ts` to use the same fixture rather than reading the env traceparent
independently.~~

Note: `browser.newContext()` does **not** inherit config-level
`extraHTTPHeaders`, so those two contexts carry no traceparent at all today —
this is what fixes them.

**`perf.ts` was deliberately left alone — the plan was wrong here.** It builds
an independent `e2e.flow.<flow>` diagnostic span and merely _records_ the env
traceparent as an attribute; it is not part of the attribution chain. Giving it
`testSpanId` would have produced **two spans sharing one span id**, corrupting
the `span_id → parent_span_id` map the ancestor walk is built on — turning a
diagnostic nicety into a correctness bug in the thing this cycle exists to
build.

Also added: `newSpanId()` exported from `otel.ts`, so the fixture mints ids
through the same helper `buildSpan` uses rather than duplicating `randomHex(8)`.
`applyTestTraceparent` is applied to the test's own context _after_
`warmupPageContext`, so optional warmup traffic — which is not part of the test
— stays out of the attribution.

- [x] **Step 4: Run, verify pass, then smoke one real spec**

Run: `devtool run -- cargo xtask check --no-test` (covers `tsc` + lint on the
harness change) — **PASS**. Run:
`devtool run -- cargo xtask e2e-local auth.spec.ts` — **PASS, 12/12 in 22.0s**.
`auth.spec.ts` was chosen because it exercises registration and login through
the shared fixtures, so it puts the new per-test traceparent on the real path.
(The `500` in the log is the deliberate "login with wrong password shows error"
negative-path test — pre-existing masked-error behaviour, not the header.)

- [x] **Step 5: Commit** (own commit — this is R1's blast radius)

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

- [x] **Step 1: Change the imports**

In both files replace `import { test, expect } from "@playwright/test";` with
`import { test, expect } from "./fixtures";`. Keep
`import type { Page } from "@playwright/test";` and every other import. Both
files already import helpers from `./fixtures`, so merge rather than duplicate
the specifier.

- [x] **Step 2: Run both specs**

Run: `devtool run -- cargo xtask e2e-local feeds.spec.ts` Expected: PASS Run:
`devtool run -- cargo xtask e2e-local atompub.spec.ts` Expected: PASS

These 9 tests become subject to `_autoPerfSpan`, which **requires the `page`
fixture** and runs `warmupPageContext` — real added per-test work these specs
did not previously do. (Their existing `setTestBudget` import does _not_ mean
the auto fixture was already active: it calls `test.info()` and works under
either `test` object.) If either spec now exceeds its budget, raise that spec's
budget rather than reverting the import — the attribution depends on it.

**Actual: both green, no budget change needed** — `feeds.spec.ts` 7/7 in 30.4s,
`atompub.spec.ts` 3/3 in 6.6s. The `page`-fixture/warmup cost the review warned
about was absorbed by the existing budgets.

- [x] **Step 3: Commit** — `e4b9324f`

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

- [x] **Step 1: Write the failing tests**

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

- [x] **Step 2: Run, verify fail**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_coverage`
Expected: FAIL — module not defined.

- [x] **Step 3: Implement `extract`**

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

- [x] **Step 4: Run, verify pass**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml server_fn_coverage`
Expected: PASS

- [x] **Step 5: Commit** — `b23b18c0`

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

- [x] **Step 2: Run, verify fail**

Run:
`devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml snapshot`
Expected: FAIL — `snapshot` module not defined.

- [x] **Step 3: Implement**

`render` uses `serde_json::to_string_pretty` over `BTreeMap`s plus a trailing
newline. `verdict` aggregates, in order: endpoint/fn-name drift (including
`None`), uncovered and unallowlisted, hollow allowlist entries, and stale
allowlist entries. Messages name the fn and both remedies verbatim (AC14).

- [x] **Step 4: Run, verify pass**

Run: `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: PASS

- [x] **Step 5: Commit** — `ebdfc4ba`

```bash
git add xtask/src/server_fn_coverage/snapshot.rs
git commit -m "feat(xtask): server-fn coverage snapshot model and verdict (#681)"
```

---

### Task 8: The e2e lane — `regenerate` / `verify` from a capture

**Files:**

- Modify: `xtask/src/lib.rs` (add `Command::ServerFnCoverage { … }` with
  `Regenerate` / `Verify`, and the `command_name()` arm)
- Create: `xtask/src/steps/server_fn_coverage_check.rs` (the e2e-lane step)
- Modify: `xtask/src/traces/run.rs:97` — bump `extract_trace` to `pub(crate)`.
  It already does exactly what is needed (`tar` + `flate2`, no shelling out),
  but is private.
- Test: in-file `#[cfg(test)]`; CLI-parse tests alongside the existing ones in
  `lib.rs:584+`

> **Scope correction (made while executing).** This task originally also carried
> the **static lane** (`run`/`check`) and the post-combo hook, with registration
> deferred to Task 9 on the reasoning that "nothing calls the new step yet, so
> `check --no-test` stays green". **That reasoning is wrong for xtask.**
> `mod steps` is private, so an unregistered `pub fn run` is not part of the
> crate's public API and `-D dead-code` rejects it outright — the commit cannot
> land at all, let alone go red on a missing snapshot. Verified empirically:
> `xtask-clippy` failed with `constant STATIC_STEP is never used` /
> `function check is never used` / `function run is never used`.
>
> A new pub item and its first consumer must therefore share a commit. So the
> static lane and the post-combo hook **move wholesale into Task 9**, landing
> atomically with the artifacts that make them green — which is where the
> original "why the registration waits" argument pointed anyway. What remains
> here is the half that is genuinely reachable on its own: the capture-driven
> core, consumed by the `server-fn-coverage` CLI arm.

**Interfaces:**

- Consumes: `render`, `extract`, `server_fns_in`
- Produces: `cargo xtask server-fn-coverage regenerate|verify`

- [x] **Step 1: Write the failing tests**

The capture-side fail-closed tests (`missing_capture_fails_closed`,
`empty_capture_fails_closed_rather_than_reporting_full_coverage`,
`unparseable_capture_fails_closed`, `whitespace_only_capture_fails_closed`) live
with the code they exercise, in `server_fn_coverage/io.rs`. `verdict`'s bite
test (AC12) is `gate_bites_on_a_newly_added_uncovered_fn` in `snapshot.rs`. The
step's own tests are the lane-level fail-closed pair:

```rust
#[test]
fn verify_from_a_missing_capture_is_an_error() {
    // Not `Ok(fail)`: a broken capture must reach the exit-2 path.
    let err = regenerate_or_verify(web_src, Path::new("/nonexistent.tar.gz"), snap, false)
        .unwrap_err();
    assert!(format!("{err:#}").contains("capture"), "{err:#}");
}

#[test]
fn an_unscannable_web_src_is_an_error_not_an_empty_inventory() { … }

#[test]
fn server_fn_coverage_parses_both_subcommands() {
    let cli = Cli::try_parse_from(["xtask", "server-fn-coverage", "regenerate"]).unwrap();
    assert_eq!(cli.command_name(), "server-fn-coverage-regenerate");
    // …and `verify`, plus `server_fn_coverage_requires_a_subcommand`.
}
```

- [x] **Step 2: Run, verify fail**

Run: `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: FAIL — subcommand and helpers not defined.

- [x] **Step 3: Implement**

`regenerate` reads
`.xtask/diagnostics/e2e-sqlite-chromium/capture-sqlite.tar.gz`, extracts
`capture/otel-traces.jsonl` (reuse `traces::run`'s existing extraction helper
rather than shelling to `tar`), runs `extract`, and writes
`docs/coverage/server-fns.json` via `render`. `verify` does the same and
compares to the committed file, failing on any difference.

Both spellings share one path-parameterized core,
`regenerate_or_verify(web_src, capture, snapshot_path, regenerate)`, with
`from_capture` as the thin shell over the repo's real roots — the same
pure-core/thin-shell seam `server_fn_registrar_check.rs` uses, and what makes
the lane testable without a checkout. Comparison is on the **rendered bytes**,
not the parsed value, so a hand-edit that happens to parse equal still counts as
drift.

`StepResult` gains `#[derive(Debug)]` — `unwrap_err()` requires `T: Debug`, and
the struct is plain data.

- [x] **Step 4: Run, verify pass**

Run: `devtool run -- cargo nextest run --manifest-path xtask/Cargo.toml`
Expected: PASS — **actual: 333 tests, 333 passed**, and
`cargo xtask check --no-test` green.

- [x] **Step 5: Commit**

```bash
git add xtask/src
git commit -m "feat(xtask): server-fn coverage regenerate/verify from an e2e capture (#681)"
```

**Why the registration waits.** A registered static-lane step would see 55
inventory fns with neither snapshot nor allowlist and fail — and pre-commit runs
the full `cargo xtask check`, so that commit could not land. Under
`-D dead-code` the _unregistered_ step cannot land either (see the scope
correction above), so the step's code and its registration both ship in **Task
9's** commit, atomically with the seeded artifacts that make them green.

---

### Task 8b: Finish Task 4's sweep — every context traced, and gated

**Discovered while executing Task 9 Step 2**, which is exactly where the plan
said to look ("a per-test orphan means Task 4 or 5 is incomplete, and that must
be fixed before proceeding rather than allowlisted around").

The first seeding run reported 51/55 covered with **608 orphan hits**, and two
of the four "uncovered" fns — `subscribe_to`, `unsubscribe_from` — were
**orphan-only**: hit, but never attributed. Walking the capture, every one of
those spans had parent span id `1111111111111111` — the _run-wide_ traceparent
that `flake.nix` builds from `traceDigit` and `playwright.config.ts` installs as
a static `use.extraHTTPHeaders`. That is the signature of a context created off
the raw `browser` fixture, which does not inherit config-level headers.

Task 4 shipped `applyTestTraceparent` and its doc even says "Must be called for
EVERY context a test uses" — but only the two _fixture_ call sites ever called
it. An enumeration of `newContext(` across `end2end/tests` found **18 sites, 15
of them untraced**: `audiences.spec.ts` ×3, `visibility.spec.ts` ×7 (two inside
local `expectPostVisible`/`expectPostHidden` helpers), `posts.spec.ts` ×4 (one
via `context.browser()!.newContext()`), `invite.spec.ts` ×1.

So the seed would have been wrong in both directions: under-counting coverage,
and inviting per-gap issues for fns the suite already drives.

**Files:**

- Modify: `end2end/tests/fixtures.ts` — add the `NewTracedContext` type and the
  `tracedContext` fixture; move `user`/`verifiedUser` onto it
- Modify: `audiences.spec.ts`, `visibility.spec.ts`, `posts.spec.ts`,
  `invite.spec.ts` — all 15 sites
- Create: `xtask/src/steps/traced_context_check.rs`
- Modify: `xtask/src/lib.rs` — declare and register the check in both arms

- [x] **Step 1: Add the `tracedContext` fixture**

A factory, not another 3-arg helper call: it closes over `traceId`/`testSpanId`
so the traceparent cannot be omitted at the call site. The caller still owns the
context's lifetime. `user` and `verifiedUser` move onto it too, so there is one
implementation rather than three.

- [x] **Step 2: Convert all 15 sites**

`visibility.spec.ts`'s two local helpers took `browser: Browser`; they now take
`newContext: NewTracedContext`, and the `Browser` type import goes.

- [x] **Step 3: Gate it**

`traced-context` forbids `.newContext(` anywhere under `end2end/tests` except
`fixtures.ts` (the sanctioned door), keyed on the **method** rather than a
`browser.` receiver — the `context.browser()!.newContext()` form in
`posts.spec.ts` would evade a receiver match. Registered in both `check` and
`validate` arms; unlike the coverage step it is green immediately, so there is
no dead-code/bootstrap problem.

- [x] **Step 4: Verify**

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS — **actual:
green, `traced-context — 31 spec file(s) clean`, `tsc` ok**, so the conversion
typechecks. The real proof is Task 9's re-run: the orphan bucket must collapse.

- [x] **Step 5: Commit**

---

### Task 8c: The diagnostics copy silently kept the first run's capture

**Discovered when Task 8b's proof re-run reported byte-identical numbers** (51
covered / 608 orphans — impossible by chance). The capture in
`.xtask/diagnostics/` was still the previous run's: its md5 was `016bb8…` while
the gcroot's was `67d3de…`.

Cause: `copy_e2e_diagnostics_between` does `fs::copy(from, to)`, and every
lifted artifact comes from the nix store, so the _previous_ copy is on disk
`0444`. `fs::copy` onto a read-only file fails `EACCES`, and the result was
discarded (`if …is_ok()`), so re-runs silently kept the first run's artifacts.

This is squarely load-bearing for #681: `verify` reads its capture from that
directory, so the gate would have compared a new build against **stale traces**
— green when it should be red, or red for a drift that no longer exists. Exactly
the dishonest-gate failure the spec exists to prevent.

**Files:** `xtask/src/steps/nix.rs`

- [x] **Step 1: Remove-then-copy, and drop the read-only bit**

`remove_file` before the copy, then `set_permissions(0o644)` after, so a later
run can overwrite even if the remove fails. Uses an explicit mode rather than
`set_readonly(false)` (which grants all-user write and trips
`clippy::permissions_set_readonly_false`).

- [x] **Step 2: Regression test**

`copy_e2e_diagnostics_overwrites_a_read_only_previous_copy` — copy, chmod the
destination `0444` as the store does, copy a changed source, assert the new
bytes land.

- [x] **Step 3: Verify against the real artifact**

Re-ran the combo (cache hit, ~6 s); the diagnostics capture now matches the
gcroot's md5, and regeneration moved to **53 covered / 444 orphans**.

---

### Task 9: The static lane, seeded from a real run, with the gaps filed

**Files:**

- Create: `docs/coverage/server-fns.json`,
  `docs/coverage/server-fns-allowlist.json`
- Create: `xtask/src/server_fn_coverage/testdata/otel-traces-seed.jsonl` (the
  AC2/AC11 fixture)
- Modify: `xtask/src/steps/server_fn_coverage_check.rs` — add the static lane
  (`run`/`check`) and the post-combo hook (`verify_after_combo`), moved here
  from Task 8
- Modify: `xtask/src/lib.rs:296` and `:328` — register
  `steps::server_fn_coverage_check::run` in both arms; and the `Command::E2e`
  arm — call `verify_after_combo` after `flaky::collect`

**The moved-in code** (written and gate-verified during Task 8, then held back
because `-D dead-code` rejects an unregistered `pub fn`; restore it verbatim
alongside the registration):

```rust
const STATIC_STEP: &str = "server-fn-coverage";

/// The one combo whose traces are authoritative (spec D6).
const AUTHORITATIVE: (&str, &str) = ("sqlite", "chromium");

/// The static-lane check, over explicit paths so it is testable without the repo.
/// A missing snapshot, an unscannable `web/src`, or an unparseable artifact is a
/// **failure**, never a pass.
fn check(web_src: &Path, snapshot_path: &Path, allowlist_path: &Path) -> StepResult {
    let (inventory, snapshot, allowlist) = match (
        inventory(web_src), read_snapshot(snapshot_path), read_allowlist(allowlist_path),
    ) {
        (Ok(i), Ok(s), Ok(a)) => (i, s, a),
        (i, s, a) => {
            let detail = [i.err(), s.err(), a.err()].into_iter().flatten()
                .map(|e| format!("{e:#}")).collect::<Vec<_>>().join("\n");
            return StepResult::fail(STATIC_STEP).detail(detail);
        }
    };
    let violations = verdict(&inventory, &snapshot, &allowlist);
    if violations.is_empty() {
        return StepResult::ok(STATIC_STEP)
            .detail(format!("{} server fn(s) accounted for", inventory.len()));
    }
    StepResult::fail(STATIC_STEP).detail(violations.join("\n"))
}

pub fn run(result: &mut CommandResult) {
    result.push(check(Path::new(WEB_SRC), Path::new(SNAPSHOT_PATH), Path::new(ALLOWLIST_PATH)));
}

/// After the authoritative combo, confirm the committed snapshot still matches
/// what the suite exercised. A no-op for every other combo (D8/D6). Skipped when
/// the combo itself failed: a failed run's capture is partial or absent, so drift
/// against it would be noise on top of the real failure.
pub fn verify_after_combo(result: &mut CommandResult, backend: &str, browser: &str) {
    if (backend, browser) != AUTHORITATIVE { return; }
    if !result.ok {
        result.push(StepResult::skip(VERIFY_STEP).detail("combo failed — no trustworthy capture"));
        return;
    }
    let step = from_capture(Path::new(CAPTURE_PATH), false)
        .unwrap_or_else(|e| StepResult::fail(VERIFY_STEP).detail(format!("{e:#}")));
    result.push(step);
}
```

Its tests, likewise moved: `static_lane_passes_when_every_fn_is_covered`,
`static_lane_bites_on_an_uncovered_fn` (the wired-up half of AC12),
`static_lane_accepts_a_substantive_allowlist_entry`,
`static_lane_fails_closed_on_a_missing_snapshot` (asserts the detail names
`REGENERATE_CMD`), `static_lane_fails_closed_on_an_unscannable_web_src`,
`static_lane_fails_closed_on_an_unparseable_snapshot`,
`e2e_lane_is_a_no_op_for_a_non_authoritative_combo` (all three other combos),
and `e2e_lane_skips_when_the_combo_failed`.

**Ordering note.** Step 1's seeding run must happen _before_
`verify_after_combo` is wired, or the combo goes red on the not-yet-existing
snapshot — which is the same bootstrap problem, just relocated to the e2e lane.
Seed first, then wire.

- [x] **Step 1: Run the authoritative combo**

Run: `devtool run -- cargo xtask e2e sqlite chromium` Expected: PASS. Run in the
**foreground** with a long timeout — a backgrounded slow gate gets killed. If
the host is loaded, run nothing else concurrently.

**Actual: PASS**, 574 s cold. (Ran three times in total: the first seeded the
now-discarded pre-8b numbers, the second proved 8b, the third was a cache hit
that refreshed the diagnostics copy once 8c landed.)

- [x] **Step 2: Regenerate and inspect**

Run: `devtool run -- cargo xtask server-fn-coverage regenerate` Then read
`docs/coverage/server-fns.json` and record: how many of the 55 are covered, and
the exact uncovered list. **This list — not the preliminary guess — is the
seed.** Confirm the orphan bucket contains only outside-any-test traffic; a
per-test orphan means Task 4 or 5 is incomplete, and that must be fixed before
proceeding rather than allowlisted around.

**Actual, after 8b and 8c: 53 of 55 covered, 444 orphan hits, zero per-test
orphans.** The orphan bucket is exactly 111 hits — one per test — across four
app-shell fns (`session`, `list_local_timeline`, `backup_warning_visible`,
`base_url_warning_visible`), every one parented to the run-wide traceparent.
That is the `_autoPerfSpan` **warmup** page load, which applies the per-test
traceparent only _after_ `warmupPageContext` precisely so warmup traffic stays
out of attribution. All four are also covered, so nothing is lost. This
satisfies AC5's "only traffic occurring outside any test".

The uncovered two — **`delete_media`** and **`revoke_session`** — have zero hits
by _either_ signal. This is the seed. (The pre-8b run's list also named
`subscribe_to`/`unsubscribe_from`; both were attribution failures, not gaps.)

- [x] **Step 3: Commit the seeding capture, and assert the allowlist against
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

> **Deviation: the raw capture is 25 MB, and byte-reproduction was dropped.**
> Committing the capture verbatim is out of the question, and the planned
> `seed_fixture_reproduces_the_committed_snapshot` test is what forced it to be
> verbatim — reproducing the snapshot byte-for-byte requires preserving every
> attributed hit _and_ the exact per-fn orphan counts. Worse, it would make the
> blob **churn**: any coverage change alters the snapshot, so the multi-MB
> fixture would have to be re-seeded, adding another copy to git history each
> time.
>
> Neither AC2 nor AC11 asks for byte-reproduction — AC2 wants the extractor
> exercised against a real capture on both signals, AC11 wants allowlist entries
> absent from that capture's **hit set**. Both survive reduction. So the fixture
> is reduced to what the extractor actually reads: one hit-chain per
> (span-name-or-endpoint, test) pair, ≤2 orphan examples per key, 8 non-`/api/`
> spans for the negative case, and only the five attributes
> `parse_spans`/`extract` consume — dropping the per-test `e2e.*_json` perf
> blobs that dominated the size. **25 MB → 387 KiB**, and it now only needs
> re-seeding when a _new_ allowlist entry appears.
>
> Replaced by `seed_capture_covers_the_committed_snapshots_fns`, which asserts
> the fixture is still the evidence behind the snapshot (every fn the snapshot
> calls covered is covered in the reduced capture) without pinning orphan
> counts. That test **caught a real reduction bug**: the pruning script's own
> regex required `endpoint` to be `#[server]`'s first argument, so
> `upload_media`
> (`#[server(input = MultipartFormData, endpoint = "/upload_media")]`) was
> dropped. Re-keyed the reduction on the span's own name + URI path so it never
> depends on reimplementing `identify()`.
>
> Also dropped the planned query-string assertion: every server fn this suite
> drives is a POST, so **none** of the full capture's 2175 `/api/` URIs carries
> a `?`. Asserting it on real data would pin an absence; query stripping stays
> pinned on the hand-authored `coverage-sample.jsonl`.

- [x] **Step 4: File one issue per genuine gap**

For each uncovered fn, file via **`jaunder-issues`** (`--type Task`, milestone
_Test infrastructure & E2E_, blocked-by nothing) describing the missing browser
flow. Where a fn has server integration coverage but no browser flow, say so —
that is the allowlist reason the spec sanctions, not an excuse. Write each issue
URL into the allowlist entry.

**Filed** — both have thorough route-level coverage on **both** backends and a
real UI surface, so "server-tested, browser-untested" is the accurate reason
rather than an excuse:

- [#706](https://github.com/jaunder-org/jaunder/issues/706) `delete_media` —
  `/media`'s Delete button sits behind a native `confirm()` dialog, which
  Playwright auto-dismisses unless the spec registers a handler; that is the
  likely reason the flow was never written.
- [#707](https://github.com/jaunder-org/jaunder/issues/707) `revoke_session` —
  needs a second session in its own browser context.

Both `--type Task`, label `coverage`, milestone _Test infrastructure & E2E_,
added to Jaunder Backlog (#1). Each names removing its own allowlist entry as
the done-condition, which AC10 then enforces.

- [x] **Step 5: Write the allowlist, register the check, verify the gate is
      green**

Add `steps::server_fn_coverage_check::run(&mut result);` to
`xtask/src/lib.rs:296` (Check) and `:328` (Validate), beside
`server_fn_registrar_check::run`. Per the Task 8 scope correction the static
lane's **code** lands here too, plus `verify_after_combo` in the `Command::E2e`
arm.

Run: `devtool run -- cargo xtask check --no-test` Expected: PASS — **actual:
green,
`server-fn-coverage — 55 server fn(s) accounted for (53 covered, 2 allowlisted)`.**

- [x] **Step 6: Observe AC16 — the artifacts do not invalidate the e2e
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

**Actual: identical** —
`/nix/store/acwp4vnk5hxlnxwv77rqr1ghxp1kq5h1-vm-test-run-jaunder-e2e-sqlite-chromium.drv`
before and after `git add`. AC16 observed.

- [x] **Step 7: Commit**

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
