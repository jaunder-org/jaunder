# Spec — host e2e loop owns its server + shares the Nix VM's e2e infrastructure (issue #249)

**Issue:** [#249](https://github.com/jaunder-org/jaunder/issues/249) —
`refactor(e2e): host e2e loop owns its server + shares the Nix VM's e2e infrastructure`
**Status:** draft (awaiting approval) **Base:** `origin/main` @ `f63ac058` —
**#256/#153 is merged** (its `e2e-local` driver is on `main`; `run-e2e.sh`
deleted; ADR-0051 single Playwright config in). Phase B is **unblocked**.
**Independent of:** #236 (build tool) — #249 needs _a_ built bundle +
`jaunder serve`, however produced; it does not depend on the builder swap.
**Relates to:** #236 (build tool — **decided: drop cargo-leptos, unify on raw
`wasm-bindgen` via `devtool csr-bundle`**), #237 (single-binary embed), #234
(shipped), #239 (shipped), #155, #212

> **Cluster context (2026-07-05).** This is the middle phase of a
> single-session, one-branch cluster done as separate commits: **Phase A #236**
> (`devtool csr-bundle`, drop cargo-leptos from the build path) → **Phase B
> #249** (this) → **Phase C #237** (embed). The phases are **independent** and
> ordered only for legible commits — #249 works against any bundle build.

## 0. Why this spec was rewritten (the original premise was invalid)

