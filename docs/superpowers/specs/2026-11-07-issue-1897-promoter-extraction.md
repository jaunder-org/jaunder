# Approved design — Issue #1897

**Authority.** Human approved “Approve Python design and parser changes” via
`cd55b65a-4f7d-426d-991a-04cc256ec206` / `issue-1897-python-design-v2`, against
baseline `82bd37273dd533026fbf07f0121865fe254583a8`. This approves this
Python/library design, including the named pinned library direction, Marko
parser-defined eligibility differences, single mutation owner, and separately
gated two-PR delivery—not implementation, privilege changes, live proof as a
unit substitute, either PR merge, or cleanup. Preserve accepted automation and
ADR-0152 guarantees. Earlier scope relay: `e7438a2f-7b73-432e-b765-aab1afb5b811`
/ `issue-1897-extraction-scope-v1`.

## Outcome

Move the ADR promoter controller and its deterministic promotion mutation from
xtask into a Nix-selected Python host package. The workflow invokes that package
with `just adr-promoter`; existing `cargo xtask adr promote` and
`adr sync-readme` commands become narrow delegates to the same Python
executable. The `cargo xtask adr promoter` command delegates only during PR 1,
then is removed in PR 2 after its workflow and guidance callers migrate. Keep
one mutation implementation and the existing authority, recovery, staging, exit
behavior, and two separately gated PR deliveries.

## Package and library choices

Put `pyproject.toml` and `adr_promoter/` under `tools/adr-promoter/`; it is not
a Cargo member. Define the `adr-promoter` console script in project metadata.
Keep the package small: `__main__.py` owns the three fixed CLI actions and exit
mapping; `controller.py` owns records, pure selectors and state transitions;
`github.py` owns the `gh` process/JSON/error adapter; `git.py` owns argv Git and
leases; `promotion.py` owns admission, Marko-backed edits, staging and README
synchronization. Tests live beside the package under `tests/`. Use ordinary
functions and small records—not a translation of the Rust trait design.

Use **Marko 2.2.4** with its GFM extension for Markdown structure and source
positions. The AST exposes block `source_span` / inline `syntax_spans`; inline
`Link` and `Image` nodes expose destination spans, including links in GFM table
cells. Parse the original UTF-8 text and patch only the destination spans in the
original byte buffer. Do not render an AST back to Markdown. Marko's offsets are
character indices into normalized text (CRLF, CR, form-feed and NUL are
normalized), not UTF-8 byte offsets; a small boundary map translates those
indices to original byte offsets. That adapter is offset conversion, not a
Markdown parser or hand-built link scanner.

Locked nixpkgs currently resolves Python 3.14.7, Marko 2.2.4 and pytest 9.1.1.
Use pytest for offline parametrized tests; Hypothesis is not required by the
observed contracts. Tree-sitter 0.25.2 with tree-sitter-markdown 0.5.1 was
rejected: it exposes byte positions but missed GFM table links in a fake input
and differed from the Rust scanner in four files during corpus comparison.
`markdown-it-py` 4.2.0 exposes block line maps, not inline byte spans. Reviewed
`cmarkgfm` / `multimark` interfaces do not document structured inline
destination spans. GitPython does not improve exact expected-OID Git leases over
the Git CLI. These are measured tradeoffs, not assumptions that a parser
serializer preserves source.

Use the existing `gh` CLI for API operations and the workflow's App token
(`GH_TOKEN`). It supplies the current GraphQL fields and request/auth mechanics;
PyGithub's App auth, pagination and typed errors do not cover these exact
merge-queue GraphQL queries and would add a second transport/auth path. Use
argv-based subprocess calls, captured stdout/stderr, and Python JSON parsing.
Treat nonzero HTTP results as errors after examining the response; treat a
GraphQL `errors[]` array as failure even when `gh` exits zero. Map 404 / GraphQL
`NOT_FOUND` to absence only at the current PR-lookup sites. Malformed JSON,
other GraphQL errors, transport/auth failures, and rate limits remain errors;
retain the current one rate-limit-reset lookup and warning behavior. The
merge-group query intentionally reads one current page (`per_page=100`), then
validates each candidate's commit parents. Do not add `--paginate` or silently
broaden the candidate set. This small error adapter preserves existing
controller behavior; it is not a replacement SDK.

