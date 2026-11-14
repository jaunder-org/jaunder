# Stop devShells from inheriting the host's `LD_LIBRARY_PATH` (#1947)

## Outcome

Entering any project devShell on Linux (`default`, `ci`, `mutants`, `ci-e2e`)
sets `LD_LIBRARY_PATH` to exactly the Nix OpenSSL and dav1d library directories.
The host's own value is dropped. Playwright's pinned Chromium then starts from a
fresh `nix develop` shell without `env -u LD_LIBRARY_PATH`.

## Cause

Each shellHook runs
`export LD_LIBRARY_PATH="<openssl>/lib:<dav1d>/lib:$LD_LIBRARY_PATH"`. On this
NixOS host the ambient value already carries `alsa-lib-1.2.16.1/lib` and
`pipewire-1.6.11-jack/lib`, built against the host's newer glibc (2.43).
Chromium loads the host's alsa-lib alongside its own older glibc and fails with
`GLIBC_2.43 not found`. Reproduced on 2026-11-14 with the pinned
`chrome-headless-shell --version`:

| `LD_LIBRARY_PATH`                  | Result |
| ---------------------------------- | ------ |
| devShell value (with host entries) | exit 1 |
| OpenSSL + dav1d only               | exit 0 |

The devShell's own entries are harmless; only the inherited tail breaks it. When
the host leaves the variable unset (as CI runners do), the current hook also
leaves a trailing `:`, an empty entry that ld.so reads as "the current
directory".

## Approach and tradeoffs

In `nix/dev-shells.nix`, change the assignment from prepend to replace. Bind the
export line once in the `let` block and use it in both the shared `shellEnv`
hook and the `ci-e2e` hook, so the two cannot drift. The OpenSSL-then-dav1d
order stays the same.

Update the test in `tools/host-proof/tests/dev_shell_tmpdir.rs`. Today it parses
the `ci-e2e` hook text and requires the `:$LD_LIBRARY_PATH` suffix. It will
instead run every evaluated hook twice, once with an inherited sentinel value
and once with the variable unset. Both times it compares the value after entry
to the exact OpenSSL-then-dav1d list, which means no sentinel and no empty
entry. On Darwin the hooks set nothing, so the test checks that an inherited
value passes through unchanged.

- **Simpler alternative rejected:** unset the variable only around browser
  launch in `just e2e-local`. That fixes one caller and leaves the rest of the
  shell exposed to the same mismatch: any Nix binary run in the shell can load
  host libraries built for a different glibc. It also adds a special case.
- **Accepted cost:** a developer who deliberately exported `LD_LIBRARY_PATH`
  before entering the shell loses it inside. They can re-export it after entry.
  Nothing in the repo (`Justfile`, `recipes/`, `scripts/`, `tools/`, `.github/`)
  reads inherited entries, and CI already runs with the variable unset.
- #1879 kept the inherited tail to stay behavior-identical with `.#ci`, not
  because something needed it. CI runners have the variable unset, so CI's
  effective value changes only by losing the empty trailing entry.

## Acceptance

Proof to execute:

- From a fresh `nix develop` shell, with the host's `LD_LIBRARY_PATH` set as it
  is today, `just e2e-local auth.spec.ts:375` passes without unsetting the
  variable.
- `cargo test --locked --manifest-path tools/Cargo.toml -p host-proof --test dev_shell_tmpdir`
  passes, and the updated test fails against the old prepend hook.
- The pre-push and CI gates stay green.

Deliverables: the `nix/dev-shells.nix` change, the updated test, and a sentence
in `CONTRIBUTING.md`'s development-setup section. That sentence will say that
entering any Linux shell, `ci-e2e` included, replaces an inherited
`LD_LIBRARY_PATH`. The same section also says the regression test covers "all
three shells"; that will be corrected to four.

## Boundaries

- No change to Nix check derivations (`nix/checks.nix`, `nix/packages.nix`).
  Their `LD_LIBRARY_PATH` lives inside a sandbox or VM, not a host shell.
- No change to the OpenSSL/dav1d library set, browser pinning or Darwin behavior
  (Darwin never sets the variable).
- No new wrapper or e2e-specific environment handling.
