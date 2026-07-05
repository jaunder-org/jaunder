# Spec — host e2e loop owns its server + shares the Nix VM's e2e infrastructure (issue #249)

**Issue:** [#249](https://github.com/jaunder-org/jaunder/issues/249) —
`refactor(e2e): host e2e loop owns its server + shares the Nix VM's e2e infrastructure`
**Status:** shipped (2026-07-05) — all §5 ACs verified (`cargo xtask validate`
green); #268 (full cargo-leptos removal) folded into this branch during
execution. **Base:** `origin/main` @ `7a5366cb` — **standalone issue** (see §0).

**Relates to (all separate, none blocking):**

- **#236** — CSR bundle build unified onto `devtool csr-bundle` +
  `cargo xtask build-csr`. **DONE** (shipped in PR #272). #249 consumes its
  `build-csr` output but is otherwise independent.
- **#237** — single-binary embed of `pkg/*`. Separate, open (Todo).
- **#268** — dev-loop replacement (`cargo leptos watch`/`serve`) + full
  cargo-leptos removal. Filed, open.
- **#269** — run the VM's zero-panic + diag-log gates in the host loop. Filed,
  open (the §3.4 spin-out).
- **#9** — batch post insert (edits `test-support`/`storage`, `--no-e2e`). Runs
  concurrently; §3.2's `devtool` choice keeps #249 out of `test-support` so the
  two branches don't collide.
- Background: #234, #239 (shipped), #153/#256 (the `e2e-local` seed/run driver,
  merged), #155 (Firefox slowness data point).

## 0. History — why this is a standalone issue now

#249 was originally _"realign cargo-leptos to a true CSR project so it owns
`index.html` + the wasm bootstrap natively."_ A de-risking spike **disproved
that premise**: cargo-leptos 0.3.5 has no CSR mode and never emits an
`index.html` (README v0.3.5: _"client-side rendering mode not supported"_;
`bin-package` is mandatory). The #234/#239 seams live in the server's Rust HTML
generation (`web::render::SPA_SHELL`, projector `document()`) and are already
closed on `main`. That whole surface is gone.

The design was briefly framed as the middle phase of a one-branch cluster (#236
→ #249 → #237). That framing is **retired**: #236 shipped on its own (PR #272),
so #249 is now a **standalone issue with its own branch/PR**. It keeps the
genuinely valuable, feasible half the #153 host-loop notes parked here: making
the host e2e loop own its server and share the VM's e2e infrastructure.

## 1. Goal

The host e2e loop and the Nix VM should **draw from the same e2e
infrastructure**, not independently reproduce each other. Today they share only
the Playwright config (via #153); the server start, its environment, the seed
fixtures, and the DB reset are duplicated and kept "in sync" **only by
comment**. This issue:

1. Makes the **host harness own the `jaunder serve` lifecycle** — the VM already
   owns its server via systemd; the host currently leans on
   `cargo leptos end-to-end` to spawn it, which is the root of the gaps below.
2. **Consolidates the duplicated fixture seed into one shared component both
   host and VM call**, so "sync" is enforced by code, not by comment.

Success is **not** "the host loop mimics the VM." It is: **one seed (fixture)
definition, one Playwright config, one canonical e2e-server env-var set, invoked
from both the host driver and the flake** — with the two environment-specific
concerns (DB reset, port binding) each done the way that environment's
constraints require, meeting at a shared `baseURL` seam.

## 2. Current duplication (what "share, don't mirror" targets)

| Piece             | Host today                                      | Nix VM today                                                | Target                                                                                          |
| ----------------- | ----------------------------------------------- | ----------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| Server start      | cargo-leptos spawns the debug bin implicitly    | systemd `ExecStart = jaunder serve`                         | **Host harness runs `jaunder serve`** (VM unchanged)                                            |
| Server env        | none of the capture env                         | `mailCaptureEnv` on `jaunder.service` (`flake.nix:744/856`) | **One canonical e2e-server env-var set**, sourced by both                                       |
| Port / baseURL    | fixed `:3000` (cargo-leptos)                    | fixed `:3000` (systemd); Playwright hard-targets it         | **Host: ephemeral `:0` + runtime.json (ADR-0035). `baseURL` param both feed. VM stays `:3000`** |
| Seed fixtures     | Rust list `e2e_local.rs:84-137`                 | Python `seed_db()` (`flake.nix:762-770` sqlite / postgres)  | **One `devtool seed-e2e`** both call                                                            |
| DB reset per run  | none — `data/jaunder.db` persists, state bleeds | stop+wipe dir (sqlite) / TRUNCATE (postgres)                | **Env-specific** (host: per-run temp DB; VM: systemctl/TRUNCATE)                                |
| Playwright config | shared ✔ (#153)                                | shared ✔ (#153)                                            | already done                                                                                    |
| Diagnostics gate  | none                                            | zero-panic (ADR-0032) + diag-log (ADR-0049) + OTel          | **out of scope → #269**                                                                         |

## 3. Design

### 3.1 Host harness owns the server

`cargo xtask e2e-local` becomes the host dev-loop command and owns the whole
lifecycle. The bundle is built by `cargo xtask build-csr` (#236, on `main`); the
server bin by `cargo build -p jaunder`. `cargo leptos end-to-end` is **retired**
as the loop entry point (cargo-leptos remains only for `watch`; full removal →
#268). The loop becomes:

```
cargo xtask build-csr            # produce target/site/pkg/*  (#236)
cargo build -p jaunder           # produce the server bin
→ make a PER-RUN temp storage dir + DB              # G3 + concurrency isolation
→ start `jaunder serve` on an EPHEMERAL port with the canonical e2e env  # G1+G2
    JAUNDER_BIND=127.0.0.1:0   JAUNDER_RUNTIME_FILE=<per-run temp>
    (dev environment → server auto-inits schema on start)
→ discover: poll runtime.json → read {ip,port}; then poll http://ip:port/ ready
→ seed fixtures via `devtool seed-e2e` (direct-DB; temp DB is fresh, no reset)   # §3.2
→ playwright test with baseURL=http://ip:port (chromium + chromium-admin, shared config)
→ tear the server down by tracked PID on EVERY exit path (incl. failure/kill)
```

Each build/serve/seed step follows the existing `e2e_local.rs` failure pattern
(`StepResult::fail` + early return), so a failed `build-csr`, a failed server
start, or a failed seed aborts the run with a non-zero exit and the tracked
teardown still fires.

**Ephemeral port, not fixed `:3000` (G1).** Binding `127.0.0.1:0` lets the OS
assign a free port; the server already reads back the resolved `local_addr()`
and writes it atomically to the runtime file (`server/src/runtime_file.rs`,
ADR-0035 — built for exactly "an ephemeral server discoverable by an
out-of-process caller," already used by the elisp harness #68/#74). Confirmed in
`server/src/commands.rs:438-443`. **No server change.** This is a _cleaner_ G1
than "refuse an occupied `:3000`": each run targets its own server's exact port,
so a stray prior server on another port can never be mistaken for this run's (it
merely leaks, which teardown-by-PID handles), it removes host-side port
conflicts (your dev server, a second e2e run), and it **enables concurrent host
runs** on distinct ports.

**Per-run temp storage dir + DB (G3).** Each run gets its own temp storage dir,
so distinct ephemeral ports **and** distinct DBs make concurrent host runs
isolated at the server + DB layer — and nothing pollutes the dev
`data/jaunder.db`. (Playwright's shared output dirs are not isolated — §5 AC 3.)
With a fresh temp DB the host needs **no reset** (the DB is empty each run). The
server starts in the **dev environment** so it auto-inits the schema on start
(`server/src/commands.rs`); the order is **fresh temp DB → start (auto-init) →
seed**.

**Teardown / stale runtime-file.** Ephemeral ports remove the _poisoning_ risk,
but a killed Playwright run must still not accumulate orphan `jaunder`
processes. The harness tracks the child PID and kills it on every exit path.
Because `Drop` is skipped on `SIGKILL` (#140), each run uses a **fresh per-run
`JAUNDER_RUNTIME_FILE`** so a stale file from a killed prior run can't be read
as live; the harness best-effort-removes it.

Mapped to the #153 gaps:

- **G1 — no orphan / no silent reuse.** Ephemeral port + runtime.json discovery
  ⇒ always drives _this run's_ server; PID-tracked teardown on every exit path ⇒
  no orphan accumulation.
- **G2 — capture-env parity.** The harness starts `jaunder serve` with the same
  env-var set the VM uses: `JAUNDER_MAIL_CAPTURE_FILE`,
  `JAUNDER_WEBSUB_CAPTURE_FILE`, the diag-log env (ADR-0049), `JAUNDER_DB`, plus
  `JAUNDER_BIND=127.0.0.1:0` and a per-run `JAUNDER_RUNTIME_FILE`. Host _values_
  differ from the VM's `/var/lib/jaunder/*` (host uses a temp dir), but the
  env-var **name set** is one canonical list (§3.3), not two.
- **G3 — fresh DB per run.** Per-run temp storage dir + DB ⇒ clean, seeded
  start; no `username is already taken`, no cross-run bleed, no pollution of the
  dev DB.

### 3.2 Shared seed — `devtool seed-e2e` (the core consolidation)

The seed _implementation_ is already one binary: **`test-support`** (both
callers shell out to its `create-user` / `set-site-config` / `reset-mail`
subcommands). What is duplicated is the **fixture argument list** — which users,
which config keys — spelled out as literal CLI strings in **three** places (host
`e2e_local.rs:84-137`, VM sqlite `seed_db` `flake.nix:762-770`, VM postgres
`seed_db`) and kept in sync only by comment.

Add **`devtool seed-e2e`**, which shells out to the `test-support` binary with
the canonical fixture args (the 3 users, 2 site-config keys, `reset-mail` used
today). Both callers invoke it:

- Host `e2e_local.rs` inline seed list → `devtool seed-e2e` (after its temp-DB
  start).
- VM `seed_db()` (both backends) → keeps its own reset, then `devtool seed-e2e`.

The canonical fixture list then lives in **one place** (`devtool`); the "these
must match" comments become enforced-by-code.

**Why `devtool`, not a new `test-support` subcommand:**

1. **Concurrency isolation from #9.** #9 is editing `test-support/src/lib.rs`
   (`seed_posts_for_user`) on a parallel branch. Putting `seed-e2e` in
   `test-support` would collide; putting it in the separate `tools/` workspace
   (`devtool`) touches no `test-support` file — `devtool` only _calls_ the
   existing `test-support` binary. Clean separation.
2. `devtool` is the crane-built tool that already runs both in hermetic Nix
   derivations and the host devShell (proven: `devtool coverage emit`,
   `devtool csr-bundle`). Its module doc already names `seed-e2e` as planned
   (`tools/devtool/src/main.rs:3`).

`devtool` **cannot link the `test-support` crate** (separate `tools/` workspace)
— it shells out to the binary.

**Locating `test-support` (do not assume host PATH).** `test-support` is on the
_guest_ PATH (`testSupportBin` in `systemPackages`) but is **not** on the host
PATH — today `e2e_local.rs:41` calls it by absolute path
(`{root}/target/debug/test-support`). So `devtool seed-e2e` takes an explicit
`--test-support-bin <path>` argument: the host driver passes its built
`target/debug/test-support`; the VM `seed_db()` passes the on-PATH
`test-support`. `devtool` never relies on host PATH.

**Prerequisites:**

1. The e2e guests list `testSupportBin` but **not** `devtoolBin` in
   `environment.systemPackages` (`flake.nix:722` sqlite / `820` postgres). This
   issue must add `devtoolBin` there, or `devtool seed-e2e` won't be on the
   guest PATH.
2. A **new** `devtool` subcommand is not callable via the host `devtool` on PATH
   until the devShell rebuilds it (a known mid-session gotcha). Host-side
   development/verification of `seed-e2e` runs it via
   `cargo run --manifest-path tools/Cargo.toml -- seed-e2e …`; Nix/CI (crane
   rebuilds `devtoolBin`) is unaffected. The plan's host verification must use
   the source-run form.

### 3.3 One canonical e2e-server env-var set + a parameterized baseURL

Define the e2e server's required env-var **names** as a single documented source
of truth: `JAUNDER_BIND`, `JAUNDER_DB`, `JAUNDER_MAIL_CAPTURE_FILE`,
`JAUNDER_WEBSUB_CAPTURE_FILE`, the diag-log var (ADR-0049),
`JAUNDER_RUNTIME_FILE`. The host driver and the flake systemd unit both consume
it (names shared; only _values_ differ per environment). A cross-language shared
constant (Rust ↔ Nix) would require codegen and is **out of scope**. Note these
vars are **not** single-sourced today: `JAUNDER_BIND`/`JAUNDER_DB`/
`JAUNDER_RUNTIME_FILE` are clap `env=` attributes in `server/src/cli.rs`, but
the capture/diag vars are read ad-hoc via `std::env::var` in
`server/src/mailer/mod.rs` + `server/src/websub/mod.rs` (the only clap `env=`
for `JAUNDER_MAIL_CAPTURE_FILE` is in `test-support/src/main.rs`). The
deliverable is therefore a single **documented list** of the e2e subset,
referenced by both the host driver and the flake — removing the "the VM sets
`mailCaptureEnv`, the host sets nothing" divergence at the definition level,
without forcing a code refactor to unify the read sites.

**Parameterized `baseURL`.** Because the host binds an ephemeral port, the
Playwright `baseURL` becomes a **parameter** rather than a hardcoded `:3000`:

- Host: the harness computes `http://ip:port` from the discovered runtime.json
  and feeds it.
- VM: **keeps fixed `:3000`** (it is hermetic — its own network namespace, no
  conflict to solve) and feeds `http://localhost:3000`.

Both feed the base URL through one env knob. **This is not already a parameter**
— `end2end/tests/helpers.ts:46` is a hardcoded
`const BASE_URL = "http://localhost:3000"` that reads no env var, and the warmup
URL (`fixtures.ts`) _separately_ reads a **different** knob
(`JAUNDER_E2E_WARMUP_URL ?? defaultWarmupUrl`). So the plan must (a) make
`helpers.ts` `BASE_URL` read a new env var (e.g. `JAUNDER_E2E_BASE_URL`) with
the `:3000` value as fallback, and (b) reconcile the pre-existing
`JAUNDER_E2E_WARMUP_URL` so both derive from the same base (warmup defaults to
`${BASE_URL}/`). Then the host feeds its discovered `http://ip:port` and the VM
feeds `http://localhost:3000` through that one knob. **Decision (issue #249):**
the VM does **not** adopt ephemeral `:0` — the churn (systemd bind, ~4
`wait_for_open_port(3000)` call sites, runtime-vs-eval-time baseURL plumbing in
`e2eRunAndCapture`, and the port-changes-per-restart interaction with
`seed_db`'s stop/wipe/start) buys only aesthetic uniformity for a conflict the
hermetic VM does not have, at real risk to the green CI matrix. `baseURL` is the
shared seam; the port _binding_ differs by environment.

## 4. Scope

**In #249:**

- §3.1 host harness owns `jaunder serve` (G1/G2/G3): ephemeral port +
  runtime.json discovery, capture env, per-run temp storage dir + DB,
  teardown-by-PID.
- §3.2 shared `devtool seed-e2e` (seed only; shells to `test-support`) used by
  host + VM; add `devtoolBin` to the two e2e guests.
- §3.3 canonical e2e-server env-var set (documented list) + parameterized
  Playwright `baseURL` (host feeds discovered port; VM feeds `:3000`).
- Retire `cargo leptos end-to-end` as the loop entry: drop
  `end2end-cmd`/`end2end-dir` (`Cargo.toml:134-135`) — they work today but would
  double-serve once `e2e-local` owns the server.

**Explicitly NOT in #249 (separate issues / non-goals):**

- VM adopting ephemeral `:0` (§3.3 decision — VM stays `:3000`).
- Host-side diagnostics/panic-gate parity → **#269**.
- Full cargo-leptos removal + `watch`/`serve` dev-loop replacement → **#268**
  (cargo-leptos stays installed for `watch` this issue).
- Single-binary embed of `pkg/*` → **#237**.
- The #155 Firefox-slowness data point (`auth.spec.ts:88`, `posts.spec.ts:628`
  on sqlite-firefox) — recorded, not actioned.
- No change to `[[workspace.metadata.leptos]]` beyond dropping the
  `end2end-cmd`; no un-embed of `SPA_SHELL`; no serving of a disk `index.html`;
  no SCSS/CSS work.

## 5. Acceptance criteria (observable)

1. **Self-contained loop.** From a clean checkout with **no** pre-existing
   server and **nothing** bound to `:3000`, `cargo xtask e2e-local` builds the
   bundle + server, starts its own `jaunder serve` on an ephemeral port
   (discovered via runtime.json), runs the chromium + chromium-admin suites, and
   stops the server — exit code reflects the Playwright result.
2. **No orphan after failure.** After a run whose Playwright step fails (or is
   killed), **no `jaunder` child process survives** (assertable by PID / port
   scan). Covered by a test that simulates a failing run.
3. **Concurrent server/DB isolation.** Two `cargo xtask e2e-local` invocations
   at once get **distinct ephemeral ports and distinct temp DBs**, and neither
   writes to the dev `data/jaunder.db` — so their servers and databases do not
   collide. _Scoped explicitly:_ this is server + DB isolation, **not** fully
   independent runs — both still run Playwright from `end2end/` and share its
   `test-results/` / HTML-report / trace output dirs. Isolating those is **out
   of scope** for #249 (a run started while another is mid-flight will overwrite
   the shared report). The AC is the port/DB claim only.
4. **Capture-env parity.** The server is started with
   `JAUNDER_MAIL_CAPTURE_FILE` + `JAUNDER_WEBSUB_CAPTURE_FILE` + diag-log set,
   so the mail/websub-dependent specs (`email.spec.ts`,
   `password_reset.spec.ts`, `feeds.spec.ts`) **pass on the host**. Baseline:
   today `e2e_local.rs:51` sets **only** `JAUNDER_MAIL_CAPTURE_FILE` on the
   Playwright process (never on the server, never
   `JAUNDER_WEBSUB_CAPTURE_FILE`), so the websub-dependent `feeds.spec.ts` has
   no capture file to read and fails on the host today — the conformance check
   is that it passes after the rework.
5. **Fresh DB per run.** Each run starts from a fresh, seeded DB — a second run
   does **not** hit `username is already taken`, and no state bleeds across
   runs.
6. **Single-test dev loop preserved.** `cargo xtask e2e-local <spec.ts[:line]>`
   runs **only** that test, fully self-contained (own ephemeral server + temp
   DB, seeded) — the existing single-test filter (`lib.rs`, `e2e_local.rs`
   `test_filter`) still works.
7. **Seed dedup.** The canonical fixture arg-list lives in **one place**
   (`devtool seed-e2e`, shelling to `test-support`), invoked by both the host
   driver and the flake `seed_db()` (both backends). The three literal seed
   lists are gone; no "these must match" comment remains load-bearing.
8. **Matrix unchanged.** `cargo xtask validate` stays green — the Nix e2e matrix
   behaves identically except that `seed_db()` now delegates to
   `devtool seed-e2e`. The VM systemd unit and its `:3000` bind are unchanged.
9. `cargo leptos end-to-end` is **no longer required** for the host loop; the
   `end2end-cmd`/`end2end-dir` leptos config is removed.

## 6. Risks & de-risking

- **Server teardown / stale runtime-file (AC 2).** Ephemeral ports remove the
  poisoning risk, but a killed run must not leak a `jaunder` child. Kill by
  tracked PID on every exit path; fresh per-run `JAUNDER_RUNTIME_FILE` (Drop
  skipped on `SIGKILL`, #140) + best-effort removal. **Test** simulating a
  failing run asserts no orphan child survives.
- **Seed on both backends (§3.2).** `devtool seed-e2e` must produce the expected
  fixture state on **SQLite and Postgres** (backend-parity, `CONTRIBUTING.md`);
  validate against `cargo xtask e2e postgres chromium` semantics even though the
  host loop itself runs SQLite. Idempotence is no longer required host-side
  (temp DB is fresh each run), but the VM path re-seeds after its reset.
- **`devtoolBin` on the guests.** Adding `devtoolBin` to the e2e guest
  `systemPackages` must not perturb the Nix e2e derivations otherwise; verify a
  full combo builds green.
- **Host bundle profile.** The host serves a slow-hydrating **debug** bundle, so
  it runs serial by default (`JAUNDER_E2E_WORKERS` default 1); the VM stays
  release. Keep that default when the build step moves to `build-csr`.
- **#9 concurrency.** #9 edits `test-support`/`storage` on a parallel branch;
  §3.2 keeps #249 out of `test-support`. If #9 lands first, re-confirm the
  `test-support` seed subcommands #249 shells to are unchanged in name/flags.

## 7. Backends / ADR

- **No migration**, no schema change.
- **No new ADR anticipated** — the design composes existing decisions: ADR-0035
  (runtime file / ephemeral discovery), ADR-0046 (out-of-process seed helper),
  ADR-0049 (diag-log), and the established `devtool`-in-sandbox pattern. If the
  plan/review finds a genuinely novel decision (e.g. the canonical env-set
  home), a short ADR draft is added then.

## 8. Resolved decisions (were open in the prior draft)

- **Seed home:** `devtool seed-e2e` (§3.2) — not a `test-support` subcommand
  (concurrency isolation from #9 + `devtool` already runs in-sandbox). Lands
  inside #249.
- **Host DB:** per-run temp storage dir + DB (§3.1); reset is env-specific (not
  shared into `devtool`).
- **VM port:** VM keeps fixed `:3000`; only `baseURL` is parameterized (§3.3).
- **Canonical env-set home:** a documented list of var **names** consumed by
  both callers (no cross-language codegen) (§3.3).
- **`end2end-cmd`/`end2end-dir`:** dropped (§4) — would double-serve
  post-rework.
