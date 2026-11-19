# Shard the Chromium E2E lanes (#1969, #1985)

## Outcome

Each backend's Chromium E2E suite stops running as one ~25-minute lane, which on
AWS-hosted runners finishes 16 s under, or past, its 1500 s Playwright cap.
Instead it runs as three isolated lanes, using the same topology as Firefox.
Every Chromium lane has clear headroom under the shared cap. Server-function
flow coverage is still derived from the complete SQLite/Chromium population and
compared with the same committed snapshot.

## Approach and tradeoffs

**Topology.** The Chromium catalog follows Firefox exactly. Per backend there
are three enabled lanes:

| Lane                                 | Playwright projects                                             |
| ------------------------------------ | --------------------------------------------------------------- |
| `<backend>-chromium-ordinary-1-of-2` | `chromium-ordinary`, `--shard=1/2`                              |
| `<backend>-chromium-ordinary-2-of-2` | `chromium-ordinary`, `--shard=2/2`                              |
| `<backend>-chromium-serial-special`  | `chromium-special-visual` → `-global-configuration` → `-invite` |

The new projects copy the Firefox lane projects. They use the same spec filters
as today's `chromium-visual`, `chromium`, `chromium-admin-site` (this includes
the Chromium-only `passkeys` spec) and `chromium-admin` projects. Their order
comes from `experimentalProjectDependencies`. `chromium-special-visual` reuses
the existing `chromium-visual` snapshot baselines through
`snapshotPathTemplate`, so no new baselines are added. The two
`*-chromium-unsplit` catalog lanes stay, but become `enabled: false`, like the
Firefox unsplit lanes. Their remaining consumer is the set of single-worker
diagnostic packages. The unsplit Chromium Playwright projects also stay, for
host-local `dev e2e-local`, production-baseline qualification and the
wasm-coverage producers. Total enabled lanes: 8 → 12, which uses up the
single-hex `traceDigit` space (16 values). Any future lane will need a format
change.

Rejected alternative: two lanes per backend (ordinary + serial-special). It adds
only two jobs, but the ordinary lane would run ~850 s on AWS instead of ~450 s,
and the catalog rules would need a third partition shape. Mirroring Firefox
removes the Chromium/Firefox special cases rather than adding one.

**One rule for every browser.** These parts currently hard-code "Chromium is
unsplit, Firefox is split three ways". Each becomes a single rule applied per
`{backend, browser}`:

- `scripts/e2e-matrix.jq` catalog validation: 3 split lanes per pair, no enabled
  unsplit lane, and per-browser dependency ordering.
- `nix/checks.nix`: the census project arguments and `-split` census topology
  are derived per browser. The postgres/chromium `playwrightTimeout = 1800`
  override is removed, so every lane uses the shared 1500 s cap and the existing
  VM resources: 3 GiB, 2 vCPU, workers=2. Ordinary Firefox lanes keep their 4
  GiB / 2 GiB disk.
- Host reconciliation: the core functions already take a browser. Its
  Firefox-only entry points gain a browser argument: `reconcile_files`, the
  `e2e-evidence reconcile` CLI, `dev e2e-reconcile`, and
  `scripts/e2e-workflow all`. The `all` mode reconciles every
  `{backend, browser}` pair.
- The catalog output contract changes. `e2e-matrix.jq` emits `lanes` (every
  enabled lane as `{backend, browser, lane}`) and `reconciles` (every
  `{backend, browser}` pair), in place of `chromium`, `firefox` and `backends`.
- CI: `e2e-chromium` and `e2e-firefox-lane` merge into one lane matrix job over
  `lanes`. `e2e-firefox-reconcile` becomes one reconcile job per entry in
  `reconciles`. `e2e gate` requires the catalog, lane and reconcile jobs. The
  required check contexts (`e2e gate`, `Validate (no e2e)`) keep their names.
  `pr-observer`'s hard-coded job and output mapping moves to match.
