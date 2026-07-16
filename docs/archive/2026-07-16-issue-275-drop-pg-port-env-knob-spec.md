# Spec — drop the `JAUNDER_PG_TEST_PORT` env knob from `devtool pg` (#275)

## Problem

`devtool pg run` and the in-process coverage path both derive the ephemeral
PostgreSQL cluster's TCP port from the **`JAUNDER_PG_TEST_PORT` environment
variable** — `resolve_port` at `tools/devtool/src/pg.rs`, with bash
`${VAR:-54329}` semantics (unset or empty ⇒ 54329) and an `.expect()` panic on
an unparseable value. #188's env-var audit flagged this as the lone place
`devtool` takes a **behavioural** parameter from the environment rather than the
CLI (`xtask` reads zero; `devtool`'s other env usage is app config or nix
store-path glue).

The variable has **no setter anywhere in the tree** — the only references are
the three lines inside `pg.rs` itself; the `scripts/with-ephemeral-postgres`
bash it mirrored was deleted in #29, and it is absent from `CONTRIBUTING.md`
(which documents the unrelated `JAUNDER_PG_TEST_URL`). The port is therefore
always 54329 in practice.

## Decision

**Remove the knob entirely rather than relocate it to a CLI flag.** The issue as
filed proposed `devtool pg run --port`. During design we concluded that since
nothing configures the port — and a throwaway loopback cluster needs no per-run
variation — the port is an internal **constant**, not a parameter. A `--port`
flag would be as unused as the env var it replaced; relocating a never-passed
env var to a never-passed flag is motion without value. Removing the parameter
altogether is the truest fix for "hidden unused parameter" and takes `devtool`'s
behavioural-env-var count to **zero** (the #188 goal). If a port collision ever
makes a knob genuinely necessary, adding `--port` later is a trivial,
non-breaking change.

The chosen fixed value (54329, not the PostgreSQL default 5432) is retained: its
purpose is to avoid colliding with a real Postgres a developer may run locally.

## Scope

Confined to **`tools/devtool/src/pg.rs`**. `main.rs` (`PgCmd::Run`), the
coverage path (`coverage::emit::run`), and `flake.nix` are unchanged — they
already call `with_ephemeral` with no port argument, so nothing downstream needs
threading.

## Acceptance criteria

1. **The env var is not read.** `rg JAUNDER_PG_TEST_PORT tools/` returns
   **zero** hits — no code references the variable. (Planning docs under
   `docs/superpowers/**` and this spec still name it until ship archives them to
   `docs/archive/**`; the criterion is about `tools/**` code, not prose.)
2. **No CLI flag is added.** `devtool pg run --help` lists no `--port` option
   and `PgCmd::Run` retains exactly its current fields (`cmd` only).
3. **The port is a private constant equal to 54329.** `with_ephemeral` boots the
   cluster on 54329 with no environment input; `resolve_port` is deleted. The
   constant is renamed from `DEFAULT_PORT` to `PORT` (the "default" name implied
   an override that no longer exists).
4. **A stray env value has no effect.**
   `JAUNDER_PG_TEST_PORT=55000 devtool pg run -- bash -c 'echo "$JAUNDER_PG_TEST_URL"'`
   prints a URL containing `:54329/` (the env value is ignored). _Verified
   manually / via the driver loop — not a committed test, since asserting it
   would mutate process-global env and race parallel tests._
5. **The constant is regression-locked.** The `urls_match_bash_parity` test is
   updated to thread the **`PORT` const** through `app_url`/`bootstrap_url`
   (replacing the hardcoded `54329` in the _call_, keeping the concrete
   `...:54329/jaunder` string in the _expected_ value) — so a change to `PORT`
   fails the assertion. A hardcoded `54329` on both sides would NOT satisfy this
   criterion. The obsolete `port_defaults_when_unset_or_empty` test is removed;
   the remaining pure-builder tests (`server_settings_disable_durability`,
   `psql_args_stop_on_error`, `initdb_args_trust_no_sync`,
   `bootstrap_sql_creates_role_and_db`) still pass unchanged.
6. **The gate stays green.** `cargo xtask check` passes: the coverage path
   (`emit::run` → `with_ephemeral`) still provisions Postgres on 54329 and the
   dual-backend tests run against it.

## Out of scope

- **Ephemeral OS-assigned port** (bind `:0`, read the assigned port, hand it to
  `pg_ctl`) — a larger structural change that would make collisions impossible;
  file separately if collision-avoidance ever becomes a real need.
- Any change to `JAUNDER_PG_TEST_URL` / `JAUNDER_PG_BOOTSTRAP_TEST_URL` (the
  cluster → wrapped-command handoff) — unrelated.

## No ADR

This reverses one #29 parity value (the env override). It is justified locally
(the bash script it mirrored is gone; the var has no consumer) and is not an
architectural decision future readers would need the rationale for beyond this
spec. No ADR.
