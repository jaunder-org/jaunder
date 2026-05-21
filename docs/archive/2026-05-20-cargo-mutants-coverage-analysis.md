# Cargo Mutants Coverage Analysis Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Run `cargo mutants` across all workspace packages and create one beads issue per survived mutant (or cluster of closely related survivors), capturing the exact mutation and function so coverage gaps are queued for fixing in later sessions. No test-writing in this pass.

**Architecture:** We run mutants per-package sequentially — `common` (pure logic, ~100 mutants), `storage` (DB layer, ~450 mutants), `web` (server functions, ~280 mutants), `server` (CLI, ~80 mutants). Each run uses a single job to avoid overwhelming the build machine. Each run produces a `mutants.out/` directory. We skip files that are structurally untestable (WASM hydration, binary entry points). After each run, survived mutants are triaged and logged as beads issues immediately — unviable and timeout outcomes are ignored.

**Tech Stack:** `cargo-mutants 27.0.0`, `cargo nextest` (test runner), Rust workspace with `common`, `storage`, `web`, `server` packages.

---

## Mutant Classification

- **Killed mutant**: Tests detected the change → good test coverage
- **Survived mutant**: No test caught the change → coverage gap or semantically equivalent mutation
- **Unviable mutant**: Mutation made code fail to compile → not a real gap
- **Timeout**: Mutation caused an infinite loop → also not a real gap
- **Superfluous/weak test signals**: A function with tests but many survived mutants; the tests exist but don't verify the behavior that mutations probe

---

## Task 1: Configure `.cargo/mutants.toml`

**Files:**
- Create: `.cargo/mutants.toml`

- [ ] **Step 1: Write the configuration file**

```toml
# .cargo/mutants.toml

# Skip files that are structurally untestable:
# - main.rs: binary entry point, no unit tests possible
# - hydrate/src/lib.rs: WASM client-side hydration, never runs in test harness
# - server/src/observability.rs: logging/tracing setup, no logic to mutate
# - server/src/assets.rs: static asset serving setup
# - storage/src/postgres/**:  cargo nextest uses sqlite::memory:, so mutations
#   to postgres code are never exercised by any test — every postgres mutant
#   would survive, producing pure noise. Postgres coverage lives in the Nix
#   VM e2e tests, which cargo mutants cannot drive.
exclude_globs = [
    "server/src/main.rs",
    "hydrate/src/lib.rs",
    "server/src/observability.rs",
    "server/src/assets.rs",
    "storage/src/postgres/**",
    "storage/src/backup/postgres.rs",
]

# Use nextest for faster test runs
test_tool = "nextest"

# Single job only — parallel builds overwhelm this machine
jobs = 1

# Auto-set timeout relative to baseline; add 20% headroom
timeout_multiplier = 1.2

# Minimum test timeout in seconds (prevents false timeouts on slow CI)
minimum_test_timeout = 60

# Cap lints so warning-as-errors don't make mutants unviable
cap_lints = true
```

- [ ] **Step 2: Verify config is read**

```bash
cargo mutants --list 2>&1 | grep -c "hydrate\|server/src/main\|observability\|assets"
```

Expected output: `0` (those files no longer appear in the mutant list)

- [ ] **Step 3: Re-count total mutants**

```bash
cargo mutants --list 2>&1 | wc -l
```

Note the count (should be slightly lower than the initial 1027).

- [ ] **Step 4: Commit the configuration**

```bash
git add .cargo/mutants.toml
git commit -m "mutants: add .cargo/mutants.toml config, skip untestable entry points"
```

---

## Task 2: Run Mutants for `common` Package

The `common` package contains pure domain logic (Password, Slug, Tag, Username, media utilities, mailer). These are the highest-value targets — pure functions with deterministic outputs and existing unit tests.

**Files:**
- Read: `common/src/password.rs`, `common/src/slug.rs`, `common/src/tag.rs`, `common/src/username.rs`, `common/src/media.rs`, `common/src/mailer.rs`

- [ ] **Step 1: Count mutants in `common`**

```bash
cargo mutants --list -p common 2>&1 | wc -l
```

Note the count.

- [ ] **Step 2: Run mutants for `common`**

```bash
cargo mutants -p common --output mutants-common.out 2>&1 | tee /tmp/mutants-common-run.log
```

This will take roughly `(mutant count) × (baseline test time ~20s)` — budget an hour or more. Watch for the summary line at the end.

- [ ] **Step 3: Examine the results**

```bash
# Summary
cat mutants-common.out/outcomes.json | python3 -c "
import json,sys
d=json.load(sys.stdin)
by_status={}
for o in d['outcomes']:
    s=o['summary']
    by_status[s]=by_status.get(s,0)+1
for k,v in sorted(by_status.items()):
    print(f'{k}: {v}')
"
```

Or if python3 is unavailable:

```bash
grep '"summary"' mutants-common.out/outcomes.json | sort | uniq -c | sort -rn
```

- [ ] **Step 4: List survived mutants**

```bash
cat mutants-common.out/outcomes.json | grep -A5 '"summary": "survived"' | grep '"description"' | sed 's/.*"description": "\(.*\)".*/\1/'
```

Or view the human-readable log:

```bash
grep "^SURVIVED\|^survived" mutants-common.out/mutants.log 2>/dev/null || \
  grep -i survived mutants-common.out/outcomes.json | head -40
```

- [ ] **Step 5: Log survived mutants as beads issues**

For each survived mutant (or tight cluster in the same function), run:

```bash
bd create \
  --title="Test gap: <package>/<file> <function> — <short mutation description>" \
  --description="cargo mutants: '<exact mutation string from outcomes.json>'. The test suite does not catch this change, meaning <what behavior is unverified>. File: <path>." \
  --type=task \
  --priority=2
```

