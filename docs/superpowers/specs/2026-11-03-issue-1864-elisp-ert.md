# Outcome

Give the existing standalone pure ERT population an independent Nix verdict
while preserving its named host check, exact ordered `devtool check --all`
inventory, and separate combined live/pure coverage verdict. The ERT slice adds
a distinct pass/fail consumer, not a performance or source-cache isolation
claim.

# Approach and tradeoffs

## Population and input contract

`devtool check ert` invokes `emacs --batch -Q -l elisp/scripts/run-tests.el`
(`tools/devtool/src/check.rs`). The runner requires `ert`, derives the project
root from its own path, adds `elisp/` to `load-path`, requires `jaunder` (which
loads the production module graph), loads each immediate `elisp/test/*-test.el`
in `directory-files` order, and runs the ERT batch. It does not select by
test-name pattern or load `*-integration.el` directly. Tests can and do load
extra Lisp: `jaunder-wait-test.el` requires `jaunder-integration-helper`;
`jaunder-coverage-test.el` loads `scripts/jaunder-coverage.el`; multiple debug
tests load `jaunder-debug-boundary-fixture.el`; reconcile batch tests load
`jaunder-reconcile-batch-fixture.el`; conflict debug tests load definitions from
`jaunder-reconcile-test.el`. Keep the existing runner authoritative; do not copy
its selection logic into the Nix expression.

The pure suite loads live-harness helper definitions, but that is not equivalent
to running live integration. Wait tests exercise pure polling and stub server
lifecycle; coverage tests stub the combined runner path. Loading the helper
defines image/server/HTTP functions and an image-directory value, but source
inspection shows no automatic server start or image read from that load.
`*-integration.el`, server process/binary/database/runtime file, actual image
fixture bytes, ExifTool and network service are not standalone-runner inputs on
that evidence. `jaunder-media-test.el` does include a Git-dependent real
temporary-repository test guarded by `skip-unless (executable-find "git")`; Git
is therefore required to preserve that test as executed rather than skipped. The
transport test executes a generated fake curl via plz, not a live network
request. Most other fixtures are temporary trees created within ERT.

The dedicated leaf's conservative source/tool contract is: the complete
non-Markdown `elisp/` tree (production sources, every test and script, and
non-Markdown fixture assets; no hand-pruned require closure), shared
`devtoolBin` including its named check definition, existing `emacsForCi` package
set (`pkgs.emacs` with `plz`, `undercover`, and `cmarkEl`), Git on PATH, and
`TZDIR=${pkgs.tzdata}/share/zoneinfo`. Run the existing command from a writable
copied source tree with repository root as cwd. `-Q` suppresses user init.
`undercover` is an actual test dependency: `jaunder-coverage-test.el` (lines
178–229) locates Undercover, starts a nested Emacs process, instruments
temporary production fixtures, and reads the generated `lcov.info`. The existing
`emacsForCi` closure supplies Undercover as well as `plz` and `cmarkEl`; retain
that exact closure rather than pruning it. Runtime validation of actual
load/read/skip identities and complete evaluated derivation closure is still
required after approval; until then, this broad source/tool contract is
deliberate and conservative. No unsupported exclusion is promised.

## Partition, consumers, and tradeoff

The proposed final devtool catalog is `docs`, `typescript`, `code`, `ert`. Add a
one-member `ert` group; remove only `ert` from `code`; keep `devtool check ert`
and every prior check identity exactly once and in the same effective `--all`
order (ERT remains after `tools-clippy`). TypeScript's group remains as already
approved. Nix validation order becomes `static-docs`, `static-typescript`,
`static-code`, `static-ert`, then existing test checks. `static-ert` invokes
`devtool check --group ert` with the inputs above and is wrapped in the existing
`nonSubstitutable` helper, preserving `static-code`'s verdict policy
(`allowSubstitutes = false`, `preferLocalBuild = true`). Already-valid local
outputs may still be reused; local validity or realization does not establish
current execution.

This proposal explicitly acknowledges the simpler alternative: keeping ERT in
the already-hermetic `static-code` group is less machinery and already runs ERT
on the Nix flake and validation path. The distinct `elisp-coverage-producer`
also runs pure and live ERT together and its host consumer owns the coverage
verdict (ADRs 0035/0162). The dedicated leaf is justified only for an
independently attributable standalone ERT verdict and its failure identity; it
offers no claimed speedup, no Elisp-source invalidation reduction (static-code
still consumes non-Markdown Elisp for formatter/byte-compile), and duplicates
pure ERT execution only in the combined coverage producer. Once `ert` is removed
from `code`, `static-code` no longer executes ERT; its overlap is non-Markdown
Elisp source consumption for formatting and byte compilation. That cost is
accepted only if the separate verdict is useful to #1864; no broader framework
or pruning is proposed.

