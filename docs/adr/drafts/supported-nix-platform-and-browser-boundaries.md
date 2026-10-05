# ADR-DRAFT: Supported Nix Platform and Browser Boundaries

- Status: proposed
- Date: 2026-10-05
- Issue: [#1684](https://github.com/jaunder-org/jaunder/issues/1684)

## Context

The pinned nixpkgs supports Apple Silicon Darwin but rejects x86_64-darwin.
Advertising flake-utils' default systems therefore includes an output family
that cannot evaluate. Linux's packaged Playwright browser and canonical Chromium
thumbnail environment are not available equivalently on Darwin.

[ADR-0191](../0191-emacs-protocol-client-flake-package-output.md) established an
independent Emacs Protocol Client output for every default flake system. Its
package role and dependencies remain valid, but system enumeration must reflect
the pinned package set rather than promising unsupported outputs.

## Decision

Enumerate x86_64-linux, aarch64-linux, and aarch64-darwin explicitly for all
per-system flake outputs, including the Emacs Protocol Client. This replaces the
default-system enumeration portion of ADR-0191, without changing the client's
output name or dependency composition.

Expose portable server, CSR, development-tool, and seed-helper packages on these
systems. Keep NixOS services and VM tests, ELF patching, diagnostic coverage
producers, canonical screenshot updates, and the theme-thumbnail environment
Linux-only.

Linux retains Nix-pinned Playwright browsers and fonts. Darwin uses the same
Nix-pinned Playwright test package but explicitly installs its matching browsers
into a writable cache keyed by package version. Shell entry must not download
browsers. Resolve the user cache path at shell runtime, respecting
XDG_CACHE_HOME and otherwise using the macOS Library/Caches directory.

## Consequences

Apple Silicon developers can build and run host browser investigations without
unsupported Linux packages leaking into the development shell. Intel macOS is
not advertised by this pin. Reintroducing it requires a supported package set
and native validation, not disabling platform checks.

Darwin's browser cache is not a hermetic Nix browser derivation. Linux CI
remains authoritative for canonical browser and screenshot verdicts. Actual
Safari evidence is distinct from Playwright WebKit evidence, and neither desktop
result proves iPhone/iPad Safari behavior.
