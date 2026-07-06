//! Subprocess-coverage probe: spawn the built `jaunder` binary and observe it.
//!
//! `CARGO_BIN_EXE_jaunder` is injected by cargo for integration tests, so this
//! needs no extra dependency and no database — a plain synchronous `#[test]`, no
//! backend annotation.
//!
//! Behavioural note: under the test/coverage build, feature unification (resolver
//! 2, this crate's dev-dependencies pull `common` with `test-utils` → `cheap-kdf`)
//! compiles `common::CHEAP_KDF_ENABLED = true` into the `jaunder` binary. `main`'s
//! fail-closed guard therefore fires *before* clap handles any argument, so the
//! process prints the cheap-kdf FATAL message to stderr and exits non-zero — it
//! never reaches `--help`/usage. We assert that real behaviour rather than a usage
//! banner that the instrumented binary cannot produce. Either way the subprocess
//! runs `main`, executing the (now un-`cov:ignore`'d) `if common::CHEAP_KDF_ENABLED`
//! line, which the in-process unit tests (they call `run`, not `main`) never reach.

// guard:no-backend — spawns the built binary as a child process; touches no database.
#[test]
fn jaunder_binary_fail_closes_under_cheap_kdf_build() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_jaunder"))
        .arg("--help")
        .output()
        .expect("failed to spawn the jaunder binary");

    // The cheap-kdf fail-closed guard runs before clap, so the process exits
    // non-zero regardless of the argument.
    assert!(
        !output.status.success(),
        "expected the cheap-kdf test build to fail-close (non-zero exit), got: {:?}",
        output.status
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cheap-kdf"),
        "expected the fail-closed guard message on stderr, got stderr: {stderr:?}"
    );
}