Use pinned Git CLI for worktree, staging and ref operations. Retirement uses the
observed expected-OID lease and verifies absence; a wrapper library does not
strengthen that guarantee. Do not use shell interpolation, add credentials,
change GitHub permissions, or invent retries.

## Existing behavior and explicit parser-eligibility differences

Promotion order remains: sort draft slugs; allocate every destination; validate
all headings and whole-set tracked admission before the first move; `git mv` and
stage; rewrite heading/status and eligible relative links; replace every
assigned draft path stem in tracked files; sync README only when both markers
exist; stage; then inspect promoted files and report link problems as warnings.
This is not an all-I/O transactional rollback. The controller retains the
current publication order after mutation: check for staged changes and required
queue contexts, run Prettier on staged Markdown, stage its results, then commit.
The Python runtime therefore includes the pinned Prettier binary, and formatting
failure remains a visible controller error.

Read and write the source files as bytes; decode UTF-8 only for parser/record
inspection. Keep the current heading rule: exact initial `# ADR-DRAFT: ` prefix
and a nonempty first-line title; patch only that prefix. Keep the current status
rule: inspect the first line whose trimmed form starts `- Status:` or `Status:`;
replace only if its trimmed remainder is exactly `proposed`. Preserve other
statuses, malformed multi-token remainders, prose, indentation, and line
terminators as today. Citation replacement remains the literal ASCII path-stem
replacement over tracked-file bytes, not a Markdown-only rewrite.

Use Marko `Link` / `Image` destination spans for relative-link adjustment,
including inline links inside GFM tables. Leave reference-style definitions and
parser-recognized code unchanged. This eligibility is broader than the current
Rust scanner's exclusions: the Rust scanner masks fenced blocks and single-line
backtick spans, but does not mask indented code blocks or multiline code spans.
Marko will leave links in those code constructs unchanged, where the Rust
scanner could rewrite their `](target)` substring. These are approval-visible
behavior changes, not claimed relocation:

```text
    [x](../indented.md)
`code
[x](../multiline.md)`
prose ](../not-a-link.md)
```

The first two are code under Marko and the last is not a link; the old scanner
would rewrite each target. Apply the existing target rule to eligible links: for
a target other than HTTP(S), `mailto:`, or a fragment, remove one leading `../`
only when a nonempty remainder follows. Keep `..`, `../`, bare names, and
noninitial `../` unchanged. Patch only source spans in the original bytes; do
not serialize Markdown.

**Approved parser-eligibility differences:** the Rust scanner also rewrites an
orphan substring such as `prose ](../x.md)` even though it is not a parsed
Markdown link. Marko leaves that malformed text alone. The offline corpus probe
found zero raw link-span differences across 295 tracked Markdown files and two
promotable drafts, but its fixtures did not establish equivalence for indented
code blocks or multiline code spans. Do not overstate corpus equality as
universal equivalence. Human approval explicitly accepts parser-defined
eligibility for real Markdown links/images, including all three differences:
malformed orphan substrings, indented code blocks, and multiline code spans. Do
not add a custom fallback scanner to conceal the delta. No new parser error,
file rejection, or stricter validation is proposed.

README generation remains its explicit table projection, not an AST round trip:
retain a row's curated title by ADR number, seed a new row from its heading,
sort rows, and replace the marker-delimited table using the existing splice and
render behavior. Read/write bytes (decoding and encoding UTF-8 without universal
newline translation); do not claim newline-style or surrounding-blank-line
preservation beyond the current splice behavior. Promotion skips a missing or
markerless README as today. Keep the standalone `sync-readme` operation's
existing read/error behavior.

