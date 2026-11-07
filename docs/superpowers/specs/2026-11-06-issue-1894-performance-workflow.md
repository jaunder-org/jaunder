# Performance workflow without xtask orchestration

## Outcome

One native-parameter Just workflow runs, compares, and imports performance
evidence. Nix selects and builds the existing pinned producers; a small serial
Just loop retains each producer before starting the next; a host-only Rust
performance command validates and assembles evidence, compares it, or performs
an explicitly authenticated baseline import. Measurement and baseline guarantees
remain intact while xtask no longer schedules or catalogs performance runs.

## Approach and tradeoffs

Expose the existing profile, backend, browser, workload, and dataset-count
capabilities as native Just recipe parameters rather than forwarding CLI flags.
Use the pinned Just interface:

```text
just perf [profile] [backends] [browsers] [workload] [posts] [authors] [revisions]
```

Defaults are `medium`, `sqlite,postgres`, an empty `browsers` value, `all`, and
unset counts. `workload` accepts `all`, `storage`, or `browser`; comma-separated
selections preserve repeatable backend/browser choices. An omitted browser list
normalizes to Chromium only when browser work is selected. An explicit browser
selection combined with `storage` is rejected; for example, the native Just
invocation `just perf small sqlite,postgres "" storage` is a valid storage-only
run. Counts, when supplied, are positive. Canonicality is determined from the
normalized selection, not whether a selector was explicitly supplied: an
explicit selection equivalent to the default backend/browser matrix remains
canonical, while a nondefault selection is not. Any count override, even one
equal to its profile default, is noncanonical. Baseline import separately
requires the `medium` profile. The small, medium, and large profiles retain
their documented dataset sizes; medium is the only canonical comparison/baseline
profile.

Nix evaluates the requested producer descriptions from the parameters and nonce.
Just walks that selected list serially, builds each existing Nix producer, and
copies its complete `performance/` output into the fresh invocation directory
before starting the next producer. A failed build or failed producer-status
check stops subsequent work but leaves already-retained producer evidence
available for the workflow's unconditional artifact upload. Nix derivations
never invoke xtask or the host command. This deliberately retains a small Just
loop: one aggregate can select/install known outputs, but does not guarantee
serial producer execution or preserve prior completed output when a later build
fails.

A narrowly scoped host-only Rust performance command consumes retained outputs;
it does not select or build producers. It validates producer status, fragment
schema/completeness, compatibility and diagnostic paths; assembles the typed
versioned result; and atomically publishes the invocation-owned result plus
stable CI upload copy. It also provides advisory comparison and explicit
baseline import, exposed as `just perf-compare <result-path>` and
`just perf-import-baseline <positive-run-id>`. Import continues to authenticate
repository/workflow/job/event/status/branch/run-attempt/artifact identity,
require canonical complete medium evidence compatible with the committed
baseline, and atomically replace the baseline only after all checks pass. CI
continues to compare only after a successful run and upload the evidence tree
even on failure.

The existing typed model and statistics remain in `tools/performance`; producer
measurement and fragment semantics remain in `tools/devtool` and Nix. The host
performance CLI is packaged with the `tools/performance` auxiliary package and
invoked by Just on the host, keeping workspace separation and host tests
unchanged. The tradeoff is that native Just parameters may be less
self-describing than the retired Clap flags; they avoid a new argument parser
and keep the public front door native to this repository.

## Acceptance

Proof to execute (focused; no full measurement matrix required):

- Native Just usage/defaults and explicit values select the same profiles,
  backends, browsers, workload classes, and counts as the current interface;
  duplicates normalize consistently and invalid/empty/conflicting selections
  fail before producer builds.
- Nix selection produces the expected named producer set for default,
  storage-only, browser-only, multi-backend/browser, and count-override cases.
  Confirm all outputs use one fresh nonce and the pinned profile/tool inputs.
  Explicit default-equivalent selectors remain canonical; nondefault normalized
  selectors and every count override are noncanonical; import independently
  rejects profiles other than medium. An empty browser parameter normalizes to
  Chromium only when browser work is enabled, and an explicit browser with
  storage-only fails before producer builds.
- A focused real invocation through the replacement Just front door runs an
  existing Nix producer, retains its output, and assembles the result with the
  producer's diagnostic references present. This is one producer, not the full
  measurement matrix; do not claim this proof before executing it.
- A small fixture-backed Just execution proves serial build/retain order, status
  validation before the next build, honest stop on build/status failure, and
  survival of completed earlier evidence after a later failure. No producer
  result may be reused as the current invocation's evidence.
- Rust leaf tests reject missing/failed status, malformed or incomplete
  fragments, incompatible selections, and missing/escaping diagnostic
  references; a complete fixture set assembles the versioned result and writes
  both result paths atomically.
- Comparison reports absolute medians/p95/spread and advisory thresholds without
  turning timing deltas into failure; malformed or incompatible evidence fails.
- Import fixtures reject each wrong
  repository/workflow/job/event/branch/head/run-attempt/artifact identity,
  unsuccessful or expired artifacts, noncanonical profile/selection, local
  provenance, incomplete results, and baseline incompatibility. Every rejection
  leaves existing baseline bytes unchanged; a valid trusted run atomically
  replaces it.
- Workflow proof shows run failure prevents comparison, while artifact upload
  still includes all available invocation evidence. Manual/local Just commands
  and the workflow use the same recipes.

Commit source, tests, updated current documentation, and the approved numberless
ADR draft plus its architecture projection. Do not commit measurement logs or
probe captures.

## Boundaries

Do not change producer measurement semantics, performance schemas/statistics,
profile sizes/defaults, baseline format, trust policy, or advisory-only
comparison. Do not add a generic workflow runner, schema framework, report
schema, parallel producer execution, or one-for-one Rust scheduler rewrite. Do
not invoke host Rust code from Nix derivations. Preserve `xtask`/`tools`
workspace separation and their existing auxiliary test coverage. Migrate all
actual workflow and documented command callers. Do not edit accepted
ADR-0028/0141 or generated `docs/README.md`; make only the approved narrow
ADR-0028 clarification and leave ADR-0141 unchanged. No implementation, matrix
run, commit, gate, push, issue mutation, or claim cleanup is authorized by this
design-recording task.
