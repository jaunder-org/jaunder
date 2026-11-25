# Native repository policy

This auxiliary package owns the fixed source contracts (`source-contracts`), the
documentation and ADR checks (`repository-docs`), the narrow `scoped-leptosfmt`
formatter adapter, and the source-policy contracts other tools share (gate
markers, server-function inventory). For example, `dev adr-format` runs
`repository-docs adr-format`, the independent read-only numbered-ADR format
verdict.

Git and checkout admission come from the shared `tools/host-tool` library; this
package keeps only the wrapper that passes its compiled source predicate
(`docs/adr/drafts/narrow-repository-policy.md`).

Git/index custody (the precommit snapshot, staging reconciliation and the shared
`static-docs` admission and post-check) belongs to `dev` itself
(`tools/dev/dev/index.py`), not to this package.

`dev` uses locked Cargo from the active checkout. The separate native Nix output
is `.#repository-policy`; it provides the binaries and never runs them during
its derivation. Its Cargo source excludes the product and other tools'
implementations; host-tool is a shared library, not another tool's
implementation. Shared workspace manifests, locks, vendor patches and toolchain
inputs remain real dependencies. `sourceAdmits` also admits Nix evaluation
inputs; `cargoSourceAdmits` describes the narrower compiled-source population. A
Nix-built executable refuses relevant nonignored untracked inputs before hook
preparation; native Cargo reads live sources. Neither path stages missing
inputs. Caller Git/checkout admission precedes best-effort hook configuration.
Artifact verdicts in retained tools keep their separate admission-bypass
boundary.

Exit 0 is success and 1 is a policy failure. Invalid argv exits 2, and so does a
refused checkout/source admission. Required failures remain failures. There is
no workflow report.

The numbered-ADR algorithms are shared with retained consumers, not copied into
a second parser. Run the focused auxiliary proof with:

```text
cargo test --locked --manifest-path tools/Cargo.toml -p repository-policy
```
