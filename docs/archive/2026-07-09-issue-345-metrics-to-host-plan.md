# Plan — #345: move `common::metrics` → `host::metrics`, delete the `metrics` feature

**Spec:** `docs/superpowers/specs/2026-07-09-issue-345-metrics-to-host.md` (the
what/why). This is the how. **For agentic workers:** drive with
`jaunder-iterate` (delegate a task to `jaunder-dispatch` if useful); tick
checkboxes in real time.

## Review header

**Goal.** Relocate the OTel metrics facade to the native-only `host` crate and
delete `common`'s `metrics` feature, so `opentelemetry` is kept out of the wasm
bundle by crate structure (ADR-0058) rather than a feature flag (ADR-0011,
amended). Pure relocation — **no behavior change**.

**Scope — in:** move `metrics.rs`; Cargo surgery in
`common`/`host`/`server`/`web`; mechanical repoint of every `common::metrics::*`
call site; amend ADR-0011 + module doc. **Scope — out:** any change to emitted
metrics/enums/semantics; exporter setup (`server::observability`); a new ADR
(this applies ADR-0058, not a new decision).

**Tasks:**

1. Relocate the facade + Cargo surgery + repoint all call sites — one atomic,
   gate-green commit (a partial move does not compile).
2. Amend ADR-0011 (facade home → `host::metrics`, cross-ref ADR-0058) — docs
   commit.

**Key risks / decisions.**

- **Atomicity is forced.** The move + all repoints + Cargo edits must land in
  one commit; the pre-commit gate compiles the whole tree and a half-moved state
  won't build. Task 1 is large but purely mechanical (a path-prefix rename + one
  file move + four Cargo edits).
- **Call-site set is bigger than the spec's abbreviation.** Authoritative
  inventory is in Task 1 (server carries the bulk). `rg -n 'common::metrics'`
  must return **zero** hits at the end.
