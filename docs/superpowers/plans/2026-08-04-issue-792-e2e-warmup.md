# Plan — issue #792, first half: measure the per-test e2e warmup

Spec:
[`docs/superpowers/specs/2026-08-04-issue-792-e2e-warmup.md`](../specs/2026-08-04-issue-792-e2e-warmup.md)
Issue: https://github.com/jaunder-org/jaunder/issues/792 (parent #788, lever 2)
For agentic workers: drive with **`jaunder-iterate`**; delegate individual tasks
via **`jaunder-dispatch`** where useful.

## Review header

**Goal.** Build the cache-busting scaffolding (`e2eSalt` / `e2eWarmup` literals
in `flake.nix`, plus a static guard against committing either), then run a 6-run
interleaved A/B on a quiescent host and record a verdict on whether the per-test
warmup earns its place. No warmup change lands this half.

**Scope — in:** `flake.nix` literals + threading; one new xtask static-check
step; `CONTRIBUTING.md` docs; the collection; a findings section in
`docs/observability.md`; the verdict comment; three follow-up issues. **Scope —
out:** deleting/keeping/conditionalising the warmup (second half, after the
checkpoint); any change to the cold package family; `#801` app-side mount cost;
worker/VM sizing; the `RETRIES=1` policy.

**Tasks.**

1. File the three follow-up issues (spec AC-8).
2. Record baseline `drvPath` for all eight e2e attrs — **before any `flake.nix`
   edit** (AC-2 evidence).
3. Add `e2eSalt` + `e2eWarmup` literals and thread them (AC-1, AC-3); prove the
   defaults are a byte-exact no-op (AC-2).
4. Add the `e2e-scaffold` guard step, tests first, wired into `check`/`validate`
   (AC-4).
5. Document the salt in `CONTRIBUTING.md` (AC-5).
6. Gate and commit the scaffolding.
7. Run the 6-run interleaved collection (AC-6). **Long-running; host must stay
   quiescent.**
8. Extract metrics and write the findings section in `docs/observability.md`
   (AC-6).
9. Post the verdict comment on #792 applying D8's rule (AC-7, AC-9).
10. Checkpoint: HALT and re-plan the second half against observed numbers.

**Key risks / decisions.**

- Task 2 is **order-critical**: once `flake.nix` is edited the baseline is
  unrecoverable without `git stash`/checkout gymnastics. Do it first, commit the
  recorded hashes into the plan's notes.
- The guard must **not** run inside the e2e derivations (AC-4 clause 2) — a
  guard wired into the e2e checks would pass the obvious test while making every
  salted measurement impossible.
- Task 7 is hours of wall-clock the agent cannot compress, and its validity
  depends on the human not using the box. Confirm before starting.
- If task 3's no-op proof fails, **stop** — an unintended rehash means the
  splice reached something it shouldn't, and every later measurement would
  rebuild the world.

## Global constraints

- **No `Co-Authored-By` trailer** on any commit.
- Stage, then commit (never `git commit -- <paths>`); the pre-commit hook runs
  the full `cargo xtask check`, so run
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-792-e2e-warmup -- cargo xtask check`
  first and let it pass clean. See **`jaunder-commit`**.
- All commands pinned to the worktree:
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-792-e2e-warmup -- <cmd>`.
- Markdown formatted with the devShell's `prettier` (never `npx`).
- Every commit message references `(#792)`.
- **Reverting `flake.nix` — mechanism, not intention.** This worktree has an
  **auto-staging hook**: edits get staged automatically, so a bare
  `git checkout -- flake.nix` restores the _staged_ (salted) version, not
  `HEAD`. The plan flips these literals ~10 times across tasks 3, 4 and 7; done
  wrong, a salted or warmup-flipped `flake.nix` silently survives into a commit
  or a measurement arm. Always revert with **both**:

  ```
  git checkout HEAD -- flake.nix
  git reset HEAD -- flake.nix
  ```

  and verify with `git diff HEAD -- flake.nix` (empty), **not** `git status`,
  which the hook makes unreliable here.

