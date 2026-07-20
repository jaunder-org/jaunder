# Plan — issue #438: transparent sqlx bridge for string newtypes

Spec:
[`2026-07-19-issue-438-sqlx-str-newtype-bridge.md`](../specs/2026-07-19-issue-438-sqlx-str-newtype-bridge.md)
Issue: jaunder-org/jaunder#438

## Review header

**Goal.** Make every derive-based string newtype a first-class sqlx column type
via a **default-on, feature-gated** bridge emitted by the `StrNewtype` derive;
convert all storage bind/decode sites; retire the hand re-parse / `cov:ignore`
debt; and install an xtask gate so the stringly-bind idiom can't return.

**Scope.**

- _In:_ the derive mechanism (AC-1); Decode-validates (AC-2); two annotations
  (`InviteCode` `secret,sqlx`; `RawToken` `no_sqlx`) + the full bind-site sweep
  (AC-3); re-parse/`cov:ignore` removal (AC-4); the enforcement gate (AC-5);
  wasm-sqlx-free proof (AC-6); a new ADR (AC-7).
- _Out:_ `RenderedHtml` (hand-rolled → #502, filed as task 1);
  `Proffered*`/`Password` (secret, bridge-less by default); numeric newtypes;
  trait-sig/validation changes.

**Tasks.**

- [x] 1. File follow-ups: note the `RenderedHtml` sqlx bridge on #502 (blocked
     on its trailer alignment).
- [x] 2. Mechanism: optional `sqlx` feature on `common`; extend the `StrNewtype`
     derive (`Opts`, `parse_opts` guards, three emission branches, two `Decode`
     helpers) with default-on/`secret`-off/`secret,sqlx`/`no_sqlx`; macro unit
     tests.
- [x] 3. Wire the feature: `storage` enables `common/sqlx`; `host` `sqlx`
     feature covers `InviteCode`; annotate `InviteCode` (`secret, sqlx`) +
     `RawToken` (`no_sqlx`).
- [x] 4. Convert invites (`InviteCode`) + retire the `build_invite_record`
     re-parse and the three `invites.rs` `cov:ignore` (AC-4).
- [x] 5. Convert users/profile (`Username`, `Email`, `DisplayName`).
- [x] 6. Convert sessions + tokens (`TokenHash` — the bulk:
     sessions/email/password reset, both backends).
- [x] 7. Convert posts + tags (`Slug`, `Tag`/`TagLabel`, `PostBody`,
     `PostTitle`, `ContentHash` where in posts).
- [x] 8. Convert audiences + media + feeds (`AudienceName`, `ContentHash`,
     `Filename`, `FeedPath`, `BackupSchedule`).
- [x] 9. Enforcement gate: new xtask step + allowlist + bite test; register it.
- [x] 10. ADR draft (AC-7) + wasm-sqlx-free check (AC-6).

**Key decisions / risks.**

- **Default-on mirrors the serde bridge** (ADR-0063): the derive emits the
  bridge for every type, `secret` drops it, `secret,sqlx` re-adds, `no_sqlx`
  opts a non-secret type out. Only annotations needed: `InviteCode`, `RawToken`
  (task 3).
- **`macros` is coverage-measured** ([memory]): the new derive branches and
  `parse_opts` guards need in-crate `syn::parse_quote!` unit tests for the
  emitted impls and each error arm; a `?`-fall-through brace may need
  `// cov:ignore`.
- **Decode validation is the integrity guard** — route through `FromStr`
  (validating) or `From<String>` (infallible); the error arm is covered by a
  bad-column `#[apply(backends)]` test, so **no new `cov:ignore`** (AC-2). This
  is the point.
- **Backend parity**: every bind/decode change lands in `sqlite/` +
  `postgres/` + shared together; dual-backend `#[apply(backends)]` tests
  (ADR-0053 TempDir hazard: bind the whole `TestEnv`).
- **`Encode` by value vs ref**: `encode_by_ref` borrows `self`; `.bind(x)` moves
  an owned newtype. For fields behind `&input`, bind `.clone()` or `&x` (test
  both owned and `&Newtype` bind, both backends) — confirm during task 2.
- **Shared vendor rebuild**: adding `sqlx` as a `common` dep may invalidate the
  shared vendor → cold rebuild ([memory]); version-match the existing workspace
  `sqlx`.
- Tasks 4–8 are independent per-domain sweeps; the gate (task 9) lands **after**
  the sweeps so it's green (allowlist holds only `rendered_html`).

**For agentic workers.** Execute with **jaunder-iterate**; the per-domain sweeps
(tasks 5–8) are good **jaunder-dispatch** candidates. One commit per task; no
`Co-Authored-By` trailer.

## Global constraints

- Worktree: `.claude/worktrees/issue-438-sqlx-str-newtype-bridge` (branch
  `worktree-issue-438-sqlx-str-newtype-bridge`).
- Per-task: `cargo xtask check` clean before commit (pre-commit runs the full
  gate); see **jaunder-commit**. Follow `CONTRIBUTING.md` (backend parity,
  coverage, dialect files). `--all-features --all-targets` after
  feature-threading changes ([memory: default check skips server-gated code]).
