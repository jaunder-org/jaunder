# Issue #792 — measurement notes

Working notes for the plan
[`2026-08-04-issue-792-e2e-warmup.md`](./2026-08-04-issue-792-e2e-warmup.md).
Raw evidence lives here; the findings that survive go to `docs/observability.md`
(spec AC-6) and the verdict comment (AC-7).

## Task 2 — baseline `drvPath` (captured 2026-08-04, before any `flake.nix` edit)

Tree state at capture: `flake.nix` **unmodified** (`git diff HEAD -- flake.nix`
empty); the only diff-from-HEAD was the staged spec/plan markdown.

**Staged-file caveat, settled empirically.** The plan review verified that
_untracked_ files do not affect `nix eval`, but these planning docs were
**staged**, and staged files _are_ part of a flake's source. Re-evaluating
`e2e-sqlite-chromium` with them staged returned `b4m5d17…` — byte-identical to
the value the reviewer captured before they existed. So staged docs do not reach
the e2e derivations either, and the baseline below is comparable against any
later tree state that differs only in `flake.nix` plus docs/xtask additions.

| Attr                                                 | `drvPath`                                                                                        |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `checks.x86_64-linux.e2e-sqlite-chromium`            | `/nix/store/b4m5d17ziz5ymqjihg72p5mkzmwim673-vm-test-run-jaunder-e2e-sqlite-chromium.drv`        |
| `checks.x86_64-linux.e2e-sqlite-firefox`             | `/nix/store/vv1nicbck2cfiiv98ydgs2yg7jspjcwn-vm-test-run-jaunder-e2e-sqlite-firefox.drv`         |
| `checks.x86_64-linux.e2e-postgres-chromium`          | `/nix/store/nyczll687gcr17shzacpy81jk0faardy-vm-test-run-jaunder-e2e-postgres-chromium.drv`      |
| `checks.x86_64-linux.e2e-postgres-firefox`           | `/nix/store/r7nlys0drfbs6b8fg34w8faybz949v0p-vm-test-run-jaunder-e2e-postgres-firefox.drv`       |
| `packages.x86_64-linux.e2e-sqlite-chromium-cold`     | `/nix/store/5ad04g0yhh0219nb3z0iryjb1x1iy9ml-vm-test-run-jaunder-e2e-sqlite-chromium-cold.drv`   |
| `packages.x86_64-linux.e2e-sqlite-firefox-cold`      | `/nix/store/hdhvm8w6b0g5hzpxkbavryd7gp8birgx-vm-test-run-jaunder-e2e-sqlite-firefox-cold.drv`    |
| `packages.x86_64-linux.e2e-postgres-chromium-cold`   | `/nix/store/x69vznbcs7pwywrrgw2yllm6kj6xw5bx-vm-test-run-jaunder-e2e-postgres-chromium-cold.drv` |
| `packages.x86_64-linux.e2e-postgres-firefox-cold`    | `/nix/store/854x5n6bcg1nnd09vihh0jfaw6nx29f8-vm-test-run-jaunder-e2e-postgres-firefox-cold.drv`  |
| `packages.x86_64-linux.jaunder` (crane-filter probe) | `/nix/store/1ia6nd3g5w9hkysc5hphrrlrj8pwv79k-jaunder-0.1.0.drv`                                  |

Capture command (re-runnable):

```sh
for a in checks.x86_64-linux.e2e-{sqlite,postgres}-{chromium,firefox} \
         packages.x86_64-linux.e2e-{sqlite,postgres}-{chromium,firefox}-cold \
         packages.x86_64-linux.jaunder; do
  echo "$a = $(nix eval --raw ".#$a.drvPath" 2>/dev/null)"
done
```

The last row is the session-length probe (plan task 2c): `flake.nix` is outside
the crane source filter, so `jaunder`'s hash must stay `1ia6nd3g…` across every
salt flip. If it ever moves, a salted run is rebuilding the Rust workspace and
the collection budget is blown.

## Task 3 — the three hash proofs (2026-08-04)

All three pass. Prefixes below; full paths are reproducible with the capture
command above.