Keep the live+pure coverage producer, its fixed artifacts and authoritative
consumer unchanged. `nix flake check` retains its existing hermetic lane. The
xtask consumer is Linux-only and uses `x86_64-linux`; CI's existing
`Validation hermetic` lane remains unchanged and consumes the expanded
validation sequence there. No hook, workflow, required-check, or test-routing
change is proposed. This ERT proposal applies the newly approved TypeScript
extraction direction: the TS decision narrowly added the TypeScript group and
leaf, and did not establish a generic extension policy. The further concrete ERT
group and leaf are authorized by the separately approved ERT spec substance
(Manager request `92b5a8a5-c6a0-4c44-99b8-0b339049bf78` /
`issue1864-ert-spec-v1`; exact human answer: “approved, with the caveat that I
don't think we actually need an ADR for this, do we?”). No separate ERT ADR is
needed; no other policy is changed. The shared `docs/ARCHITECTURE.md`
committed-direction projection remains the TypeScript shared integration owner's
responsibility and must retain this provenance, consistent with ADR-0127; do not
edit shared architecture/catalog/Nix/xtask files in this prep.

## Execution timing

Reuse the approved TypeScript emitter/parser direction: monotonic timing around
only the ERT child, one bounded versioned stderr event after completion with
`check=ert`, elapsed milliseconds and outcome; no paths or diagnostic contents.
Host projection may accept it only from current captured devtool child output
and retains whole-step duration separately. Nix may report current ERT child
execution only when the event is observed in the current build's live stream and
positively attributed to a builder executing this check. Existing
`BuildStderrTee` transport alone does not establish builder attribution. Request
duration, derivation identity, local validity, realization/reuse, retained logs,
and `nix log` are not execution timing evidence. On local reuse or absent
positive correlation, execution duration is unavailable; do not purge caches,
force execution, or infer timing from history. The transport/projection is a
later shared-owner dependency serialized with TypeScript work; approval of this
ERT proposal does not authorize shared edits.

# Acceptance

After implementation approval, without changing other group members:

- Capture actual selected source files plus ERT test identities and their
  statuses, including skipped identities; compare against the standalone
  runner's baseline and show the Git-dependent test executes rather than skips.
  Verify pure selection excludes live integration identities, and a newly
  matching pure test is included. Capture helper loads, child processes,
  fixture/config reads and any environment dependencies; inspect the complete
  Nix derivation closure and add any newly discovered inputs conservatively.
- Confirm the exact Nix direct source boundary and `emacsForCi`, Git and
  timezone inputs; confirm `nonSubstitutable` attributes remain
  `allowSubstitutes=false` and `preferLocalBuild=true`. Show ordinary valid
  local-store reuse remains possible and separately record reuse/local validity
  vs actual current execution. No cache purge or forced rerun.
- A relevant Elisp/test input affects the ERT leaf; an unrelated path outside
  the complete direct and transitive closure (e.g. `server/src/lib.rs`) does
  not. A deliberate failing ERT test fails host and Nix ERT verdicts; restoring
  it passes. Confirm unchanged `static-code` work still runs its other group
  members and document that its non-Markdown Elisp source overlap remains.
- Confirm the existing named command and exact ordered, duplicate-free `--all`
  inventory, resulting four groups and Nix leaf order, x86_64-linux validate
  consumer, unchanged CI hermetic lane, and unchanged pure+live coverage
  producer/consumer verdict. A controlled child fixture proves monotonic child
  interval and one event for success/failure; host consumes current child output
  only. Nix timing requires current live builder attribution; historical-only
  event is ignored and unavailable is emitted when attribution is absent or
  local output is reused.

These are future executable acceptance criteria, not work performed or
authorized here. This source census is complete to static inspection; runtime
load/skip/read identity and evaluated-closure validation are legitimately
deferred.

# Boundaries

In scope: standalone pure ERT population contract, ERT group/leaf proposal,
event and current-execution semantics, and these planning artifacts (spec and
approved outline; no separate ADR). Out of scope: implementation, live
integration/coverage changes, formatter/byte-compilation changes, input pruning,
generic timing infrastructure, hook/CI topology or required-check changes,
shared catalog/Nix/xtask/architecture edits, and mutation proofs, builds or
tests. Commit preservation is authorized separately.
