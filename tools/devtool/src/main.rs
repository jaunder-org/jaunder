//! Internal in-sandbox dev tool. Runs inside the Nix coverage/e2e build
//! sandboxes where `xtask` (host-only) is unavailable. Subcommand tree is
//! deliberately extensible: `coverage emit`, `wasm-coverage` and diagnostic build.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

mod check;
mod coverage;
mod diagnostic_build;
mod digest;
mod doctests;
#[cfg(feature = "performance")]
mod performance;
#[cfg(feature = "system-artifacts")]
mod system_artifacts;
mod vite_manifest;
mod wasm_coverage;

#[derive(Parser)]
#[command(name = "devtool", about = "Jaunder in-sandbox dev tooling", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Coverage pipeline subcommands.
    #[command(subcommand)]
    Coverage(CoverageCmd),
    /// WebAssembly coverage evidence lifecycle for the Nix browser producers.
    #[command(subcommand)]
    WasmCoverage(WasmCoverageCmd),
    /// Structured preparation and artifact recording for diagnostic CSR builds.
    #[command(subcommand)]
    DiagnosticBuild(DiagnosticBuildCmd),
    /// Doctest gate subcommands.
    #[command(subcommand)]
    Doctests(DoctestsCmd),
    /// Observe the retained fixed ERT request.
    Check(CheckArgs),
    /// Compile and stage the closed system-theme inventory independently of CSR output.
    #[cfg(feature = "system-artifacts")]
    SystemArtifacts(SystemArtifactsArgs),
    #[cfg(feature = "performance")]
    /// Versioned performance fragment producers and validators.
    #[command(subcommand)]
    Performance(Box<performance::PerformanceCmd>),
}

#[derive(clap::Args)]
struct CheckArgs {
    /// Retained timing consumer: `ert`.
    name: String,
    /// Auto-fix (the formatters) instead of verifying.
    #[arg(long)]
    fix: bool,
}

#[cfg(feature = "system-artifacts")]
#[derive(clap::Args)]
struct SystemArtifactsArgs {
    /// Output directory for the staged inventory and packaged assets.
    #[arg(long)]
    out: PathBuf,
    /// Internal qualification-only system styling fixture (`a`, `b-app`, or `b-theme`).
    #[cfg(feature = "qualification")]
    #[arg(long)]
    fixture: Option<String>,
}

#[derive(Subcommand)]
enum CoverageCmd {
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
}

#[derive(Subcommand)]
enum WasmCoverageCmd {
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

#[derive(Subcommand)]
enum DoctestsCmd {
    /// Run the workspace doctests and emit the reconciliation status.
    Emit {
        /// Directory to write emitted artifacts into (defaults to CWD).
        #[arg(long, default_value = ".")]
        out: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Coverage(CoverageCmd::Emit { out }) => coverage::emit::run(&out),
        Command::Coverage(CoverageCmd::ValidateStatus { status }) => {
            coverage::validate_status::run(&status)
        }
        Command::WasmCoverage(WasmCoverageCmd::Initialize) => wasm_coverage::initialize(),
        Command::WasmCoverage(WasmCoverageCmd::Map { site_src }) => wasm_coverage::map(&site_src),
        Command::WasmCoverage(WasmCoverageCmd::Finalize) => wasm_coverage::finalize(),
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
        Command::Doctests(DoctestsCmd::Emit { out }) => doctests::emit::run(&out),
        Command::Check(args) => check::run(&args.name, args.fix),
        #[cfg(feature = "system-artifacts")]
        Command::SystemArtifacts(args) => {
            #[cfg(feature = "qualification")]
            let fixture = args.fixture.as_deref();
            #[cfg(not(feature = "qualification"))]
            let fixture = None;
            system_artifacts::run(&args.out, fixture)
        }
        #[cfg(feature = "performance")]
        Command::Performance(command) => performance::run(*command),
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
                "devtool",
                "diagnostic-build",
                "write-coverage-artifacts",
                "--metadata",
                "metadata.json",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "devtool",
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
        assert!(Cli::try_parse_from(["devtool", "wasm-coverage", "map"]).is_err());
        assert!(matches!(
            Cli::try_parse_from([
                "devtool",
                "wasm-coverage",
                "map",
                "--site-src",
                "/source/site",
            ])
            .expect("mapping command parses")
            .command,
            Command::WasmCoverage(WasmCoverageCmd::Map { site_src }) if site_src == Path::new("/source/site")
        ));
    }
    #[test]
    fn coverage_emit_parses_the_producer_output_directory() {
        let emit = Cli::try_parse_from(["devtool", "coverage", "emit", "--out", "coverage"])
            .expect("coverage emit");
        assert!(matches!(
            emit.command,
            Command::Coverage(CoverageCmd::Emit { out }) if out == "coverage"
        ));
        assert!(Cli::try_parse_from(["devtool", "coverage", "experiment-worker"]).is_err());
    }
}
