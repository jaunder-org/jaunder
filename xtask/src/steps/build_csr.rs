//! `xtask build-csr` — build the CSR wasm bundle on the host without cargo-leptos.
//! Compiles the `csr` crate to wasm, then runs the checked-out `devtool`
//! postprocessor. The PATH `devtool` is the devShell's pinned release binary and
//! can predate this checkout; building the local crate is required for e2e to
//! exercise a changed bundle postprocessor. Debug is faster for the dev loop;
//! `--release` matches CI's optimized wasm.
//!
//! Output lands in `target/site/`: `manifest.json` is build-only, while
//! `index.html` and `pkg/**` form the served bundle root.

use std::{fs, path::Path};

use xshell::{Shell, cmd};

use crate::git;
use crate::result::{CommandResult, StepResult};

/// Build `csr` to wasm and post-process it into the served bundle. `release`
/// selects the optimized profile (CI parity); the default debug build is faster
/// for the dev loop.
pub fn run(sh: &Shell, result: &mut CommandResult, release: bool) {
    let root_start = std::time::Instant::now();
    let Ok(root) = git::toplevel(Path::new(".")) else {
        result.push(
            StepResult::fail("build-csr")
                .detail("cannot locate repo root".to_owned())
                .with_duration(root_start.elapsed()),
        );
        return;
    };
    sh.change_dir(&root);

    let profile = if release { "release" } else { "debug" };
    let mut cargo_args = vec!["build", "-p", "csr", "--target", "wasm32-unknown-unknown"];
    if release {
        cargo_args.push("--release");
    }
    let wasm_start = std::time::Instant::now();
    if cmd!(sh, "cargo").args(cargo_args).run().is_err() {
        result.push(
            StepResult::fail("build-csr-wasm")
                .detail("cargo build -p csr (wasm32-unknown-unknown) failed".to_owned())
                .with_duration(wasm_start.elapsed()),
        );
        return;
    }
    result.push(StepResult::ok("build-csr-wasm").with_duration(wasm_start.elapsed()));

    let wasm = format!("{root}/target/wasm32-unknown-unknown/{profile}/csr.wasm");
    let bundle_root = Path::new(&root).join("target/site");
    match bundle_root.try_exists() {
        Ok(true) => {
            let cleanup_start = std::time::Instant::now();
            if let Err(error) = fs::remove_dir_all(&bundle_root) {
                result.push(
                    StepResult::fail("build-csr-clean")
                        .detail(format!(
                            "removing previous CSR bundle root {}: {error}",
                            bundle_root.display()
                        ))
                        .with_duration(cleanup_start.elapsed()),
                );
                return;
            }
        }
        Ok(false) => {}
        Err(error) => {
            result.push(
                StepResult::fail("build-csr-clean")
                    .detail(format!(
                        "checking previous CSR bundle root {}: {error}",
                        bundle_root.display()
                    ))
                    .with_duration(root_start.elapsed()),
            );
            return;
        }
    }
    let out = bundle_root.to_string_lossy().into_owned();
    let bundle_start = std::time::Instant::now();
    if cmd!(
        sh,
        "cargo run --manifest-path tools/devtool/Cargo.toml -- csr-bundle --wasm {wasm} --out {out}"
    )
    .run()
    .is_err()
    {
        result.push(
            StepResult::fail("build-csr-bundle")
                .detail("checked-out devtool csr-bundle failed".to_owned())
                .with_duration(bundle_start.elapsed()),
        );
        return;
    }
    result.push(StepResult::ok("build-csr-bundle").with_duration(bundle_start.elapsed()));
}