- **wasm hygiene (spec P3).** All `web` call sites are already SSR-only (they
  used `common/metrics`, enabled only under web's `server` feature). Repointing
  to `host::metrics` — also present only under `server` — must not introduce a
  `host::metrics` reference reachable by the wasm build. Verify with
  wasm-clippy.
- **Dev-deps travel with the test.** `metrics.rs`'s `#[tokio::test]` needs
  `tokio`
  - `opentelemetry_sdk` as `host` dev-deps (they leave `common`).

## Global constraints

- Keep the tree **gate-green at every commit**; the pre-commit hook runs
  `cargo xtask check`. Serialize edit → gate → commit (no edits mid-gate).
- No `Co-Authored-By` trailer. Follow `CONTRIBUTING.md`. No new storage tests,
  so the dual-backend template does not apply here.

---

## Task 1 — Relocate the facade, repoint all call sites, Cargo surgery

**One atomic commit.** Do all of the following, then gate, then commit.

### 1a. Move the module file

- `git mv common/src/metrics.rs host/src/metrics.rs`.
- In `host/src/metrics.rs`, rewrite the module doc (currently lines 1–5, "shared
  by `web`, `server`, and the CLI … See ADR-0011") to state the new home and
  rationale: native-only facade living in `host` per ADR-0058; instruments are
  no-ops without a `MeterProvider`; exporter setup stays in
  `server::observability`; refs ADR-0011 (amended) + ADR-0058. Body is otherwise
  unchanged.
- `host/src/lib.rs`: add `pub mod metrics;` (unconditional — **no**
  `#[cfg(feature)]`) alongside `auth`, `capture`, `error`. Add a one-line tenant
  note to the crate doc mirroring the existing `[capture]`/`[error]`/`[auth]`
  sentences (metrics tenant, issue #345, ADR-0011/0058).

### 1b. `common/Cargo.toml` — delete the feature and its deps

- Remove line 24: `opentelemetry = { workspace = true, optional = true }`
  (`[dependencies]`).
- Remove line 32: `metrics = ["dep:opentelemetry"]` (`[features]`).
- Remove the two dev-deps (lines 38–39): `opentelemetry = { workspace = true }`
  and
  `opentelemetry_sdk = { workspace = true, features = ["metrics", "testing"] }`.
- `common/src/lib.rs`: delete lines 7–8 (`#[cfg(feature = "metrics")]` +
  `pub mod metrics;`).

### 1c. `host/Cargo.toml` — take the deps

- `[dependencies]` line 12:
  `common = { path = "../common", features = ["metrics"] }` →
  `common = { path = "../common" }`.
- `[dependencies]`: add `opentelemetry.workspace = true`.
- `[dev-dependencies]` (currently `tempfile`, `tracing-subscriber`): add
  `tokio = { workspace = true }` and
  `opentelemetry_sdk = { workspace = true, features = ["metrics", "testing"] }`
  (mirrors what `common` carried, satisfying the `#[tokio::test]` +
  `InMemoryMetricExporter`).

### 1d. `server/Cargo.toml` and `web/Cargo.toml`

- `server/Cargo.toml` line 15:
  `common = { workspace = true, features = ["metrics"] }` →
  `common = { workspace = true }` (server already deps `host`, line 16).
- `web/Cargo.toml` line 70: delete `"common/metrics",` from the `server` feature
  list (web already lists `"dep:host"` on line 69).

### 1e. Repoint every call site (`common::metrics` → `host::metrics`)

Mechanical prefix rename in each file below. **In `host` itself, use
`crate::metrics`** (not `host::metrics`).

- **host:** `host/src/error.rs:330` → `crate::metrics::error(...)`.
- **storage** (deps `host` directly): `storage/src/sessions.rs` (45, 47–49,
  228), `storage/src/users.rs` (101, 103–104, 674) → `host::metrics::*`.
- **web** (SSR-only bodies): `web/src/posts/mod.rs` (298, 468, 612, 650),
  `web/src/auth/mod.rs` (89–91, 109, 118–124, 165, 169),
  `web/src/auth/server.rs` (71, 80), `web/src/email/mod.rs` (49, 50),
  `web/src/invites/mod.rs` (42), `web/src/password_reset/mod.rs` (62, 63,
  66, 83) → `host::metrics::*`.
- **server:** `server/src/atompub/mod.rs` (63, 90, 92, 94, 96, 320),
  `server/src/media_manager.rs` (120, 132, 134–137, 275–279, 381),
  `server/src/media.rs` (112, 120, 123, 125, 127, 324), `server/src/commands.rs`
  (136–139, 206), `server/src/feed/handlers.rs` (35, 39),
  `server/src/feed/worker.rs` (165, 166, 212, 220, 228, 239, 254),
  `server/src/backup.rs` (65, 66, 101, 102, 109, 111, 113) → `host::metrics::*`.

> Note: the local `let metrics = …` bindings in `host/src/metrics.rs:256`
> (moved) and `server/src/observability.rs:1081` are variables, **not** the
> module path — leave them.

### Verify (before commit)

- `rg -n 'common::metrics|common/metrics|feature = "metrics"|features = \["metrics"\]'`
  across `common host storage server web` → **zero** hits.
- `cargo nextest run -p host metrics` → the moved
  `login_records_outcome_attribute` test **PASSES** in its new home.
- `cargo xtask validate --no-e2e` (via `devtool run`) → green: static + clippy +
  **wasm-clippy** (proves no `opentelemetry`/`host::metrics` in the wasm build,
  spec P3) + coverage (metrics.rs stays host-compiled + measured).

### Commit

`jaunder-commit` (runs `cargo xtask check` clean first). Message e.g.
`refactor(#345): move metrics facade to host, delete common metrics feature`.

---

## Task 2 — Amend ADR-0011; module-doc cross-links

**Docs-only commit.**

- `docs/adr/0011-unified-observability.md`: amend the "facade home" decision —
  the facade now lives at **`host::metrics`** (unconditional), not
  `common::metrics` behind a `metrics` feature. State the reason: `host` is
  native-only (ADR-0058), so `opentelemetry` is excluded from wasm structurally;
  add a dated clarification note (as ADR-0058 does) referencing #345 and
  ADR-0058. Do **not** flip status; the observability decision stands, only the
  facade's home moves.
- Ensure no other **live** doc/`//!` still points readers at `common::metrics`
  (grep source doc-comments); fix any stragglers. **Do not touch
  `docs/archive/*`** — those are frozen historical snapshots and legitimately
  name the old path.
- `prettier -w` the edited Markdown before staging (avoids the pre-commit
  restage double-commit).

### Verify

- `rg -n 'common::metrics' docs/adr/ ':!docs/archive'` → zero (ADRs must point
  at the new path; archive and spec/plan may quote the old path historically).
- `cargo xtask check` green (unchanged from Task 1; docs-only).

### Commit

`docs(#345): point ADR-0011 facade home at host::metrics (ADR-0058)`.

---

## Self-review

- **Compiles at every commit?** Task 1 is atomic (move + all repoints + Cargo);
  Task 2 is docs-only. ✓
- **No placeholders?** Every file + line + Cargo edit enumerated above. ✓
- **Spec acceptance covered?** end-state 1–2 (Tasks 1a–1c), 3 (1e), 4 (1d), 5
  (storage repoint 1e), 6 (Task 2 + 1a doc), 7 (Task 1 verify). ✓
