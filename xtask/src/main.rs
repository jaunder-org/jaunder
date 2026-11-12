use clap::Parser;
use xtask::{Cli, Command, StepResult};

fn report_leaf(step: StepResult) -> i32 {
    let label = if step.skipped {
        "skip"
    } else if step.ok {
        " ok "
    } else {
        "FAIL"
    };
    println!("[{label}] {}", step.name);
    if let Some(detail) = step.detail {
        for line in detail.lines() {
            println!("  {line}");
        }
    }
    if step.ok || step.skipped { 0 } else { 1 }
}

fn main() {
    let cli = Cli::parse();
    if cli.json && !cli.command.produces_json_payload() {
        eprintln!("--json is not supported for `{}`", cli.command_name());
        std::process::exit(2);
    }
    // Artifact verdicts are deliberately plain commands: no build hook, lifecycle
    // sidecar, StepResult, or presentation wrapper can select their evidence.
    let artifact = xtask::artifact_verdict(&cli.command);
    if let Some(verdict) = artifact {
        match verdict {
            Ok(detail) => println!("{detail}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    let json = cli.json;
    let command_name = cli.command_name();
    match cli.command {
        Command::QualificationLeaf {
            expected_harness,
            source,
            target,
        } => {
            match xtask::production_baseline::run_leaf(
                expected_harness.as_deref(),
                source.as_deref(),
                target.as_deref(),
            ) {
                Ok(detail) => println!("{detail}"),
                Err(error) => {
                    eprintln!("qualification: {error:#}");
                    std::process::exit(1);
                }
            }
        }
        Command::CheckLeaf { name } => {
            if json {
                eprintln!("xtask: --json is not supported for `check-leaf`");
                std::process::exit(2);
            }
            let exit_code = match xtask::run_check_leaf_process(&name) {
                Ok(step) => report_leaf(step),
                Err(error) => {
                    eprintln!("xtask check-leaf: {error}");
                    2
                }
            };
            std::process::exit(exit_code);
        }
        command => {
            let cli = Cli { json, command };
            let result = xtask::invocation_trace::execute(cli);
            match result {
                Ok(result) => {
                    result.report(json);
                    eprintln!(
                        "xtask-done: command={} ok={} exit={} duration_ms={}",
                        result.command,
                        result.ok,
                        result.exit_code(),
                        result.duration_ms
                    );
                    std::process::exit(result.exit_code());
                }
                Err(err) => {
                    eprintln!("xtask: {err:#}");
                    eprintln!(
                        "xtask-done: command={command_name} ok=false exit=2 error={:?}",
                        err.to_string()
                    );
                    std::process::exit(2);
                }
            }
        }
    }
}
