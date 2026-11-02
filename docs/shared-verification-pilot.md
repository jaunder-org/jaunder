# Shared verification pilot: Markdown formatting

**Status:** #1862 established the leaf contract and bounded harness conclusions;
approved #1863 routes production narrow precommit and clean prepush through that
leaf on `x86_64-linux`, as specified below. Local actual-consumer proof is
complete for the reviewed implementation. Hosted CI and the final intended-tree
leaf verdict remain pending; CI routing itself is unchanged.

## Contract

The pilot check is the existing `.#checks.x86_64-linux.static-docs` derivation.
It copies the Nix-filtered source and runs `devtool check --group docs`; the
docs group contains only `prettier-markdown`, which verifies Markdown using
Prettier. Its verdict is the formatter/derivation exit status, not a status
artifact. Preserve ADR-0178's existing ordered, duplicate-free `static-docs`
then `static-code` partition; this pilot neither reorders nor expands it.

| Entry point                      | Current behavior                                                                                             | Contract to preserve in any separately approved routing                                                                                                                                                    |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Narrow staged-Markdown precommit | Existing ordered host Fix and distinct Markdown-sensitive checks, followed by one safe index reconciliation. | After success, require no unstaged/nonignored-untracked changes and raw/index identity for every tracked Markdown and formatter-config input; then request the leaf and verify Git state stayed unchanged. |
| Clean prepush                    | Clean-tree refusal, host verify-only checks, auxiliary/product tests and workspace doctests.                 | At the existing Markdown Verify position, replace only `prettier-markdown`; require the same full input identity before request and unchanged Git state afterward.                                         |
| CI                               | Existing x86_64-linux validation requests `static-docs` within its hermetic lanes and required aggregate.    | Preserve the required lane and aggregate. A shared-verdict harness is not evidence that production CI routing changed.                                                                                     |

The current staged-Markdown route is broader than this one leaf: it also runs
identifier/sequence checks, ADR checks, document-link checks and flow-document
parity. `prettier-end2end` is a separate Markdown-eligible host check and
remains outside `static-docs`. Do not describe the pilot as parity for the full
narrow route, broad precommit, `check`, or `validate`.

## Inputs and ownership

`staticDocsSrc` includes Markdown files, `.prettierrc.json` and
`.prettierignore`, excluding `.claude`. The tool identity is the full
`docsDevtoolBin` closure, not just the devtool source: its filtered tools source
includes `tools/{coverage,csr_bundle,devtool,doctests,traced-run}`,
`tools/Cargo.toml`, `tools/Cargo.lock`, and `csr/index.html`, plus the declared
Cargo source/dependency closure. `docsToolsSrc` generates optional-dependency
placeholders for `common`, `host`, `storage`,
`tools/diagnostic-coverage-runtime`, and `tools/performance`; include that
derivation and its pinned Rust/vendor inputs. The static derivation also pins
Prettier, its configuration, and the Nix platform. Compare evaluated derivation
identity and semantic inputs; a textual Nix edit alone does not imply
invalidation.

The admitted precommit changes remain limited to the existing staged-only set of
regular Markdown additions/modifications. Host finishes global
`prettier -w **/*.md` Fix and safe restaging first. A small status/raw-index
guard refuses residual unstaged or nonignored-untracked state and compares raw
worktree blob OIDs with index OIDs for every tracked input selected by
`staticDocsSrc`: Markdown, `.prettierrc.json`, and `.prettierignore` at any
depth, excluding `.claude`. This rejects clean/EOL conversion even for unchanged
files, so ordinary Git-flake worktree bytes match the indexed inputs. The clean
prepush consumer applies the same full-population guard. After either request,
the existing Git status and raw staged diff are compared; tracked, staged, or
nonignored-untracked mutation/query failure fails the hook without restaging,
while retaining any original Nix failure. No new eligibility or broader check
family is introduced. The request uses existing `build_check` for
`.#checks.x86_64-linux.static-docs` with
`--no-update-lock-file --no-write-lock-file`; tracked `flake.lock` must remain
unchanged.

Host owns Git admission/classification, formatting writes, restaging,
commit/push cleanliness, index reconciliation and hook ordering. Nix owns
immutable source/tool identity, sandbox execution and the derivation verdict.
Nix does not inspect or mutate the live index. Keep the current host-only
`xtask` boundary.

## Accepted proof conclusions and limits

The bounded x86_64-linux non-production harness compared the complete filtered
docs input and docs-tool source/lock closure with an alternate admitted index,
refused a stale source snapshot before request, and passed unchanged-content
post-Fix, commit-equivalent, clean-prepush and CI-leaf model requests at one
static-docs identity. A Markdown change changed identity; a tested unrelated
Rust-source change did not. A deliberate formatter failure reached the Nix
request and propagated failure in each request model. Re-requesting a valid
output demonstrated local-store reuse; this does not establish substituter use
or a builder rerun. `allowSubstitutes = false` does not force a rebuild when a
valid local output exists.

The required status-as-data control used the existing doctests producer and
`doctests-gate`, without migrating doctests: the producer emitted a failing
status and the gate rejected it. The producer's exit-0 run receipt was pruned
and remains executor-reported, while the output status, gate dependency and
failing gate verdict were retained and independently reviewed.

The harness receipts establish only their recorded leaf-level behavior and do
not replace Git-backed production hook proof. Production routing is
x86_64-linux-only: host/stage/admission failures skip the precommit leaf while
retaining one safe reconciliation, and leaf failure does not restage again;
prepush replaces only Markdown Verify after clean-tree admission. Other
platforms retain prior host behavior. The guarantee assumes a cooperative single
writer, not immunity to concurrent external mutation. CI retains its existing
hermetic leaf and aggregate; no workflow duplication or broader platform
equivalence is claimed.

### #1863 local production-consumer proof

A temporary tracked Markdown input exercised the actual narrow precommit path:
host Prettier Fix changed the input, the existing safe reconciliation staged
that fix once, the complete Markdown/config raw-worktree-to-index guard passed,
and the ordinary Git-backed `static-docs` leaf passed. A normal commit hook and
clean prepush then requested that same derivation for the same proof content;
the local-store output was reused. This establishes local reuse for that exact
proof input, not substituter use or builder execution a second time.

Actual precommit and clean prepush both refused a clean CRLF-converted tracked
Prettier config before requesting Nix. A temporary real builder fault also
propagated through actual precommit, prepush, and `ci-validate hermetic`:
precommit retained one reconciliation and no retry; prepush stopped before later
phases; hermetic reported the failed docs leaf while its independent static-code
check still passed. The exact current required-aggregate predicate was
negatively exercised against a failed lane. These are local proof controls, not
hosted CI results.

The proof input and fault were temporary and are absent from the intended tree.
The successful derivation included that temporary Markdown file, so it is not a
final intended-PR tree verdict. The earlier #1862 harness derivation/results are
historical foundation only; source/docs changed afterward, so its identity is
not the current final-tree identity. Obtain normal final-tree leaf verification
and hosted required CI on the eventual delivered ref before claiming delivery.

## Ownership and update trigger

Production routing is owned by #1863 and follows the approved narrow contract
above. Preserve producer-plus-verdict coupling for status-as-data checks; do not
treat producer success alone as the verdict.
