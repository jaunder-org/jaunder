# SMTP Configuration Type Home — Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `jaunder-iterate`
> (delegating individual tasks through `jaunder-dispatch` when useful). Steps
> use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the host-only `SmtpConfig` aggregate out of `storage`, complete
the direct-owner import cutover, and preserve all SMTP behavior.

**Architecture:** Add one focused `host::smtp_config` tenant containing only the
dependency-free aggregate over `common` SMTP value types. Keep persistence reads
and errors in `storage`; make `storage` and `server` import the aggregate and
TLS enum from their owning crates without compatibility re-exports.

**Tech Stack:** Rust 2024, Cargo workspace, SQLx, Lettre, rustdoc,
cargo-nextest, cargo-xtask.

**Spec:**
[`docs/superpowers/specs/2026-08-14-issue-257-smtp-config-types.md`](../specs/2026-08-14-issue-257-smtp-config-types.md)

## Review

**Scope in:** `SmtpConfig` type move; owner-direct imports; obsolete re-export
and duplicate TLS-test removal; host crate/architecture documentation; focused
regression tests and repository gate.

**Scope out:** SMTP web settings, wire DTOs, value-type redesign, storage-query
redesign, credential semantics, #673, #855's other modules, new ADRs.

**Tasks:**

1. Move the aggregate, complete the import/test cutover, update current
   architecture documentation, verify, and commit one behavior-neutral change.

**Key decisions and risks:**

- ADR-0058 makes `host`, not dual-target `common`, the shared floor for this
  server-only aggregate.
- The moved rustdoc must describe only host/common concepts; storage-specific
  assembly links remain beside the storage reader.
- Clean cutover: no `storage` alias for `SmtpConfig`, `SmtpTlsMode`, or
  `InvalidSmtpTlsMode`.
- Risk is compile-time surface drift, not intended runtime behavior; existing
  `common`, dual-backend `storage`, and `server::mailer` tests are the
  regression contract.

## Global Constraints

- Preserve every `SmtpConfig` field, field type, visibility, derive, behavioral
  guarantee, and runtime semantic.
- Preserve configuration keys, stored representations, SQL, defaults, mailer
  construction, TLS selection, credential handling, public errors, and web
  behavior.
- `host::smtp_config` may depend on `common` but no workspace crate above it
  (ADR-0058).
- Keep `SmtpConfigError`, `load_smtp_config`, SQLx classification, and their
  storage-specific tests in `storage`.
- Add no compatibility aliases, deprecated paths, new behavioral tests, ADR, or
  glossary term.
- Follow `CONTRIBUTING.md`: both-backend storage tests remain
  backend-parametric; invoke commands through `devtool run --`; no
  `Co-Authored-By` trailer.

## File Structure

- Create `host/src/smtp_config.rs`: the focused host-only SMTP relay
  configuration aggregate and layer-local rustdoc.
- Modify `host/src/lib.rs`: expose `smtp_config` and include it in the crate
  tenant map.
- Modify `storage/src/smtp.rs`: consume the host aggregate; retain storage
  errors/reader/tests; remove aggregate definition, TLS compatibility re-export,
  and redundant TLS parser/display tests.
- Modify `storage/src/site_config.rs`: import `host::smtp_config::SmtpConfig`;
  retain direct `common::smtp_tls_mode::SmtpTlsMode` ownership.
- Modify `server/src/mailer/smtp.rs`: import `SmtpConfig` and `SmtpTlsMode` from
  `host` and `common` respectively in production and tests.
- Modify `docs/ARCHITECTURE.md`: add SMTP relay configuration to the `host`
  crate's current responsibility list.
- Modify this plan in place: tick each completed step before the corresponding
  gate/commit.

---

### Task 1: Move the SMTP configuration aggregate and complete the clean cutover

**Files:**

- Create: `host/src/smtp_config.rs`
- Modify: `host/src/lib.rs`
- Modify: `storage/src/smtp.rs`
- Modify: `storage/src/site_config.rs`
- Modify: `server/src/mailer/smtp.rs`
- Modify: `docs/ARCHITECTURE.md`
- Modify: `docs/superpowers/plans/2026-08-14-issue-257-smtp-config-types.md`

**Interfaces:**

- Consumes: `common::smtp_host::SmtpHost`, `common::smtp_port::SmtpPort`,
  `common::smtp_tls_mode::SmtpTlsMode`, `common::smtp_username::SmtpUsername`,
  `common::smtp_password::SmtpPassword`, and `common::smtp_sender::SmtpSender`.
- Produces:

```rust
// host/src/smtp_config.rs
#[derive(Clone, Debug)]
pub struct SmtpConfig {
    pub host: SmtpHost,
    pub port: SmtpPort,
    pub tls_mode: SmtpTlsMode,
    pub username: Option<SmtpUsername>,
    pub password: Option<SmtpPassword>,
    pub sender: SmtpSender,
}
```

- Preserves these exact callable signatures, using owner-qualified types here to
  make the module cutover explicit (the source may import the short names):
  - `SiteConfigStorage::get_smtp_config(&self) -> sqlx::Result<Option<host::smtp_config::SmtpConfig>>`
    remains an async trait method.
  - `load_smtp_config(store: &dyn SiteConfigStorage) -> Result<Option<host::smtp_config::SmtpConfig>, SmtpConfigError>`
    remains a public async function.
  - `LettreMailSender::from_config(config: &host::smtp_config::SmtpConfig) -> Result<LettreMailSender, BuildMailerError>`
    remains a public associated function.