## Ownership and command compatibility

There is one mutation owner: Python implements `promote` and `sync-readme`, and
the controller calls those same Python functions. Remove the Rust mutation
bodies (`run_promote` and `sync_readme_at`) in PR 1; do not leave a second
writer. Keep the public `cargo xtask adr promote` and
`cargo xtask adr sync-readme` spellings as fixed-action delegates to the
selected Python package. `sync-readme` is documented recovery; `promote` remains
the existing automation primitive. During PR 1 only, `cargo xtask adr promoter`
also delegates to the Python `controller` action while the workflow/guidance
move to Just. PR 2 removes that controller CLI/help/dispatch/delegate after the
trusted-main invocation is observed. These are fixed command adapters, not a
generic RPC or serialized function-call interface, and they preserve child
status and diagnostics. Keep the feature-author prohibition on direct promotion
in `CONTRIBUTING.md` and `docs/adr/drafts/README.md`, retain the documented
`sync-readme` recovery command, and update `CONTRIBUTING.md`'s generic
controller rerun guidance to `just adr-promoter` before removing `adr promoter`
in PR 2.

Rust remains the read-only consumer for `adr-format`, `adr-readme-parity`,
`adr-view-parity`, `doc-links`, and sequence/collision checks. Keep the parsers
and reports those gates call, but no Rust promotion or README writer. Python
promotion tests must prove final output passes the Rust gates. The Python
controller does not depend on or invoke xtask.

Every successful controller outcome (`IgnoredEvent`, `NotRearmed`, `Existing`,
`NoChanges`, `Created`, `Replaced`, `Rearmed`) prints useful detail and exits 0.
A safe policy refusal is not a workflow error. Actual API/JSON/process failures,
unsafe inconsistent state, or an unverified postcondition go to stderr and exit
nonzero. If a write command reports failure, observe state and accept it only if
the exact desired postcondition is verified. The two mutation delegates preserve
success as zero and failure as nonzero; no error becomes a safe refusal merely
to make CI green.

## Nix-selected invocation

Export `packages.adr-promoter` from `nix/packages.nix` using the flake-locked
nixpkgs `python3Packages.buildPythonApplication` and Python 3.14.7. Runtime
`marko` is 2.2.4; pytest 9.1.1 is the offline `nativeCheckInputs` with
`doCheck = true`. Its console entry point is `bin/adr-promoter`. Wrap that entry
point in a `writeShellApplication` whose runtime PATH contains `gh` 2.100.0, Git
2.55.0 and Prettier 3.9.6 for the controller's staged-Markdown formatting step;
expose the wrapper as `packages.adr-promoter` with
`meta.mainProgram = "adr-promoter"`. The package-specific source filter must
admit `tools/adr-promoter/` sources and tests; the existing `toolsSourceFilter`
admits Cargo sources, not this Python package. No PyPI, pip, ambient Python, or
mutable install.

The package output, not a bare PATH lookup, is selected from the active checkout
with this recipe:

```just
adr-promoter:
    nix run --accept-flake-config --no-update-lock-file --no-write-lock-file .#adr-promoter -- controller
```

The workflow retains its one existing CI-shell entry and token/checkout/
concurrency setup, changing only its command to `just adr-promoter`. Nix `run`
selects the current checkout's flake output, executes from the repository root,
and does not enter a nested devShell or select an ambient command. The legacy
xtask delegates use the same Nix output with fixed actions `promote`,
`sync-readme`, or `controller`. New nonignored package source must be staged or
intent-to-added before Nix selection, per ADR-0225. Missing inputs or failed
realization are errors, never fallback to an old executable.

Read-only offline verification resolved locked nixpkgs revision
`d6524aaca2ff07876657ae2b323f24be4874944b`, evaluated the exact Python/Marko/
pytest/GitHub/Git/Prettier versions, and successfully built a Python environment
with Marko and pytest. The existing current-checkout `.#xtask` output path also
resolves through `nix eval`; a cold `nix run .#xtask -- --help` exceeded its
120-second bound. The proposed package does not exist yet, so no claim is made
that its executable has run. The direct, locked local-flake selection path is
specified; no lockfile or Nix source changed.

