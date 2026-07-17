//! Stage the runtime CSR site tree into `$OUT_DIR/site/` for `rust-embed`.
//!
//! The server embeds its own client bundle (`pkg/jaunder.{js,wasm}` + the
//! precompressed `.br`/`.gz` siblings + wasm-bindgen `snippets/`) and the
//! `public/` assets (`favicon.ico`) so a released `--release` binary is
//! self-contained (ADR-0003/0008, #237). `server/src/site.rs`'s
//! `#[folder = "$OUT_DIR/site"]` embeds whatever this script stages.
//!
//! Sources:
//! - `pkg/` ← `JAUNDER_CSR_BUNDLE_DIR` (the bundle dir itself, e.g. Nix's
//!   `csrWasmBundle`) if set, else `<workspace>/target/site/pkg` (the host
//!   `cargo xtask build-csr` output).
//! - `public/` ← `JAUNDER_PUBLIC_DIR` if set, else `<workspace>/public`; its
//!   contents land at the `$OUT_DIR/site/` root.
//!
//! Fail-closed rule (env-keyed, **not** `PROFILE`-keyed — crane's shared
//! release `buildDepsOnly` runs this with no bundle env, so a profile-keyed
//! panic would break the whole Nix graph):
//! - `JAUNDER_CSR_BUNDLE_DIR` **set** but its dir is missing / has no
//!   `jaunder.wasm` → `panic!` (a declared-release build must never ship an
//!   empty client).
//! - env **unset** and the host default is absent → warn and stage an **empty**
//!   `$OUT_DIR/site` so the crate still compiles (keeps `buildDepsOnly`,
//!   coverage, clippy, and a bare `cargo build` green). With an empty embed the
//!   handler finds no asset and serves the SPA shell for every path — the CSR
//!   client can't boot, but the build is green until a bundle is staged.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// What to do with the CSR `pkg/` source, given whether a usable bundle was
/// found (`pkg_ok`) and whether the build **declared** it needs one by setting
/// `JAUNDER_CSR_BUNDLE_DIR` (`bundle_declared`). Names the load-bearing
/// fail-closed rule in one place; `main` is thin build-time fs/env plumbing.
#[derive(Debug, PartialEq, Eq)]
enum PkgAction {
    /// A usable bundle exists — stage it into the embedded site.
    Stage,
    /// The build declared it needs the bundle but none is present — a release
    /// artifact must never ship an empty client, so fail the build.
    FailClosed,
    /// No bundle env and none on disk — a developer/deps-only/coverage build;
    /// stage an empty site so the crate still compiles.
    TolerateEmpty,
}

/// The fail-closed decision: declared-but-missing fails; unset-and-missing is
/// tolerated. Never keyed on build profile (crane's shared release
/// `buildDepsOnly` runs this with no bundle env).
fn decide_pkg_action(pkg_ok: bool, bundle_declared: bool) -> PkgAction {
    if pkg_ok {
        PkgAction::Stage
    } else if bundle_declared {
        PkgAction::FailClosed
    } else {
        PkgAction::TolerateEmpty
    }
}

fn main() {
    // crap:allow: a build script — structurally unreachable by the coverage test
    // harness (build.rs runs at build time, has no test target), so 0 coverage is
    // inherent, not a gap. It is thin env-resolution + fs staging; the
    // load-bearing fail-closed rule is named in `decide_pkg_action`, and the
    // panic path is exercised by AC2's deliberate empty-bundle build.
    let Some(out_dir) = env::var_os("OUT_DIR") else {
        panic!("cargo sets OUT_DIR for build scripts");
    };
    let out_dir = PathBuf::from(out_dir);
    let Some(manifest_dir) = env::var_os("CARGO_MANIFEST_DIR") else {
        panic!("cargo sets CARGO_MANIFEST_DIR for build scripts");
    };
    let manifest_dir = PathBuf::from(manifest_dir);
    // `CARGO_MANIFEST_DIR` is `server/`; the workspace root is its parent.
    let Some(workspace) = manifest_dir.parent() else {
        panic!("server/ has a parent workspace dir");
    };
    let workspace = workspace.to_path_buf();

    let site_dir = out_dir.join("site");

    // Rerun when either source env var changes.
    println!("cargo:rerun-if-env-changed=JAUNDER_CSR_BUNDLE_DIR");
    println!("cargo:rerun-if-env-changed=JAUNDER_PUBLIC_DIR");

    // Resolve the pkg source: the env value IS the pkg dir (Nix's `csrWasmBundle`
    // root holds `jaunder.wasm` directly), else the host `target/site/pkg`.
    let bundle_env = env::var_os("JAUNDER_CSR_BUNDLE_DIR");
    let (pkg_src, bundle_declared) = match &bundle_env {
        Some(dir) => (PathBuf::from(dir), true),
        None => (workspace.join("target/site/pkg"), false),
    };
    println!("cargo:rerun-if-changed={}", pkg_src.display());

    // Resolve the public source.
    let public_src = match env::var_os("JAUNDER_PUBLIC_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => workspace.join("public"),
    };
    println!("cargo:rerun-if-changed={}", public_src.display());

    // Start from a clean staging dir (OUT_DIR persists across incremental
    // builds, so clear any stale copy before re-staging).
    let _ = fs::remove_dir_all(&site_dir);
    fs::create_dir_all(&site_dir)
        .unwrap_or_else(|e| panic!("creating {}: {e}", site_dir.display()));

    let pkg_ok = pkg_src.is_dir() && pkg_src.join("jaunder.wasm").is_file();

    match decide_pkg_action(pkg_ok, bundle_declared) {
        PkgAction::Stage => copy_tree(&pkg_src, &site_dir.join("pkg")),
        PkgAction::FailClosed => panic!(
            "JAUNDER_CSR_BUNDLE_DIR is set to {} but that dir is missing or has no jaunder.wasm; \
             a release binary must never ship without its CSR bundle (#237, ADR-0003/0008)",
            pkg_src.display()
        ),
        PkgAction::TolerateEmpty => println!(
            "cargo:warning=CSR bundle not found at {} (JAUNDER_CSR_BUNDLE_DIR unset); \
             staging an empty embedded site. Run `cargo xtask build-csr` to populate it.",
            pkg_src.display()
        ),
    }

    // Public assets land at the site root (`public/favicon.ico` → site/favicon.ico).
    if public_src.is_dir() {
        copy_tree(&public_src, &site_dir);
    }
}

/// Recursively copy `src`'s contents into `dst`, skipping any file whose name
/// ends in `.d.ts` (dev-only wasm-bindgen type defs — dead weight in the
/// binary). Subdirectories (e.g. wasm-bindgen `snippets/`) are preserved.
fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap_or_else(|e| panic!("creating {}: {e}", dst.display()));
    let entries = fs::read_dir(src).unwrap_or_else(|e| panic!("reading {}: {e}", src.display()));
    for entry in entries {
        let entry =
            entry.unwrap_or_else(|e| panic!("reading dir entry under {}: {e}", src.display()));
        let path = entry.path();
        let file_type = entry
            .file_type()
            .unwrap_or_else(|e| panic!("stat {}: {e}", path.display()));
        let name = entry.file_name();
        let dst_path = dst.join(&name);
        if file_type.is_dir() {
            copy_tree(&path, &dst_path);
        } else if name.to_string_lossy().ends_with(".d.ts") {
            // Skip TypeScript type-definition files.
        } else {
            fs::copy(&path, &dst_path).unwrap_or_else(|e| {
                panic!("copying {} -> {}: {e}", path.display(), dst_path.display())
            });
        }
    }
}
