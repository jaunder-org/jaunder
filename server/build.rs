//! Verify and stage the runtime CSR site tree into `$OUT_DIR/site/` for `rust-embed`.
//!
//! A supplied bundle root contains a build-only `manifest.json`, rendered `index.html`,
//! and verified `pkg/**` representations. The manifest is the sole naming interface:
//! this script emits the host-side role URLs and exact immutable-cache inventory, then
//! stages only the rendered shell and declared runtime representations. `manifest.json`
//! never enters the embedded site (ADR-0003, #869).
//!
//! `JAUNDER_CSR_BUNDLE_DIR` names a bundle root. When it is unset, the host default is
//! `<workspace>/target/site`; absent local output intentionally produces an empty,
//! non-production embed so dependency-only and bare local compilation remain possible.
//! Any declared root, or any present local root, is verified and fails closed before the
//! server compiles.

#[path = "src/build_staging.rs"]
mod build_staging;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use build_staging::{prepare_staging_with, stage_bundle};
use csr_bundle::{Manifest, Role};

#[derive(Debug, PartialEq, Eq)]
enum BundleAction {
    Stage,
    FailClosed,
    TolerateEmpty,
}

fn decide_bundle_action(bundle_exists: bool, bundle_declared: bool) -> BundleAction {
    if bundle_exists {
        BundleAction::Stage
    } else if bundle_declared {
        BundleAction::FailClosed
    } else {
        BundleAction::TolerateEmpty
    }
}

fn main() {
    // crap:allow: Cargo executes build scripts outside normal test targets. The named
    // decision and staging helpers are unit-tested in their source modules.
    let out_dir =
        PathBuf::from(env::var_os("OUT_DIR").unwrap_or_else(|| panic!("cargo sets OUT_DIR")));
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR")
            .unwrap_or_else(|| panic!("cargo sets CARGO_MANIFEST_DIR")),
    );
    let workspace = manifest_dir
        .parent()
        .unwrap_or_else(|| panic!("server has workspace parent"));
    let site_dir = out_dir.join("site");
    let generated = out_dir.join("csr_bundle_data.rs");

    println!("cargo:rerun-if-env-changed=JAUNDER_CSR_BUNDLE_DIR");
    println!("cargo:rerun-if-env-changed=JAUNDER_PUBLIC_DIR");

    let bundle_env = env::var_os("JAUNDER_CSR_BUNDLE_DIR");
    let bundle_root = bundle_env
        .as_ref()
        .map_or_else(|| workspace.join("target/site"), PathBuf::from);
    println!("cargo:rerun-if-changed={}", bundle_root.display());

    let public_src =
        env::var_os("JAUNDER_PUBLIC_DIR").map_or_else(|| workspace.join("public"), PathBuf::from);
    println!("cargo:rerun-if-changed={}", public_src.display());

    match decide_bundle_action(bundle_root.is_dir(), bundle_env.is_some()) {
        BundleAction::Stage => {
            let manifest = load_bundle(&bundle_root);
            write_generated_data(&generated, Some(&manifest));
            prepare_staging_with(
                &site_dir,
                |path| fs::remove_dir_all(path),
                |path| fs::create_dir_all(path),
                || {
                    stage_bundle(&bundle_root, &site_dir, &public_src, &manifest)
                        .unwrap_or_else(|error| panic!("{error}"));
                },
            )
            .unwrap_or_else(|error| panic!("{error}"));
        }
        BundleAction::FailClosed => panic!(
            "JAUNDER_CSR_BUNDLE_DIR is set to {} but no bundle root exists; declared CSR bundles must verify before compilation",
            bundle_root.display()
        ),
        BundleAction::TolerateEmpty => {
            write_generated_data(&generated, None);
            prepare_staging_with(
                &site_dir,
                |path| fs::remove_dir_all(path),
                |path| fs::create_dir_all(path),
                || {
                    build_staging::stage_public_tree(&public_src, &site_dir)
                        .unwrap_or_else(|error| panic!("{error}"));
                },
            )
            .unwrap_or_else(|error| panic!("{error}"));
            println!(
                "cargo:warning=CSR bundle not found at {} (JAUNDER_CSR_BUNDLE_DIR unset); staging non-production empty site",
                bundle_root.display()
            );
        }
    }
}

fn load_bundle(root: &Path) -> Manifest {
    let manifest_path = root.join("manifest.json");
    let bytes = fs::read(&manifest_path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", manifest_path.display()));
    let manifest = Manifest::from_json(&bytes)
        .unwrap_or_else(|error| panic!("invalid {}: {error}", manifest_path.display()));
    manifest
        .verify_bundle(root)
        .unwrap_or_else(|error| panic!("invalid bundle {}: {error}", root.display()));
    assert!(
        root.join("index.html").is_file(),
        "invalid bundle {}: missing rendered index.html",
        root.display()
    );
    manifest
}

fn write_generated_data(path: &Path, manifest: Option<&Manifest>) {
    let source = if let Some(manifest) = manifest {
        let glue = manifest
            .role(Role::Glue)
            .unwrap_or_else(|error| panic!("verified glue role missing: {error}"));
        let wasm = manifest
            .role(Role::Wasm)
            .unwrap_or_else(|error| panic!("verified wasm role missing: {error}"));
        let mut paths = manifest
            .assets
            .iter()
            .map(|asset| asset.path.as_str())
            .collect::<Vec<_>>();
        paths.sort_unstable();
        let paths = paths
            .iter()
            .map(|path| format!("    {path:?},"))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "pub const GLUE_URL: Option<&str> = Some({glue:?});\npub const WASM_URL: Option<&str> = Some({wasm:?});\npub const MANIFEST_PATHS: &[&str] = &[\n{paths}\n];\n",
            glue = format!("/{}", glue.path),
            wasm = format!("/{}", wasm.path),
        )
    } else {
        "pub const GLUE_URL: Option<&str> = None;\npub const WASM_URL: Option<&str> = None;\npub const MANIFEST_PATHS: &[&str] = &[];\n".to_owned()
    };
    fs::write(path, source).unwrap_or_else(|error| panic!("writing {}: {error}", path.display()));
}
