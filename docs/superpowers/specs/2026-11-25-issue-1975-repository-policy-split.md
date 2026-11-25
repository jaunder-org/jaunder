# Issue #1975: narrow repository-policy to source contracts and docs checks

## Outcome

`tools/repository-policy` holds only the source contracts, the documentation/ADR
checks, `scoped-leptosfmt` and the shared source-policy contracts they own
(markers, server-function inventory). The Git and checkout-admission helpers
move to `tools/host-tool`. The informational census and the Org timezone
manifest are retired. After this change the policy crate no longer depends on
the product crate `common`.

## Approach and tradeoffs

**Git and checkout helpers → `host-tool`.** `git.rs` (Git command builder,
tracked files, hooks path, config) and `checkout.rs` (untracked-input admission
through `nix eval`, checkout selection, best-effort hook installation) move into
`host-tool`. Today `host-tool` holds only the shared JSON report contract
(ADR-0247). Its only consumers, `host-proof` and `coverage` (`host` feature),
are also the only outside users of these helpers. So `host-tool` becomes "what
every host tool does": report its result and prepare its checkout.
`repository-policy` gains a dependency on `host-tool`. The three per-tool
checkout wrappers (host-proof's, coverage-check's, and
`prepare_policy_checkout`) collapse into one
`host_tool::checkout::prepare(owner, source_admits)`. repository-policy's
executables pass the `JAUNDER_REPOSITORY_POLICY_SOURCE` value compiled into that
package. Admission and hook behavior are unchanged. `host-tool` gains `anyhow`,
a `tempfile` dev-dependency, and a minimal Git test fixture for the moved tests.
`repository-policy`'s `test_support` calls `host_tool::git::at`.
`tools/host-tool/src` joins the lint-suppression policed roots.

Alternative: a new `tools/checkout` crate. Rejected: it adds a workspace member
and its packaging for the same two consumers.

**Nix invalidation.** Today `toolsSourceFilter` (`nix/packages.nix:890`) admits
`tools/host-tool/` into `toolsSrc`. That source feeds the in-derivation
coverage-sandbox, doctests, perf-producer and `tools-doctests` builds, none of
which compiles host-tool. Without a filter change, every edit to a Git or
checkout helper would rebuild them. Exclude `tools/host-tool/` sources except
its `Cargo.toml`, as is already done for host-proof: add it to the placeholders
and to `hostRunMembers`.

**Gain, stated plainly.** coverage and host-proof keep their `repository-policy`
dependency, so the Git move removes no dependency edge and no syn/oxc compile.
The move clarifies ownership, nothing more. `repository-policy` now also links
host-tool's `clap` and `rustix`. The real reduction in size and dependencies
comes from the two retirements.

