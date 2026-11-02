# Shared verification pilot: Markdown formatting

**Status:** human-approved x86_64-linux proof contract, under Manager-relayed
request `2f4fdfaf-0b52-43dc-a70f-c0aebe353e2d` (`pilot1862-markdown-v1`), for
bounded proof only. Production hook/CI routing is not implemented. This page
records the enduring contract and qualified proof conclusions from #1862 for its
downstream consumer, #1863. It is not a description of the current local hooks
and does not authorize implementation or policy changes.

## Contract

The pilot check is the existing `.#checks.x86_64-linux.static-docs` derivation.
It copies the Nix-filtered source and runs `devtool check --group docs`; the
docs group contains only `prettier-markdown`, which verifies Markdown using
Prettier. Its verdict is the formatter/derivation exit status, not a status
artifact. Preserve ADR-0178's existing ordered, duplicate-free `static-docs`
then `static-code` partition; this pilot neither reorders nor expands it.

| Entry point                      | Current behavior                                                                                                                  | Contract to preserve in any separately approved routing                                                                                                  |
| -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Narrow staged-Markdown precommit | Host Fix-mode formatting and Markdown-sensitive host checks, followed by host index reconciliation.                               | Model/request only after global host Fix and safe restaging; compare the immutable Nix input to the resulting admitted index before requesting the leaf. |
| Clean prepush                    | Clean-tree refusal, then host verify-only checks, auxiliary/product tests and workspace doctests; no hermetic derivation request. | A future consumer must establish clean-tree admission before requesting the same leaf; retain all existing host checks.                                  |
| CI                               | Existing x86_64-linux validation requests `static-docs` within its hermetic lanes and required aggregate.                         | Preserve the required lane and aggregate. A shared-verdict harness is not evidence that production CI routing changed.                                   |

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

The admitted precommit input is limited to the existing staged-only set of
regular Markdown additions/modifications. Host finishes global
`prettier -w **/*.md` Fix and safe restaging first. The Nix snapshot must match
the admitted index for every relevant source/config input; mismatch refuses the
shared request rather than using stale input. No partially staged, unstaged,
untracked, deleted, renamed, non-Markdown or otherwise dirty-state expansion is
included.

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

These results establish harness behavior only. No actual hook or CI workflow was
routed through Nix; no production equivalence, other platform, broader check
family, full validation, or untested tool/config invalidation is claimed. The
x86_64-linux result is not cross-platform evidence. No ADR or production policy
changed.

## Ownership and update trigger

#1863 exclusively owns any production hook/CI routing and must update this page
and the current verification guide to match the implemented behavior and
validated scope. Do not infer routing, platform expansion, or admission changes
from this approved pilot. Those decisions require their own approval. Preserve
producer-plus-verdict coupling for status-as-data checks; do not treat producer
success alone as the verdict.