#249 was filed as _"realign cargo-leptos to a true CSR project so it owns
`index.html` + the wasm bootstrap natively."_ A de-risking spike **disproved the
premise**: **cargo-leptos 0.3.5 has no CSR mode and never emits an
`index.html`** — it is SSR/hydration-only (README v0.3.5: _"Build server and
client for hydration (client-side rendering mode not supported)"_; `bin-package`
is a mandatory field, no lib-only project; source confirms no template read /
bootstrap injection — that is **Trunk's** model). The #234/#239 seams also live
in the **server's Rust HTML generation** (embedded `web::render::SPA_SHELL`,
projector `document()`), not in cargo-leptos output, and are **already closed on
`main`** (#234 is CLOSED/COMPLETED).

Consequences, all recorded on the issues:

- The cargo-leptos CSR config **flip is impossible and unnecessary** — no
  realignment of `[[workspace.metadata.leptos]]` (the only edit here is dropping
  `end2end-cmd`, B4 — it works today but would double-serve once `e2e-local`
  owns the server; full removal is the dev-loop follow-up).
- The build-tool / shell-ownership question (a _tool_ owning the shell means
  **Trunk**, not cargo-leptos) is **folded into #236**.
- #249 keeps the genuinely valuable, feasible half the #153 host-loop notes
  parked here.

## 1. Goal

The host e2e loop and the Nix VM should **draw from the same e2e
infrastructure**, not independently reproduce each other. Today they share only
the Playwright config (via #153); the server start, its environment, the seed
fixtures, and the DB reset are duplicated and kept "in sync" only by comment.
This issue:

1. Makes the **host harness own the `jaunder serve` lifecycle** (the enabling
   piece — the VM already owns its server via systemd; the host currently relies
   on `cargo leptos end-to-end` to spawn it, which is why the gaps below exist).
2. **Consolidates the duplicated pieces into shared components both host and VM
   call**, so "sync" is enforced by code, not by comment.

Success is **not** "the host loop mimics the VM" — it is "there is one
definition of the e2e server env, one seed (fixture) definition, one Playwright
config, invoked from both the host driver and the flake" (reset is inherently
environment-specific — §3.1).

## 2. Current duplication (what "share, don't mirror" targets)

| Piece             | Host today                                      | Nix VM today                                                       | Target                                                                                             |
| ----------------- | ----------------------------------------------- | ------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------- |
| Server start      | cargo-leptos spawns the debug bin implicitly    | systemd `ExecStart = jaunder serve` (e2e-check service)            | **Host harness runs `jaunder serve`** (VM unchanged)                                               |
| Server env        | none of the capture env                         | `JAUNDER_BIND`, `JAUNDER_DB`, `mailCaptureEnv` (`flake.nix:48-56`) | **One canonical e2e-server env set**, sourced by both                                              |
| Port / baseURL    | fixed `:3000` (cargo-leptos `site-addr`)        | fixed `:3000` (systemd bind), Playwright hard-targets it           | **Host: ephemeral `:0` + runtime.json discovery (ADR-0035); baseURL a Playwright param both feed** |
| Seed fixtures     | Rust list in `e2e_local.rs:86-153`              | Python `seed_db()` (`flake.nix:759` sqlite / `890` postgres)       | **One `devtool seed-e2e`** both call (already planned)                                             |
| DB reset per run  | none — `data/jaunder.db` persists, state bleeds | stop+wipe dir (sqlite) / TRUNCATE (postgres)                       | **Env-specific** (host temp DB; VM keeps systemctl — not shareable)                                |
| Playwright config | shared ✔ (#153)                                | shared ✔ (#153, loads `playwright.config.ts`)                     | already done                                                                                       |
| Diagnostics gate  | none                                            | zero-panic gate (ADR-0032) + OTel + diag-log gate (ADR-0049)       | **optional** host-side parity (candidate spin-out)                                                 |

## 3. Design

### 3.1 Host harness owns the server

The bundle is built by the #236 builder (`cargo xtask build-csr` =
`cargo build -p csr` + `devtool csr-bundle`) — or, before Phase A lands, by
whatever produces `target/site/pkg/*` today; #249 is agnostic to it. The server
bin builds via `cargo build -p jaunder`. So the host loop becomes:

```
build bundle (build-csr / any) + `cargo build -p jaunder`   # produce pkg/* + the server bin
→ make a PER-RUN temp storage dir + DB   # G3 + concurrency: isolates the DB, not just the port
→ start `jaunder serve` on an EPHEMERAL port with the canonical e2e env   # G1+G2 (§3.3)
    JAUNDER_BIND=127.0.0.1:0  JAUNDER_RUNTIME_FILE=<temp>  (dev environment)   # auto-inits schema
→ discover the port: poll runtime.json → read {ip,port}, then poll http://ip:port/ ready
→ seed fixtures (direct-DB via `devtool seed-e2e`, after schema exists)   # shared seed (§3.2)
→ playwright test with baseURL=http://ip:port (chromium + chromium-admin, shared config)
→ tear the server down by PID unconditionally (incl. on failure/kill); guard removes runtime.json
```

A **fresh temp storage dir per run** gives each run its own SQLite DB (or
Postgres schema/db), so distinct ephemeral ports **and** distinct DBs make
concurrent host runs truly independent — and nothing pollutes the dev
`data/jaunder.db`. This resolves §8's DB-location question in favour of a temp
dir. The server must start in the **dev environment** so it auto-inits the
schema (`server/src/commands.rs:~385-401`); otherwise seed-after-start hits an
empty DB.

**Ephemeral port, not fixed `:3000` (G1).** Binding `127.0.0.1:0` lets the OS
assign a free port; the server already reads back the resolved `local_addr()`
and writes it atomically to the runtime file (`server/src/runtime_file.rs`,
ADR-0035 — built for exactly "an ephemeral server discoverable by an
out-of-process caller," and already used by the elisp harness #68/#74). **No
server change.** This is a _cleaner_ G1 than "refuse an occupied `:3000`": each
run targets its own server's exact port, so a stray prior server cannot be
mistaken for this run's (it merely leaks, which teardown-by-PID handles) — and
it removes host-side port conflicts (your dev server, a second e2e run) and
**enables concurrent host runs** (sqlite + postgres, or multiple suites) on
distinct ports. Give each run a **fresh temp `JAUNDER_RUNTIME_FILE`** so a stale
file from a `SIGKILL`-ed prior run (Drop skipped — #140) can't mislead
discovery.

**Ordering dependency.** Seeding writes to the DB directly (not over HTTP) but
needs the schema, which the dev-environment server auto-inits on start
(`server/src/commands.rs:~385-401`). So the host order is **fresh temp DB →
start (auto-init) → seed**. With a per-run temp DB the host needs **no reset**
(the DB is empty each run). Reset is **not** shared into `devtool` — the VM's
SQLite reset is `systemctl stop jaunder → rm -rf …/data → start`
(`flake.nix:765`), which also clears the server's in-memory caches; an in-guest
CLI can do neither, so each environment keeps its own reset (host: temp DB; VM:
its existing systemctl/TRUNCATE). What `devtool seed-e2e` shares is **the seed**
(§3.2).

`cargo leptos end-to-end` (which spawns its _own_ server) is **retired as the
loop entry point**; `cargo xtask e2e-local` becomes the host dev-loop command
and owns the whole lifecycle. cargo-leptos remains for `build` (and `watch` for
live-reload dev).

- **G1 — no orphan / no silent reuse.** The server binds an **ephemeral port**
  (`JAUNDER_BIND=127.0.0.1:0`) and the harness discovers the resolved port from
  the runtime file (above), so it always drives _this run's_ server — a stray
  prior server on another port can't poison the run (the pre-#234-orphan case
  the #153 notes describe). The harness tracks the child PID and tears it down
  on every exit path, including a failed/killed Playwright run, so orphans don't
  accumulate.
- **G2 — capture-env parity.** The harness starts `jaunder serve` with the same
  env the VM sets: `JAUNDER_MAIL_CAPTURE_FILE`, `JAUNDER_WEBSUB_CAPTURE_FILE`,
  the diag-log env (ADR-0049), `JAUNDER_DB`, plus `JAUNDER_BIND=127.0.0.1:0` and
  a per-run `JAUNDER_RUNTIME_FILE`. Host paths differ from the VM's
  `/var/lib/jaunder/*` (host uses a temp dir), but the **env-var set** is one
  canonical list (§3.3), not two.
- **G3 — fresh DB per run.** Each run gets a **per-run temp storage dir + DB**,
  so it starts clean and seeded (via `devtool seed-e2e`, §3.2) — no reset needed
  on the host, no `username is already taken`, no cross-run bleed, and no
  pollution of the dev `data/jaunder.db`. Combined with the ephemeral port (G1),
  concurrent runs are fully isolated (distinct port **and** DB).

### 3.2 Shared seed — `devtool seed-e2e` (the core consolidation)

**What's already shared and what isn't.** The seed _implementation_ is already
one binary: **`test-support`**. Both callers shell out to it — the host
`e2e_local.rs:86-153` and the VM `seed_db()` (`flake.nix:759`/`890`) each run
`test-support create-user` / `set-site-config` / `reset-mail`. What's duplicated
is only the **fixture argument list** (which users, which config keys), kept "in
sync" by comment. (The **reset** is _not_ shareable — §3.1: the VM's SQLite
reset is systemctl-coupled and clears server caches; it stays
environment-specific.)

`devtool` is the right home for the shared arg-list because it is the
crane-built tool that runs in hermetic Nix derivations as well as the host
devShell (proven: the coverage check runs `devtool coverage emit` in-sandbox) —
precisely because xtask is host-only. Its module doc already names
**`seed-e2e`** as a planned migration (`tools/devtool/src/main.rs:3-4`).

Add `devtool seed-e2e` that **shells out to the `test-support` binary** with the
canonical fixture args (it cannot _link_ `test-support` — separate `tools/`
workspace). Both callers invoke it:

- Host `e2e_local.rs` inline seed list → `devtool seed-e2e` (after its temp-DB
  start).
- VM `seed_db()` (both backends) → keeps its own reset, then `devtool seed-e2e`.

**Prerequisite (cold review):** the e2e guests list `testSupportBin` but **not**
`devtoolBin` in `environment.systemPackages` (`flake.nix:730`/`828`) — B1 must
add `devtoolBin` there, or `devtool seed-e2e` won't be on the guest PATH. The
canonical fixture list then lives in **one place** (`devtool`); the "these must
match" comments become enforced-by-code. Lands **inside #249** (plan B1).

### 3.3 One canonical e2e-server env set + a parameterized baseURL

Define the e2e server's required env (bind, DB, mail/websub capture, diag-log,
runtime-file) as a single source of truth. The host driver and the flake systemd
unit both consume it (names shared; only _values_ differ per environment). This
removes the "the VM sets `mailCaptureEnv`, the host sets nothing" divergence at
the definition level.

Because the host binds an ephemeral port (§3.1), the Playwright **`baseURL`
becomes a parameter** rather than a hardcoded `:3000`: the harness computes it
(host: the discovered `ip:port`; VM: its fixed `:3000`) and feeds it to the
_shared_ `playwright.config.ts` (via the env its `helpers` `BASE_URL` reads).
Both environments drive the same config through one knob instead of two
hardcoded ports. Whether the VM also adopts ephemeral `:0` + runtime.json (full
uniformity) or keeps fixed `:3000` (it's hermetic — no conflict) is a plan-time
call; either way baseURL is the shared seam.

### 3.4 Optional — host-side diagnostics/panic gate (candidate spin-out)

The VM runs a zero-panic gate (ADR-0032) + diag-log gate (ADR-0049) + OTel
capture the host lacks. Running the same gate host-side would extend the share
to _verification_, not just setup. This is **additive** (the loop works without
it) and is a good **spin-out issue** rather than #249 scope — noted so it isn't
lost.

## 4. Scope

**In #249:**

- §3.1 host harness owns `jaunder serve` (G1/G2/G3).
- §3.3 canonical e2e-server env set.
- Retire `cargo leptos end-to-end` as the loop entry; `cargo xtask e2e-local`
  owns it.

- §3.2 shared `devtool seed-e2e` (seed only; shells to `test-support`) used by
  host + VM (plan task B1).
- Drop `end2end-cmd`/`end2end-dir` from the leptos config (plan B4 — they work
  today but would double-serve post-rework).

**Spun out:**

- §3.4 host-side diagnostics/panic-gate parity (new issue).
- Build-tool + shell ownership (cargo-leptos vs Trunk vs raw wasm-bindgen) →
  **#236**.
- Single-binary embed of `pkg/*` → **#237**.
- The #155 Firefox-slowness data point (`auth.spec.ts:88`, `posts.spec.ts:628`
  on sqlite-firefox) — recorded, not actioned.
- Dev-loop replacement + full cargo-leptos/leptos-config removal → new follow-up
  (plan T0).

**Explicitly NOT done:** no change to `[[workspace.metadata.leptos]]` beyond
dropping the `end2end-cmd` (B4) — full leptos-config removal is the dev-loop
follow-up; no un-embed of `SPA_SHELL`; no serving disk `index.html`; no SCSS/CSS
work (served CSS is committed + rust-embedded, not bundle-produced).

## 5. Sequencing & relationships

```
#153 / #256  (playwright-config dedup + e2e-local seed/run driver)   ← MERGED (on main)
  └─enables→ #249  (host harness owns the server + shared seed/reset/env)   ← THIS
              cluster siblings (independent, one branch): #236 (build tool), #237 (embed)
              spin-outs: host-side diag gate; dev-loop replacement
```

- **#153/#256 → #249:** #249 rewrites `xtask/src/steps/e2e_local.rs`, landed by
  #256 (now on `main`). No longer gated — extend the merged file directly.
- **#236, #237:** cluster siblings done in the same branch as separate commits,
  but **independent** — #249 needs only _a_ built bundle + `jaunder serve`, not
  the #236 builder swap. #236 also carries the build-tool/shell-ownership
  decision (raw wasm-bindgen chosen).

## 6. Risks & de-risking

- **cargo-leptos CSR premise — CLOSED.** The spike (README v0.3.5 + source +
  reproducible `missing field bin-package`) settled that cargo-leptos can't own
  the shell. No config flip; that whole risk surface is gone.
- **Base freshness:** #256/#153, #224, #145 are already merged (this spec was
  re-baselined onto `origin/main` @ `f63ac058` during the cold-review pass).
  Re-derive any line refs against that tree before implementing.
