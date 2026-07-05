//! Internal in-sandbox dev tool. Runs inside the Nix coverage/e2e build
//! sandboxes where `xtask` (host-only) is unavailable. Subcommand tree is
//! deliberately extensible: `coverage emit`, `csr-bundle`, and `seed-e2e` exist
//! today; `pg`-migration of the remaining shell scripts is tracked separately.

use clap::{Parser, Subcommand};

mod check;
mod coverage;
mod csr_bundle;
mod pg;
mod run;
mod seed_e2e;

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
    /// Ephemeral PostgreSQL subcommands.
    #[command(subcommand)]
    Pg(PgCmd),
    /// Run one program (no shell), capturing output to .xtask/run/ and returning
    /// a structured JSON result; exits with the child's exit code.
    Run(RunArgs),
    /// Run the non-compiling static checks (#188): one by name, or `--all`.
    Check(CheckArgs),
    /// Post-process a built `csr.wasm` into the served CSR bundle
    /// (`pkg/jaunder.{js,wasm}`): wasm-bindgen + rename + js wasm-ref fix. Shared
    /// by the host build and the Nix `csrWasmBundle` derivation (#236).
    CsrBundle(CsrBundleArgs),
    /// Seed the canonical e2e fixtures (users + site-config + mail-reset) by
    /// shelling out to `test-support`. The single fixture list shared by the
    /// host loop and the flake VM `seed_db()` (#249).
    SeedE2e(SeedE2eArgs),
}

#[derive(clap::Args)]
struct CheckArgs {
    /// Which check to run (omit and pass `--all` to run every check).
    name: Option<String>,
    /// Run all the non-compiling static checks.
    #[arg(long, conflicts_with = "name")]
    all: bool,
    /// Auto-fix (the formatters) instead of verifying.
    #[arg(long)]
    fix: bool,
}

#[derive(clap::Args)]
struct CsrBundleArgs {
    /// Path to the built `csr.wasm` (crane output or `target/.../csr.wasm`).
    #[arg(long)]
    wasm: std::path::PathBuf,
    /// Output directory for the bundle (the site `pkg` dir).
    #[arg(long)]
    out: std::path::PathBuf,
}

#[derive(clap::Args)]
struct SeedE2eArgs {
    /// Target database URL (passed to test-support as JAUNDER_DB).
    #[arg(long)]
    db: String,
    /// Mail-capture file to reset.
    #[arg(long)]
    mail_file: String,
    /// Path to the `test-support` binary — the on-PATH name on the VM guest, the
    /// built `target/debug/test-support` on the host.
    #[arg(long)]
    test_support_bin: std::path::PathBuf,
}

#[derive(clap::Args)]
struct RunArgs {
    /// Working directory for the command (defaults to the current directory).
    #[arg(long)]
    cwd: Option<std::path::PathBuf>,
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

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Coverage(CoverageCmd::Emit { out }) => coverage::emit::run(&out),
        Command::Pg(PgCmd::Run { cmd }) => pg::run_command(&cmd),
        Command::Run(args) => run::run(&args.cmd, args.cwd, args.timeout),
        Command::Check(args) => check::run(args.name.as_deref(), args.all, args.fix),
        Command::CsrBundle(args) => csr_bundle::run(&args.wasm, &args.out),
        Command::SeedE2e(args) => seed_e2e::run(&args.db, &args.mail_file, &args.test_support_bin),
    }
}
