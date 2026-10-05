# Issue #1684 — macOS development validation

## Boundary and implementation

- Rebased infrastructure work onto `origin/main` at `f777479b`, which includes
  #1685's portable runtime identity fix. Removed the superseded local Linux-only
  shutdown workaround; use upstream runtime identity and shutdown behavior.
- Linux retains the existing Nix browser/font/thumbnail environment. Darwin uses
  the Nix-pinned Playwright CLI with explicit browser installation into a
  writable versioned cache. No browser download occurs during shell entry.
- Darwin browser cache paths must be exported by the shell hook: `$HOME` inside
  a plain `mkShell` environment attribute stays literal. Cache location respects
  `XDG_CACHE_HOME`, falling back to `$HOME/Library/Caches`.
- Portable package outputs are server, site, CSR WASM/bundle, devtool, and
  test-support. Thumbnail and diagnostic coverage outputs remain Linux-only.
- Supported systems are x86_64-linux, aarch64-linux, and aarch64-darwin. The
  pinned nixpkgs rejects x86_64-darwin; advertising it via eachDefaultSystem was
  invalid.

## Evidence on aarch64-darwin

After rebasing, the first focused run successfully built the CSR/server/support
binaries, started the server and collector, and seeded the database. Browser
launch failed because the corrected cache did not yet contain WebKit.

Explicit provisioning:

```sh
nix develop .#ci -c playwright install webkit
```

Focused proof:

```sh
nix develop .#ci -c cargo xtask e2e-local post-image-fit.spec.ts --browser webkit
```

Result: **5 passed (16.7s)**; xtask returned success, including its panic gate.
Retained trace:

`.xtask/e2e-local/run-ORHkVy/webkit/capture/otel-traces.jsonl`

Command log: `/tmp/issue-1684-webkit-provisioned.log` (host-local, not durable).
A telemetry meter-shutdown warning appeared, but the runner returned success;
this is not a claim that telemetry shutdown was warning-free.

The platform regression expression returned `true` on this host. It verifies
public output boundaries across all supported systems and forces native shell
and portable-package derivation evaluation. Foreign derivations require native
runners because source preparation uses import-from-derivation.

`cargo fmt --check` and `git diff --check` also passed.

The commit gate's full host tests additionally exposed two Darwin assumptions: a
worktree-root test compared canonical paths to `/tmp` aliases, and the
production-baseline lease helpers read UID and process start time from `/proc`.
The root test now compares canonical paths; lease helpers use rustix's effective
UID and processkit's platform process-info reader. Both focused regressions
pass. This makes the host lease test portable; NixOS production-baseline
execution still requires Linux.

The 12 `steps::e2e_local::tests` tests passed. A new platform guard refuses
`--update-visual-snapshots` on non-Linux hosts before artifact preparation;
executing that command on macOS returned the expected Linux-only error in 0 ms.
Linux update plans and independent Chromium/Firefox lifecycles remain unchanged.
The platform regression expression also runs in the existing Linux host CI lane.

Host-path audit: `host_server`, `sandbox`, `e2e_local`, and devtool's
PostgreSQL, provisioning, and CSR-bundle paths contain no remaining `/proc` or
ELF patching assumptions. `xtask/src/steps/nix.rs` deliberately targets
x86_64-linux checks and VM artifacts; it requires a Linux builder rather than
being a portable host lane. This macOS host has no remote Linux builder
configured, so Linux execution is left to authoritative CI.

## Actual desktop Safari proof

Safari initially refused WebDriver session creation because remote automation
was disabled. After the operator enabled it, actual Safari 27.0.1
(22625.1.29.11.28), macOS platform build 26A434, completed eight measurements
through Apple's `/usr/bin/safaridriver` (not Playwright WebKit or a simulator).

The named `cargo xtask sandbox issue-1684-safari --profile standard` instance
served a published Markdown Post with locally uploaded 1200×800 and 80×50 SVG
images. Authentication, uploads, and Post creation used same-origin requests
inside Safari. Signed-out checks removed both the Session cookie and advisory
localStorage marker, then loaded Local and the public permalink. An initial
probe retained that marker and redirected to login; it was corrected before
collecting the final evidence.

Final geometry (CSS pixels; actual viewport widths, not just requested sizes):

| Surface          | Signed in | Viewport width | Wide image  | Body content width |
| ---------------- | --------- | -------------- | ----------- | ------------------ |
| Home             | yes       | 390            | 266×177.328 | 266                |
| Home             | yes       | 1280           | 924×616     | 924                |
| Owner permalink  | yes       | 390            | 202×134.656 | 202                |
| Owner permalink  | yes       | 1280           | 860×573.328 | 860                |
| Local            | no        | 390            | 266×177.328 | 266                |
| Local            | no        | 1280           | 924×616     | 924                |
| Public permalink | no        | 390            | 202×134.656 | 202                |
| Public permalink | no        | 1280           | 860×573.328 | 860                |

In all eight cases the small image remained 80×50, the wide image fit the body
without changing its natural aspect ratio (within subpixel rounding), and
measured document overflow was zero. Screenshots were scrolled to show the Post
body and visually inspected for Home and Local at the narrow viewport.

Retained local evidence: `.xtask/issue-1684-safari/geometry.json`, eight PNGs,
`setup.json`, and sandbox/driver logs. The named sandbox remains available for
follow-up inspection; the supervised server and Safari driver were stopped.
These fixtures establish built-in-theme desktop Safari behavior, not every
custom Theme Package, image format, or mobile Safari variant.

## Remaining acceptance evidence

- Actual iPhone/iPad Safari remains separate device-specific verification;
  desktop Safari and Playwright WebKit do not establish mobile Safari behavior.
- Native Linux regression execution and canonical screenshot/thumbnail parity.
  No image CSS changes or tentative #1677 checkout edits were imported here.