- **Server teardown / stale runtime-file (G1):** ephemeral ports remove the
  _poisoning_ risk, but a killed Playwright run must still not accumulate orphan
  `jaunder` processes. The harness kills by tracked PID; because Drop is skipped
  on `SIGKILL` (#140), it uses a fresh per-run `JAUNDER_RUNTIME_FILE` so a stale
  file can't be read as live, and best-effort-removes it. Cover with a test that
  simulates a failing run and asserts no orphan child survives.
- **Shared-seed backend coverage (§3.2):** the `--reset` path must handle both
  SQLite (file wipe) and Postgres (TRUNCATE) correctly; the VM's Postgres reset
  is the reference. Validate against `cargo xtask e2e postgres chromium`
  semantics even though the host loop itself runs SQLite.
- **Host bundle profile:** the host loop serves a slow-hydrating _debug_ bundle,
  so it runs serial by default (`e2e_local.rs`, workers=1, `JAUNDER_E2E_WORKERS`
  override); the VM stays release. Keep that default — don't regress it when the
  build step moves to `build-csr`.

## 7. Success criteria

- `cargo xtask e2e-local` **builds, starts, and stops its own `jaunder serve`**
  on an ephemeral port (discovered via runtime.json); leaves no orphan after a
  failed run; two concurrent runs don't collide (distinct ports **and** temp
  DBs).
