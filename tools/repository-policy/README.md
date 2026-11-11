# Native repository policy

This auxiliary package currently owns the Task 1 boundaries of issue #1898:

- `just adr-format` runs `repository-docs adr-format`, the independent read-only
  numbered-ADR format verdict.
- `just precommit` runs `git-index-safety precommit`. Just owns the named host
  check populations and fail-fast order; the native process owns one Git/index
  reconciliation, including after a failed host check.
- The x86_64-linux prepush Markdown slot runs `git-index-safety prepush-docs`,
  with clean/raw-index admission and a mandatory post-request check even after
  Nix failure.

Just uses locked Cargo from the active checkout. The separate native Nix output
is `.#repository-policy`; it provides both binaries and never runs them during
its derivation. Its Cargo source excludes product and other tools'
implementations. Shared workspace manifests, locks, vendor patches and toolchain
inputs remain real dependencies. `sourceAdmits` also admits Nix evaluation
inputs; `cargoSourceAdmits` describes the narrower compiled-source population. A
Nix-built executable refuses relevant nonignored untracked inputs before hook
preparation; native Cargo reads live sources. Neither path stages missing
inputs. Caller Git/checkout admission precedes best-effort hook configuration.
Artifact verdicts in retained tools keep their separate admission-bypass
boundary.

Exit 0 is success and 1 is a policy/safety/request failure. Invalid argv
exits 2. Checkout/source admission retains the caller boundary: precommit exits
1; repository-docs and prepush-docs exit 2. Required failures remain failures.
The docs request retains its GC root under `.xtask/gcroots/static-docs`, live
stderr and `.xtask/diagnostics/static-docs/build.log`; on failure, a
successfully captured log also produces `failure-excerpt.log`. Diagnostic-write
failures warn without hiding the primary verdict. There is no workflow report or
snapshot protocol.

The Git and numbered-ADR algorithms are shared with retained xtask consumers,
not copied into a second parser. Remaining policies and workflows are not yet
migrated. Run the focused auxiliary proof with:

```text
cargo test --locked --manifest-path tools/Cargo.toml -p repository-policy
```
