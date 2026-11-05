//! Verify and stage the Vite CSR site tree into `$OUT_DIR/site/` for `rust-embed`.
//!
//! Vite's ordinary `.vite/manifest.json` is a build-only inventory. The build
//! script resolves the HTML entry and wasm-bindgen WASM output from its source
//! keys, emits only the host constants needed by the server, and excludes the
//! manifest and private system-theme inventory from the embedded site.
//!
//! `JAUNDER_CSR_BUNDLE_DIR` names a Vite output root. When it is unset, the host
//! default is `<workspace>/target/site`; absent local output intentionally
//! produces an empty, non-production embed so dependency-only and bare local
//! compilation remain possible. Any declared root, or any present local root,
//! is verified and fails closed before the server compiles.

#[path = "src/build_staging.rs"]
mod build_staging;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use build_staging::{prepare_staging_with, stage_public_tree, stage_vite_output};
use host::vite_manifest::{CsrViteOutputs, csr_outputs};

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
    // crap:allow: Cargo executes this build-script entrypoint outside host coverage targets;
    // its exact Cargo environment and staging side effects require build-script runtime.
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
    let system_artifacts_dir = out_dir.join("system-artifacts");
    let generated = out_dir.join("csr_vite_data.rs");

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
            let assets = load_vite_bundle(&bundle_root);
            let system_inventory = host::system_theme::load_system_artifact_inventory(
                &host::system_theme::DirectorySystemArtifactSource::new(
                    &bundle_root.join("system-artifacts"),
                ),
            )
            .unwrap_or_else(|error| panic!("invalid system artifacts: {error}"));
            verify_vite_shell(&bundle_root, &assets, &system_inventory);
            write_generated_data(&generated, Some(&assets));
            prepare_staging_with(
                &site_dir,
                |path| fs::remove_dir_all(path),
                |path| fs::create_dir_all(path),
                || {
                    stage_vite_output(&bundle_root, &site_dir)
                        .unwrap_or_else(|error| panic!("{error}"));
                    prepare_staging_with(
                        &system_artifacts_dir,
                        |path| fs::remove_dir_all(path),
                        |path| fs::create_dir_all(path),
                        || {
                            stage_verified_system_artifacts(
                                &system_inventory,
                                &system_artifacts_dir,
                            )
                            .unwrap_or_else(|error| panic!("{error}"));
                        },
                    )
                    .unwrap_or_else(|error| panic!("{error}"));
                },
            )
            .unwrap_or_else(|error| panic!("{error}"));
        }
        BundleAction::FailClosed => panic!(
            "JAUNDER_CSR_BUNDLE_DIR is set to {} but no Vite site root exists; declared CSR sites must verify before compilation",
            bundle_root.display()
        ),
        BundleAction::TolerateEmpty => {
            write_generated_data(&generated, None);
            prepare_staging_with(
                &site_dir,
                |path| fs::remove_dir_all(path),
                |path| fs::create_dir_all(path),
                || {
                    stage_public_tree(&public_src, &site_dir)
                        .unwrap_or_else(|error| panic!("{error}"));
                    prepare_staging_with(
                        &system_artifacts_dir,
                        |path| fs::remove_dir_all(path),
                        |path| fs::create_dir_all(path),
                        || {},
                    )
                    .unwrap_or_else(|error| panic!("{error}"));
                },
            )
            .unwrap_or_else(|error| panic!("{error}"));
            println!(
                "cargo:warning=CSR Vite site not found at {} (JAUNDER_CSR_BUNDLE_DIR unset); staging non-production empty site",
                bundle_root.display()
            );
        }
    }
}

fn load_vite_bundle(root: &Path) -> CsrViteOutputs {
    let outputs = csr_outputs(root)
        .unwrap_or_else(|error| panic!("invalid Vite CSR output {}: {error}", root.display()));
    assert!(
        root.join("index.html").is_file(),
        "invalid Vite CSR output {}: missing rendered index.html",
        root.display()
    );
    outputs
}

fn stage_verified_system_artifacts(
    inventory: &host::system_theme::SystemArtifactInventory,
    destination: &Path,
) -> Result<(), String> {
    host::system_theme::stage_system_artifact_inventory(inventory, destination)
        .map_err(|error| format!("staging system artifacts: {error}"))?;
    let staged = host::system_theme::load_system_artifact_inventory(
        &host::system_theme::DirectorySystemArtifactSource::new(destination),
    )
    .map_err(|error| format!("invalid staged system artifacts: {error}"))?;
    if staged.application().content().bytes() != inventory.application().content().bytes()
        || staged
            .themes()
            .zip(inventory.themes())
            .any(|(staged, source)| staged.package_bytes() != source.package_bytes())
    {
        return Err("staged system artifacts disagree with verified source".into());
    }
    Ok(())
}

fn verify_vite_shell(
    root: &Path,
    outputs: &CsrViteOutputs,
    system_inventory: &host::system_theme::SystemArtifactInventory,
) {
    let shell = fs::read_to_string(root.join("index.html"))
        .unwrap_or_else(|error| panic!("reading Vite CSR index.html: {error}"));
    let entry_url = format!("/{}", outputs.entry_js);
    let wasm_url = format!("/{}", outputs.wasm);
    let stylesheet_url = system_inventory
        .application()
        .content_digest()
        .content_url()
        .to_string();
    assert_eq!(
        shell.matches(&entry_url).count(),
        1,
        "Vite index.html must select its HTML-entry JavaScript output exactly once"
    );
    assert_eq!(
        shell.matches(&wasm_url).count(),
        1,
        "Vite index.html must select the manifest-mapped WASM URL exactly once"
    );
    assert!(
        shell.contains(&format!("const __jaunderWasmUrl = \"{wasm_url}\";"))
            && shell.contains("window.__jaunderWasmFetch = fetch(__jaunderWasmUrl);"),
        "Vite index.html must retain the classic early WASM fetch"
    );
    assert!(
        shell.contains(&format!(
            "href=\"{stylesheet_url}\" data-jaunder-application-stylesheet"
        )),
        "Vite index.html must select the installed application stylesheet URL"
    );
    let wasm_preload = shell.split("<link").any(|link| {
        link.split('>')
            .next()
            .is_some_and(|tag| tag.contains("rel=\"preload\"") && tag.contains(&wasm_url))
    });
    assert!(!wasm_preload, "ADR-0121 forbids a WASM preload");
    assert!(
        !shell.contains("{{WASM_URL}}") && !shell.contains("{{APPLICATION_STYLESHEET_URL}}"),
        "Vite index.html contains an unresolved build URL marker"
    );
}

fn write_generated_data(path: &Path, assets: Option<&CsrViteOutputs>) {
    let source = if let Some(assets) = assets {
        let paths = assets
            .files
            .iter()
            .map(|path| format!("    {path:?},"))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "pub const ENTRY_JS_URL: Option<&str> = Some({entry:?});\npub const WASM_URL: Option<&str> = Some({wasm:?});\npub const VITE_ASSET_PATHS: &[&str] = &[\n{paths}\n];\n",
            entry = format!("/{}", assets.entry_js),
            wasm = format!("/{}", assets.wasm),
        )
    } else {
        "pub const ENTRY_JS_URL: Option<&str> = None;\npub const WASM_URL: Option<&str> = None;\npub const VITE_ASSET_PATHS: &[&str] = &[];\n".to_owned()
    };
    fs::write(path, source).unwrap_or_else(|error| panic!("writing {}: {error}", path.display()));
}
