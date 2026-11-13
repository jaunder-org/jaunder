//! Bind the executable to its own Nix output. Qualification refuses to admit a
//! harness unless the running tool is that harness's compiled output, so a host
//! `cargo` build deliberately carries no provenance.
fn main() {
    // Cargo-run propagates rustc-env metadata to children; the build input uses
    // a distinct name so a parent tool's identity cannot qualify a host rebuild.
    println!("cargo:rerun-if-env-changed=JAUNDER_QUALIFICATION_BUILD_NIX_OUTPUT");
    let output = std::env::var("JAUNDER_QUALIFICATION_BUILD_NIX_OUTPUT").unwrap_or_default();
    assert!(
        output.is_empty() || output.starts_with("/nix/store/"),
        "qualification output must be a Nix store path"
    );
    println!("cargo:rustc-env=JAUNDER_QUALIFICATION_NIX_OUTPUT={output}");
}
