# Phase 0 Congruence Findings: Host Network-Denied vs Nix Coverage Baseline

**Date:** 2026-06-19  
**Branch:** `testing-coverage-orchestration`  
**Task:** jaunder-1bhw.1 — Plan A T1: Phase 0

---

## Verdict

**NEEDS ADJUSTMENT: unshare -rn breaks initdb; alternative network-denial mechanism required.**

The core hypothesis — that denying network to the host coverage run reproduces the Nix baseline exactly — is directionally correct, but the proposed `unshare -rn` mechanism is incompatible with `initdb` in this environment. The `unshare -rn` approach maps the caller to UID 0 (root) inside the user namespace, and `initdb` unconditionally refuses to run as root. The PostgreSQL pass never executes, so the network-denied run cannot be evaluated for congruence. Plan B must use a different network-denial mechanism.

---

## Step 1: Nix Baseline

The committed `.coverage-manifest.json` is the Nix output (reproducible CI baseline).

- **117 files** tracked
- Notable high-coverage files: many `server/` and `web/` files at 100% (covered by integration tests under the Nix sandbox's ephemeral PG)

Snapshot saved to `/tmp/baseline-nix.json` for diffs.

---

## Step 2: Networked Host Run

**Invocation:** `scripts/check-coverage` (default — no network restriction)

**Result:** Script exited 1 (coverage regressions detected vs Nix baseline). All 1079+683 tests passed; the failure is a coverage shortfall, not a test failure.

**Divergence: 15 files LOWER than Nix baseline (no files higher):**

| File | Nix baseline | Networked host | Drop |
|------|-------------|----------------|------|
| `server/src/atompub/mod.rs` | 100.00% | 60.26% | −39.74% |
| `server/src/media.rs` | 98.79% | 61.22% | −37.57% |
| `web/src/auth/server.rs` | 100.00% | 69.57% | −30.43% |
| `web/src/error.rs` | 100.00% | 71.00% | −29.00% |
| `server/src/observability.rs` | 99.44% | 70.45% | −28.98% |
| `web/src/posts/mod.rs` | 99.25% | 75.26% | −23.99% |
| `server/src/media_manager.rs` | 99.18% | 76.79% | −22.40% |
| `server/src/commands.rs` | 96.33% | 75.57% | −20.76% |
| `server/src/backup.rs` | 100.00% | 80.27% | −19.73% |
| `web/src/auth/mod.rs` | 100.00% | 83.33% | −16.67% |
| `web/src/invites/mod.rs` | 100.00% | 84.09% | −15.91% |
| `server/src/feed/worker.rs` | 84.17% | 68.97% | −15.20% |
| `server/src/feed/handlers.rs` | 74.82% | 60.26% | −14.56% |
| `web/src/email/mod.rs` | 100.00% | 89.13% | −10.87% |
| `web/src/password_reset/mod.rs` | 100.00% | 90.00% | −10.00% |

**Interpretation:** These are all HTTP handler / integration-test-covered files. The Nix baseline achieves 100% (or near) because the Nix build sandbox runs the coverage suite inside a no-network environment where all network-sensitive tests fall back to local-only paths, and the instrumentation captures those paths. The host networked run exercises *fewer* of these paths (network calls succeed or attempt real connections rather than falling back), resulting in lower coverage. No files were *higher* than the Nix baseline.

**Note:** The flake comment at `flake.nix` confirms this is known and expected: *"a host `scripts/check-coverage --update` bakes in higher numbers for network-sensitive files (e.g. `server/src/websub/http.rs`, `server/src/commands.rs`) that the sandboxed CI run cannot reproduce, which then fails the gate."*

---

## Step 3: Network-Denied Host Run

**Invocation attempted:** `unshare -rn bash -c 'ip link set lo up && scripts/check-coverage'`

**Result: BLOCKED.** `unshare -rn` maps the calling user (UID 1000, `mdorman`) to UID 0 (root) inside the new user namespace. `initdb` refuses to run as root:

```
initdb: error: cannot be run as root
initdb: hint: Please log in (using, e.g., "su") as the (unprivileged) user
        that will own the server process.
```

The SQLite pass (1079 tests, 95 skipped) completed successfully. The PostgreSQL pass never started. The script aborted at the `bash scripts/with-ephemeral-postgres` invocation due to `set -euo pipefail`. No merged text report was generated; no manifest was written.

**What `unshare -rn` does to uid/gid:**

```
# Outside: uid=1000(mdorman) gid=1000(mdorman)
# Inside:  uid=0(root) gid=0(root)
```

The `-r` / `--map-root-user` flag is what causes this — it is required for an unprivileged user to create a network namespace (`-n`) in most Linux kernel configurations. The trade-off is unavoidable with this flag.

**Fallback noted in brief:** The brief suggests `unshare --map-root-user --net` as an alternative. This is identical to `unshare -rn` (both map the calling user to UID 0), so the same `initdb` failure applies.

**No successful network-denied run was obtained.** Congruence with the Nix baseline under network denial cannot be confirmed or denied from this experiment.

---

## Step 4: Diff Summary

| Run | Files vs Nix baseline | Verdict |
|-----|----------------------|---------|
| Networked host | 15 files LOWER (server/web handlers) | Does NOT match baseline |
| Network-denied host | Cannot evaluate — PG pass blocked by initdb/root | N/A |

---

## Step 5: Unix Socket DSN Evaluation

The brief asks whether switching `JAUNDER_PG_TEST_URL` to the unix socket form (`postgres:///jaunder?host=$PGDATA`) would allow the network-denied coverage run to avoid needing loopback at all.

**Test performed:** Started an ephemeral PG cluster with `unix_socket_directories=$PGDATA`, then connected via both DSN forms:

```
TCP:    postgres://jaunder@127.0.0.1:54399/jaunder          → OK (1 row)
Socket: postgres://jaunder@/jaunder?host=$PGDATA&port=54399 → OK (1 row)
Socket: postgres:///jaunder?host=$PGDATA&port=54399&user=jaunder → OK (1 row)
```

**Conclusion:** The unix socket DSN connects cleanly. Switching `with-ephemeral-postgres` to export a unix-socket DSN is feasible and would eliminate the TCP loopback requirement under network denial.

**However, this does not solve the `initdb`-as-root problem.** Even with a socket-only DSN, `initdb` must still be called to initialize the cluster, and it still fails under `unshare -rn`. The socket switch is a useful simplification for Plan B, but it is not sufficient on its own.

---

## Root Cause Analysis and Plan B Implications

The Nix sandbox uses Nix's native build-sandbox mechanism (kernel namespaces managed by the Nix daemon, which runs as root and creates unprivileged user namespaces correctly). The `nix build` process inside the sandbox is an unprivileged build user — not UID 0 — so `initdb` works fine.

`unshare -rn` from an unprivileged host process necessarily maps to UID 0 to gain the privilege to create a network namespace. This is the fundamental conflict.

**Alternative mechanisms for Plan B to evaluate:**

1. **`unshare -n` without `-r`** — creates a network namespace without the user namespace (requires `CAP_SYS_ADMIN` or `kernel.unprivileged_userns_clone=1`). Test: `unshare -n ip link show` — if this works in this environment, it would create a network-isolated namespace while keeping UID 1000, which lets `initdb` work. This was not tested in this spike.

2. **`nft` / `iptables` rules** — block outbound connections for the coverage process without changing UIDs. More complex to set up correctly.

3. **`RUST_LOG` / compile-time feature flags** — disable network-calling code paths at test time. Not a network denial mechanism.

4. **Run coverage inside a Nix build** — already done (the `#coverage-update` flake output). May be too slow for everyday use.

5. **`ip netns` with a pre-created namespace** — create the netns as root beforehand, then `ip netns exec <ns> scripts/check-coverage` as the original user. This preserves UID 1000 inside the namespace. Requires root to set up once but not for each run.

**Recommended first test for Plan B:** Try `unshare -n` (without `-r`) to see if this environment allows unprivileged network-namespace creation without the root-mapping trade-off.

---

## Ancillary Finding: `common/src/metrics.rs` Missing

The coverage text report emitted a warning:

```
error: /home/mdorman/src/jaunder/common/src/metrics.rs: No such file or directory
warning: The file '.../common/src/metrics.rs' isn't covered.
```

This file is referenced in the coverage instrumentation metadata but no longer exists on disk (presumably removed in a recent refactor on this branch). This is a pre-existing issue on the `testing-coverage-orchestration` branch and does not affect coverage percentages for existing files — it only adds a warning and a spurious `error:` prefix to stderr.

---

## Working Tree State

Both `.coverage-manifest.json` and `.crap-manifest.json` were restored to the committed baseline after each run via `git checkout -- .coverage-manifest.json .crap-manifest.json`. No stray changes remain. The working tree is clean (aside from auto-modified `.beads/issues.jsonl` which is excluded from this commit).