- `nix/cache-policy.json` adds the six new Chromium checks as `final` and their
  drivers as `support`. It also removes the four retired `*-chromium-unsplit`
  check and driver entries, because disabled lanes produce neither (#1954).

**Coverage authority moves from one producer to the SQLite/Chromium reconcile.**
Today `sqlite-chromium-unsplit`'s producer step checks that its capture came
from a registered `/nix/store` output, then compares the capture with
`docs/coverage/server-fns.json`. After the change:

1. Each of the three SQLite/Chromium producers only verifies its own lane, as
   every other producer does now.
2. Coverage runs from lifted lane artifacts and fails closed. It first runs the
   SQLite/Chromium reconcile, which requires the exact enabled lane set, an
   identical independent census whose population equals the union of the lanes'
   reports with no duplicates, and a successful phase sidecar for every lane.
   Only then does it union the three captures' `covered` and `orphans` maps and
   compare the rendered snapshot byte-for-byte. A missing lane, or a failed or
   partial one, fails coverage.
3. Coverage verification is part of the `dev e2e-reconcile sqlite chromium`
   verdict. CI runs it in the `sqlite/chromium` reconcile job. Locally, the two
   supported workflows that reach it both re-run all three SQLite/Chromium
   producers in the same invocation before reading their artifacts:
   - `dev e2e` / `dev validate` runs coverage after every producer settles.
   - `dev e2e-regenerate-coverage` regenerates the snapshot from the union.

   There is no separate coverage command. Each producer deletes its own earlier
   files before it builds, so a lane re-run replaces that lane's evidence. A
   manual `dev e2e-reconcile` over hand-assembled artifacts is a diagnostic, not
   the supported proof.

Cost accepted: two existing rules are relaxed.

- ADR-0238 requires coverage inputs to come from "the selected successful
  immutable output" and to reject stale evidence from the wrong output. This
  check, that the capture came from a registered `/nix/store` output, is
  dropped.
- ADR-0203 says "Producer/consumer pairs remain within one job". Coverage now
  runs in a different job from the producers that made its inputs.

CI's reconcile job only has downloaded artifacts, so the registered-output check
cannot run there. The reconcile proof takes its place: every lane succeeded, and
the populations are complete and disjoint. This is the same trust that already
backs every Firefox verdict.

Each capture is extracted on its own before the union, because span ids are only
unique within one lane. Coverage is a set union of per-test hits, so `covered`
should not change. Orphan reasons embed a lane-specific span id, so they would
differ from the unsplit lane's, but the snapshot has no orphans today. If the
snapshot does change, that is a finding to explain before accepting it.

**Cost.** Four more producer jobs, at ~1.5–2 min of setup and ~1.3 GiB fetched
each, plus two light reconcile jobs. In exchange the E2E chain wall-clock gets
shorter, and so does each lane's exposure to AWS-runner slowness. The AWS
slowdown itself is not investigated (see Boundaries).

**Decision record.** A new ADR draft in `docs/adr/drafts/` narrowly supersedes
these clauses:

- ADR-0203: "Chromium remains one unsplit lane per backend", and
  "Producer/consumer pairs remain within one job" as it applies to coverage.
- ADR-0238: coverage selects the immutable output of one successful producer.
- ADR-0081: coverage comes from one `sqlite × chromium` run. It becomes the
  union of the reconciled SQLite/Chromium lanes. `docs/ARCHITECTURE.md`,
  `CONTRIBUTING.md` and `docs/observability.md` prose is updated in the same
  change.

## Acceptance

Executed proof (results go in the PR or an issue comment, not the tree):

- **Same tests.** `playwright test --list` over the old unsplit Chromium
  projects and over the six split Chromium projects yields the same stable test
  identities (file, line, column, title path), with no test duplicated across
  shards.
- **Gate green.** Regression tests cover the uniform catalog rules, the
  per-browser reconcile, coverage refusing a missing or failed SQLite/Chromium
  lane, and the CI topology. `dev check` passes. CI's `e2e gate` passes with all
  12 producers and 4 reconciles.
- **Coverage unchanged.** In every after-run, the `sqlite/chromium` reconcile
  job verifies coverage against the unchanged committed
  `docs/coverage/server-fns.json`. A coverage failure in any after-run is
  root-caused before acceptance and checked against #2003's nondeterminism. It
  is never dismissed as a flake.
- **Headroom.** Every Chromium lane's Playwright phase takes ≤ 900 s (60% of
  1500 s) in every after-run. At least three of those lane observations are on
  AWS-hosted runners, with the runner provider read from the job's runner logs.
  This closes #1985.
- **Measured.** A comment on #1969 records the E2E chain duration (catalog start
  → `e2e gate` end) and the per-lane Playwright time, for at least five runs
  before (recent `main`) and five after (PR pushes and reruns). It also records
  the change in runner time.

Deliverables committed: catalog, Playwright projects, Nix checks, cache policy,
`scripts/`, host-proof verifier, `dev`, CI workflow, `pr-observer`, tests, the
ADR draft and doc updates.

## Boundaries

- No change to workers per lane (2), VM sizing, retries, duration budgets, the
  1500 s cap or the boot/trace/panic checks.
- The Firefox topology and its disabled measurement controls stay as they are.
  No Chromium measurement-control comparison machinery is added.
- No investigation of AWS-versus-Azure runner speed, and no work on the result
  lift's 4 GB log archive or #2003's `server-fn-coverage-verify` failure. Those
  stay with their own issues.
- No renaming of the existing `experimentalProjectDependencies` key or Firefox
  project names.
