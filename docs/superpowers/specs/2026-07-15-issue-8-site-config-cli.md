# Spec — `jaunder site-config` CLI (set/get/list) + retire test-support site-config seeding

Issue: #8 (`cli: add a jaunder command for setting site_config values`), milestone
_Developer tooling & DX_.

## Problem & current reality

`site_config` is a free-form key/value store (`site_config(key TEXT PRIMARY KEY, value
TEXT NOT NULL)`), exposed as `Arc<dyn SiteConfigStorage>` on `AppState`. The **shipped
`jaunder` binary has no way to read or write it** — the only generic setter lives in the
never-shipped `test-support` binary (`test-support set-site-config`).

Issue #8 as filed names three seeding files (`end2end/run-e2e.sh`, `flake.nix` psql VM
seed, `scripts/seed-e2e-fixtures.sh`) that **no longer exist** — #249 already unified
them into `devtool seed-e2e`, which shells each fixture step to `test-support`. So the
issue's original "de-duplicate three SQL copies" goal is already done. The **live**
value of #8 is:

1. **Ops + parity:** put `site-config set/get/list` in the production `jaunder` binary
   (the issue's literal ask, and "also useful for ops").
2. **Testing-infra conversion** (the second half of this task's directive): move the e2e
   seed's `site_config` writes off the never-shipped `test-support` binary onto the real
   `jaunder site-config set`, and retire the now-dead `test-support set-site-config`.

## Scope

**In scope**

- New `jaunder site-config` nested subcommand group with `set`, `get`, `list` leaves.
- New `SiteConfigStorage::list()` trait method (the one storage-layer gap — no
  enumerate-all method exists) + generic impl + updates to the two test-double
  implementors.
- Migrate the e2e seed's two `site_config` steps to `jaunder site-config set`, threading
  a real (cheap-kdf-OFF) `jaunder` binary through `devtool seed-e2e` and its two callers
  (flake VM `seed_db`, host `xtask e2e-local`).
- Migrate the **second** live consumer of `test-support set-site-config`:
  `end2end/tests/seed.ts`'s `seedConfigViaTool` (called 5× by `invite.spec.ts` to flip
  `site.registration_policy` / `site.base_url` mid-suite) → `jaunder site-config set`.
- Make `jaunder` resolvable in the seed contexts: add `jaunderBin` to **both** flake VM
  `environment.systemPackages` (today only `test-support`/`devtool` are on the VM PATH;
  `jaunder` runs only as a systemd service), so bare `jaunder` resolves in the VM exactly
  as `test-support` does. (Host e2e already has `target/debug` on PATH.)
- Retire `test-support`'s `set-site-config` subcommand + `set_site_config` lib fn + their
  tests (dead only after **both** consumers above stop calling them).

**Recorded tradeoff (cold review).** `test-support set_site_config` is literally
`state.site_config.set(key, value)` — the same `SiteConfigStore` call the new handler
makes — so migrating the seed changes _which process wraps the call, not which code runs_;
it buys dogfooding of the shipped command in e2e at the cost of a two-binary seed and a new
partial-seed failure mode (mitigated by D6's fast-fail ordering). Part 1 (the command +
`list()` + tests) is independently valuable; the seed/`end2end` migration (parts of this
directive) is the lower-value, higher-coupling half. It is **in scope because the task
directive explicitly mandates converting the testing infra** — flagged here, not silently
adopted.

**Out of scope (non-goals)**

- Typed/validated keys or a key allowlist. Keys/values stay **free-form**, mirroring
  storage semantics — `site.registration_policy` isn't even a declared `const` today, and
  the seed sets it. Validation is per-typed-accessor at read time and stays there.
- Migrating the seed's `create-user`/`reset-mail` steps off `test-support`. They must
  stay: `test-support create-user` deliberately skips the `CliBypass` registration metric
  the e2e suite might assert on (`test-support/src/lib.rs:95-99`), and `reset-mail` is
  capture-file logic with no `jaunder` equivalent. Only the `site_config` steps move.
- An ADR. The novel bits (first nested clap subcommand group; seed now spans two
  binaries) are local implementation shape, not cross-cutting architecture; the existing
  ADR set (backend parity, coverage gate, capture-dir contract) is a higher altitude.
  Recorded here instead. _(Cold review may challenge this.)_

## Design decisions

### D1 — Nested clap subcommand group

`jaunder site-config` is a subcommand group (`#[command(subcommand)]`) with leaf actions
`set`/`get`/`list`, giving `jaunder site-config set …`. This is jaunder's first nested
subcommand; the flat top-level `Commands` enum gains one `SiteConfig { action }` arm that
delegates to a small `SiteConfigAction::execute` match, preserving the "one small arm per
command, low CRAP" dispatch shape (`commands.rs:34-92`).

`StorageArgs` (`--storage-path`, `--db`, `--db` env `JAUNDER_DB`) is `#[command(flatten)]`
on **each leaf** (as every other DB-backed subcommand does), so `jaunder site-config set K
V --db …` reads naturally and `JAUNDER_DB` is honored (the seed relies on the env, exactly
as `test-support set-site-config` did).

A doc-comment breadcrumb on the `SiteConfig` dispatch arm records this "one thin arm →
`SiteConfigAction::execute` match" nesting pattern for the next command group to copy —
cheaper than an ADR (see non-goals). The group/leaf **long help enumerates the known
keys** (the `pub const`s in `storage/src/site_config.rs`, e.g. `site.registration_policy`,
`feeds.websub_hub_url`) so operators can discover valid keys without reading source —
mitigating the silent-typo cost of free-form keys (D2) without reintroducing validation.

### D2 — `set <key> <value>` (positional, upsert)

Positional `key` then `value` (git-config-style, ergonomic for ops). Calls
`SiteConfigStorage::set` (an upsert). Prints a confirmation to **stderr**
(`set site_config <key> = <value>`), mirroring `test-support`'s handler; stdout stays
clean. Exit 0 on success. The `value` arg sets `allow_hyphen_values = true` so a value
beginning with `-`/`--` (or any free-form string) is not misparsed as a flag; `--`
separates flags from positionals as usual.

### D3 — `get <key>` (positional)

Prints the raw stored value to **stdout** followed by a newline (scriptable). An **absent
key exits non-zero** with a stderr diagnostic (git-config semantics) rather than printing
nothing and exiting 0 — so scripts can distinguish "unset" from "empty".

### D4 — `list`

Prints every entry as `key=value`, **one per line, ordered by key**, to stdout. Empty
store prints nothing (exit 0). The formatting is a pure helper (`format_entries`)
unit-tested directly, so the handler stays a thin print and stdout-capture is unnecessary.

`list` is a **human/discovery view, not a machine round-trip**: a value containing a
newline or `=` is ambiguous under `key=value` lines. `get <key>` is the lossless,
scriptable accessor. (No `unset`/delete leaf — the storage trait has no delete; clearing
is "set empty", which the keys that treat empty as unset already honor. A future `unset`
would need a storage delete first.)

### D5 — `SiteConfigStorage::list()`

Add a required trait method
`async fn list(&self) -> sqlx::Result<Vec<(String, String)>>` returning all rows ordered
by key. One generic impl on `SiteConfigStore<DB>`
(`SELECT key, value FROM site_config ORDER BY key`) covers both backends (ADR-0019 — zero
backend divergence; the `$1`-placeholder pattern already works for both). The two
**test-double** implementors gain a trivial map-backed `list`:
`storage/src/smtp.rs:207` (`MapConfigStore`) and `server/src/mailer/mod.rs:69`
(`MapConfigStore`). Adding a required (non-defaulted) method is deliberate: a default
returning `vec![]` would silently under-report for any future implementor. The trait is
`#[cfg_attr(feature = "test-utils", mockall::automock)]`, so `MockSiteConfigStorage`
(used in `media_manager.rs`, `feed/{handlers,regenerate,worker}.rs`) auto-generates the
new method — no mock call-sites break; only the two hand-written doubles need editing.
Both doubles live in `#[cfg(test)]` modules (not coverage-gate-measured); their `list`
impls stay trivial.

### D6 — Seed migration (site_config steps → `jaunder`)

`seed_e2e::seed_invocations()` tags each step with its target binary
(`TestSupport | Jaunder`). The two `site_config` steps become
`jaunder site-config set <key> <value>` (`Jaunder`); `create-user`/`reset-mail` stay
`TestSupport`. `seed_e2e::run` gains a `jaunder_bin: &Path` param and dispatches
`Command::new(bin_for_step)` with `JAUNDER_DB=db` (env unchanged — `jaunder`'s
`StorageArgs.--db` reads `JAUNDER_DB`). `devtool seed-e2e` gains `--jaunder-bin`. Callers:

- `xtask/src/steps/e2e_local.rs` — pass `--jaunder-bin {root}/target/debug/jaunder` (the
  exact path already spawned for `serve`; `cargo build -p jaunder` → cheap-kdf OFF).
- `flake.nix` both `seed_db()` blocks — pass `--jaunder-bin jaunder` (bare, PATH-resolved,
  consistent with the existing `--test-support-bin test-support`), which the added
  `systemPackages` entry (see Scope) makes resolve to the crane-release `jaunderBin`.

**Fast-fail ordering.** The `Jaunder` (site_config) steps run **first** in
`seed_invocations()`, before the `test-support create-user` steps. So a wrong
`--jaunder-bin` (a cheap-kdf build) aborts on an **empty** DB rather than after 3 users
are created — no confusing half-seeded state, and the failure surfaces at the migrated
step. `seed_e2e::run`'s `bail!` message includes the offending binary path.

**Safety:** every `jaunder` build the seed can invoke has `cheap-kdf` OFF, so the
`server/src/main.rs` fail-closed guard does not fire. Only coverage/unit-test builds have
it on, and those never seed. (Each `jaunder` one-shot inits telemetry like the existing
VM `jaunder create-pg-db` call — same minor exporter churn, not a new class of noise.)

### D7 — Migrate the `end2end/` consumer, then retire `test-support set-site-config`

`end2end/tests/seed.ts`'s `seedConfigViaTool(key, value)` currently
`execFileSync("test-support", ["set-site-config", "--key", key, "--value", value])`. It
moves to `execFileSync("jaunder", ["site-config", "set", key, value])` (bare `jaunder`,
resolved via PATH in both VM — via the new `systemPackages` entry — and host, which has
`target/debug` on PATH). Its doc comment updates to name the new subcommand. The **5
call-sites in `invite.spec.ts` are unchanged** — only the helper body moves.
`seedPostsViaTool` in the same file stays on `test-support` (`seed-posts` is
test-support-only), so `test-support` remains on PATH.

Only **after both** consumers (the `devtool seed-e2e` path in D6 and this `seed.ts`
helper) stop calling it, remove `test-support`'s `SetSiteConfig` subcommand
(`test-support/src/main.rs`: variant, dispatch arm, `cmd_set_site_config`), the
`set_site_config` lib fn + its `set_site_config_tests` (`test-support/src/lib.rs`), and the
`SetSiteConfig` leg of `run_dispatches_db_commands_against_a_temp_db`
(`test-support/src/main.rs`). After the sweep, `rg set-site-config`/`set_site_config`
returns no hits outside archived docs and this spec/plan — no dead code left behind.

## Acceptance criteria

Each is observable so ship's conformance review can tell delivered from not.

- **AC1 (set upsert).** A handler test against a TempDir SQLite DB: `cmd_site_config_set`
  with `("feeds.websub_hub_url", "https://x/")` then read back via `state.site_config.get`
  returns `Some("https://x/")`; a second set on the same key with a new value overwrites
  (upsert), not errors.
- **AC2 (get).** `cmd_site_config_get` on a set key returns `Ok` (executing the value
  print — so the print line is covered); on an absent key returns `Err` (→ non-zero exit).
  A value beginning with `-` parses (via `allow_hyphen_values`) and round-trips. (Exact
  stdout bytes are additionally exercised by the e2e path, AC9.)
- **AC3 (list ordering + format).** `SiteConfigStorage::list` returns all entries ordered
  by key; `format_entries` renders them as `key=value\n` lines in that order (pure unit
  test); empty input → empty string. `list` is documented as a human view; `get` is the
  scriptable accessor.
- **AC4 (parse).** clap parse tests: `site-config set K V`, `site-config get K`,
  `site-config list` parse to the expected variants; `site-config set K` (missing value)
  is a clap error; a `set` value beginning with `-` parses; `--db`/`JAUNDER_DB` is accepted
  on each leaf.
- **AC5 (backend parity).** `SiteConfigStorage::list` is exercised on **both** SQLite and
  Postgres (the `site_config.rs` `backends`/`Backend` dual-backend test harness), asserting
  identical ordered results. (The ordering match holds because all keys are controlled
  lowercase-dotted ASCII — SQLite `BINARY` and Postgres locale collation agree there; it is
  not asserted as a universal cross-collation invariant.)
- **AC6 (seed shape).** `seed_e2e`'s `canonical_fixture_invocations` test asserts the two
  `site_config` steps come **first**, are `jaunder site-config set site.registration_policy
  open` and `jaunder site-config set feeds.websub_hub_url https://hub.test.local/` tagged to
  the `Jaunder` binary, and `create-user`×3 + `reset-mail` follow, tagged `TestSupport`.
- **AC7 (end2end migration).** `end2end/tests/seed.ts`'s `seedConfigViaTool` invokes
  `jaunder site-config set …`; `invite.spec.ts` (which drives the invite-only registration
  flow through it) stays green in the e2e matrix (AC9), proving the migrated helper still
  flips `site.registration_policy` live.
- **AC8 (retirement).** After the sweep, `rg 'set-site-config'` and
  `rg 'set_site_config'` over the tree (excluding archived docs and this spec/plan) return
  no hits; `test-support --help` no longer lists `set-site-config`.
- **AC9 (gate + e2e).** `cargo xtask validate --no-e2e` is green (static, clippy, coverage
  — new code covered, no CRAP/coverage regression), and the full `{sqlite,postgres}×{chromium,firefox}`
  e2e matrix is green (proves the real `jaunder site-config set` seeds both backends in
  both VM and host contexts).

## Files touched (map)

- `storage/src/site_config.rs` — trait method + generic impl + dual-backend test.
- `storage/src/smtp.rs`, `server/src/mailer/mod.rs` — `MapConfigStore` doubles gain `list`.
- `server/src/cli.rs` — `SiteConfig` variant + `SiteConfigAction` enum + parse tests.
- `server/src/commands.rs` — dispatch arm, `SiteConfigAction::execute`,
  `cmd_site_config_{set,get,list}`, `format_entries`, handler tests.
- `tools/devtool/src/seed_e2e.rs` — binary-tagged invocations (site_config first),
  `run(jaunder_bin)`, updated `canonical_fixture_invocations` test.
- `tools/devtool/src/main.rs` — `SeedE2eArgs.--jaunder-bin`, dispatch.
- `xtask/src/steps/e2e_local.rs` — pass `--jaunder-bin {root}/target/debug/jaunder`.
- `flake.nix` — both `seed_db()` blocks pass `--jaunder-bin jaunder`; add `jaunderBin` to
  both VM `environment.systemPackages`.
- `end2end/tests/seed.ts` — `seedConfigViaTool` → `jaunder site-config set` (+ doc comment).
- `test-support/src/main.rs`, `test-support/src/lib.rs` — remove `set-site-config`.
