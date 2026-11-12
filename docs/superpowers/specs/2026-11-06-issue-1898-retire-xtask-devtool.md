# Simplify residual tooling capabilities and retire obsolete endpoints — issue #1898

## Outcome

Finish a finite residual tool system with convenient single-command workflows,
clear surviving owners and lower TOTAL maintained complexity. Retire redundant
xtask/devtool endpoints, launchers and dedicated workspace/build/vendor/catalog/
report scaffolding only after each real capability has an adequate survivor or
an explicitly approved disposition. Deleting or renaming binaries alone is not
success; no replacement omnibus or generic workflow/reporting engine is built.

## Authority and prior design

### Qualified Task2 checkpoint acceptance

Human decision `1898-task2-maintenance-reconsideration-v1`, background
`8156ef02-d6ad-46ed-a2f9-587fa0160e77`, exact answer **Accept qualified
checkpoint**, relayed by Manager, accepts Task2 as a **POSITIVE-COST
INTERMEDIATE** at the supplied **+277** snapshot, not demonstrated maintenance
reduction. The overall #1898 lower-total outcome and original acceptance
criteria below remain binding; no future savings are promised or credited. Added
approval-documentation costs are recounted separately from that approved
snapshot.

Selected-proof limits and eight historical missing streams remain qualified;
acceptance does not reconstruct evidence or expand assurance. No Tasks3–7,
workspace redesign, shipping or cleanup is authorized. A normal hook-backed
commit requires Lead's immediate exact-tree approval. Earlier
proposed/intake-only wording below is retained as historical status, not current
Task2 authority.

### Historical design intake

PROPOSED amendment, not approved implementation. Human restart “Yes, please”,
relayed by Manager, authorizes intake/design only. The rewritten #1898/#1859/M33
scope supersedes old tracker pause wording for this assignment, not product
gates. Original `1898-design-spec-v1` request
d619d302-ad1b-4e89-899f-c6abfcb898ee, “Approve design only”, bound
baseline82bd37273dd533026fbf07f0121865fe254583a8 /
treeadadf31155998c7572708e9cbddd85c33b83c9fb. Exact originals remain preserved.
R1 b2aba4e8-d99c-4aec-a96d-98aab2939ff1 remains binding: individual manual
command dispositions, known operator/safety retention and explicit material-loss
confirmation; one linked whole-endpoint ADR; “None known” external users is not
absence proof and adds no compatibility facade. All13 historical prerequisites
are accepted/closed; actual consumer applicability, not labels, gates removal.

## Qualified ERT timing approval

Human decision `1898-ert-timing-envelope-v1`, background
`a8718c2c-09e5-4739-a79f-c388da8d7454`, answer **Accept honest request timing**,
relayed by Manager, authorizes the existing Instant/completion-event owner to
invoke only the fixed named Just ERT leaf. Emacs argv lives once in Just. ERT's
monotonic interval is explicitly **request duration including Just startup and
evaluation**, never exact Emacs-child execution. Preserve one bounded completion
event for completed success/failure, no fabricated completion on spawn error,
inherited streams, independent host/hermetic populations and verdicts. No new
timer/wrapper/framework or #1933 expansion; lower-total-maintenance acceptance
remains unchanged.

Manager released the TSC hold after actual PR1937 merge
`7d8850c455c061a06c2b275479acc25174dae86d` (tree
`e12f7c17765c58a3b73490420b95bb4c361363d0`, combined-main run43121355416). Its
landed passive public request owner remains intact while the unchanged TSC argv
moves once into the shared named Just leaf. Retire the obsolete devtool TSC
invocation/event without expanding observation scope or weakening its limits.

## Approach and tradeoffs

1. Just owns local composition and standard pinned invocations; Nix owns pinned
   inputs, immutable outputs and producer sequencing; native CI owns distributed
   scheduling/artifacts/aggregates. Keep existing focused family owners rather
   than repeat landed Node, sandbox, performance, qualification frontdoor, PR or
   promoter migrations. Keep traced-run independent.
2. Retain the ORIGINAL approved static-definition clause: put each pinned check
   invocation/argument set in a named Just leaf; host composition and hermetic
   Nix leaves call that same recipe. Preserve populations/order, Fix/Verify,
   target/platform limits, offline Cargo homes and host/sandbox cargo-deny
   modes. Nix admits recipe sources and exact pinned tools. Shared definitions
   do not imply shared host/hermetic verdicts or new universal #1864 sharing.
