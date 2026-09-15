# Shared CI failure signatures across pull requests

## Outcome

When `cargo xtask pr watch` reaches `checks-failed`, it best-effort identifies
the same substantive failure on another active pull-request head or current
`main` and adds a machine-readable `shared_failure` annotation. The original
failure outcome and actionable job pointer remain authoritative whether
enrichment succeeds, finds nothing, times out, or cannot parse GitHub evidence.

## Load-bearing decisions

- This work builds on #1498's landed current-head Actions job identity and
  direct/transitive requirement classification from ADR-0195.
- ADR-0087 remains authoritative: xtask owns observation, `gh` is the sole
  transport, and `pr watch` remains read-only. No Rust GitHub client is added.
- Shared-failure lookup starts only after `checks-failed` is established and
  never changes that outcome, detail, pointer, or exit semantics.
- Candidate selection first captures the current open pull-request head SHAs and
  current `main` SHA. It then lists this repository's primary-CI workflow runs
  created during the preceding 24 hours, orders them by `created_at` descending
  and numeric run ID descending, and takes at most 50. From that bounded set it
  excludes the subject run, non-eligible heads, and incomplete runs, then
  retains the first (newest) completed run per eligible head SHA.
- A newer completed green run or rerun suppresses an older failure for that
  head. A newer in-progress run does not displace its latest completed run.
- A candidate comparison reads only failed jobs from those selected runs. One
  exact normalized error block on at least one other eligible head is a shared
  failure; equality is checked on normalized text, not digest alone.
- Normalization precedes extraction. It removes ANSI escapes and GitHub's
  job/step/timestamp log prefix, converts CRLF to LF, trims trailing whitespace,
  and collapses consecutive blank lines to one. It preserves leading
  indentation, case, paths, versions, hashes, numbers, and all other substantive
  text.
- Anchor detection ignores leading ASCII whitespace and selects the last line
  beginning literally with `error:`, `Error:`, or `fatal:` after normalization.
  GitHub Actions' terminal `Error: Process completed with exit code ...` and
  `Error: The operation was canceled.` runner markers are not anchors.
  Extraction includes that anchor and each following blank line, `Caused by:`
  line (also ignoring leading whitespace), or indented nonblank line; it stops
  before the first other line. A block longer than eight logical lines or 4 KiB
  of UTF-8, or a log with no qualifying block, remains unclassified rather than
  being truncated.
- The complete enrichment operation has a hard ten-second wall-clock budget.
  API, authentication, rate-limit, timeout, expired-log, malformed-response,
  unsupported-log, or parse failure returns the original unannotated report.
- `shared_failure` is omitted unless at least one match exists. Its initial
  machine-readable shape is:

  ```json
  {
    "version": 1,
    "algorithm": "github-actions-error-block-v1",
    "digest_sha256": "<64 lowercase hexadecimal characters>",
    "excerpt": "<the complete normalized UTF-8 error block>",
    "matches": [
      {
        "source": {
          "kind": "pull-request",
          "number": 1437,
          "url": "https://github.com/jaunder-org/jaunder/pull/1437"
        },
        "head_sha": "<40 lowercase hexadecimal characters>",
        "run": { "id": 34710221556, "url": "<workflow run URL>" },
        "job": {
          "id": 103597518863,
          "name": "Validate (no e2e)",
          "url": "<failed job URL>"
        }
      }
    ]
  }
  ```

  `source` is exactly either the shown pull-request object or
  `{ "kind": "main" }`; fields are never `null`. `digest_sha256` hashes the
  exact UTF-8 bytes in `excerpt`. `matches` contains at most ten entries,
  ordered by run `created_at` descending, numeric run ID descending, then
  numeric job ID ascending. The subject failure is not included as a match.

- No new Jaunder domain vocabulary or architectural decision is introduced; this
  refines the existing PR-observation boundary from ADR-0087 and ADR-0135.

## Acceptance

- The failed `Validate (no e2e)` jobs in
  [run 34710221556](https://github.com/jaunder-org/jaunder/actions/runs/34710221556/job/103597518863)
  and
  [run 34709632870](https://github.com/jaunder-org/jaunder/actions/runs/34709632870/job/103606175199)
  normalize to the same signature despite different timestamps and step
  durations:

  ```text
  error: failed to download `jiff v0.2.35`

  Caused by:
    attempting to make an HTTP request, but --offline was specified
  ```

- A merely similar error whose preserved path, version, identifier, or cause
  differs does not match.
- A shared match preserves `outcome: checks-failed`, its original detail and
  pointer, while adding the complete versioned `shared_failure` object.
- A unique failure, absent anchor, over-limit block, unavailable log, malformed
  comparison payload, GitHub failure, or ten-second timeout preserves the
  original report without a `shared_failure` field.
- Candidate selection proves the 24-hour and 50-run limits and their ordering,
  open-head/current-main restriction, subject exclusion, and latest-completed-
  run-per-head rule.
- A green latest completed run suppresses an older matching failure for the same
  head; an in-progress run does not suppress the latest completed failure.
- Comparison never inspects successful jobs, non-primary workflows, closed or
  superseded pull-request heads, or runs outside the repository.
- Existing direct/transitive requirement classification, readiness, rerun
  settledness, merge-queue behavior, observer/armer separation, and
  `checks-failed` exit semantics remain unchanged.
- Deterministic tests cover shared exact match, similar non-match, bounded
  history and match ordering/cardinality, green supersession, in-progress-run
  treatment, query/log/parse failure, and timeout degradation.

## Boundaries

- No fuzzy matching, template inference, version/path/number erasure, or
  repository-wide incident declaration.
- No job rerun, cancellation, branch mutation, issue mutation, ownership
  selection, merge action, or suppression of required checks.
- No durable failure database, telemetry service, cross-repository search, or
  background daemon.