- **Single-test dev loop:** `cargo xtask e2e-local <spec.ts[:line]>` runs _only_
  that test, fully self-contained (own ephemeral server + temp DB, seeded) — no
  pre-existing server, no `:3000` conflict with a running dev server. (The
  single-test filter already exists, `lib.rs:118/343`; the rewrite must preserve
  it.) The headline DX win.
- The server is started with the VM's capture env; mail/websub-dependent specs
  (`email.spec.ts`, `password_reset.spec.ts`, `feeds.spec.ts`) pass on the host.
- Each run starts from a fresh, seeded DB (no cross-run bleed).
- The canonical **fixture arg-list lives in one place** (`devtool seed-e2e`,
  shelling out to `test-support`), invoked by both the host driver and the flake
  `seed_db()` — the duplicated lists are gone.
- `cargo xtask e2e-local` is **green** from a clean checkout, exercising the
  same fixtures + Playwright config the VM does
  (`cargo xtask e2e sqlite chromium`).
- `cargo xtask validate` stays green (the Nix e2e matrix behavior is unchanged
  except that `seed_db()` now delegates to the shared subcommand).
- `cargo leptos end-to-end` is no longer required for the host loop.

## 8. Open questions (for the plan)

_Resolved during the cold-review passes:_ §3.2 lands **inside #249** (B1); host
DB is a **per-run temp dir** (§3.1); reset is env-specific (not shared);
`end2end-cmd`/`end2end-dir` are **dropped** (plan B4 — they currently work via
`e2e-local`, but would double-serve once `e2e-local` owns the server). Still
open:

- Where does the canonical e2e-server env set live so both Rust (xtask/devtool)
  and Nix (flake) consume it without re-duplication? (A shared constant vs. a
  documented list.)
- Does the VM also adopt ephemeral `:0` + runtime.json discovery (full host/VM
  uniformity), or keep its hermetic fixed `:3000` and only parameterize
  `baseURL`? The latter is less VM churn; the former makes the discovery path
  identical everywhere.
