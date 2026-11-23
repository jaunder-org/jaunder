# Issue #1963: keep the CI storage probe in one place

## Outcome

Every CI storage measurement comes from one script, `scripts/ci-storage-probe`.
The five copied `probe()` blocks and the inline `df` observations in
`.github/workflows/ci.yml` become one-line calls to it. The uploaded storage
artifacts keep their names and their `capture_exit` / `missing_evidence`
markers.

## Approach and tradeoffs

`bash scripts/ci-storage-probe <report> <label>` (invoked through `bash`, like
the other repo scripts) creates the report's parent directory, appends one
section headed `=== <label> ===` to `<report>` and exits 0 whatever its
measurements do (a wrong argument count is a usage error, exit 2). Each
measurement is written as its title, its output, then `capture_exit=0` or
`capture_exit=<code> missing_evidence=true`, exactly as today. Every call takes
the same measurements:

- timestamp, `TMPDIR` and `$GITHUB_WORKSPACE`;
- for `/nix/store`, `$GITHUB_WORKSPACE` and `/tmp`: `df` bytes (`-B1`), `df`
  inodes and `findmnt` (a missing `findmnt` is a marked failure like any other,
  replacing today's unmarked "findmnt unavailable" note);
- `free -b`.

The `final` label alone adds allocated bytes (`du -s -x -B1`, 10 s timeout each)
of `/nix/store`, `$GITHUB_WORKSPACE/.xtask/tmp` and `$GITHUB_WORKSPACE`, as the
final snapshots do today (human decision: `du` only in `final`).

`.xtask/` is still current: the devShell's startup hook creates
`<checkout>/.xtask/tmp` and exports it as `TMPDIR` (`CONTRIBUTING.md:78-86`).
Two stale details are dropped. Nothing creates `.xtask/tmp/workspace`; only the
current probe blocks print it. And `df`/`findmnt` on `.xtask/tmp` only repeat
the workspace filesystem, while at baseline the directory does not exist yet.
Its allocated bytes (`du`) stay, because those show how much the shells' temp
files used.

Call sites:

| Job               | Label(s)                                        | Report / artifact                                                      |
| ----------------- | ----------------------------------------------- | ---------------------------------------------------------------------- |
| validate-host     | `baseline`                                      | `host-resource-baseline.txt` / `validate-host-resource-baseline`       |
| validate-coverage | `baseline`, `final`                             | `coverage-storage-snapshot.txt` / `validate-coverage-storage-snapshot` |
| e2e-lane          | `baseline`, `after ci-nix shell entry`, `final` | `lane-storage-snapshot.txt` / `e2e-storage-snapshot-<lane>`            |

The e2e `after ci-nix shell entry` call runs the script inside
`nix develop .#ci-nix`, replacing the inline `df` step, so that measurement is
now uploaded rather than only logged. The `after producer` and post-upload
`final` inline `df` steps are removed: the `final` snapshot (taken after the
producer and its trace summary) already records the same filesystems. Those
steps also observed `/`, which leaves the path set; on GitHub-hosted runners it
is the workspace filesystem, which `findmnt` on `$GITHUB_WORKSPACE` identifies.

**Why a plain Bash script.** It runs on the runner before the devShell exists,
which the host baseline needs (it is uploaded before heavy work in case the
runner runs out of disk). A `dev` subcommand would need `nix develop` first,
paying shell-entry cost and depending on the Nix store it observes. A composite
action would still carry the same shell body plus an `action.yml` wrapper. This
sits beside, not against, ADR-0251 (`dev` owns workflow composition): the probe
composes nothing, and the inline YAML it replaces was not `dev` either.

**Accepted changes to report content** (human-approved unification): the host
baseline now uses the shared paths and adds `findmnt`/`TMPDIR`, and drops
`du target` (the target directory is empty before validation). Coverage and e2e
reports switch `df -h`/`du -h` to byte counts and gain `free -b`. Because `du`
stays in `final` only, no lane gains measurable runtime. `du /nix/store`
routinely hitting its timeout records `capture_exit=124 missing_evidence=true`;
that is expected, as it is today.

**Diagnostic only.** A failed measurement is recorded, not raised, and never
makes the script exit non-zero. Every probe step uses `continue-on-error: true`
(the host baseline gains it). The one deliberate exception stays: the host
baseline upload keeps `if-no-files-found: error`, so a report that was never
created (broken wiring) still fails the host lane.

## Acceptance

Proof to execute:

- A pytest regression test (alongside `tools/dev/tests/test_e2e_workflow.py`,
  which already tests a repo Bash script) passes: `baseline` then `final` into
  one report yield both labelled sections with a `capture_exit` line per
  measurement, allocated bytes only under `final`, and exit 0; with
  `GITHUB_WORKSPACE` pointing at a nonexistent directory, its measurements
  record `missing_evidence=true` and the script still exits 0.
- On the PR's CI run: `validate-host-resource-baseline`,
  `validate-coverage-storage-snapshot` and each `e2e-storage-snapshot-<lane>`
  artifact exists under the same name and contains the expected labelled
  sections and markers.
- `rg 'probe\(\)|df -' .github/workflows/ci.yml` finds nothing.

Deliverables to commit: the script, its test and the `ci.yml` call sites.
CONTRIBUTING needs no edit: it describes `scripts/` generically
(`CONTRIBUTING.md:31`).

## Boundaries

- No change to which jobs exist, to the `Validation test checks` job split, or
  to fail-fast behaviour (both dropped from scope in the issue).
- No new test framework or `dev` integration for the probe.
- Artifact names, retention and `if-no-files-found` settings are unchanged.
