# Plan — drop the `JAUNDER_PG_TEST_PORT` env knob from `devtool pg` (#275)

Spec:
[`docs/superpowers/specs/2026-07-16-issue-275-drop-pg-port-env-knob.md`](../specs/2026-07-16-issue-275-drop-pg-port-env-knob.md)

## Review header

**Goal.** Remove `devtool`'s only behavioural env parameter: delete the
`JAUNDER_PG_TEST_PORT` read, make the ephemeral-cluster port a private constant
`PORT = 54329`, and add **no** CLI flag (per the spec's Decision). Nothing sets
the var, so this is dead-knob removal, not a behaviour change.

**Scope.**

- _In:_ `tools/devtool/src/pg.rs` only — delete `resolve_port` + the env read,
  rename `DEFAULT_PORT` → `PORT`, regression-lock the constant in an existing
  test, drop the now-obsolete test.
- _Out:_ `main.rs` / `emit.rs` / `flake.nix` (already port-agnostic — no
  threading needed); no `--port` flag; no OS-assigned-port change (spec "Out of
  scope").

**Tasks (one line each).**

1. Remove the env knob from `pg.rs` and lock `PORT` via
   `urls_match_bash_parity`.

**Key risks / decisions.**

- No `--port` flag by design (spec Decision) — a conformance reviewer must
  confirm the flag was _intentionally_ omitted, not forgotten.
- The regression lock must thread the `PORT` **const** through
  `app_url`/`bootstrap_url` — a hardcoded `54329` on both sides would not lock
  anything (spec AC-5).
- `devtool` lives in the `tools/` workspace (not the main workspace) — its tests
  run via `--manifest-path tools/Cargo.toml`, and its `.rs` is not measured by
  the main coverage gate, so there are no coverage markers to manage here.

**For agentic workers.** Execute with `jaunder-iterate` (optionally delegating
the single task via `jaunder-dispatch`); tick the checkbox in real time. This is
one small, self-contained commit.

## Global constraints

- Follow `CONTRIBUTING.md`. Commit only after `cargo xtask check` is green
  (`jaunder-commit`). **No `Co-Authored-By` trailer.**
- The change is confined to one file; keep it a single commit.

---

## Task 1 — remove the `JAUNDER_PG_TEST_PORT` knob; hardcode `PORT = 54329`

**Files**

- `tools/devtool/src/pg.rs` (edit)

**Interfaces** — `with_ephemeral` / `run_command` / `PgCmd::Run` signatures are
**unchanged**; the port stops being environment-derived and becomes the module
constant. No caller changes.

**Edits**

1. Rename the constant (`pg.rs:17`):

   ```rust
   const PORT: u16 = 54329;
   ```

2. Delete `resolve_port` **and its doc comment** entirely (`pg.rs:28-36`):

   ```rust
   // DELETE:
   /// `JAUNDER_PG_TEST_PORT` with bash `${VAR:-54329}` semantics: unset OR empty ⇒ default.
   fn resolve_port(raw: Option<&str>) -> u16 { … }
   ```

3. Replace the env read in `with_ephemeral` (`pg.rs:161`) with the constant,
   leaving the rest of the function body untouched (it already threads a local
   `port` into `server_settings` / `bootstrap` / `app_url` / `bootstrap_url`):

   ```rust
   // was: let port = resolve_port(std::env::var("JAUNDER_PG_TEST_PORT").ok().as_deref());
   let port = PORT;
   ```

4. In `#[cfg(test)] mod tests`, **delete** the obsolete env test
   (`pg.rs:239-244`):

   ```rust
   // DELETE:
   #[test]
   fn port_defaults_when_unset_or_empty() {
       assert_eq!(resolve_port(None), 54329);
       assert_eq!(resolve_port(Some("")), 54329);
       assert_eq!(resolve_port(Some("55000")), 55000);
   }
   ```

5. Regression-lock `PORT` by threading the **const** through the existing parity
   test (`pg.rs:246-256`) — the call uses `PORT`, the expected string keeps the
   literal `54329`, so any change to `PORT` fails the assertion:

   ```rust
   #[test]
   fn urls_match_bash_parity() {
       assert_eq!(
           app_url(HOST, PORT),
           "postgres://jaunder@127.0.0.1:54329/jaunder"
       );
       assert_eq!(
           bootstrap_url(HOST, PORT),
           "postgres://postgres@127.0.0.1:54329/postgres"
       );
   }
   ```

   (The other pure-builder tests — `server_settings_disable_durability`,
   `psql_args_stop_on_error`, `initdb_args_trust_no_sync`,
   `bootstrap_sql_creates_role_and_db` — are unchanged.)

**Verify the unit tests**

```
cargo test --manifest-path tools/Cargo.toml -p devtool pg::
```

Expected: PASS. `urls_match_bash_parity` still green (now via `PORT`);
`port_defaults_when_unset_or_empty` no longer exists; no reference to
`resolve_port` remains → compiles clean (no dead-code/unused warning).

**Verify the acceptance criteria** (spec §Acceptance)

- **AC-1** — `rg JAUNDER_PG_TEST_PORT tools/` → **zero** hits.
- **AC-2** — `PgCmd::Run` still has only `cmd`; building and running
  `cargo run --manifest-path tools/Cargo.toml -p devtool -- pg run --help` shows
  **no** `--port`.
- **AC-3** — `resolve_port` gone; `PORT` is the sole port source, `= 54329`.
- **AC-4** (manual, needs the devShell's `initdb`/`pg_ctl`/`psql`) — a stray env
  value is ignored:
  ```
  JAUNDER_PG_TEST_PORT=55000 cargo run --manifest-path tools/Cargo.toml -p devtool \
    -- pg run -- bash -c 'echo "$JAUNDER_PG_TEST_URL"'
  ```
  Expected: output contains `:54329/` (env value has no effect).
- **AC-5** — the lock threads `PORT` (edit 5 above), not a second literal.
- **AC-6** — `cargo xtask check` green (the coverage path `emit::run` →
  `with_ephemeral` still boots Postgres on 54329 and runs the dual-backend
  tests).

**Commit** (after `cargo xtask check` is green — `jaunder-commit`):

```
refactor(devtool): drop JAUNDER_PG_TEST_PORT env knob; hardcode ephemeral port (#275)
```

- [ ] Task 1 complete