- Ship gate: `cargo xtask validate` (with e2e — touches live create/read paths).

---

## Task 1 — File the RenderedHtml follow-up

**Do:** Comment on **#502** (or add a checklist item) recording that once
`RenderedHtml` gains the standard trailer, it should also take the default sqlx
bridge (it's the last hand-rolled string newtype; its storage binds stay
allowlisted by the task-9 gate meanwhile). No code.

**Verify:** the note exists on #502. Via **jaunder-issues**.

---

## Task 2 — Mechanism: derive + `common` sqlx feature

**Files:** `common/Cargo.toml`, `macros/src/str_newtype.rs` (+ its unit tests).

**Do:**

- `common/Cargo.toml`: add `sqlx = { workspace = true, optional = true }`
  (version- matched to the workspace) and a `sqlx = ["dep:sqlx"]` feature. No
  default.
- `macros/src/str_newtype.rs`:
  - `Opts`: add `sqlx: bool`, `no_sqlx: bool`.
  - `parse_opts`: two new `parse_nested_meta` arms; guards mirroring the
    existing `serde && !secret` guard (str_newtype.rs:314): reject
    `sqlx && !secret` (bare `sqlx` only meaningful on a secret),
    `no_sqlx && secret`, `no_sqlx && sqlx`.
  - Emission — weave `#[cfg(feature = "sqlx")]` sqlx tokens into **three**
    branches: the `secret` branch (emit iff `opts.sqlx`), the `infallible`
    early-return (emit by default — infallible types are stored), and the
    default branch (emit unless `opts.no_sqlx`).
  - Two `Decode` helpers mirroring `serde_impls`/`serde_impls_infallible`
    (str_newtype.rs:119/223): `sqlx_impls` (validating → `FromStr`) and
    `sqlx_impls_infallible` (→ `From<String>`). Both emit generic
    `Type`/`Encode`/`Decode<DB: Database>` delegating to `str`/`&str`.

**Tests (macros — coverage-measured):** `syn::parse_quote!` unit tests
asserting: each guard's error arm; that the emitted token stream contains the
three impls for a plain type, for `secret, sqlx`, and is absent for `secret` /
`no_sqlx`. (`?`-fall- through brace → `// cov:ignore` per [memory].)

**Run:** `cargo nextest run --manifest-path macros/Cargo.toml` (xtask-excluded
crate convention). Then `cargo check -p common --features sqlx` compiles the
emitted impls.

**Commit:** `macros: str_newtype sqlx bridge, default-on except secret (#438)`.

---

## Task 3 — Wire the feature + annotate the two exception types

**Files:** `storage/Cargo.toml`, `host/Cargo.toml`, `host/src/invite.rs`,
`common/src/token.rs`.

**Do:**

- `storage/Cargo.toml`: enable `common/sqlx` (via
  `common = { …, features = ["sqlx"] }`).
- `host/Cargo.toml`: ensure the on-by-default `sqlx` feature reaches
  `common/sqlx` so `InviteCode` (host) gets the bridge.
- `InviteCode` (`host/src/invite.rs`): `#[str_newtype(secret, sqlx)]`.
- `RawToken` (`common/src/token.rs`): `#[str_newtype(no_sqlx)]`.

**Verify:** `cargo check -p storage --all-features` compiles (bridge impls in
scope); `InviteCode`/`TokenHash` now have `sqlx::Type`; `RawToken` does **not**
(a throwaway `.bind(raw_token)` fails to compile — confirm, then remove). No
bind-site change yet.

**Run:** `cargo check -p storage --all-features --all-targets`.

**Commit:**
`storage/host: enable common/sqlx; annotate InviteCode + RawToken (#438)`.

---

## Task 4 — Convert invites + retire the re-parse / cov:ignore (AC-4)

**Files:** `storage/src/invites.rs`, `storage/src/helpers.rs`
(`build_invite_record`), `storage/src/{sqlite,postgres}/mod.rs` (invite binds).

**Do:** Replace `.bind(code.as_ref())` → `.bind(&code)` / `.bind(code)` (per
ownership); change `query_as`/row mapping to decode `InviteCode` directly;
delete the hand re-parse in `build_invite_record` and the `cov:ignore-start`
block (`invites.rs:103-107`) + inline `cov:ignore` (`invites.rs:164`).
`create_invite`'s `generate_token()` path becomes typed end-to-end.

**Tests:** `#[apply(backends)]` — round-trip an `InviteCode` (bind + decode); a
**decode-error** test binding a bad raw string into the invite-code column and
asserting a decode error (covers the Decode error arm, no `cov:ignore`).

**Run:** `cargo nextest run -p storage invite` (both backends PASS).

**Commit:**
`storage: bind/decode InviteCode via sqlx bridge; drop re-parse (#438)`.

---

## Task 5 — Convert users / profile

