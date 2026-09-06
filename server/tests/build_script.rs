#[path = "../src/build_staging.rs"]
mod build_impl;

use std::cell::Cell;
use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::io;
use std::path::Path;

use csr_bundle::{Asset, Manifest, Representation, Role};
use tempfile::TempDir;

fn bundle_root() -> (TempDir, Manifest) {
    let root = TempDir::new().expect("bundle root");
    let glue = asset(Role::Glue, "js", b"glue bytes");
    let wasm = asset(Role::Wasm, "wasm", b"wasm bytes");
    let manifest = Manifest {
        version: csr_bundle::VERSION,
        assets: vec![glue, wasm],
    };
    for asset in &manifest.assets {
        for representation in asset.representations.values() {
            let bytes = representation_bytes(&representation.path);
            let path = root.path().join(&representation.path);
            fs::create_dir_all(path.parent().expect("representation parent"))
                .expect("create representation parent");
            fs::write(path, bytes).expect("write representation");
        }
    }
    let glue = manifest.role(Role::Glue).expect("glue role");
    let wasm = manifest.role(Role::Wasm).expect("WASM role");
    fs::write(
        root.path().join("index.html"),
        shell(&format!("/{}", glue.path), &format!("/{}", wasm.path)),
    )
    .expect("write shell");
    (root, manifest)
}

fn asset(role: Role, extension: &str, identity: &[u8]) -> Asset {
    let identity_digest = csr_bundle::digest(identity);
    let path = format!("pkg/{identity_digest}.{extension}");
    let gzip_path = format!("{path}.gz");
    let brotli_path = format!("{path}.br");
    let representations: BTreeMap<_, _> = [
        (
            "identity".into(),
            Representation {
                path: path.clone(),
                sha256: csr_bundle::digest(&representation_bytes(&path)),
            },
        ),
        (
            "gzip".into(),
            Representation {
                path: gzip_path.clone(),
                sha256: csr_bundle::digest(&representation_bytes(&gzip_path)),
            },
        ),
        (
            "br".into(),
            Representation {
                path: brotli_path.clone(),
                sha256: csr_bundle::digest(&representation_bytes(&brotli_path)),
            },
        ),
    ]
    .into_iter()
    .collect();
    Asset {
        role: Some(role),
        path,
        sha256: identity_digest,
        representations,
    }
}

fn representation_bytes(path: &str) -> Vec<u8> {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("gz") => b"gzip bytes".to_vec(),
        Some("br") => b"brotli bytes".to_vec(),
        Some("js") => b"glue bytes".to_vec(),
        _ => b"wasm bytes".to_vec(),
    }
}

fn shell(glue: &str, wasm: &str) -> String {
    format!(
        r#"<script>const __jaunderWasmUrl = "{wasm}"; window.__jaunderWasmFetch = fetch(__jaunderWasmUrl);</script>
<link rel="stylesheet" href="/style/jaunder.css" />
<script type="module">import {{initMeasured}} from "{glue}"; performance.mark("jaunder.module.before_init"); initMeasured(window.__jaunderWasmFetch ?? __jaunderWasmUrl);</script>"#
    )
}

fn stage(root: &TempDir, manifest: &Manifest, public: &TempDir) -> Result<(), String> {
    let site = TempDir::new().expect("staging root");
    build_impl::stage_bundle(root.path(), site.path(), public.path(), manifest)
        .map_err(|error| error.to_string())
}

#[test]
fn public_tree_stages_without_a_declared_bundle() {
    let public = TempDir::new().expect("public root");
    let site = TempDir::new().expect("staging root");
    let stylesheet = public.path().join("style/jaunder.css");
    fs::create_dir_all(stylesheet.parent().expect("stylesheet parent"))
        .expect("create stylesheet parent");
    fs::write(&stylesheet, "body {}").expect("write stylesheet");

    build_impl::stage_public_tree(public.path(), site.path()).expect("stage public tree");

    assert_eq!(
        fs::read_to_string(site.path().join("style/jaunder.css")).expect("read staged stylesheet"),
        "body {}"
    );
}
#[test]
fn build_script_staging_cleanup_failure_aborts_before_create_or_copy() {
    let created = Cell::new(false);
    let copied = Cell::new(false);
    let site = Path::new("/injected/out/site");

    let error = build_impl::prepare_staging_with(
        site,
        |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected remove failure",
            ))
        },
        |_| {
            created.set(true);
            Ok(())
        },
        || copied.set(true),
    )
    .unwrap_err();

    assert_eq!(error.operation, "removing");
    assert_eq!(error.path, site);
    assert_eq!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<io::Error>())
            .map(io::Error::kind),
        Some(io::ErrorKind::PermissionDenied)
    );
    assert!(!created.get(), "staging directory must not be recreated");
    assert!(!copied.get(), "assets must not be copied");
}

