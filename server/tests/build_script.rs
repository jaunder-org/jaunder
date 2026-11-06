use std::fs;
use tempfile::TempDir;

use host::vite_manifest::csr_outputs;

#[path = "../src/build_staging.rs"]
mod build_impl;

fn vite_output() -> TempDir {
    let root = TempDir::new().expect("Vite output root");
    fs::create_dir_all(root.path().join(".vite")).unwrap();
    fs::create_dir_all(root.path().join("assets")).unwrap();
    for (path, bytes) in [
        (
            "index.html",
            b"<script src=\"/assets/app.js\"></script>".as_slice(),
        ),
        ("assets/app.js", b"entry JS".as_slice()),
        ("assets/client-hash.wasm", b"optimized wasm".as_slice()),
        ("assets/client-hash.wasm.br", b"brotli wasm".as_slice()),
        ("assets/client-hash.wasm.gz", b"gzip wasm".as_slice()),
    ] {
        fs::write(root.path().join(path), bytes).unwrap();
    }
    fs::write(
        root.path().join(".vite/manifest.json"),
        r#"{
          "index.html": {"file":"assets/app.js","src":"index.html","isEntry":true,"assets":["assets/client-hash.wasm"]},
          "csr_bg.wasm": {"file":"assets/client-hash.wasm","src":"csr_bg.wasm"}
        }"#,
    )
    .unwrap();
    root
}

#[test]
fn standard_vite_manifest_resolves_entry_and_wasm_then_stages_runtime_files() {
    let vite = vite_output();
    let site = TempDir::new().expect("staging root");
    let outputs = csr_outputs(vite.path()).expect("read Vite build inventory");
    assert_eq!(outputs.entry_js, "assets/app.js");
    assert_eq!(outputs.wasm, "assets/client-hash.wasm");

    build_impl::stage_vite_output(vite.path(), site.path()).expect("stage Vite output");
    assert!(site.path().join(&outputs.entry_js).is_file());
    assert!(site.path().join(&outputs.wasm).is_file());
    assert!(site.path().join("assets/client-hash.wasm.br").is_file());
    assert!(site.path().join("assets/client-hash.wasm.gz").is_file());
    assert!(!site.path().join(".vite/manifest.json").exists());
}
