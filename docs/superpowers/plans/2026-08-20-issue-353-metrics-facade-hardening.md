# Metrics Facade Hardening Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks to a subagent via `jaunder-dispatch` when
> useful). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Harden `host::metrics` by pinning the login outcome metric attribute
and documenting the cached-meter initialization contract.

**Architecture:** Extend the existing single in-memory exporter test in
`host/src/metrics.rs`; do not add another process-global provider install.
Update only the module-level docs for the `LazyLock`/`MeterProvider` ordering
contract. No runtime metric behavior changes.

**Tech Stack:** Rust, OpenTelemetry 0.30,
`opentelemetry_sdk::metrics::InMemoryMetricExporter`, `cargo nextest`,
`cargo xtask check`.

## Review Header

**Scope in:** `host/src/metrics.rs` docs and
`host::metrics::tests::every_emitter_exports_its_instrument`.  
**Scope out:** exporter setup, metric names, enum vocabularies, dashboards,
async observable gauges, CLI/test-support OTLP capture.

**Tasks:**

1. Document the `MeterProvider`-before-first-emit contract in `host::metrics`.
2. Add the login `outcome=invalid_credentials` assertion to the existing
   metric-export test.
3. Run the focused test, gate with `cargo xtask check`, tick plan boxes, and
   commit.

**Key risks/decisions:**

- `global::set_meter_provider` is process-global; keep one exporter/provider
  test.
- `host::metrics` intentionally no-ops without a provider; document ordering
  without implying every process must export.
- Current `email_send_result` assertions are real coverage and stay untouched.

## Global Constraints

- Follow the approved spec:
  `docs/superpowers/specs/2026-08-20-issue-353-metrics-facade-hardening.md`.
- Do not change metric names, attribute keys, attribute values, enum variants,
  emitter signatures, or exporter setup.
- Keep exporter setup owned by `server::observability` per ADR-0011 and #345.
- Keep `host::metrics` native-only by crate structure; do not add a Cargo
  feature gate.
- Use `devtool run -- <cmd>` for all run-and-inspect commands. No `npx`,
  package-manager wrappers, shell pipelines, or `Co-Authored-By` trailers.

---

### Task 1: Document Cached Meter Initialization Contract

**Files:**

- Modify: `host/src/metrics.rs:1-13`

**Interfaces:**

- Consumes: Existing module doc comment and `static M: LazyLock<Instruments>`.
- Produces: Module docs that state the process ordering invariant.

- [x] **Step 1: Update the module docs**

Edit the opening `//!` block in `host/src/metrics.rs` so it explicitly states:

```rust
//! Instruments are cached in a [`LazyLock`], so the first metric emission in a
//! process fixes which global `MeterProvider` backs the facade's instruments.
//! Binaries that export metrics must install their provider before any emitter is
//! called; processes without metrics setup intentionally keep the no-op provider.
```

Keep the existing cardinality/no-caller-text and ADR-0011/ADR-0058 context. Do
not change imports or code in this step unless rustdoc requires a path
adjustment.

- [x] **Step 2: Inspect docs locally**

Run: `devtool run -- cargo xtask check --no-test`  
Expected: PASS for static checks. If rustdoc/linking/clippy reports wording or
link issues, fix only the doc comment and rerun this command.

Do not commit yet; Task 2 should land with the docs as one small hardening
commit.

### Task 2: Assert Login Outcome Attribute

**Files:**

- Modify: `host/src/metrics.rs:323-415`
- Test: `host/src/metrics.rs` in-module test
  `host::metrics::tests::every_emitter_exports_its_instrument`

**Interfaces:**

- Consumes: Existing helper
  `counter_attributes(metrics: &[ResourceMetrics], name: &str) -> Vec<BTreeSet<(String, String)>>`.
- Produces: A login metric assertion that fails if the `outcome` key or
  `InvalidCredentials` mapping changes.

- [x] **Step 1: Add the failing assertion**

Add a one-pair helper next to the existing `attrs`/`attrs4` helpers:

```rust
    fn attrs1(pairs: [(&str, &str); 1]) -> BTreeSet<(String, String)> {
        pairs
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect()
    }
```

Then, after the existing `email_send_result` assertions and before
`let errors = ...`, add:

```rust
        let logins = counter_attributes(&metrics, "jaunder.auth.logins");
        assert!(
            logins.contains(&attrs1([("outcome", "invalid_credentials")])),
            "login did not record outcome=invalid_credentials; got {logins:?}"
        );
```

This is intentionally a one-attribute counter; do not force it through the
existing two-pair `attrs` helper.

- [x] **Step 2: Prove the assertion is live**

Temporarily change the expected string in the new assertion from
`"invalid_credentials"` to `"invalid_credentials_MUTANT"`.

Run:
`devtool run -- cargo nextest run -p host metrics::tests::every_emitter_exports_its_instrument`  
Expected:
FAIL, with the new assertion message showing
`login did not record outcome=invalid_credentials` or the mutant expected value
mismatch.

Restore the expected string to `"invalid_credentials"` immediately after
observing the failure.

- [x] **Step 3: Run the focused test green**

Run:
`devtool run -- cargo nextest run -p host metrics::tests::every_emitter_exports_its_instrument`  
Expected:
PASS.

- [ ] **Step 4: Gate and commit**

Run: `devtool run -- cargo xtask check`  
Expected: PASS.

Stage exactly:

```bash
git add host/src/metrics.rs docs/superpowers/plans/2026-08-20-issue-353-metrics-facade-hardening.md
```

Commit exactly:

```bash
git commit -m "test(host): harden metrics facade assertions (#353)"
```

No `Co-Authored-By` trailer.

### Task 3: Record Plan Completion and Validate Exact HEAD

**Files:**

- Modify:
  `docs/superpowers/plans/2026-08-20-issue-353-metrics-facade-hardening.md`

**Interfaces:**

- Consumes: Task 2 committed source/doc hardening.
- Produces: A fully checked-off plan commit, then validation evidence for that
  exact final HEAD.

- [ ] **Step 1: Confirm Task 2 left only plan checkbox edits**

Run: `devtool run -- git status --short`  
Expected: either no output, or only
`M docs/superpowers/plans/2026-08-20-issue-353-metrics-facade-hardening.md`.

If any source file is dirty, stop and resolve it before validation; do not hide
source changes in the plan-completion commit.

- [ ] **Step 2: Tick every remaining checkbox**

Edit this plan so every Task 1, Task 2, and Task 3 checkbox is checked
(`- [x]`). This is bookkeeping only; do not change the approved task content.

- [ ] **Step 3: Gate and commit the final plan state**

Run: `devtool run -- cargo xtask check`  
Expected: PASS.

Stage exactly:

```bash
devtool run -- git add docs/superpowers/plans/2026-08-20-issue-353-metrics-facade-hardening.md
```

Commit exactly:

```bash
devtool run -- git commit -m "docs: record issue 353 plan completion"
```

No `Co-Authored-By` trailer.

- [ ] **Step 4: Validate exact final HEAD**

Run: `devtool run -- cargo xtask validate --no-e2e`  
Expected: PASS.

- [ ] **Step 5: Confirm clean final worktree**

Run: `devtool run -- git status --short`  
Expected: no output.