**Files:** `storage/src/users.rs`, `storage/src/{sqlite,postgres}/mod.rs`.
**Types:** `Username`, `Email`, `DisplayName` (incl. the `Option`
`&**`/`.map(|d| &**d)` forms at `users.rs:264,429`). **Do/Tests/Run/Commit:**
convert binds + decodes; `#[apply(backends)]` round-trip + decode-error for
each; `cargo nextest run -p storage user`;
`storage: bind/decode Username/Email/DisplayName via sqlx bridge (#438)`.

---

## Task 6 — Convert sessions + tokens (the bulk)

**Files:** `storage/src/sessions.rs`, `email.rs`, `password.rs`,
`storage/src/{sqlite,postgres}/{mod,sessions}.rs`. **Type:** `TokenHash` (~57
sites — session, email-verification, password-reset token columns).
**Do/Tests/Run/Commit:** convert all `token_hash.as_ref()` binds + decodes;
`#[apply(backends)]` round-trip + decode-error (one representative per table);
`cargo nextest run -p storage 'session|email|password'`;
`storage: bind/decode TokenHash via sqlx bridge (#438)`.

---

## Task 7 — Convert posts + tags

**Files:** `storage/src/posts.rs`, `storage/src/{sqlite,postgres}/posts.rs`.
**Types:** `Slug`, `Tag`/`TagLabel` (confirm which are stored), `PostBody`,
`PostTitle` (infallible-decode). Includes `&*input.body`,
`input.title.as_deref()` (Option), the cursor `tag_slug`/`tag` binds.
**Do/Tests/Run/Commit:** convert binds + decodes; `#[apply(backends)]`
round-trip + decode-error (validating types) / round-trip (infallible);
`cargo nextest run -p storage post`;
`storage: bind/decode post/tag string newtypes via sqlx bridge (#438)`.

---

## Task 8 — Convert audiences + media + feeds

**Files:** `storage/src/audiences.rs`, `media.rs`, `feed_cache.rs`,
`feed_events.rs`, `storage/src/{sqlite,postgres}/media.rs`. **Types:**
`AudienceName`, `ContentHash` (`sha256`), `Filename`, `FeedPath`,
`BackupSchedule` (confirm validating vs infallible → `site_config`).
**Do/Tests/Run/Commit:** convert binds + decodes; `#[apply(backends)]`
round-trip + decode-error; `cargo nextest run -p storage 'audience|media|feed'`;
`storage: bind/decode audience/media/feed string newtypes via sqlx bridge (#438)`.

---

## Task 9 — Enforcement gate (AC-5)

**Files:** `xtask/src/steps/sqlx_newtype_bind_check.rs` (new, modelled on
`proffered_secret_check.rs`), `xtask/src/lib.rs` (register at **both** step-list
call sites — `check` ~L294 and `validate` ~L324, exactly where
`proffered_secret_check::run` appears twice, or it won't gate on `validate`),
`docs/CONTRIBUTING.md`/gate docs if the check list is documented.

**Do:** Source-scan `storage/src` `.rs`; **fail** on `.bind(<expr>.as_ref())`,
`.bind(&*<expr>)`, `.bind(&**<expr>)`, and the `Option` map-deref
`.bind(<opt>.map(|x| &*x))` / `&**x` forms. Do **not** police `.as_str()`. Carry
an explicit, annotated **allowlist** (only `rendered_html` binds, tagged `#502`,
plus any genuine non-newtype `.as_ref()` the sweep surfaced). Emit a
`StepResult` like the model gate.

**Tests (xtask, coverage per [memory]):** unit tests — the gate **passes** on
the converted tree and **fails** on a fixture line reintroducing
`.bind(x.as_ref())` on a newtype (proves it bites); allowlist entry honored.

**Run:** `cargo nextest run --manifest-path xtask/Cargo.toml sqlx_newtype`; then
`devtool run -- cargo xtask check` shows the new `[ ok ] sqlx-newtype-bind`
step.

**Commit:** `xtask: gate against stringly newtype binds in storage (#438)`.

---

## Task 10 — ADR + wasm-sqlx-free proof

**Files:** `docs/adr/drafts/<slug>.md` (numberless — **jaunder-adr**), a
wasm-feature assertion.

**Do:**

- Draft the ADR (sqlx analogue of ADR-0063): default-on/`secret`-off/`no_sqlx`
  shape
  - why (`Encode` = storability), Decode-validates, `common`-sqlx-feature + wasm
    isolation, the gate. Numbered at ship by `cargo xtask adr promote`.
- AC-6: confirm the CSR/wasm build graph has no `sqlx` — the existing
  `wasm-clippy` gate compiles `common` for wasm32 with the feature off; add an
  explicit assertion (e.g. a
  `#[cfg(all(target_arch = "wasm32", feature = "sqlx"))] compile_error!` guard
  in `common`, or a doc note) that `common/sqlx` is never enabled for wasm.

**Run:** `devtool run -- cargo xtask validate` (full, with e2e) green.

**Commit:** `docs: ADR for the sqlx string-newtype bridge (#438)` (+ the wasm
guard).
