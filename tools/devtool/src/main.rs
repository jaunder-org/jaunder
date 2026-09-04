//! Internal in-sandbox dev tool. Runs inside the Nix coverage/e2e build
//! sandboxes where `xtask` (host-only) is unavailable. Subcommand tree is
//! deliberately extensible: `coverage emit`, `csr-bundle`, and `seed-e2e` exist
//! today; `pg`-migration of the remaining shell scripts is tracked separately.

use clap::{Parser, Subcommand};

mod check;
mod coverage;
mod csr_bundle;
mod doctests;
mod pg;
mod provision;
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
    /// Post-process a built `csr.wasm` into the served CSR bundle
    /// (`pkg/jaunder.{js,wasm}`): wasm-bindgen + rename + js wasm-ref fix. Shared
    /// by the host build and the Nix `csrWasmBundle` derivation (#236).
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
    group: Option<check::CheckGroup>,
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
    wasm: std::path::PathBuf,
    /// Output directory for the bundle (the site `pkg` dir).
    #[arg(long)]
    out: std::path::PathBuf,
    /// Optional experiment arm label embedded in the direct wasm-init trace detail.
    #[arg(long)]
    wasm_experiment_arm: Option<String>,
    /// Optional tiny custom section embedded after optimisation to perturb module shape.
    #[arg(long)]
    wasm_shape_section: Option<String>,
    /// Number of same-named shape custom sections to append.
    #[arg(long, default_value_t = 1)]
    wasm_shape_section_count: u32,
}

#[derive(clap::Args)]
struct SeedE2eArgs {
    /// Target database URL (passed to test-support as JAUNDER_DB).
    #[arg(long)]
    db: String,
    /// Path to the `test-support` binary — the on-PATH name on the VM guest, the
    /// built `target/debug/test-support` on the host.
    #[arg(long)]
    test_support_bin: std::path::PathBuf,
    /// Path to the real `jaunder` binary (runs the `site-config set` steps). Bare
    /// `jaunder` on the VM guest (systemPackages), the built
    /// `target/debug/jaunder` on the host. Must be a non-cheap-kdf build.
    #[arg(long)]
    jaunder_bin: std::path::PathBuf,
}

#[derive(clap::Args)]
struct ProvisionNodeModulesArgs {
    /// The tsc type-dep closure to symlink. Defaults to $E2E_TYPES_NODE_MODULES,
    /// exported by the Nix devShell.
    #[arg(long)]
    types_node_modules: Option<std::path::PathBuf>,
    /// The nix-matched @playwright/test to pin. Defaults to $E2E_PLAYWRIGHT_TEST,
    /// exported by the Nix devShell.
    #[arg(long)]
    playwright_test: Option<std::path::PathBuf>,
    /// Repo or worktree root; provisions <root>/end2end/node_modules.
    #[arg(long, default_value = ".")]
    root: std::path::PathBuf,
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

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Coverage(CoverageCmd::Emit { out }) => coverage::emit::run(&out),
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
        Command::CsrBundle(args) => csr_bundle::run(
            &args.wasm,
            &args.out,
            args.wasm_experiment_arm.as_deref(),
            args.wasm_shape_section.as_deref(),
            args.wasm_shape_section_count,
        ),
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
    use clap::Parser;

    use super::*;

    #[test]
    fn check_groups_are_closed_and_mutually_exclusive_selectors() {
        let docs = Cli::try_parse_from(["devtool", "check", "--group", "docs"])
            .expect("docs group parses");
        assert!(matches!(
            docs.command,
            Command::Check(CheckArgs {
                name: None,
                group: Some(check::CheckGroup::Docs),
                all: false,
                ..
            })
        ));
        let code = Cli::try_parse_from(["devtool", "check", "--group", "code"])
            .expect("code group parses");
        assert!(matches!(
            code.command,
            Command::Check(CheckArgs {
                group: Some(check::CheckGroup::Code),
                ..
            })
        ));
        assert!(Cli::try_parse_from(["devtool", "check", "fmt", "--group", "code"]).is_err());
        assert!(Cli::try_parse_from(["devtool", "check", "--group", "docs", "--all"]).is_err());
        assert!(Cli::try_parse_from(["devtool", "check", "--group", "unknown"]).is_err());
    }
}