3. Use native tools and existing libraries before custom machinery. Retain only
   real project policy/evidence/lifecycle algorithms, not copied parser/SDK/Git
   engines. Keep Leptosfmt's narrow fail-closed scope/UTF8/error adapter unless
   a supported command demonstrates equivalent semantics. Infrastructure Python
   is allowed when focused/easier maintenance (#1848), not a uniform rewrite.
4. Repository policies live in one auxiliary repository-policy package with
   repository-docs and source-contracts executables: fixed typed policy
   operations, existing independent verdicts/diagnostics, shared private parser
   functions. They own no tool invocation definitions, workflow selection,
   extensible rule registry, generic check catalog, process execution or
   reports. ADR/doc/flow/sequence and source-contract operations remain
   independent named leaves. This avoids one package/wrapper per check and a
   renamed check-leaf dispatcher. Reuse current syn/Oxc and documented gate
   parsing behavior. The same package separately owns SINGLE-PROCESS
   git-index-safety and shared checkout admission/best-effort hook-preparation
   library code; no shell-split reconciliation or silent loss of automatic
   bootstrap. Its Org-timezone extractor reuses common and its scoped-leptosfmt
   preserves the narrow gap.
5. Qualification algorithms move together into one auxiliary qualification
   package/qualification-host executable; its four existing Just recipes and
   bounded harness/source/target arguments stay the interface. Reuse
   host/common, jsonschema and native Git/Nix primitives. Keep safe-open leases,
   selected tool/ helper identity, lifecycle and ordinary/styling/smoke
   publication distinctions. A separate package is cheaper than adding
   qualification host deps/features and source exclusions to performance's
   product/WASM-admitted package.
6. Focused proof lives in one auxiliary host-proof library with direct
   test-local and e2e-local binaries and named Just frontdoors, not a supervisor
   CLI. Reuse current ProcessKit/Tokio adapters, nextest/Playwright, canonical
   seeding and Nix membership predicates. Preserve test-local's artifact
   lock/CSR/worker/ environment contract and e2e-local's immutable
   artifacts/unique roots, readiness, capture and cleanup status/order. Do not
   absorb standalone sandbox, change existing shutdown semantics or construct
   another process-control layer.
7. The finite ignored `surviving-owners.md` delta selects every remaining home:
   existing coverage/doctests/performance crates own their named evidence,
   producer, budget, mutation and diagnostic interfaces; qualification adds its
   distinct system-artifact producer, never executing host qualification in Nix.
   Host-proof owns trace-analysis and selected source/cache diagnostics; fixed
   Just recipes retain serial trace/benchmark requests and host-window
   assertions. PG custody moves once into tools/coverage's nondefault
   ephemeral-postgres library module; coverage calls its scoped closure and
   host-proof pg-run depends on that library. No product-host source/feature or
   reverse dependency. Canonical seeding uses test-support, not a second fixture
   writer. Preserve Org extractor exact-ID ambiguity/partial
   status/unknown-coverage contracts, no discovery/server/import; manual
   capabilities retain useful diagnostics.
8. Remove forwarding/catalog/envelope and exclusive build/source/vendor paths
   with their LAST real consumer. Deliberately rehome compensating auxiliary
   tests before deleting a workspace. No compatibility wrapper without actual
   evidence. Explicitly DEFER experiment-only coverage disposition to #1898
   Lead: retain its devtool endpoints/build/tests until current caller/operator
   readback and approved retire/preserve/material-loss decision are complete.
   Thus FULL devtool/scaffolding retirement is NOT claimable while that defer
   remains.

The retained algorithms and modest package interfaces are accepted costs for
preserving real guarantees. Giant recipes, blanket Rust-to-Python/shell
rewrites, a generic checker/runner/registry, or dozens of wrapper packages add
maintenance without satisfying the outcome. Existing source shows policy gaps in
raw tools, not proof that every COTS alternative is inadequate; compare concrete
replacements at their real consumers. Keep this finite source/disposition
evidence ignored, not a permanent maintained command catalog.

## Acceptance

- Every residual command/alias, real source/build/filter/test/docs/CI consumer
  has a finite delivered/retire/defer/WNF disposition and named
  survivor/trigger. The finite owner delta is review evidence, not an enduring
  catalog. R26 names retained experimental devtool endpoints, Lead owner and
  completion trigger; a defer never counts as endpoint removal. Bootstrap and
  every last consumer must be resolved/delivered before final system retirement
  claims. All workflows remain convenient documented single commands; useful
  behavior is not lost.
- Show lower combined Rust/Just/shell/Nix/Python/test/doc/package maintenance,
  including new CLI/manifests/source admission costs, not just deleted Rust LOC.
- Preserve #1865's selected request/verdict identity, relevant-input
  invalidation, legitimate reuse, distinct modes/checks and meaningful failure
  through actual consumers; status-as-data requires its verifier.
  Missing/malformed/incomplete or wrong-output evidence cannot pass. Store/cache
  reuse is not new execution.
- Preserve ADR0237 fail-fast local composition and mandatory failure-path Git/
  index reconciliation, staged-subset/clean-tree admission and docs post-checks;
  no obsolete exhaustive gate-wide report is required. Required CI producer/
  reconciliation/aggregate joins and backend/browser/test/coverage populations
  remain authoritative and unchanged.
- At all selected surviving interfaces prove policy/refusal/read-error
  distinctions, source/tool drift, artifact/root/lock/cleanup lifetime and
  status, incomplete evidence, ordinary-vs-styling publication, no-evidence
  smoke and caller argv. Move meaningful existing tests, replace obsolete
  plumbing tests; do not test dependency engines or hide removed populations
  with a crate deletion.
- Preserve source/provenance/authentication, smoke-vs-qualification, exact-head
  PR approval/checks and privileged promoter identity/serialization. Managed
  cleanup remains forbidden. No altered language/workspace boundary is implicit.
- Preserve useful ordinary timing/diagnostics; #1933 owns passive selected
  observations/local retention/native CI14-day incidents. No PR1932 receipts,
  retiring-xtask instrumentation, new retention service or blanket tracing gate.
- Reconcile exact shared regions with #1864/#1933 before writing; retain their
  selected behavior/docs. Cold review and explicit amended-spec approval precede
  risk-triggered outline review/approval, separately assigned implementation and
  normal hook-backed commits; eventual exact-head merge approval is separate.

## Boundaries

No new scheduler, Just patch/PATH interception, generic supervisor, protocol,
parser or reporting framework; no broad re-audit/reimplementation of landed
families, #1753 resumption, #1933 takeover or PR1932 transplant. This draft and
its linked proposed ADR/projection are ignored review artifacts only. Do not
replace the staged approved originals, implement, commit, merge, change tracker,
weaken assurance, delete preserved work or perform checkout/branch cleanup.