#[test]
fn public_assets_cannot_overwrite_the_shell_or_manifest_inventory() {
    let (bundle, manifest) = bundle_root();
    let public = TempDir::new().expect("public root");
    fs::write(public.path().join("index.html"), "public shell").expect("write public shell");
    let error = stage(&bundle, &manifest, &public).expect_err("shell collision must fail closed");
    assert!(
        error.contains("index.html overwrites a reserved CSR bundle path"),
        "{error}"
    );

    fs::remove_file(public.path().join("index.html")).expect("remove public shell");
    fs::write(public.path().join("manifest.json"), "public manifest")
        .expect("write public manifest");
    let error = stage(&bundle, &manifest, &public)
        .expect_err("build-only manifest collision must fail closed");
    assert!(
        error.contains("manifest.json overwrites a reserved CSR bundle path"),
        "{error}"
    );

    fs::remove_file(public.path().join("manifest.json")).expect("remove public manifest");
    let glue = manifest.role(Role::Glue).expect("glue role");
    let collision = public.path().join(&glue.path);
    fs::create_dir_all(collision.parent().expect("collision parent"))
        .expect("create collision parent");
    fs::write(collision, "public glue").expect("write public glue");
    let error = stage(&bundle, &manifest, &public).expect_err("bundle collision must fail closed");
    assert!(
        error.contains("overwrites a reserved CSR bundle path"),
        "{error}"
    );
}

#[test]
fn stale_or_mismatched_staged_shell_fails_closed() {
    let (bundle, manifest) = bundle_root();
    let public = TempDir::new().expect("public root");
    let wasm = manifest.role(Role::Wasm).expect("WASM role");
    fs::write(
        bundle.path().join("index.html"),
        shell("/pkg/stale-glue.js", &format!("/{}", wasm.path)),
    )
    .expect("replace shell");
    let error = stage(&bundle, &manifest, &public).expect_err("stale shell must fail closed");
    assert!(error.contains("exactly one glue URL"), "{error}");

    let glue = manifest.role(Role::Glue).expect("glue role");
    fs::write(
        bundle.path().join("index.html"),
        format!(
            "{}\n/{}",
            shell(&format!("/{}", glue.path), &format!("/{}", wasm.path)),
            glue.path
        ),
    )
    .expect("replace shell");
    let error = stage(&bundle, &manifest, &public).expect_err("mismatched shell must fail closed");
    assert!(error.contains("exactly one glue URL"), "{error}");
}

#[test]
fn final_staged_inventory_and_digest_must_match_the_verified_manifest() {
    let (bundle, manifest) = bundle_root();
    let public = TempDir::new().expect("public root");
    fs::create_dir_all(public.path().join("pkg")).expect("create public package directory");
    fs::write(public.path().join("pkg/extra.js"), "extra")
        .expect("write public inventory disagreement");
    let error =
        stage(&bundle, &manifest, &public).expect_err("extra staged asset must fail closed");
    assert!(
        error.contains("unexpected bundle file: pkg/extra.js"),
        "{error}"
    );

    fs::remove_dir_all(public.path().join("pkg")).expect("remove public extra asset");
    manifest
        .verify_bundle(bundle.path())
        .expect("source bundle verifies before the staged copy");

    let glue = manifest.role(Role::Glue).expect("glue role");
    fs::write(bundle.path().join(&glue.path), "changed after verification")
        .expect("replace verified source asset");
    let error =
        stage(&bundle, &manifest, &public).expect_err("staged digest mismatch must fail closed");
    assert!(error.contains("digest mismatch"), "{error}");
}
