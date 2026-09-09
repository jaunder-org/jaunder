//! Internal in-sandbox dev tool. Runs inside the Nix coverage/e2e build
//! sandboxes where `xtask` (host-only) is unavailable. Subcommand tree is
//! deliberately extensible: `coverage emit`, `wasm-coverage`, `csr-bundle`, and
//! `seed-e2e` exist today; `pg`-migration of the remaining shell scripts is
//! tracked separately.

use std::path::PathBuf;

use anyhow::Result;
use check::CheckGroup;
use clap::{Parser, Subcommand};

mod check;
mod coverage;
mod csr_bundle;
mod diagnostic_build;
mod digest;
mod doctests;
mod pg;
mod provision;
mod run;
mod seed_e2e;
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
    /// Ephemeral PostgreSQL subcommands.
    #[command(subcommand)]
    Pg(PgCmd),
    /// Run one program (no shell), capturing output to .xtask/run/ and returning
    /// a structured JSON result; exits with the child's exit code.
    Run(RunArgs),
    /// Run the migrated static checks (#188/#276): one by name, or `--all`.
    Check(CheckArgs),
    /// Post-process a built `csr.wasm` into a verified CSR bundle root
    /// (`manifest.json`, rendered `index.html`, and content-addressed `pkg/**`).
    /// Shared by the host build and the Nix `csrWasmBundle` derivation (#236/#869).
    CsrBundle(CsrBundleArgs),
    /// Seed the canonical e2e fixtures (users + site-config + mail-reset) by
    /// shelling out to `test-support`. The single fixture list shared by the
    /// host loop and the flake VM `seed_db()` (#249).
    SeedE2e(SeedE2eArgs),
    /// Symlink the tsc type-dep closure + the nix-matched Playwright into
    /// `<root>/end2end/node_modules` (gitignored, so absent in fresh checkouts and
    /// worktrees). Run by the devShell shellHook and by `check tsc` (#229).
    ProvisionNodeModules(ProvisionNodeModulesArgs),
}

#[derive(clap::Args)]
struct CheckArgs {
    /// Which check to run (omit and pass `--group` or `--all` to select a set).
    name: Option<String>,
    /// Run one stable static check group.
    #[arg(long, conflicts_with_all = ["name", "all"])]
    group: Option<CheckGroup>,
    /// Run all migrated static checks.
    #[arg(long, conflicts_with = "name")]
    all: bool,
    /// Auto-fix (the formatters) instead of verifying.
    #[arg(long)]
    fix: bool,
    /// Run Cargo-backed checks with workspace-specific offline Cargo config.
    #[arg(long)]
    sandbox_cargo: bool,
}

#[derive(clap::Args)]
struct CsrBundleArgs {
    /// Path to the built `csr.wasm` (crane output or `target/.../csr.wasm`).
    #[arg(long)]
    wasm: PathBuf,
    /// Output directory for the bundle root (`manifest.json`, `index.html`, `pkg/**`).
    #[arg(long)]
    out: PathBuf,
    /// Optional experiment arm label embedded in the direct wasm-init trace detail.
    #[arg(long)]
    wasm_experiment_arm: Option<String>,
    /// Optional tiny custom section embedded after optimisation to perturb module shape.
    #[arg(long)]
    wasm_shape_section: Option<String>,
    /// Number of same-named shape custom sections to append.
    #[arg(long, default_value_t = 1)]
    wasm_shape_section_count: u32,
    /// Write a truthful before/after LLVM coverage-section status for the
    /// diagnostic bundle. Requires the companion identity output and minicov pin.
    #[arg(long, requires_all = ["diagnostic_toolchain_identity", "diagnostic_minicov_version"])]
    diagnostic_coverage_metadata: Option<PathBuf>,
    /// Write exact compiler, LLVM, wasm-bindgen, wasm-opt, and minicov identities.
    #[arg(long, requires_all = ["diagnostic_coverage_metadata", "diagnostic_minicov_version"])]
    diagnostic_toolchain_identity: Option<PathBuf>,
    /// Exact minicov crate version selected by the diagnostic Cargo feature.
    #[arg(long, requires_all = ["diagnostic_coverage_metadata", "diagnostic_toolchain_identity"])]
    diagnostic_minicov_version: Option<String>,
}

#[derive(clap::Args)]
struct SeedE2eArgs {
    /// Target database URL (passed to test-support as JAUNDER_DB).
    #[arg(long)]
    db: String,
    /// Path to the `test-support` binary — the on-PATH name on the VM guest, the
    /// built `target/debug/test-support` on the host.
    #[arg(long)]
    test_support_bin: PathBuf,
    /// Path to the real `jaunder` binary (runs the `site-config set` steps). Bare
    /// `jaunder` on the VM guest (systemPackages), the built
    /// `target/debug/jaunder` on the host. Must be a non-cheap-kdf build.
    #[arg(long)]
    jaunder_bin: PathBuf,
}