Use `--priority=1` for security-sensitive functions (`verify`, `authenticate`, `require_auth`, session/token handling).

---

## Task 3: Run Mutants for `storage` Package

The `storage` package contains the SQLite implementation and shared storage logic. The postgres implementation files are excluded (see config) because `cargo nextest` uses `sqlite::memory:` and would never exercise postgres mutations. This is still the largest package (~450 mutants after exclusions).

- [ ] **Step 1: Count mutants in `storage`**

```bash
cargo mutants --list -p storage 2>&1 | wc -l
```

- [ ] **Step 2: Run mutants for `storage`**

```bash
cargo mutants -p storage --output mutants-storage.out 2>&1 | tee /tmp/mutants-storage-run.log
```

This is the largest package (~450 mutants × ~20s ≈ several hours). Run in background and check progress:

```bash
cargo mutants -p storage --output mutants-storage.out > /tmp/mutants-storage-run.log 2>&1 &
echo "PID: $!"
```

Check progress:

```bash
tail -f /tmp/mutants-storage-run.log
```

- [ ] **Step 3: Examine results**

```bash
grep -i "survived\|killed\|unviable\|timeout" /tmp/mutants-storage-run.log | tail -5
```

- [ ] **Step 4: List survived mutants grouped by file**

```bash
grep '"summary": "survived"' -A 10 mutants-storage.out/outcomes.json | \
  grep '"path"' | sort | uniq -c | sort -rn
```

- [ ] **Step 5: Log survived mutants as beads issues**

Same pattern as Task 2 Step 5: one `bd create` per survived mutant or per same-function cluster. Use `--priority=1` for auth/session/password survivors.

---

## Task 4: Run Mutants for `web` Package

The `web` package contains Leptos server functions. Many functions are SSR-only and tested via integration tests. Some mutations may be unviable due to `#[cfg(feature = "ssr")]` guards.

- [ ] **Step 1: Count mutants in `web`**

```bash
cargo mutants --list -p web 2>&1 | wc -l
```

- [ ] **Step 2: Run mutants for `web`**

```bash
cargo mutants -p web --output mutants-web.out 2>&1 | tee /tmp/mutants-web-run.log
```

- [ ] **Step 3: Note the unviable rate**

A high unviable rate in `web` is expected — client-side Leptos component code (`web/src/pages/*.rs`) compiles differently under the test harness than in WASM. This is normal. Focus only on `survived` mutants.

```bash
grep -i "unviable\|survived" /tmp/mutants-web-run.log | tail -10
```

- [ ] **Step 4: List survived mutants in web**

```bash
grep '"summary": "survived"' -A 10 mutants-web.out/outcomes.json | \
  grep -E '"description"|"path"' | paste - - | head -30
```

- [ ] **Step 5: Log survived mutants as beads issues**

Same pattern as Task 2 Step 5. Note: `web/src/pages/` survivors are often unviable-adjacent (Leptos view code); inspect each before creating an issue — if the mutation is inside a `view!` macro or `Effect::new` body, skip it.

---

## Task 5: Run Mutants for `server` Package

The `server` package contains CLI commands and the application runner. Much of this is integration-level code that's hard to unit-test. Expect a lower kill rate here.

- [ ] **Step 1: Run mutants for `server`**

```bash
cargo mutants -p server --output mutants-server.out 2>&1 | tee /tmp/mutants-server-run.log
```

- [ ] **Step 2: Note results**

CLI parsing and command dispatch typically have low testability via unit tests. Survived mutants here are expected — note them but don't treat them as high priority compared to `common` survivors.

---

## Task 6: Final Triage and Commit

- [ ] **Step 1: Confirm all survived mutants are logged**

```bash
for pkg in common storage web server; do
  echo "=== $pkg ==="
  grep -c '"summary": "survived"' mutants-$pkg.out/outcomes.json 2>/dev/null || echo "0 (run not complete)"
done
echo "=== beads issues created ==="
bd list --status=open | grep "Test gap:" | wc -l
```

The beads issue count should be close to (but may be less than) the total survived count — related survivors in the same function can be grouped into one issue.

- [ ] **Step 2: Commit the mutants config**

```bash
git add .cargo/mutants.toml
git commit -m "mutants: add .cargo/mutants.toml configuration"
```

---

## Interpreting Results: Quick Reference

| Situation | Meaning | Action |
|-----------|---------|--------|
| `survived`: `replace fn -> bool with true` | Function always returns true and tests pass | Add test asserting the false case |
| `survived`: `replace fn -> String with ""` | Function ignores its input; empty string passes tests | Add test verifying non-empty output for given input |
| `survived`: `replace && with \|\|` | Boolean logic not fully covered | Add test for the "both conditions false" case |
| `survived`: `delete !` | Negation not verified | Add test for the inverted case |
| `survived`: `replace Ok(x) with Ok(Default)` | Return value shape not checked | Add assertion on the actual value returned |
| High survived rate in a file with many tests | Tests exist but don't assert on outputs | Review test assertions, add `.expect`/`assert_eq!` |

---

## Notes on Scope

- **postgres/**: Excluded entirely. `cargo nextest` uses `sqlite::memory:` — mutating postgres code produces a build that tests never exercise, so every postgres mutant would survive and create noise. Postgres coverage is the responsibility of the Nix VM e2e tests, which cargo mutants cannot drive.
- **web/src/pages/**: Client-side Leptos code (`view!` macros, `Effect::new`) is structurally untestable in the mutants harness. Unviable mutants here are expected.
- **Superfluous tests**: `cargo mutants` cannot directly identify superfluous tests (that requires per-test coverage attribution). The proxy is: tests in a file whose mutants all **survive** — the tests run but don't constrain behavior.