**Retire census** (`source-contracts census`, `dev census [--json]`, about 3,000
lines including its language-server and Emacs process code). It was built for
the `docs/codebase-audits.md` method (#1222). It gates nothing, and nothing
calls it except that doc. Its current output was reviewed with the user:

- Candidates come out in alphabetical or hash order, not ranked.
- Both semantic cells report "unavailable".
- Churn emits 604k candidates.
- Clones are listed as hashes with no function names or line numbers.

The user chose retirement over moving it. `docs/codebase-audits.md` keeps the
audit method, but every reference to the command changes: lines 16, 28–101 and
247–252. Its "Running and interpreting the census" section is replaced by a
short list of the ad-hoc tools to use for that step: `git log` churn, ast-grep,
`cargo metadata`, rust-analyzer, and agent-assisted slice audits.

The census tests go with it:

- `tests/concern_native.rs:363-470` (the census fixture, its helper and three
  tests);
- the `dev` pins in `tools/dev/tests/conftest.py` and `test_workflows.py`.

**Retire `org-timezone-manifest`** (binary, module, tests,
`dev org-timezone-manifest`, CONTRIBUTING's "Read-only Org timezone evidence"
section). It extracted post-ID→timezone pairs from original `.org` files after
#1921's UTC backfill. No step consumes its output, and the user chose
retirement. It is the policy crate's only use of `common`, so that dependency
goes too.

**Unchanged:** coverage still uses `repository_policy::markers`, and host-proof
still uses `server_fns` and `source_inventory`. Those are shared source-policy
contracts with one canonical home (decided with the user). Both crates therefore
keep their `repository-policy` dependency, but no longer for Git helpers.

**Dependency cleanup:** remove the crate dependencies that only census or the
manifest used (`common`, `url`, and any others that become unused). Update the
Nix `bins` list and its comment, `dev`'s CLI/forwarding and its Python tests,
and the README.

**Decision record:** a numberless ADR draft
(`docs/adr/drafts/narrow-repository-policy.md`) narrowly supersedes:

- ADR-0246's clause that the package owns Org-timezone extraction and the shared
  checkout-admission library;
- ADR-0258's binary list;
- ADR-0247's report-only scope for `host-tool`.

For ADR-0247, the superseded scope includes its claim that host-tool exists "so
repository-policy keeps owning no reports or process execution" (0247:42-44).
host-tool now runs `git` and `nix eval`, and repository-policy depends on it.
repository-policy still emits no `host_tool::report`.

The draft also records the user-confirmed retirement of two capabilities
(ADR-0246 requires explicit human confirmation for capability loss).

Docs to update in the same change:

- `docs/ARCHITECTURE.md` (projection; :4684-4697 and the `checkout.rs` path at
  :4838)
- `CONTRIBUTING.md`: :336-364 (remove the manifest section) and :518 (the
  `git.rs` path)
- ADR-0258's list at :33 (via the draft's supersession)
- the `nix/repository-policy.nix` header comment
- the crate README: its "excludes … other tools' implementations" claim now
  includes host-tool

## Acceptance

- `tools/repository-policy` has no `census` module, no `org_timezone_manifest`
  module or binary, no `git.rs` or `checkout.rs`, and no `common` or `url`
  dependency. `cargo tree -p repository-policy` shows no `common`.
- `host-proof` and `coverage` import Git and checkout helpers from `host-tool`
  only. `rg 'repository_policy::(git|checkout)' tools` matches nothing outside
  `repository-policy`.
- `tests/native.rs`'s source-admission test is rewritten to assert all of:
  - `common/` and `macros/` are no longer admitted to the policy closure;
  - `tools/host-tool/src/*` is admitted to the policy closure;
  - `tools/host-tool/src/*` is absent from the product, csr, wasm and
    system-artifacts sources.
- Editing a `tools/host-tool/src` file does not change `toolsSrc`. A Nix
  assertion beside the filter proves this, as it already does for host-proof.
- `dev census` and `dev org-timezone-manifest` no longer exist. All other
  binaries and `dev` commands keep their command lines.
- The moved helper tests pass in `host-tool`. The `repository-policy`,
  `host-proof`, `coverage` and `tools/dev` test suites pass.
- `dev check` passes, and gate behavior is otherwise unchanged.
- The ADR draft, `docs/ARCHITECTURE.md`, `CONTRIBUTING.md`, the crate README,
  and `docs/codebase-audits.md` describe the new layout. No live doc or code
  mentions `dev census`, `source-contracts census` or `org-timezone-manifest`.
  Unrelated uses of "census" (the e2e census, endpoint censuses) stay.

## Boundaries

- Do not move or redesign the source contracts, docs checks, `markers`,
  `server_fns`, `source_inventory` or `scoped-leptosfmt`.
- Do not change the checkout-admission or hook-installation behavior. This is a
  move only. In scope as direct consequences: the `toolsSourceFilter` exclusion
  and the one-line `POLICED_ROOTS` addition.
- No replacement census tooling and no timezone-correction feature.
- Historical ADRs other than the narrow supersession stay as written.
