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

#[cfg(unix)]
#[test]
fn csr_staging_adds_only_owner_write_to_independent_files() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    let vite = vite_output();
    let site = TempDir::new().expect("fresh staging root");
    let files = [
        ("index.html", 0o444),
        ("assets/app.js", 0o4451),
        ("assets/client-hash.wasm", 0o440),
        ("assets/client-hash.wasm.br", 0o404),
        ("assets/client-hash.wasm.gz", 0o444),
    ];
    for (name, mode) in files {
        fs::set_permissions(vite.path().join(name), fs::Permissions::from_mode(mode)).unwrap();
    }
    symlink("app.js", vite.path().join("assets/file-link.js")).unwrap();
    fs::set_permissions(
        vite.path().join("assets"),
        fs::Permissions::from_mode(0o555),
    )
    .unwrap();
    let root_mode = fs::metadata(site.path()).unwrap().permissions().mode();
    let control = TempDir::new().unwrap();
    fs::create_dir(control.path().join("assets")).unwrap();
    let directory_mode = fs::metadata(control.path().join("assets"))
        .unwrap()
        .permissions()
        .mode();
    let bytes = files.map(|(name, _)| fs::read(vite.path().join(name)).unwrap());

    build_impl::stage_vite_output(vite.path(), site.path()).expect("stage readonly CSR output");
    for ((name, mode), bytes) in files.into_iter().zip(bytes) {
        let src = fs::metadata(vite.path().join(name)).unwrap();
        let dst = fs::metadata(site.path().join(name)).unwrap();
        assert_eq!(src.permissions().mode() & 0o7777, mode, "source {name}");
        assert_eq!(
            dst.permissions().mode() & 0o7777,
            mode | 0o200,
            "destination {name}"
        );
        assert_eq!(fs::read(vite.path().join(name)).unwrap(), bytes);
        assert_eq!(fs::read(site.path().join(name)).unwrap(), bytes);
        assert_ne!((src.dev(), src.ino()), (dst.dev(), dst.ino()));
    }
    let link = site.path().join("assets/file-link.js");
    assert!(fs::symlink_metadata(&link).unwrap().file_type().is_file());
    assert_eq!(
        fs::read(&link).unwrap(),
        fs::read(vite.path().join("assets/app.js")).unwrap()
    );
    assert_eq!(
        fs::metadata(&link).unwrap().permissions().mode() & 0o7777,
        0o4651
    );
    assert_ne!(
        fs::metadata(&link).unwrap().ino(),
        fs::metadata(vite.path().join("assets/app.js"))
            .unwrap()
            .ino()
    );
    assert_eq!(
        fs::read_link(vite.path().join("assets/file-link.js")).unwrap(),
        std::path::Path::new("app.js")
    );
    assert_eq!(
        fs::metadata(site.path()).unwrap().permissions().mode(),
        root_mode
    );
    assert_eq!(
        fs::metadata(site.path().join("assets"))
            .unwrap()
            .permissions()
            .mode(),
        directory_mode
    );
    assert_eq!(
        fs::metadata(vite.path().join("assets"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o555
    );
    fs::write(site.path().join("assets/app.js"), b"updated").unwrap();
    fs::remove_dir_all(site.path()).unwrap();
    assert_eq!(
        fs::read(vite.path().join("assets/app.js")).unwrap(),
        b"entry JS"
    );
    assert_eq!(
        fs::metadata(vite.path().join("assets/app.js"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o4451
    );
    fs::set_permissions(
        vite.path().join("assets"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
}

#[cfg(unix)]
#[test]
fn csr_staging_keeps_directory_symlink_copy_failure() {
    use std::os::unix::fs::symlink;

    let vite = vite_output();
    let site = TempDir::new().unwrap();
    symlink("assets", vite.path().join("directory-link")).unwrap();
    let error = build_impl::stage_vite_output(vite.path(), site.path()).unwrap_err();
    assert!(error.to_string().starts_with(&format!(
        "copying {} to {}:",
        vite.path().join("directory-link").display(),
        site.path().join("directory-link").display()
    )));
    assert!(vite.path().join("assets/app.js").is_file());
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
