//! Coverage producers and evidence gates that run inside Nix derivations,
//! where host-only tooling is unavailable: the instrumented suite's emit and
//! status validation, the browser WebAssembly coverage lifecycle and the
//! diagnostic CSR build's artifact operations.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod diagnostic_build;
mod emit;
mod validate_status;
mod vite_manifest;
mod wasm;

#[derive(Parser)]
#[command(
    name = "coverage-sandbox",
    about = "Jaunder in-derivation coverage tooling",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the instrumented suite and emit reports + status + diagnostics.
    Emit {
        /// Directory to write emitted artifacts into (defaults to CWD).
        #[arg(long, default_value = ".")]
        out: String,
    },
    /// Validate completed coverage producer evidence.
    ValidateStatus {
        /// Path to the coverage producer status JSON.
        #[arg(long)]
        status: PathBuf,
    },
    /// WebAssembly coverage evidence lifecycle for the Nix browser producers.
    #[command(subcommand)]
    Wasm(WasmCmd),
    /// Structured preparation and artifact recording for diagnostic CSR builds.
    #[command(subcommand)]
    DiagnosticBuild(DiagnosticBuildCmd),
}

#[derive(Subcommand)]
enum WasmCmd {
    /// Create the sentinel status and retain the content-addressed served module.
    Initialize,
    /// Merge the browser profile and map it to original Rust source.
    Map {
        /// Source root used as the llvm-cov path-equivalence destination.
        #[arg(long)]
        site_src: PathBuf,
    },
    /// Retain capture status or finalize an early Playwright failure.
    Finalize,
}

