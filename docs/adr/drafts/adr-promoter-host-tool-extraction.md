# ADR-DRAFT: Nix-Selected Python ADR Promoter

- Status: proposed
- Date: 2026-11-07
- Issue: [#1897](https://github.com/jaunder-org/jaunder/issues/1897)

## Context

[ADR-0152](../0152-adr-numbering-happens-after-merge.md) assigns ADR promotion
to a trusted GitHub Actions workflow and xtask's serialized controller. The
controller includes recovery, GitHub identity/check correlation and a
tracked-draft mutation. Python is the selected language direction and the human
explicitly requires maintained libraries where they simplify the work; a
stdlib-only port or hand-built Markdown scanner is not the design goal.

The host execution boundary is established by
[ADR-0028](../0028-devtool-vs-xtask-boundary.md); `gh` transport and Rust PR
observation by [ADR-0087](../0087-xtask-github-pr-observation.md); Nix-selected
host-tool provenance by [ADR-0225](../0225-nix-cached-host-tooling.md); the
append-only log/materialized view by
[ADR-0127](../0127-architecture-view-materialized-from-adrs.md). This proposal
changes the promoter implementation owner, not ADR-0152's authority, acceptance
event or recovery guarantees.

## Decision

**Proposed:** package the controller and its one promotion mutation in
`tools/adr-promoter/`, independent of Cargo. Keep five focused Python modules:
`__main__.py` for its fixed commands/exit mapping, `controller.py` for records
and policy, `github.py` for `gh` process/JSON/error handling, `git.py` for argv
Git/leases, and `promotion.py` for admission, Markdown edits, staging and README
sync. Tests live under `tests/`. Use Marko 2.2.4 with its GFM extension: its AST
exposes source spans for links and images, including table cells. Read/write
source as bytes, decode only for parsing, patch destination spans in the
original buffer, and never reserialize an AST. Marko positions are character
offsets in normalized text, so a small tested boundary map converts them to
original UTF-8 byte offsets. The locked nixpkgs set currently provides Python
3.14.7, Marko 2.2.4 and pytest 9.1.1. The proposed runtime closes over `gh`
2.100.0, Git 2.55.0 and Prettier 3.9.6.

Tree-sitter 0.25.2 with tree-sitter-markdown 0.5.1 was rejected after an offline
GFM-table fixture found no destination spans and a tracked-document comparison
found four differences. `markdown-it-py` 4.2.0 exposes line maps, not inline
spans. Marko's documented GFM extension and source-span API were confirmed
against a locked Nix build and fake table/link/code/fence inputs; a 295-file
comparison matched the existing link spans exactly. Its character indices and
newline normalization require the explicit tested byte-boundary map.

Use existing `gh api` / `gh api graphql` with the App's `GH_TOKEN` and argv-only
Git CLI for lease-sensitive operations. PyGithub would add another auth/client
path while not supplying the custom merge-queue GraphQL model. The merge-group
run query retains its existing one-page limit (`per_page=100`) and parent
verification; do not add pagination that changes candidate selection. Parse
nonzero HTTP response bodies and GraphQL `errors[]` even on exit zero; map
`NOT_FOUND` only at current PR-lookup sites. Preserve the existing
rate-limit-reset lookup/warning and treat other malformed/API/process failures
as errors, not `NotRearmed`. GitPython offers no advantage over explicit
`--force-with-lease=<ref>:<observed-oid>` and verification.

Existing Rust command consumers are handled explicitly. Python owns `promote`
and `sync-readme`; remove Rust `run_promote` and `sync_readme_at` in PR 1. Keep
`cargo xtask adr promote` and `cargo xtask adr sync-readme` as fixed-command
delegates to the same Nix-selected executable. `sync-readme` is documented
recovery and `promote` remains the automation primitive; these delegates are not
generic RPC. During PR 1 only, `cargo xtask adr promoter` delegates to the
Python `controller` while the workflow/guidance migrate to Just. Preserve the
feature-author prohibition on direct promotion in `CONTRIBUTING.md` and
`docs/adr/drafts/README.md`; retain the documented `sync-readme` recovery
command. Update `CONTRIBUTING.md`'s generic controller rerun instruction to
`just adr-promoter`. PR 2 removes the controller CLI/help/dispatch/delegate
after trusted-main execution is observed. Rust `adr-format`,
`adr-readme-parity`, `adr-view-parity`, `doc-links`, and sequence/collision
checks remain read-only Rust consumers and keep their parsers/reporting.
Python-generated output must pass these gates. No Rust mutation or controller
remains after PR 2; no `host-tools` crate is added.

**ADR-0087 clarification:** Ordinary PR observation now belongs to
`tools/pr-observer`, with landing exposed through `just pr-land`.
`xtask/src/pr/gh.rs` re-exports the current shared observation transport and
classification seam and retains a small native mutation wrapper for the
test-only promoter armer. The Python promoter has its own narrow adapter over
the same pinned `gh` executable because it cannot call that Rust library. It
does not introduce a Python GitHub SDK, general shared API layer, duplicate
controller policy, or new workflow/permission path. This proposed
language-specific adapter is an explicit exception to ADR-0087's
single-Rust-wrapper ownership, not a silent claim that the same Rust file serves
Python.

The controller preserves ADR-0152's trusted-main/App authority, distinct
coalescing and per-PR recovery, no active cancellation, immutable head/diff, two
resumable orphan states, changed-ref refusal, GitHub conflict plus exact
local-merge proof, exact-OID lease and verified absence, exact event/PR/head/
ref/group/check correlation, verified arm, and visible failure records. After
mutation, it retains the existing staged-Markdown Prettier pass and restages
those files before commit; formatter failure is a visible error. Every
successful outcome (`IgnoredEvent`, `NotRearmed`, `Existing`, `NoChanges`,
`Created`, `Replaced`, `Rearmed`) exits zero. Only actual errors or failed
postconditions exit nonzero. Promotion remains acceptance; the merge queue alone
writes `main`.

**Approved parser-eligibility differences:** the Rust line scanner rewrites an
orphan textual `](../x.md)`, links inside indented code blocks, and links inside
multiline code spans; Marko does not expose those as inline `Link`/`Image`
destination spans. Human approval explicitly accepts Marko's syntax eligibility,
including all three differences. Offline checks found no span differences in 295
tracked Markdown files or the two tracked promotable drafts, but the probe did
not include the indented/multiline-code cases and corpus equality is not
universal equivalence. Full acceptance fixtures must explicitly exercise all
three differences. No fallback scanner is proposed.

README stays an explicit projection: preserve existing curated titles by ADR
number, seed new titles from headings, sort rows, and apply the current marker
splice/render rules. Status and heading edits keep the exact existing textual
rules; parser adoption adds no new rejection or newline-format guarantee.

## Nix execution and staged delivery

Export `packages.adr-promoter` from `nix/packages.nix` using the flake-locked
`python3Packages.buildPythonApplication` and `python3Packages.marko` 2.2.4;
pytest 9.1.1 is the offline `nativeCheckInputs` with package checks enabled.
Wrap its executable with runtime `gh`, Git and Prettier. A package-specific
source filter admits its Python sources/tests because the existing
`toolsSourceFilter` admits Cargo files only. No PyPI, `pip`, or mutable install
is used. Select the current checkout's package directly:

```just
adr-promoter:
    nix run --accept-flake-config --no-update-lock-file --no-write-lock-file .#adr-promoter -- controller
```

The workflow retains its existing single CI-shell entry, trusted-main checkout,
App-token setup, permissions and concurrency; it changes the final command to
`just adr-promoter`. `nix run .#adr-promoter` is selected from the active flake,
not a bare command on PATH, and does not enter another devShell. The legacy
xtask subcommands use the same Nix output with their fixed action. New package
source must be staged or intent-to-added before selection under ADR-0225's
source admission rule.

PR 1, with its own human review and merge gate, adds the Python package/Nix
output, tests and Just recipe; switches the workflow; changes `promote` and
`sync-readme` to Python delegates; and temporarily changes `promoter` to a
controller delegate. The Rust mutation bodies are removed in that same change,
so there is one writer. Observe a trusted-main workflow operation and its PR,
ref, and check result. PR 1 does not complete #1897.

The PR 1 caller migration includes `CONTRIBUTING.md`'s generic controller-rerun
guidance and the workflow caller moving to Just. PR 2 has a separate review and
human merge gate: after confirming those callers and trusted-main observation,
it removes the obsolete Rust controller, state machine, GitHub/Git adapters and
controller tests plus the temporary `adr promoter` CLI/help/dispatch/delegate.
The fixed `promote` and `sync-readme` delegates remain as existing
mutation/recovery command spellings; they contain no Rust policy or mutation.
#1897 completes only after PR 2 merges and acceptance passes; a failed
Nix-selected path blocks PR 2 until repaired or reverted.

## Consequences

Human approved this Python/library design and the Marko parser-eligibility
differences via `cd55b65a-4f7d-426d-991a-04cc256ec206` /
`issue-1897-python-design-v2`, against baseline
`82bd37273dd533026fbf07f0121865fe254583a8`. This numberless ADR remains
`proposed`: approval is not implementation, privilege, live-proof-as-unit-test,
merge, or cleanup authorization. Marko removes a custom Markdown link/code/fence
scanner; the offline experiment verifies byte-span mapping on targeted cases and
the current tracked corpus, not full-pipeline output. No accepted ADR Decision
text is edited; this draft records proposed clarifications to ADR-0028, -0087,
-0152 and -0225. ADR-0141's workspace/test boundaries remain unchanged. Future
edits to accepted ADRs remain subject to ADR-0127 and separate review.
