# Issue #745 — the server-fn coverage snapshot must be deterministic

**Status:** spec, awaiting approval (revised after cold soundness review)
**Issue:** [#745](https://github.com/jaunder-org/jaunder/issues/745) **Refs:**
[#681](https://github.com/jaunder-org/jaunder/issues/681) (the gate),
[ADR-0081](../adr/0081-empirical-server-fn-flow-coverage.md),
[#740](https://github.com/jaunder-org/jaunder/pull/740) (where it surfaced, and
whose `6b0fad2c` workaround this cycle undoes)

## Problem

`docs/coverage/server-fns.json` is byte-compared against a value regenerated
from an e2e run's traces, but that value is **not reproducible for a given
commit**. Two executions of the same combo on the same tree produce different
`covered` sets, so `server-fn-coverage-verify` can fail with no code change, and
no regeneration converges — whichever capture you commit, the other environment
produces the other answer.

### What the issue got wrong, and why it changes the fix

#745 states the attribution is not merely unstable but **wrong** — that the
snapshot "is binding a `posts::create` hit from a _different_ test" to a test
that never creates a post. **That is not what happens.**

In every capture examined, each `/api/` hit chains to the test whose browser
context actually issued the request. There is no cross-test leak. What varies is
**how far a test's page gets before the run tears down**:

- `authed-flash.spec.ts:64` asserts a redirect (`waitForURL(/\/app$/)`) and
  ends. Its page is still booting WASM.
- The booted client then fetches the cockpit's data. Those requests carry that
  test's `traceparent`, so they attribute to that test — correctly — but they
  start _after_ its `e2e.test` span has closed.
- The boot is **progressively truncated** at a different point each run.
  Requests from that test, relative to its own span end:

  | run | how far the `/app` boot got                                                                                               |
  | --- | ------------------------------------------------------------------------------------------------------------------------- |
  | 1   | `auth/get_session` +426 ms, `backup/is_warning_visible` +435 ms, `site/is_base_url_warning_visible` +438 ms — stops there |
  | 0   | …plus `timeline/list_home_feed` +247 ms                                                                                   |
  | 2   | …plus `posts/get_default_audience_selection` +404 ms                                                                      |
  | 3   | …plus `posts/get_default_audience_selection` +389 ms                                                                      |

Only two fns flap, and the reason is precise: `timeline::list_home_feed` and
`posts::get_default_audience_selection` are the only fns this test hits
**exclusively** after its span closed. It hits `auth::get_session` post-test
too, but also at −1275 ms during `register`, so that pair is already recorded
and cannot flap.

The test really did drive those fns, via its page's boot, after its assertions.
The `posts::create` in the issue's diff hunk is a **mislabelled key**. The real
delta introduced by `6b0fad2c` is exactly **one pair** across the whole file:

```
posts::get_default_audience_selection
  + "owner: jaunder_home_redirect='app' makes the pre-paint script redirect / → /app"
```

`posts::create` does not carry that title on `19368ffe` or on `6b0fad2c`. On
`19368ffe` the redirect test's title appears under seven keys —
`auth::get_session`, `backup::is_warning_visible`, `registration::get_policy`,
`registration::register`, `site::is_base_url_warning_visible`,
`timeline::list_home_feed`, `timeline::list_local_timeline` — and
`posts::get_default_audience_selection` is _not_ among them, because its
appearance there **is** the instability.

So the defect is narrower than filed and needs no propagation fix: **the
snapshot byte-compares a timing-dependent observation of a concurrent system.**

### Why it never converges locally

The e2e check is a Nix derivation. A second `cargo xtask e2e sqlite chromium` on
an unchanged tree completed in **4 s** — it replayed the cached output. A dev
box therefore produces one capture and keeps replaying it, while CI's first
build produces its own. Regenerating from either can never agree with the other.
This is mechanism #745 inferred but did not identify.

### What is stable and what is not — measured

Four forced re-executions of `checks.x86_64-linux.e2e-sqlite-chromium` on this
tree (one baseline build plus three `nix build --rebuild`), each capture fed
through the **real** `cargo xtask server-fn-coverage regenerate`:

| quantity                    | run 0 | run 1 | run 2 | run 3   |
| --------------------------- | ----- | ----- | ----- | ------- |
| covered fn keys             | 54    | 54    | 54    | 54      |
| orphan keys and reason sets | equal | equal | equal | equal   |
| `(fn, test)` pairs          | 963   | 962   | 964   | 964     |
| whole file byte-identical   | \-    | no    | no    | = run 2 |

54 covered + 1 allowlisted (`sessions::revoke`) = the 55-fn inventory.

**The covered key set and the orphan reason sets are identical across every run.
Only the title sets move — by a single title.** The orphan reasons are stable by
construction, not by luck: the only reason observed is `unknown-parent:<id>`
naming the run-wide traceparent, and `flake.nix:942-944` builds that id from a
constant digit (`traceDigit`, `flake.nix:924`).

The committed snapshot (CI-derived, via `6b0fad2c`) is byte-identical to derived
runs 2 and 3. It was never _wrong_ — it is one of the outcomes.

**Projecting each of the four derived snapshots onto the format D2 proposes
yields byte-identical files** (`sha256:2a3ee3af3ea69702…`). That is the direct
evidence for D1.

## Decisions

### D1 — Compare what the gate asserts; stop comparing the evidence

`verdict()` decides red or green from `snapshot.covered.contains_key(qualified)`
alone (`snapshot.rs:125`); its only other read is `snapshot.covered.keys()` for
staleness (`snapshot.rs:152`). It has never read a test title. `check()` reads
`.len()` (`server_fn_coverage_check.rs:72`); the seed test reads `.keys()`. So
the byte-compare in `regenerate_or_verify` is the _only_ thing that makes 964
titles load-bearing for the build — and they are the one part measured to be
irreproducible.

The compared artifact therefore carries the covered fn keys and the orphan
reason sets; the titles move to a file that is regenerated but never compared.

This is the argument `Coverage::orphans` already makes for storing reason sets
rather than counts (`extract.rs:95-100`): a count "changes whenever anyone adds
or removes an e2e test anywhere in the suite … counts would make this artifact
churn on unrelated PRs and go spuriously red." The title sets have the same
property and did not get the same treatment.

### D2 — `docs/coverage/server-fns.json` becomes keys + orphan reasons

```json
{
  "covered": ["audiences::create", "auth::get_session", "..."],
  "orphans": { "auth::get_session": ["unknown-parent:1111111111111111"] }
}
```

`covered` becomes a sorted array of `<vertical>::<ident>`. It stays
byte-compared exactly as today, preserving "a hand-edit that happens to parse
equal is still drift" — what makes the committed artifact provably what
regeneration produces.

**What the compared file still catches, after the shrink.** Worth stating so the
next simplification has to argue with it:

- The static lane already catches inventory↔snapshot mismatch (a new
  `#[server]` fn with no flow; a snapshot key that is no longer a fn).
- The e2e byte-compare uniquely catches **a fn that stops being driven by any
  test** — delete the last test exercising it and the derived key set shrinks,
  which the static lane cannot see because it never runs the suite.

That second property is the whole point of the e2e lane and survives intact,
because it is a statement about _keys_.

### D3 — `docs/coverage/server-fns-evidence.json` holds the titles, uncompared

```json
{ "covered": { "posts::create": ["authenticated user can create a post…"] } }
```

Committed, so a reader on GitHub can still see which flows drive each fn — the
property ADR-0081 leans on. Rewritten by `regenerate`. Never compared, so a racy
title can never redden a build.

**Rendering contract, identical to the compared file's:** `BTreeMap`/`BTreeSet`
ordering, `serde_json::to_string_pretty`, one trailing newline. It is not
compared, but an unstable rendering would produce spurious _git_ diffs on every
regenerate with no gate to catch them — the same reason `render()` exists at all
(`snapshot.rs:80-87`).

### D4 — The evidence file's key set is cross-checked, in the static lane

An uncompared committed file can go stale silently. `verdict` therefore also
fails when the evidence file's key set differs from **the compared file's
`covered` array** — not its `orphans` keys. Today every orphan key is also a
covered key, so the distinction is latent; it stops being latent the moment a fn
is hit only during the `_autoPerfSpan` warmup, which yields an orphan key and no
covered key. The evidence file mirrors `covered`, and only `covered`.

The check is deterministic — the key sets are measured stable across all four
runs — and needs no capture, so it belongs in the **static lane** (`check`,
`validate --no-e2e`) for immediate feedback rather than behind the e2e matrix.

**Its limits, stated rather than implied.** It catches a partial write or a bad
merge resolution, and an evidence file predating a newly covered fn. It does
**not** catch the staleness that will actually happen most often: titles that
went stale because tests were renamed or deleted while the key set stayed the
same. After this change the evidence file is committed, uncompared, and guarded
only on its key set, so its title lists can rot and stay green. ADR-0081 leans
on this artifact for "flow documentation can state coverage as a checked fact
rather than a promise"; the _fact_ half survives in the compared file, the
_which flows_ half becomes a promise again. That is a real cost of this design,
accepted knowingly, and it is what the out-of-scope issue below exists to
revisit.

### D5 — No time-window rule. Rejected with evidence, not on taste

The obvious-looking fix is to refuse to attribute a hit whose span began after
its test's span closed — "the test drove this while it was running". It was
implemented against all four captures and **does not work**:

- It removes the wide-margin class (the redirect test's `/app` boot traffic):
  all four runs then agree there.
- It **exposes** a narrow-margin one. `tags::list` for "TagInput: invalid tag
  text shows an error" starts at **+31 ms** in run 0 and **−135 / −88 / −90 ms**
  in runs 1–3. It straddles the end stamp by tens of milliseconds.
- Derived title sets remained non-identical across the four runs.

The rule relocates the race from "did it beat teardown" to "did it beat the end
stamp" — decisive at 400 ms, a coin flip at 30 ms. It also drops real hits
arbitrarily: run 0's `tags::list` evidence vanishes while runs 1–3 keep theirs.
With titles uncompared it buys nothing, so it is not shipped.

ADR-0081 independently rejects time-window correlation for the same underlying
reason ("the suite runs `fullyParallel` and the windows overlap"), which is
reinforcement rather than a new argument.

Harness-side variants were considered and rejected: closing the page before
stamping the span end breaks Playwright's failure-artifact capture (its `page`
fixture teardown runs after ours); waiting for quiescence costs teardown latency
on ~118 tests and never settles for a polling page; blocking post-body requests
discards genuinely in-flight evidence.

**This decision's evidence is not preserved in the tree.** The prototype was
throwaway and the four captures (~2.2 MB compressed each, ~26 MB of JSONL once
extracted) are not committed. A future reader wanting to reopen the time-window
idea — and they will, because it is the obvious fix — has the method (below) but
must re-run it. AC9 mitigates this for D1, which is the claim that actually
gates the build; D5 is recorded honestly as
argued-from-evidence-since-discarded.

### D6 — The `6b0fad2c` workaround is undone by regenerating normally

`6b0fad2c` ("take the CI-derived snapshot to work around #745") is on `main` via
#740. It is labelled as a workaround, but left in place it reads as a legitimate
baseline. Both artifacts are regenerated from a real local capture in this
cycle, which is only meaningful because the compared file now converges.

### D7 — ADR-0081 is amended, not superseded

This change falsifies two things ADR-0081 already decided: that the snapshot is
"regenerated (fail-on-drift) in the e2e lane" as a single file, and its
Consequence that title-edit churn is "accepted as the signal working, **confined
to one file**." ADR-0081 is still `Status: proposed`, so amending it in place is
cleaner than a superseding ADR that would split the rationale across two
documents. The amendment records D1, D3, D4's limits, and D5 — including that no
misattribution exists, so a future reader does not go hunting for a propagation
bug.

## Acceptance criteria

- **AC1** `docs/coverage/server-fns.json` contains no test titles. `covered` is
  a sorted array of qualified names; `orphans` keeps its current shape. Because
  a title and a qualified name are the same type, the guard is a **shape**
  assertion: a unit test fails if any `covered` entry does not match
  `^[a-z_][a-z0-9_]*::[a-z0-9_]+$` (no spaces, exactly one `::`).
- **AC2** `docs/coverage/server-fns-evidence.json` is committed, renders under
  D3's contract (sorted keys, sorted titles, pretty-printed, trailing newline),
  and maps every key in the compared file's `covered` array to that fn's titles.
- **AC3** `server-fn-coverage-verify` fails on **any** byte difference in the
  compared file. **No such unit test exists today** — the byte-compare branch is
  only reachable through `regenerate_or_verify`, which requires a real capture
  tarball (`server_fn_coverage_check.rs:106`). This AC therefore requires both a
  new test and whatever seam makes the compare reachable without a tarball.
- **AC4** `check` fails when the evidence file's key set differs from the
  compared file's `covered` array, in either direction, naming the offending
  keys and the remedy. Unit-tested from fixture files, both directions. The
  message must state the remedy as the two steps it is — a capture-producing e2e
  run, then `REGENERATE_CMD` — since `cargo xtask server-fn-coverage regenerate`
  fails immediately without a capture (`io.rs:68-74`).
- **AC5** A missing or unparseable evidence file is a **failure**, never a pass
  — the same fail-closed rule the compared file has, and deliberately _not_ the
  `read_allowlist` template (`io.rs:97-100`), where missing means empty means
  pass.
- **AC6** The static lane's existing verdicts are unchanged: an uncovered fn
  fails, an allowlisted one passes, a stale allowlist entry fails, a snapshot
  key that is not a `#[server]` fn fails. Their fixtures must be rewritten for
  the new `covered` type — `covered_with()` (`snapshot.rs:202-210`) and the
  three inline JSON fixtures at `server_fn_coverage_check.rs:198, 215, 229`.
- **AC7** The seed-capture tests still pass:
  `every_allowlist_entry_is_absent_from_the_seed_captures_hit_set`,
  `each_signal_finds_fns_on_its_own_in_the_real_capture`, and
  `seed_capture_covers_the_committed_snapshots_fns` — the last of which does
  **not** compile unmodified, since it calls `snapshot.covered.keys()`
  (`server_fn_coverage_check.rs:362`) on what becomes a `Vec`.
- **AC8** Both artifacts are regenerated from a capture produced on this branch,
  undoing `6b0fad2c`, and the committed compared file is byte-identical to what
  `regenerate` produces from that capture. (Re-running
  `cargo xtask e2e sqlite chromium` afterwards is _not_ evidence — it replays
  the cached derivation in 4 s and asserts a file equals itself.)
- **AC9** Determinism is demonstrated by a **repo-resident** test, not by a PR
  body. The three distinct full derived snapshots from the four forced
  re-executions are committed as testdata, and a unit test asserts that
  projecting each onto D2's format yields byte-identical output. This converts
  "trust the measurement" into a check anyone can run in milliseconds, following
  the precedent the reduction script sets
  (`server_fn_coverage_check.rs:308-313`: committed "so a reader can regenerate
  the fixture and diff instead of taking it on trust"). The captures themselves
  (~2.2 MB compressed, ~26 MB of JSONL once extracted) are not committed; the
  spec records the method — one `nix build` plus three `nix build --rebuild` of
  `.#checks.x86_64-linux.e2e-sqlite-chromium`, regenerating from each output's
  `capture-sqlite.tar.gz`.
- **AC10** `CONTRIBUTING.md:533-534,545,548` and
  `docs/observability.md:71-72, 124-125` describe both files and state which one
  the gate compares. `docs/observability.md:71`'s table row ("server fn → the
  named tests that drove it") becomes wrong on landing and must change, not
  merely gain a sibling.
- **AC11** ADR-0081 is amended per D7.
- **AC12** #745 is answered with the corrected mechanism before the PR merges —
  including the one-pair `6b0fad2c` delta above — so "the attribution is wrong"
  is not left standing as repo history.

## Out of scope — filed separately

**Should the evidence file carry per-test titles at all?** The split removes the
_gate_ consequence of the 964-title list, but the evidence file still rewrites
~66 KB whenever anyone adds, renames, or deletes an e2e test, and (per D4) its
titles can rot unnoticed. That is a question about what evidence is worth
committing and how it stays true — it deserves deciding on its own merits rather
than as a rider on a determinism fix.