- [x] **Step 1: Run the focused baseline tests**

Run separately:

```bash
devtool run -- cargo nextest run -p common smtp_tls_mode
devtool run -- devtool pg run -- cargo nextest run -p storage smtp
devtool run -- cargo nextest run -p jaunder mailer::smtp
```

Expected: PASS. This records the existing TLS token/default contract,
both-backend SMTP storage reads/defaults/errors/redaction, and Lettre
construction/TLS/credential behavior before the type move.

- [x] **Step 2: Create the host-owned aggregate**

Create `host/src/smtp_config.rs` with the complete struct interface above and
the existing field documentation. Rewrite the type-level rustdoc to say that
every field is already validated by its own `common` value type and that the
aggregate configures the host's outbound SMTP relay. Do not mention or link
`SiteConfigStorage`, SQLx, `storage`, or web forms.

Add `pub mod smtp_config;` to `host/src/lib.rs` and extend the crate-level
tenant list to name SMTP relay configuration. Do not add dependencies: `host`
already depends on `common`.

- [x] **Step 3: Cut storage over to the host type**

In `storage/src/smtp.rs`, import `host::smtp_config::SmtpConfig`. Delete the
local `SmtpConfig` definition and the
`pub use common::smtp_tls_mode::{InvalidSmtpTlsMode, SmtpTlsMode};`
compatibility re-export. Keep `SmtpConfigError`, `load_smtp_config`, `classify`,
and all storage behavior tests. Remove the six now-unused top-level imports of
`SmtpHost`, `SmtpPassword`, `SmtpPort`, `SmtpSender`, `SmtpTlsMode`, and
`SmtpUsername`. Inside `mod tests`, import `SmtpHost`, `SmtpPort`, `SmtpSender`,
and `SmtpTlsMode` from their owning `common` modules for the remaining aggregate
fixtures and storage assertions. `SmtpPassword` and `SmtpUsername` remain
constructed through the existing `common::test_support` helpers and need no
named import.

Delete only these now-duplicate owner tests from `storage/src/smtp.rs`:

```text
tls_mode_parses_plain
tls_mode_parses_starttls
tls_mode_parses_tls
tls_mode_rejects_unknown_string
tls_mode_display_renders_expected_strings
```

Keep tests that use `SmtpTlsMode` while proving storage reads/defaults or
mailer-independent redaction.

In `storage/src/site_config.rs`, replace the `crate::smtp::SmtpConfig` import
with `host::smtp_config::SmtpConfig`. Keep its existing direct
`common::smtp_tls_mode::SmtpTlsMode` import and every query/default/error path
unchanged.

- [x] **Step 4: Cut the mailer over to owner-direct imports**

In `server/src/mailer/smtp.rs`, replace both production and test imports from
`storage::{SmtpConfig, SmtpTlsMode}` with:

```rust
use common::smtp_tls_mode::SmtpTlsMode;
use host::smtp_config::SmtpConfig;
```

Do not change mailer construction, match arms, fixtures, assertions, or error
messages.

Search the Rust workspace for `storage::SmtpConfig`, `storage::SmtpTlsMode`,
`storage::smtp::SmtpConfig`, `storage::smtp::SmtpTlsMode`, and
`InvalidSmtpTlsMode`. Expected: no obsolete path remains; `InvalidSmtpTlsMode`
appears only at its definition in `common::smtp_tls_mode`.

- [x] **Step 5: Update the architecture view**

In `docs/ARCHITECTURE.md`'s workspace crate table, add SMTP relay configuration
to the `host` responsibility list. Do not alter ADR-0058: this move applies its
existing decision and introduces no new architectural trade-off.

- [x] **Step 6: Run focused regression tests**

Run the same three commands separately:

```bash
devtool run -- cargo nextest run -p common smtp_tls_mode
devtool run -- devtool pg run -- cargo nextest run -p storage smtp
devtool run -- cargo nextest run -p jaunder mailer::smtp
```

Expected: PASS with the same observable assertions; the storage command
exercises both SQLite and PostgreSQL through the existing backend template.

- [x] **Step 7: Tick the implementation steps, run the per-commit gate, and
      inspect the complete diff**

Mark Steps 1–6 complete in this plan, then stage exactly the intended
issue-cycle tree before its gate:

```bash
git add host/src/smtp_config.rs host/src/lib.rs storage/src/smtp.rs storage/src/site_config.rs server/src/mailer/smtp.rs docs/ARCHITECTURE.md docs/superpowers/specs/2026-08-14-issue-257-smtp-config-types.md docs/superpowers/plans/2026-08-14-issue-257-smtp-config-types.md
devtool run -- cargo xtask check
```

Expected: PASS for formatting, static checks, clippy, instrumented
SQLite/PostgreSQL coverage, and repository structural gates. Inspect any
mechanical formatting fixes made by the check, then rerun the exact `git add`
command above so the staged tree is precisely the tree the gate checked. Inspect
it:

```bash
devtool run -- git status --short
devtool run -- git diff --cached
```

Expected: status names no intended unstaged or untracked path; the staged diff
contains only the approved specification, plan, type move, direct-import
cutover, duplicate-test deletion, and architecture update.

- [x] **Step 8: Commit the checked issue-cycle change**

The complete checked tree is already staged by Step 7. Reinspect the staged diff
if it changed, then commit without a pathspec:

```bash
git commit -m "refactor(host): own SMTP configuration aggregate (#257)"
```

Expected: one clean commit; the pre-commit hook repeats the cached gate
successfully; no `Co-Authored-By` trailer.