## Preserved controller and staged delivery

Preserve trusted-main authority; separate generation coalescing from per-PR
dequeue recovery; no active cancellation; immutable open head/diff; only the two
documented controller-owned orphan resumptions; changed-ref fail-closed
behavior; GitHub conflict plus exact-local-merge proof; leased ref deletion and
verified absence; exact event/PR/head/ref/group/check correlation; verified arm;
and visible workflow/PR/ref failure state. Promotion remains acceptance and the
merge queue the only writer to `main`. Strict FIFO, event ledgers, artifact
queues, new privileges, and shell control are out of scope.

**PR 1**, separately reviewed and human-merge-gated, adds the Python package,
locked Nix output/runtime and tests; changes the workflow to
`just adr-promoter`; converts `adr promote`/`sync-readme` to Python delegates;
and temporarily makes `adr promoter` a delegate. Rust mutation bodies are
removed in the same change, leaving exactly one writer. PR 1 does not complete
#1897. Observe a trusted-main workflow operation and ordinary PR/ref/check
result.

**PR 2**, with its own review and human merge gate, updates `CONTRIBUTING.md`'s
controller rerun instruction and any remaining actual callers to Just, then
removes the Rust controller/state machine, its GitHub/Git adapters and
controller tests, plus the temporary `adr promoter` CLI/help/dispatch/delegate.
The `adr promote` and `adr sync-readme` delegates remain as fixed
mutation/recovery commands; they contain no Rust policy or mutation. #1897
completes only after PR 2 merges and acceptance passes. If PR 1's Nix-selected
path fails, retain PR-1 compatibility and repair/revert before proposing PR 2.

## Acceptance after spec approval

- Locked Nix evaluation confirms the Python/Marko/pytest/GitHub/Git/Prettier
  package attributes; offline Nix builds the Python test/runtime closure; the
  Just recipe selects the exact current-checkout `packages.adr-promoter` output
  and its runtime closure.
- Fake GH/Git adapters cover controller transitions, expected-OID lease refusals
  and verification, exact event/PR/head/ref/group/check identity, pending-rerun
  precedence, safe refusal vs operational error, and every exit outcome. No live
  GitHub or privileged operation is a unit proof.
- Marko source-span tests cover GFM table links, inline links/images, fenced and
  indented code, single- and multiline code spans, reference definitions, link
  titles, Unicode and CRLF/CR/form-feed/NUL-to-byte mapping. The current probe
  proves source-span parity against the Rust scanner and byte-exact link-only
  rewrites; its heading/status assertions are spot checks, not full-pipeline
  output proof.
- Before implementation acceptance, use fake Git/`gh` adapters and a temporary
  tracked fixture repository with two cross-referencing drafts plus other
  tracked citation files. Run the full `promote` path and compare complete
  resulting file bytes, assigned numbers, staged paths, headings/status,
  citation replacements, Marko link edits, README markers/curated titles, and
  final warnings against explicit expected outputs. Also test `sync-readme`
  directly and run the existing Rust ADR format/parity/link gates on the result.
  Include all three parser-eligibility differences as approved fixtures; do not
  claim the existing probe already proves full-pipeline equivalence.
- PR 1 and PR 2 each pass independent review and their own human merge gate.

## Boundaries and approval

Human approval covers the Python/library design and parser behavior in this spec
at the stated baseline. The separate ADR remains a numberless `proposed`
decision record; architecture prose remains prospective, not a claim of shipped
behavior. Approval authorizes neither implementation, privilege changes, live
promotion proof as a unit substitute, either PR merge, nor cleanup. No source,
Nix expression, workflow, lockfile, accepted ADR Decision, generated ADR index,
tracker, privilege, or live automation is changed by this design.