| Attr                         | baseline  | 3d: literals in, defaults | 3e: `e2eSalt = "probe"` | 3f: `e2eWarmup = false` |
| ---------------------------- | --------- | ------------------------- | ----------------------- | ----------------------- |
| `e2e-sqlite-chromium`        | `b4m5d17` | `b4m5d17` ✅ same         | `4izlp0q` ✅ differs    | `308p743` ✅ differs    |
| `e2e-sqlite-firefox`         | `vv1nicb` | `vv1nicb` ✅              | `xs0jhrs` ✅            | `qqshv0r` ✅            |
| `e2e-postgres-chromium`      | `nyczll6` | `nyczll6` ✅              | `gci86r0` ✅            | `7zaz7x0` ✅            |
| `e2e-postgres-firefox`       | `r7nlys0` | `r7nlys0` ✅              | `j4r69mq` ✅            | `zff4hb6` ✅            |
| `e2e-sqlite-chromium-cold`   | `5ad04g0` | `5ad04g0` ✅              | `289qb7q` ✅            | `5ad04g0` ✅ unchanged  |
| `e2e-sqlite-firefox-cold`    | `hdhvm8w` | `hdhvm8w` ✅              | `r66d2dy` ✅            | —                       |
| `e2e-postgres-chromium-cold` | `x69vznb` | `x69vznb` ✅              | `ycd94ri` ✅            | —                       |
| `e2e-postgres-firefox-cold`  | `854x5n6` | `854x5n6` ✅              | `hl5y0sq` ✅            | `854x5n6` ✅ unchanged  |
| `jaunder` (crane probe)      | `1ia6nd3` | `1ia6nd3` ✅              | `1ia6nd3` ✅ unchanged  | —                       |

What each column establishes:

- **3d (AC-2, and AC-3's identity half)** — adding the wiring with both literals
  at their defaults changes **nothing**. The cachix pulls and any local e2e
  store paths survive the scaffolding landing.
- **3e (AC-1)** — a non-empty salt moves all eight e2e derivations, so
  `traces run` cannot return a cached suite result. `jaunder` is **unchanged**,
  confirming `flake.nix` sits outside the crane source filter: a salted run
  re-runs the VM suite without rebuilding the Rust workspace. This is what
  bounds the collection session to suite time rather than build time.
- **3f (AC-3)** — `e2eWarmup` is load-bearing on exactly the four warm gate
  checks; the cold packages are untouched, as expected since they never set the
  warmup token.

Independently corroborated: an out-of-band review agent produced `b4m5d17`,
`854x5n6`, `1ia6nd3`, `4izlp0q`, `hl5y0sq` and `308p743` from its own probe of
the same edits, matching every value above.

After 3f the tree was restored (`e2eSalt = ""`, `e2eWarmup = true`) and
`e2e-sqlite-chromium` re-evaluated to `b4m5d17…` — back at baseline.

## Task 4 — guard evidence (2026-08-04)

Unit tests: 6/6 pass —
`cargo test --manifest-path xtask/Cargo.toml e2e_scaffold`. (Not `-p xtask`:
root `Cargo.toml:14` has `exclude = ["xtask"]`, so xtask is a separate workspace
and `-p xtask` matches nothing.)

**AC-4 clause 1** — three `cargo xtask check --no-test` runs:

| `flake.nix` state   | exit | step result                                               |
| ------------------- | ---- | --------------------------------------------------------- |
| `e2eSalt = "run1"`  | 1    | `[FAIL] e2e-scaffold` at `flake.nix:898`, naming the salt |
| `e2eWarmup = false` | 1    | `[FAIL] e2e-scaffold` at `flake.nix:902`, naming warmup   |
| both at defaults    | 0    | `[ ok ] e2e-scaffold`                                     |

The two failure details verbatim:

```
[FAIL] e2e-scaffold — flake.nix:898: `e2eSalt` is set — revert it to `""` before
committing. A non-empty salt changes every e2e derivation hash, so CI rebuilds
all four combos from scratch with no cache hit and nothing fails loudly (#792)

[FAIL] e2e-scaffold — flake.nix:902: `e2eWarmup` is not `true` — restore it
before committing. It disables the per-test warmup on all four gate checks,
silently changing what the gate tests (#792)
```

`check --no-test` rather than `validate --no-e2e`: `Command::Validate` runs
`clean_tree_precheck` and returns early on a dirty tree
(`xtask/src/lib.rs:448-454`, `:712-728`), and a salted `flake.nix` **is** a
dirty tree — so `validate` would fail on `clean-tree` without ever reaching the
guard, producing evidence about the wrong thing. `check` has no such precheck.
In CI the tree is clean by construction, so a _committed_ salt is exactly what
the guard sees there.

**AC-4 clause 2 — the guard cannot run inside an e2e derivation, by
construction.** `flake.nix:266-272` excludes `/xtask/` from the source filter,
with a comment saying exactly this (an accidental `cargo xtask` inside a
derivation fails loudly rather than running stale); `:1112` and `:1161` repeat
it for the other filters. The step lives entirely under `xtask/src/**` and no
`testScript` invokes xtask. Corroborating evidence: with `e2eSalt = "probe"` the
salted attrs still **evaluate** to derivations (task 3e's table), so the salted
build path is intact.

No real `nix build` was spent on this clause: it would purchase no information
over the source-filter argument and would compete with task 7's quiescence
budget.
