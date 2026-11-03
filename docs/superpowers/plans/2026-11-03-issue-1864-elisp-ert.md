# #1864 ERT implementation outline

**Status:** approved to begin implementation planning; implementation and proofs
remain gated on their applicable authorization. Human approval received via
Manager request `01f6ad43-1aa5-4f5e-98b9-b459b4135231` /
`issue1864-ert-outline-v1`, exact answer: “Approve outline and begin”.

**Approval/provenance:** ADR-0178 specified two groups and leaves; it did not
authorize general extraction. The newly approved TS direction narrowly added the
TypeScript group and leaf. This concrete further ERT group/leaf is authorized by
the separately approved ERT spec substance, Manager request
`92b5a8a5-c6a0-4c44-99b8-0b339049bf78` / `issue1864-ert-spec-v1`, exact human
answer: “approved, with the caveat that I don't think we actually need an ADR
for this, do we?” No generic extension policy is inferred; no separate ERT ADR
is needed and no other policy is changed. The shared architecture projection
must retain this provenance per ADR-0127.

## Dependencies and ownership

- One shared integration writer (TypeScript Lead) owns the devtool group
  catalog/`--all` order, `nix/checks.nix`, xtask transport/orchestration and
  consumer, shared architecture projection, and common timing implementation.
  ERT changes to those shared paths serialize with TypeScript; do not
  parallelize writers there.
- ERT-specific leaf source/input design follows the approved ERT spec: existing
  `devtool check ert`, complete non-Markdown `elisp/`, `devtoolBin`, existing
  `emacsForCi`, Git and `TZDIR`. Keep static-code's other members and combined
  pure+live coverage producer/consumer unchanged.
- Use existing Nix non-substitutable leaf convention. No source pruning, generic
  framework, hook/CI/required-check changes, or cache-forcing mechanism.

## Proof sequence after the required implementation approval

1. Record baseline `devtool check ert` population; capture selected/load paths,
   test identities and individual outcomes including skips. Confirm
   Git-dependent identity executes (not skips), pure selection excludes live
   integration identities, and a newly matching pure test would be selected.
   Trace child processes, fixture/config reads and environment; inspect full
   evaluated derivation closure and add discovered inputs conservatively.
2. Confirm the exact direct source boundary and tool closure (`emacsForCi`, Git,
   timezone); `nonSubstitutable` attributes remain `allowSubstitutes=false` and
   `preferLocalBuild=true`. Relevant Elisp/test changes invalidate the leaf; an
   unrelated path outside complete direct and transitive closure (e.g.
   `server/src/lib.rs`) does not. Do not purge caches.
3. Record an ordinary valid local-store reuse separately from execution.
   Reuse/local validity/realization and request duration are not proof the
   current request ran ERT. Current execution timing is reported only with
   positively correlated current-build live builder evidence; otherwise
   unavailable. Historical logs and `nix log` are never timing proof.
4. A deliberate failing ERT test must fail host and Nix ERT verdicts; restore it
   and confirm pass. Confirm other `static-code` group members still run and
   document remaining Elisp source overlap for formatting/byte compilation.
   Verify exact duplicate-free `--all` ordering, four groups, Nix leaf order,
   x86_64-linux validation consumer, unchanged hermetic CI lane and unchanged
   combined coverage verdict.
5. Validate timing transport with a controlled child fixture for monotonic
   interval and one bounded event on success/failure; host consumes current
   child output only. Nix accepts timing only with current live builder
   attribution; historical-only markers are ignored.

## Boundaries and later gates

These are proposed acceptance proofs, not performed work or present authority.
Obtain outline approval before implementation/proofs. Preserve the later
integrated-tree review and commit/verification boundaries; no hook, workflow, CI
topology, required-check, shared architecture/catalog/Nix/xtask or tracker
changes outside the stated shared-owner integration. No implementation or proof
is authorized by this outline itself.
