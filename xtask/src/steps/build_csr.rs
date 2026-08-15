//! `xtask build-csr` — build the CSR wasm bundle on the host without cargo-leptos.
//! Compiles the `csr` crate to wasm, then runs the checked-out `devtool`
//! postprocessor. The PATH `devtool` is the devShell's pinned release binary and
//! can predate this checkout; building the local crate is required for e2e to
//! exercise a changed bundle postprocessor. Debug is faster for the dev loop;
//! `--release` matches CI's optimized wasm.
//!
//! Output lands in `target/site/pkg/` (`jaunder.{js,wasm}` + wasm-bindgen's
//! `.d.ts`/`snippets`), where `jaunder serve` serves it from `site_root`.

use std::path::Path;

use xshell::{Shell, cmd};

use crate::git;
use crate::result::{CommandResult, StepResult};

/// Build `csr` to wasm and post-process it into the served bundle. `release`
/// selects the optimized profile (CI parity); the default debug build is faster
/// for the dev loop.
pub fn run(sh: &Shell, result: &mut CommandResult, release: bool) {
    let Ok(root) = git::toplevel(Path::new(".")) else {
        result.push(StepResult::fail("build-csr").detail("cannot locate repo root".to_owned()));
        return;
    };
    sh.change_dir(&root);

    let profile = if release { "release" } else { "debug" };
    let mut cargo_args = vec!["build", "-p", "csr", "--target", "wasm32-unknown-unknown"];
    if release {
        cargo_args.push("--release");
    }
    if cmd!(sh, "cargo").args(cargo_args).run().is_err() {
        result.push(
            StepResult::fail("build-csr-wasm")
                .detail("cargo build -p csr (wasm32-unknown-unknown) failed".to_owned()),
        );
        return;
    }
    result.push(StepResult::ok("build-csr-wasm"));

    let wasm = format!("{root}/target/wasm32-unknown-unknown/{profile}/csr.wasm");
    let out = format!("{root}/target/site/pkg");
    if cmd!(
        sh,
        "cargo run --manifest-path tools/devtool/Cargo.toml -- csr-bundle --wasm {wasm} --out {out}"
    )
    .run()
    .is_err()
    {
        result.push(
            StepResult::fail("build-csr-bundle")
                .detail("checked-out devtool csr-bundle failed".to_owned()),
        );
        return;
    }
    result.push(StepResult::ok("build-csr-bundle"));
}