#[derive(clap::Args)]
struct ProvisionNodeModulesArgs {
    /// The tsc type-dep closure to symlink. Defaults to $E2E_TYPES_NODE_MODULES,
    /// exported by the Nix devShell.
    #[arg(long)]
    types_node_modules: Option<PathBuf>,
    /// The nix-matched @playwright/test to pin. Defaults to $E2E_PLAYWRIGHT_TEST,
    /// exported by the Nix devShell.
    #[arg(long)]
    playwright_test: Option<PathBuf>,
    /// Repo or worktree root; provisions <root>/end2end/node_modules.
    #[arg(long, default_value = ".")]
    root: PathBuf,
}

#[derive(clap::Args)]
struct RunArgs {
    /// Working directory for the command (defaults to the current directory).
    #[arg(long)]
    cwd: Option<PathBuf>,
    /// Kill the command after this many seconds (default: no limit).
    #[arg(long)]
    timeout: Option<u64>,
    /// The program and its arguments, after `--`.
    #[arg(trailing_var_arg = true, required = true)]
    cmd: Vec<String>,
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

#[derive(Subcommand)]
enum PgCmd {
    /// Run a command with a throwaway PostgreSQL 16 cluster.
    Run {
        /// Command (and its arguments) to run, after `--`.
        #[arg(trailing_var_arg = true, required = true)]
        cmd: Vec<String>,
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
        Command::Pg(PgCmd::Run { cmd }) => pg::run_command(&cmd),
        Command::Run(args) => run::run(&args.cmd, args.cwd, args.timeout),
        Command::Check(args) => check::run(
            args.name.as_deref(),
            args.group,
            args.all,
            args.fix,
            args.sandbox_cargo,
        ),
        Command::CsrBundle(args) => {
            let diagnostic_artifacts = match (
                args.diagnostic_coverage_metadata.as_deref(),
                args.diagnostic_toolchain_identity.as_deref(),
                args.diagnostic_minicov_version.as_deref(),
            ) {
                (Some(metadata_status), Some(toolchain_identity), Some(minicov_version)) => {
                    Some(csr_bundle::DiagnosticArtifacts {
                        metadata_status,
                        toolchain_identity,
                        minicov_version,
                    })
                }
                (None, None, None) => None,
                _ => anyhow::bail!(
                    "diagnostic CSR bundle arguments must select metadata, toolchain identity, and minicov together"
                ),
            };
            csr_bundle::run(
                &args.wasm,
                &args.out,
                args.wasm_experiment_arm.as_deref(),
                args.wasm_shape_section.as_deref(),
                args.wasm_shape_section_count,
                diagnostic_artifacts.as_ref(),
            )
        }
        Command::SeedE2e(args) => {
            seed_e2e::run(&args.db, &args.test_support_bin, &args.jaunder_bin)
        }
        Command::ProvisionNodeModules(args) => {
            let paths =
                provision::StorePaths::resolve(args.types_node_modules, args.playwright_test)?;
            provision::run(&args.root, &paths)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::Parser;

    use super::*;

    #[test]
    fn diagnostic_bundle_artifacts_are_an_all_or_nothing_contract() {
        assert!(
            Cli::try_parse_from([
                "devtool",
                "csr-bundle",
                "--wasm",
                "csr.wasm",
                "--out",
                "pkg",
                "--diagnostic-coverage-metadata",
                "metadata.json",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "devtool",
                "csr-bundle",
                "--wasm",
                "csr.wasm",
                "--out",
                "pkg",
                "--diagnostic-coverage-metadata",
                "metadata.json",
                "--diagnostic-toolchain-identity",
                "toolchain.json",
                "--diagnostic-minicov-version",
                "0.3.8",
            ])
            .is_ok()
        );
    }

    #[test]
    fn check_groups_are_closed_and_mutually_exclusive_selectors() {
        let docs = Cli::try_parse_from(["devtool", "check", "--group", "docs"])
            .expect("docs group parses");
        assert!(matches!(
            docs.command,
            Command::Check(CheckArgs {
                name: None,
                group: Some(CheckGroup::Docs),
                all: false,
                ..
            })
        ));
        let code = Cli::try_parse_from(["devtool", "check", "--group", "code"])
            .expect("code group parses");
        assert!(matches!(
            code.command,
            Command::Check(CheckArgs {
                group: Some(CheckGroup::Code),
                ..
            })
        ));
        assert!(Cli::try_parse_from(["devtool", "check", "fmt", "--group", "code"]).is_err());
        assert!(Cli::try_parse_from(["devtool", "check", "--group", "docs", "--all"]).is_err());
        assert!(Cli::try_parse_from(["devtool", "check", "--group", "unknown"]).is_err());
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
}