## File structure

| File                                       | Change                                                                         |
| ------------------------------------------ | ------------------------------------------------------------------------------ |
| `flake.nix`                                | new `e2eSalt` / `e2eWarmup` literals; splice in `mkE2eCombo` + `e2eWarmChecks` |
| `xtask/src/steps/e2e_scaffold_check.rs`    | new guard step (pure `problems()` + `run()` + unit tests)                      |
| `xtask/src/lib.rs` (`mod steps`, `:24-50`) | register `pub mod e2e_scaffold_check;` (there is no `steps.rs`/`steps/mod.rs`) |
| `xtask/src/lib.rs`                         | call the step in the `check` and `validate` step lists                         |
| `CONTRIBUTING.md`                          | salt documentation (AC-5)                                                      |
| `docs/observability.md`                    | `#792` findings section (AC-6)                                                 |

---

## Task 1 — File the three follow-up issues

Separable concerns surfaced by the spec; filed first so they can be picked up
concurrently rather than blocked behind this cycle. Use **`jaunder-issues`**
(milestone "Test infrastructure & E2E", label `test-infra`; the Firefox one also
`observability`).

- [x] **1a.** → **#817** (P3). "e2e: compare sqlite vs postgres suite
      performance" — note that every `traces run` in #792 collected postgres
      traces at gate-identical settings, so the data exists; cite this plan's
      findings section.
- [x] **1b.** → **#818** (P2). "e2e/web: why is Firefox ~1.5× slower than
      chromium?" — size it from #794's per-phase boot breakdown across the four
      populations #792 collects. Note Firefox reports no long-tasks (no
      `longtask` PerformanceObserver), so that lens is chromium-only. Related to
      #801.
- [x] **1c.** → **#819** (P3). "e2e: re-baseline the remaining per-test envelope
      post-#791" — `e2e.context_mint`, fixture setup/teardown, span export: the
      part of #788's 28–31 % that is not warmup.

Each must be a sub-issue of #788 (`sub_issue_write`), matching #791–#795.

**Verify:** three issue numbers exist, each linked under #788, each citing #792.

---

## Task 2 — Record baseline `drvPath` for all eight e2e attrs

**Order-critical: nothing in `flake.nix` may change before this runs.** This is
AC-2's "before" half, and it is unrecoverable once the file is edited.

The eight attrs span two namespaces (there is no `checks.…-cold`):

- `checks.x86_64-linux.e2e-{sqlite,postgres}-{chromium,firefox}`
- `packages.x86_64-linux.e2e-{sqlite,postgres}-{chromium,firefox}-cold`

- [x] **2a.** Confirm the tree is clean of tracked modifications
      (`git status --porcelain`) — `nix eval` hashes tracked-and-modified files,
      so a stray tracked edit invalidates the comparison. Untracked files are
      fine.