#[derive(Subcommand)]
enum DiagnosticBuildCmd {
    /// Rewrite copied manifests and record the source identity.
    PrepareSource {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        minicov: PathBuf,
        #[arg(long)]
        runtime: PathBuf,
        #[arg(long)]
        source_identity: PathBuf,
        #[arg(long)]
        nix_source: PathBuf,
    },
    /// Require the Rust and Clang compiler reports to identify LLVM 22.
    ValidateLlvm {
        #[arg(long)]
        rustc_version: PathBuf,
        #[arg(long)]
        clang_version: PathBuf,
    },
    /// Locate Cargo's unique root LLVM IR and rlink artifacts.
    DiscoverRoot {
        #[arg(long)]
        target_release: PathBuf,
        #[arg(long)]
        root_ir: PathBuf,
        #[arg(long)]
        root_rlink: PathBuf,
        #[arg(long)]
        manifest: PathBuf,
    },
    /// Retain Cargo's uniquely linked CSR wasm and update the IR manifest.
    RetainLinkedWasm {
        #[arg(long)]
        target_release: PathBuf,
        #[arg(long)]
        manifest: PathBuf,
        #[arg(long)]
        retained_wasm: PathBuf,
    },
    /// Record coverage metadata boundaries and exact diagnostic toolchain identity.
    WriteCoverageArtifacts {
        #[arg(long)]
        metadata: PathBuf,
        #[arg(long)]
        toolchain_identity: PathBuf,
        #[arg(long)]
        minicov_version: String,
        #[arg(long)]
        input_wasm: PathBuf,
        #[arg(long)]
        wasm_bindgen_wasm: PathBuf,
        #[arg(long)]
        wasm_opt_wasm: PathBuf,
    },
    /// Assert that wasm-bindgen retained coverage metadata after bundling.
    AssertCoverage {
        #[arg(long)]
        metadata: PathBuf,
    },
    /// Write the durable status after the failure-retaining diagnostic pipeline.
    WriteInstrumentedStatus {
        #[arg(long)]
        status: PathBuf,
        #[arg(long)]
        pipeline_exit: i32,
    },
    /// Write the successful baseline bundle status.
    WriteBaselineStatus {
        #[arg(long)]
        status: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Emit { out } => emit::run(&out),
        Command::ValidateStatus { status } => validate_status::run(&status),
        Command::Wasm(WasmCmd::Initialize) => wasm::initialize(),
        Command::Wasm(WasmCmd::Map { site_src }) => wasm::map(&site_src),
        Command::Wasm(WasmCmd::Finalize) => wasm::finalize(),
        Command::DiagnosticBuild(DiagnosticBuildCmd::PrepareSource {
            source,
            minicov,
            runtime,
            source_identity,
            nix_source,
        }) => diagnostic_build::prepare_source(
            &source,
            &minicov,
            &runtime,
            &source_identity,
            &nix_source,
        ),
        Command::DiagnosticBuild(DiagnosticBuildCmd::ValidateLlvm {
            rustc_version,
            clang_version,
        }) => diagnostic_build::validate_llvm(&rustc_version, &clang_version),
        Command::DiagnosticBuild(DiagnosticBuildCmd::DiscoverRoot {
            target_release,
            root_ir,
            root_rlink,
            manifest,
        }) => diagnostic_build::discover_root(&target_release, &root_ir, &root_rlink, &manifest),
        Command::DiagnosticBuild(DiagnosticBuildCmd::RetainLinkedWasm {
            target_release,
            manifest,
            retained_wasm,
        }) => diagnostic_build::retain_linked_wasm(&target_release, &manifest, &retained_wasm),
        Command::DiagnosticBuild(DiagnosticBuildCmd::WriteCoverageArtifacts {
            metadata,
            toolchain_identity,
            minicov_version,
            input_wasm,
            wasm_bindgen_wasm,
            wasm_opt_wasm,
        }) => diagnostic_build::write_coverage_artifacts(
            &metadata,
            &toolchain_identity,
            &minicov_version,
            &input_wasm,
            &wasm_bindgen_wasm,
            &wasm_opt_wasm,
        ),
        Command::DiagnosticBuild(DiagnosticBuildCmd::AssertCoverage { metadata }) => {
            diagnostic_build::assert_coverage(&metadata)
        }
        Command::DiagnosticBuild(DiagnosticBuildCmd::WriteInstrumentedStatus {
            status,
            pipeline_exit,
        }) => diagnostic_build::write_instrumented_status(&status, pipeline_exit),
        Command::DiagnosticBuild(DiagnosticBuildCmd::WriteBaselineStatus { status }) => {
            diagnostic_build::write_baseline_status(&status)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::Parser;

    use super::*;

    #[test]
    fn diagnostic_coverage_metadata_cli_requires_all_identity_inputs() {
        assert!(
            Cli::try_parse_from([
                "coverage-sandbox",
                "diagnostic-build",
                "write-coverage-artifacts",
                "--metadata",
                "metadata.json",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "coverage-sandbox",
                "diagnostic-build",
                "write-coverage-artifacts",
                "--metadata",
                "metadata.json",
                "--toolchain-identity",
                "toolchain.json",
                "--minicov-version",
                "0.3.8",
                "--input-wasm",
                "input.wasm",
                "--wasm-bindgen-wasm",
                "bindgen.wasm",
                "--wasm-opt-wasm",
                "optimized.wasm",
            ])
            .is_ok()
        );
    }

    #[test]
    fn wasm_coverage_mapping_requires_an_explicit_source_root() {
        assert!(Cli::try_parse_from(["coverage-sandbox", "wasm", "map"]).is_err());
        assert!(matches!(
            Cli::try_parse_from([
                "coverage-sandbox",
                "wasm",
                "map",
                "--site-src",
                "/source/site",
            ])
            .expect("mapping command parses")
            .command,
            Command::Wasm(WasmCmd::Map { site_src }) if site_src == Path::new("/source/site")
        ));
    }
    #[test]
    fn coverage_emit_parses_the_producer_output_directory() {
        let emit = Cli::try_parse_from(["coverage-sandbox", "emit", "--out", "coverage"])
            .expect("coverage emit");
        assert!(matches!(
            emit.command,
            Command::Emit { out } if out == "coverage"
        ));
        assert!(Cli::try_parse_from(["coverage-sandbox", "experiment-worker"]).is_err());
    }
}
