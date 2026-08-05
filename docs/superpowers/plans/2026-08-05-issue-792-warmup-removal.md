# Plan — issue #792, second half: remove the warmup

Spec:
[`2026-08-04-issue-792-e2e-warmup.md`](../specs/2026-08-04-issue-792-e2e-warmup.md)
First half:
[`2026-08-04-issue-792-e2e-warmup.md`](./2026-08-04-issue-792-e2e-warmup.md)
Verdict:
https://github.com/jaunder-org/jaunder/issues/792#issuecomment-5186123216 For
agentic workers: drive with **`jaunder-iterate`**.

## Review header

**Goal.** Act on the measured verdict — delete the per-test warmup and
everything that existed only to serve it, then repair what its removal makes
wrong.

**The verdict this implements** (measured, not predicted): warmup costs 113
s/combo chromium and 139 s firefox, buys back 11.7 s and ~0 s. Median suite
duration falls 226.8→174.0 s (chromium) and 323.7→256.0 s (firefox). No
flakiness cost in either arm.

**Scope — in:** delete `warmupPageContext`, the dead `maybeWarmupPage`, the
`e2e.warmup` span, three `JAUNDER_E2E_WARMUP*` env vars, and the `e2eWarmup`
scaffolding; rename the now-misnamed `warmupEnv` parameter and the `-cold`
package family (**keep and rename** — the workers=1 isolation is real and #801
needs it); narrow the guard to `e2eSalt`; update CONTRIBUTING.md,
docs/observability.md, ADR-0096; record the decision as an ADR draft. **Scope —
out:** `e2eSalt` itself (permanent tooling, stays); the remaining envelope
(#819); firefox's tax (#818); backend comparison (#817); #801.

**Tasks.**

1. ADR draft — "the e2e suite does not pre-warm" (record the decision while the
   rationale is fresh).
2. Delete the fixture-side warmup (`fixtures.ts`) — the code, the span, the env
   vars, the dead export.
3. Delete the flake-side warmup: the `e2eWarmup` literal and its token; rename
   `warmupEnv` → `extraEnv`.
4. Rename the `-cold` family to name what it actually is, and
   `traces run --cold` with it.
5. Narrow the `e2e-scaffold` guard to `e2eSalt` only; update its tests.
6. Repair the prose: CONTRIBUTING.md, docs/observability.md, ADR-0096, and the
   `#681` orphan-bucket comments that explain themselves in terms of warmup.
7. Verify: full `cargo xtask validate` (all four e2e combos) — the deletion must
   be behaviourally identical to arm B, and only a real run proves it.
8. Ship: `jaunder-ship` — final review, archive planning docs, PR, **merge waits
   for the user**.

**Key risks / decisions.**

- **The deletion is not the same change arm B tested.** Arm B disabled the
  warmup via the env flag; this removes the code path. Expected to be identical,
  but task 7's real e2e run is what establishes it — do not skip it on the
  strength of the A/B.
- **`e2e.warmup` disappearing is a trace-schema change.** #794 documents that
  span in `docs/observability.md`'s span tree; anything analysing it must
  tolerate its absence. Arm A traces in the store still contain it — that's
  fine, they're historical.
- **The orphan bucket (#681) may still be needed** for non-warmup run-level
  traffic. Removing warmup does not automatically make it dead code; check
  before touching, and only fix the _comments_ if the mechanism still earns its
  place.
- Renaming a flake attr family is user-visible for anyone with muscle memory for
  `traces run --cold`. The old name must not linger in docs.

## Global constraints

- **No `Co-Authored-By` trailer.** Stage, then commit; the pre-commit hook runs
  the full `cargo xtask check`.
- All commands pinned:
  `devtool run --cwd /home/mdorman/src/jaunder/.claude/worktrees/issue-792-e2e-warmup -- <cmd>`.
- Markdown via the devShell's `prettier` (never `npx`).
- Every commit references `(#792)`.
- `e2eSalt` stays at `""`; the `e2e-scaffold` guard still enforces it.

## File structure

| File                                                             | Change                                                       |
| ---------------------------------------------------------------- | ------------------------------------------------------------ |
| `docs/adr/drafts/<slug>.md`                                      | new numberless ADR draft (numbered at ship)                  |
| `end2end/tests/fixtures.ts`                                      | delete warmup fn, wrapper, span, env parsing                 |
| `flake.nix`                                                      | drop `e2eWarmup`; rename `warmupEnv`; rename the cold family |
| `xtask/src/traces/run.rs`, `lib.rs`                              | rename the `--cold` flag and `e2e_attr`'s parameter          |
| `xtask/src/steps/e2e_scaffold_check.rs`                          | narrow to `e2eSalt`; update tests                            |
| `CONTRIBUTING.md`, `docs/observability.md`, `docs/adr/0096-*.md` | prose repair                                                 |

---

## Task 1 — ADR draft: the e2e suite does not pre-warm

Spec D9 deferred the ADR to the verdict; the verdict is in. Use
**`jaunder-adr`** (numberless draft in `docs/adr/drafts/`, numbered by
`cargo xtask adr promote` at ship).

- [ ] **1a.** Draft `docs/adr/drafts/e2e-does-not-pre-warm.md` recording: the
      decision (no per-test warmup), the measured basis (link the verdict
      comment and `docs/observability.md`'s findings section), and the
      consequence that every test's first navigation is now genuinely cold —
      which is the honest state, and which the trace now shows rather than
      hides.
- [ ] **1b.** State the superseding relationship to ADR-0096 explicitly if that
      ADR's warmup rationale is materially affected (it discusses warmup traffic
      and the orphan bucket) — supersede only if it is actually wrong now, not
      merely dated.

**Verify:** draft exists, cites measured numbers rather than asserting them.

---

## Task 2 — Delete the fixture-side warmup

`end2end/tests/fixtures.ts`. Remove, checking each for other callers first:

- [ ] **2a.** `warmupPageContext` (`:234-261`) and its call in `_autoPerfSpan`
      (`:590`).
- [ ] **2b.** `maybeWarmupPage` (`:263-268`) — **already dead** (zero callers
      since `b6451579`), so this is cleanup the deletion makes obvious rather
      than a behaviour change.
- [ ] **2c.** The `e2e.warmup` span emission (`:939-960`) and the
      `warmupStartMs`/`warmupEndMs` plumbing.
- [ ] **2d.** `defaultWarmupUrl`, `defaultWarmupTimeoutMs`,
      `parseWarmupTimeoutMs`, and the reads of `JAUNDER_E2E_WARMUP`,
      `JAUNDER_E2E_WARMUP_URL`, `JAUNDER_E2E_WARMUP_TIMEOUT_MS`. Grep the whole
      repo for each name — the env vars are documented in prose too (task 6).
- [ ] **2e.** **Do not disturb the auto-fixture registration order.**
      `fixtures.ts:418-423` warns it is load-bearing and fragile: Playwright
      sets auto fixtures up in registration order, and reordering silently
      collapses `e2e.context_mint` to zero width. Removing code _inside_
      `_autoPerfSpan` is safe; changing the key order is not.
- [ ] **2f.** `tsc` clean: `devtool run -- tsc` (or the gate's `tsc` step).

**Verify:** no `warmup` identifier remains in `end2end/`; `cargo xtask check`
green; the span tree now has no `e2e.warmup` node.

---

## Task 3 — Delete the flake-side warmup and rename `warmupEnv`

- [ ] **3a.** Remove the `e2eWarmup` literal and its comment; in `e2eWarmChecks`
      the env string becomes just `" JAUNDER_E2E_RETRIES=1"`. Keep the
      `RETRIES=1` comment — it explains a policy that still holds.
- [ ] **3b.** Rename the `mkE2eCombo` / `mkE2e*Check` / `e2eRunAndCapture`
      parameter `warmupEnv` → `extraEnv`. It was already a misnomer (the cold
      family used it for `WORKERS=1`, per its own apologetic comment at
      `flake.nix:947-948`); with warmup gone the name would be pure
      misdirection. The `e2eSalt` splice rides this same parameter — keep that.
- [ ] **3c.** Re-check the salt still works after the rename:
      `e2eSalt = "probe"` moves all eight attrs; empty is a no-op against the
      new baseline. (The baseline moves in this commit — that is expected and
      fine, since the warmup token genuinely changed. Record the new hashes.)

**Verify:** `rg -i warmup flake.nix` returns nothing; salt still busts.

---

## Task 4 — Rename the `-cold` family for what it is

With warmup gone, every gate run's first navigation is cold, so "cold" no longer
distinguishes anything — the family's only remaining difference is `WORKERS=1`.
**Decision: keep and rename** (confirmed with the user); the single-worker
isolation is genuinely useful and #801 needs it for clean per-navigation
attribution.

- [ ] **4a.** Rename `e2eColdPackages` → `e2eSingleWorkerPackages`, attrs
      `e2e-<backend>-<browser>-cold` → `e2e-<backend>-<browser>-single-worker`,
      and `nameSuffix` to match. Chosen over `-solo`/`-serial` for being
      unambiguous at a glance; these attrs are typed rarely, so length is cheap.
- [ ] **4b.** Rewrite the family's comment (`flake.nix:957-965`): drop
      "cold-cache variants (no warmup)", state the real purpose — one worker, so
      per-navigation timings are free of contention — and keep the `#61`
      VM-memory and `#270` budget-scale notes, which are still true.
- [ ] **4c.** `xtask`: rename the `traces run --cold` flag to `--single-worker`
      and `e2e_attr`'s `cold: bool` parameter with it
      (`xtask/src/lib.rs:335-353`, `traces/run.rs:29-40`, and the `after_help`
      example). Update the doc comment quoting the old attr paths.
- [ ] **4d.** `rg -n 'cold' xtask/ flake.nix docs/ CONTRIBUTING.md` and fix
      every surviving reference that means the variant (leave genuine
      cache-warmth language like `cacheWarmth: "cold"` alone — that is a real,
      still-accurate concept, and after this change the gate produces _more_ of
      it).

**Verify:** `nix eval` resolves the new attr names and not the old;
`traces run --single-worker` builds them.

---

## Task 5 — Narrow the guard to `e2eSalt`

- [ ] **5a.** Remove the `e2eWarmup` clause from `problems()` and its
      missing-literal check; keep the salt clause and the "missing literal fails
      loudly" behaviour for the salt.
- [ ] **5b.** Update the unit tests: drop `flags_disabled_warmup` and the warmup
      half of `flags_both_when_both_set`; keep the comment-mention test (the
      salt's comment block still names it).
- [ ] **5c.** Update the module doc comment — it currently explains both
      literals.
- [ ] **5d.** `cargo test --manifest-path xtask/Cargo.toml e2e_scaffold`.

**Verify:** guard still fails a salted tree, passes clean.

---

## Task 6 — Repair the prose

- [ ] **6a.** `CONTRIBUTING.md:744-750` — the four gate checks are described as
      running "with `JAUNDER_E2E_WARMUP=1` (default)". Remove that clause.
- [ ] **6b.** `CONTRIBUTING.md` — the cold-package list (`:757-764`) gets the
      new names and the new explanation; the `#792` scaffolding section loses
      its `e2eWarmup` paragraph and keeps the salt.
- [ ] **6c.** `docs/observability.md` — the span tree (`:38-46`) loses the
      `e2e.warmup` line and its "only when JAUNDER_E2E_WARMUP is on" note; the
      spans list (`:24-26`) loses `e2e.warmup`; the warmup semantics paragraph
      (~`:601-615`) goes; `:294-295` and other `--cold` references get the new
      flag name. **Leave the #792 findings section intact** — it is a historical
      record of a measurement, and its `e2e.warmup` figures were true when
      taken.
- [ ] **6d.** `docs/adr/0096-e2e-trace-capture-vs-attribution.md` — it states
      the warmup's duration "is measured nowhere, which is what blocks #792".
      That is now resolved and the warmup is gone; update the rationale to past
      tense rather than deleting the reasoning, and cross-reference the new ADR.
- [ ] **6e.** The `#681` orphan-bucket comments
      (`xtask/src/server_fn_coverage/snapshot.rs:157,457`,
      `extract.rs:96,179,372`) explain orphan traffic _in terms of warmup_.
      **First establish whether the bucket still has a source** — if other
      run-level traffic still lands there, keep the mechanism and fix only the
      explanation; if warmup was its only source, that is a finding worth its
      own issue rather than an opportunistic deletion here.

**Verify:** `rg -in 'warmup' -- docs/ CONTRIBUTING.md xtask/ end2end/ flake.nix`
returns only the historical findings section and ADR history.

---

## Task 7 — Verify with a real run

- [ ] **7a.** `cargo xtask validate` (full, all four e2e combos) in **background
      mode**. This is the task that proves deleting the code is equivalent to
      arm B's env-flag flip — the A/B does not establish it.

      **Expect `server-fn-coverage` drift, and treat it as evidence rather than
      noise.** That snapshot is byte-compared against what a real run produces,
      but only in the **e2e lane** (`from_capture`), so `check` cannot see it and
      this is the first place it surfaces. Its committed orphan entries are the
      app-shell fns hit by the warmup's `/` load (`snapshot.rs:155-157`,
      `:455-463`); with the warmup gone that traffic no longer exists, so those
      entries should vanish. If they do, that **answers task 6e**: the warmup was
      the orphan bucket's source for those fns. Regenerate, commit the new
      snapshot, and write the observability doc's "expect exactly these orphans"
      paragraph from the regenerated truth rather than from a prediction.

- [ ] **7b.** Compare the four combo durations against arm B's medians (chromium
      ~174 s, firefox ~256 s sqlite). A material regression means the deletion
      did something the flag did not, and is a stop-and-diagnose.
- [ ] **7c.** Confirm `expected = 130`, `unexpected = 0` per combo.

**Verify:** validate green; durations within noise of arm B.

---

## Task 8 — Ship

- [ ] **8a.** `jaunder-ship`: final review of `main...HEAD`,
      `cargo xtask adr     promote` for the ADR draft, archive the
      spec/plans/notes, push, open the PR.
- [ ] **8b.** **HALT for merge.** Review gates the merge (CONTRIBUTING.md, as
      corrected this session) — `cargo xtask pr land` is the approval event and
      is the user's call.
- [ ] **8c.** On merge, release #792 to **Done** in project #1.

## Self-review

- Every element of the verdict's "what lands next" paragraph maps to a task:
  fixture code → 2, env vars → 2d, span → 2c, `e2eWarmup` → 3a, `warmupEnv`
  misnomer → 3b, cold family + `--cold` → 4, docs → 6, ADR-0096 → 6d.
- Task 1 (ADR) is first so the rationale is recorded while fresh, per the spec's
  "record decisions now, ship only backstops".
- No task touches `e2eSalt`, #819/#818/#817 scope, or #801.
- Task 6e is deliberately conditional — it says "establish whether", not
  "delete", because removing a mechanism whose remaining justification is
  unverified is exactly the kind of opportunistic change that should be its own
  issue.