- [x] **2b.** For each of the eight attrs, run `nix eval --raw .#<attr>.drvPath`
      and record the result verbatim in a scratch note under
      `docs/superpowers/plans/` (or the plan's own notes).
- [x] **2c.** Also record
      `nix eval --raw .#packages.x86_64-linux.jaunder.drvPath` — the spec claims
      `flake.nix` is outside the crane source filter, so this one must be
      **unchanged** after task 3 too. Verifying it here makes the session-length
      risk mitigation checkable rather than asserted.

**Verify:** nine recorded store paths, tree confirmed clean at capture time.

---

## Task 3 — Add the `e2eSalt` and `e2eWarmup` literals

Both live in the same `let` block that already holds `e2eCombos` / `mkE2eCombo`
(`flake.nix:896-980`), so they are in scope without threading new parameters
through `mkE2eCombo`'s signature.

- [x] **3a.** Declare the literals above `e2eCombos`, with comments stating what
      each is for, that `e2eSalt` must be reverted before committing, and that
      `xtask`'s `e2e-scaffold` guard enforces both:

      ```nix
      # Cache-busting salt for e2e measurement runs (#792). Nix caches the e2e
      # check derivations, so a repeated `cargo xtask traces run` returns a
      # CACHED result rather than re-running the suite — silently handing back
      # traces from whenever it was last built, possibly a CI runner. Set this to
      # a distinct value per measurement run to force a fresh build; REVERT TO ""
      # before committing. The `e2e-scaffold` xtask guard fails the gate if a
      # non-empty value is committed. Empty is a byte-exact no-op: it must not
      # change any e2e derivation hash.
      e2eSalt = "";

      # A/B scaffolding for #792: flip to false to build the no-warmup arm.
      # `true` must reproduce the historical `warmupEnv` string byte-exactly.
      # Guarded like e2eSalt — a committed `false` would silently disable warmup
      # on all four gate checks.
      e2eWarmup = true;
      ```

- [x] **3b.** Splice the salt inside `mkE2eCombo` so it reaches **both**
      families from one site. `warmupEnv` is already the generic extra-env
      string interpolated into the VM `testScript` (`flake.nix:615`), so
      appending to it reaches the derivation hash:

      ```nix
      mk {
        checkName = "jaunder-e2e-${backend}-${browser}${nameSuffix}";
        warmupEnv =
          warmupEnv
          + pkgs.lib.optionalString (e2eSalt != "") " JAUNDER_E2E_SALT=${e2eSalt}";
        inherit
          browser
          traceId
          traceParent
          vmMemory
          vmCores
          ;
      }
      ```

      (`warmupEnv` moves out of the `inherit` list; the env var itself is inert —
      nothing reads `JAUNDER_E2E_SALT`. Its only job is to change the hash.)

- [x] **3c.** Make the warm checks' `warmupEnv` respect `e2eWarmup`
      (`flake.nix:949`), keeping the existing comment:

      ```nix
      warmupEnv =
        pkgs.lib.optionalString e2eWarmup " JAUNDER_E2E_WARMUP=1"
        + " JAUNDER_E2E_RETRIES=1";
      ```

      With `e2eWarmup = true` this is `" JAUNDER_E2E_WARMUP=1 JAUNDER_E2E_RETRIES=1"`
      — byte-identical to today.

- [x] **3d. Prove the no-op (AC-2).** Re-run task 2b's eight `nix eval --raw`
      commands with both literals at their defaults. **All eight must equal the
      recorded baseline.** Re-run 2c: `jaunder.drvPath` must also be unchanged.
      If any differ, **stop and diagnose** — a rehash here means the splice
      reached more than intended.

- [x] **3e. Prove the salt works (AC-1).** Set `e2eSalt = "probe"`, re-run the
      eight evals: all eight must now **differ** from baseline. Revert to `""`.

- [x] **3f. Prove the warmup literal works (AC-3).** Set `e2eWarmup = false`,
      confirm the four warm-check hashes change and the four cold ones do not
      (cold never set the warmup token). Revert to `true`.

**Verify:** three recorded hash comparisons — defaults equal baseline, salted
all differ, warmup-flipped differs on the four warm attrs only.

---

## Task 4 — The `e2e-scaffold` guard step (tests first)

Follows the repo's pure-`problems()` + `run()` + in-file `#[cfg(test)]` pattern
— see `xtask/src/steps/no_full_reload_check.rs` for the shape to copy.

- [x] **4a.** Write `xtask/src/steps/e2e_scaffold_check.rs` with a **pure**
      function over `flake.nix`'s source text, and unit tests **first**:

      ```rust
      /// The failure detail when #792's measurement scaffolding is left set, or
      /// `None` when both literals are at their committed defaults. Pure given the
      /// `flake.nix` source, so it is unit-tested directly.
      pub fn problems(source: &str) -> Option<String>
      ```

      Tests (expected FAIL before the impl exists):

      - `e2eSalt = "";` + `e2eWarmup = true;` → `None`
      - `e2eSalt = "run1";` → `Some`, detail naming `e2eSalt` and #792
      - `e2eWarmup = false;` → `Some`, detail naming `e2eWarmup`
      - both set → `Some`, detail naming both
      - neither literal present at all → `Some` (a renamed/deleted literal must
        **fail loudly**, never silently disable the guard — same reasoning as
        `no_full_reload_check`'s missing-root hard failure)

      Run: `cargo test --manifest-path xtask/Cargo.toml e2e_scaffold` — expect
      FAIL, then PASS. **Not `-p xtask`**: root `Cargo.toml:14` has
      `exclude = ["xtask"]`, so xtask is a separate workspace and `-p xtask`
      resolves to nothing. The gate itself uses the `--manifest-path` form
      (`xtask/src/steps/host_tests.rs:12-17`).

- [x] **4b.** Add `run(result: &mut CommandResult)` reading `flake.nix`, pushing
      `StepResult::ok("e2e-scaffold")` / `.fail(...).detail(...)`. A missing or
      unreadable `flake.nix` is a hard failure.

- [x] **4c.** Register the module. There is **no `xtask/src/steps.rs` or
      `steps/mod.rs`** — steps live in an inline `mod steps { … }` block in
      `xtask/src/lib.rs:24-50`, alphabetically ordered. Add
      `pub mod e2e_scaffold_check;` between `doctest_fences` (`:28`) and
      `e2e_local` (`:29`). Then call
      `steps::e2e_scaffold_check::run(&mut result);` in **both** step lists —
      the `validate` list (`:455-475`, near `steps::no_full_reload_check::run`)
      and the `check` list (`:411-430`).

- [x] **4d. AC-4 clause 1.** Three runs: salted → non-zero naming `e2eSalt`;
      `e2eWarmup = false` → non-zero naming `e2eWarmup`; both at defaults →
      passes. Revert between each (see the revert mechanism below).

      **Command choice is not incidental.** `cargo xtask validate` runs
      `clean_tree_precheck` first and **returns early on a dirty tree**
      (`xtask/src/lib.rs:448-454`, `:712-728`). A salted `flake.nix` *is* a dirty
      tree, so plain `validate --no-e2e` fails on `clean-tree` and never reaches
      the guard — the message would name the dirty tree, not the literal, which is
      misleading evidence for AC-4. Use `cargo xtask validate --no-e2e
      --allow-dirty` as the direct AC-4 evidence path, and/or
      `cargo xtask check --no-test`, which has no clean-tree precheck. Record
      which was used.

- [x] **4e. AC-4 clause 2 — by construction, not by build.** The guard cannot
      run inside an e2e derivation: `flake.nix:266-272` excludes `/xtask/` from
      the source filter (with a comment saying exactly that — an accidental
      `cargo xtask` inside a derivation fails loudly rather than running stale),
      repeated at `:1112` and `:1161`; the step lives entirely under
      `xtask/src/**`, and no `testScript` invokes xtask. Record this argument
      plus a successful `nix eval` of a salted e2e attr's `drvPath` as the
      evidence.

      **Do not spend a real `nix build` VM run on this.** It purchases no
      information over the source-filter argument and competes directly with
      task 7's quiescence budget.

**Verify:** unit tests pass; 4d's three exit codes with the command recorded;
4e's argument recorded with a salted `drvPath` evaluation.

---

## Task 5 — Document the salt in `CONTRIBUTING.md`

- [x] **5a.** Add a short subsection near the existing e2e/Nix-VM material
      (`CONTRIBUTING.md:194` "Running e2e tests locally" / `:731` "Nix VM
      checks"): what `e2eSalt` is for (Nix caching silently returns stale suite
      results), how to use it (distinct value per measurement run), that it
      **must be reverted**, and that `e2e-scaffold` fails the gate otherwise.
      Mention `e2eWarmup` as #792-scoped scaffolding. Deliberately **not** in
      `docs/observability.md` — that file is about tracing; it gets the findings
      (task 8), not the mechanism. Place it so it does not contradict
      `CONTRIBUTING.md:744-750`, which currently describes all four gate checks
      as running "with `JAUNDER_E2E_WARMUP=1` (default)" — still true while
      `e2eWarmup = true`, but the new section must not read as if the flag were
      now optional.
- [x] **5b.** `devtool run -- prettier -w CONTRIBUTING.md`. (Done implicitly —
      `cargo xtask check` runs prettier in Fix mode and reformatted it.)

**Verify:** section present, both literals named, revert requirement explicit.

---

## Task 6 — Gate and commit the scaffolding

- [ ] **6a.** `devtool run --cwd <worktree> -- cargo xtask check` — green.
- [ ] **6b.** Confirm both literals are at their defaults (the guard now
      enforces this, which is itself the proof).
- [ ] **6c.** Stage, then commit, subject line exactly:
      `test-infra(e2e): salt/warmup scaffolding and its guard (#792)`

**Verify:** clean gate, one commit, no `Co-Authored-By`.

---

## Task 7 — Run the interleaved collection

**Long-running (likely multiple hours) and the only task whose validity depends
on the human.** Confirm the host can stay quiescent before starting.

Per spec D6: six runs in order `A1, B1, A2, B2, A3, B3`, each with a distinct
salt, arm B being `e2eWarmup = false`.

- [ ] **7a.** Record the session baseline `/proc/loadavg`.
- [ ] **7b.** For each run: set the two literals; record their values, the salt,
      and `/proc/loadavg`; execute
      `devtool run --cwd <worktree> -- cargo xtask traces run` in **Bash
      background mode**; on completion record `/proc/loadavg` again and the
      `nix build --print-out-paths` store path for each of the four combos.
- [ ] **7c.** Apply D6's discard rule: a run that aborts, or whose load
      materially exceeds baseline, is discarded with its reason recorded and
      re-run with a fresh salt. Do **not** silently renumber.
- [ ] **7d.** Restore both literals to their defaults when collection ends,
      using the Global-constraints revert mechanism (`git checkout HEAD`
      **plus** `git reset HEAD`), verified with `git diff HEAD -- flake.nix`.

**Verify:** six valid runs, twenty-four combo store paths, a discard log
(possibly empty), `git diff HEAD -- flake.nix` empty.

---

## Task 8 — Extract metrics and write the findings section

Extraction is a separate operator step: `traces run` deletes the `TempDir` it
extracts traces into (`xtask/src/traces/run.rs:64-112`) and nothing in xtask
reads a suite-level duration. Each run's distinct salt means its store path is
distinct, so all six runs' reports remain readable after the fact.

- [ ] **8a.** For each of the 24 combo store paths, `jq '.stats'` on
      `<out>/playwright-report-<backend>.json` → `duration`, `flaky`,
      `unexpected`.
- [ ] **8b. Re-obtain the trace files first.** `traces run` extracts into a
      `TempDir` it then deletes, so after a run there is nothing to hand to
      `traces analyze`. For each of the 24 combo store paths, extract
      `capture/otel-traces.jsonl` from `<out>/capture-<backend>.tar.gz` (the
      same member `run.rs:76` / `extract_trace` at `:136` pull), naming each
      file by run/arm/combo so the populations stay distinguishable.

- [ ] **8c. Aggregate the secondary metrics — hand-rolled, not
      `traces analyze`.** The spec's secondary set is `e2e.warmup` **p50** and
      per-combo total (arm A only); the envelope decomposition
      (`e2e.context_mint`, warmup, fixture setup, teardown/export) for both
      arms; and first-navigation `navigation.request` **p50** for both arms.

      `cargo xtask traces analyze` **cannot produce these**: it reports
      slowest-spans, per-project `e2e.test` averages, hotspot `max_ms` tables and
      #794 span coverage (`xtask/src/traces/analyze.rs:97-129`) — there is **no
      median/percentile aggregation anywhere** in `analyze.rs`/`render.rs`, and
      `navigation.request` appears only as a phase key reported as `max_ms`
      (`analyze.rs:286`). Aggregate over the JSONL directly (`ctx_execute` is the
      right surface for this — it is computation over data, not a command to
      observe). Record the aggregation code alongside the numbers so AC-6's
      "re-derivable" requirement covers the secondary metrics too, not just
      `.stats`.

      Run `traces analyze` as well for the coverage/hotspot context it *does*
      give, but do not source the spec's p50s from it.

- [ ] **8d.** Write the findings section in `docs/observability.md` following
      the existing convention (`:414` "#155 — post-CSR Firefox e2e tax
      (findings, 2026-07-02)", `:461`, `:506`): per-run table with run label,
      arm, salt, literal values, loadavg before/after, per-combo `.stats`, **and
      the exact extraction commands and aggregation code**, so a reviewer can
      re-derive both the medians and the secondary p50s. Discarded runs appear
      with their reason.
- [ ] **8e.** `prettier -w docs/observability.md`; gate; commit
      (`docs(observability): #792 warmup A/B findings (#792)`).

**Verify:** AC-6 satisfied — table complete, commands recorded, discards
visible.

---

## Task 9 — Post the verdict

- [ ] **9a.** Compute, per browser, over sqlite only: median suite duration per
      arm (n=3), and the **sum** of `flaky + unexpected` across each arm's three
      runs.
- [ ] **9b.** Apply spec D8 in order: flakiness veto first (arm B's sum > arm
      A's ⇒ keep warmup for that browser regardless of speed), then faster
      median wins. A split verdict is a legitimate outcome.
- [ ] **9c.** Comment on #792 with everything AC-7 requires — the medians, the
      flaky/unexpected sums, arm A's `e2e.warmup` cost, both arms' envelope
      decomposition, both arms' first-nav `navigation.request` p50 — plus, per
      AC-9, **the rule as stated in the spec and the numbers it was applied
      to**, including the veto check per browser. Link the findings section.

**Verify:** comment posted; a reader can re-apply the rule to the quoted numbers
and reach the same verdict.

---

## Task 10 — Checkpoint

- [ ] **10a.** HALT. Present the verdict and re-plan the second half against
      observed numbers — delete / keep / browser-conditional, with the footprint
      the spec sketches (fixtures, three env vars, the `warmupEnv` param's now-
      misleading name, the cold family's collapsed identity and the `--cold`
      flag, `CONTRIBUTING.md` — including `:744-750`, which describes all four
      gate checks as running "with `JAUNDER_E2E_WARMUP=1` (default)" —
      `docs/observability.md`, ADR-0096's rationale, and the dead
      `maybeWarmupPage` export).

`#792` stays open across this checkpoint.

## Self-review

- Every spec AC maps to a task: AC-1→3e, AC-2→2/3d, **AC-3→3d + 3f** (3d
  establishes the byte-identity AC-3 actually asks for; 3f only proves the
  literal is load-bearing), AC-4→4d/4e, AC-5→5, AC-6→7/8, AC-7→9c, AC-8→1,
  AC-9→9b/9c.
- The Nix mechanism (3b/3c) was verified **empirically** during plan review, not
  merely read: with the literals spliced at their defaults all eight e2e
  `drvPath`s are identical to baseline; with `e2eSalt = "probe"` all eight
  differ; with `e2eWarmup = false` the four warm attrs differ and the four cold
  ones do not. Untracked files do not affect `nix eval`. Tasks 3d/3e/3f are
  therefore expected to pass — if one does not, something else changed.
- Task 2 precedes any `flake.nix` edit; the plan says so twice because getting
  it wrong is unrecoverable.
- No task smuggles warmup removal — task 10 is a halt, not an implementation.
- The only task that cannot be verified by the agent alone is 7 (host
  quiescence), and it is flagged as requiring human confirmation before it
  starts.
