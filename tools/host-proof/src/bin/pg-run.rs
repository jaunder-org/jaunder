//! `pg-run -- <command> [args]`: run a command against a throwaway PostgreSQL
//! cluster (coverage's ephemeral-postgres custody), propagating its exit code.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match args.first().map(String::as_str) {
        Some("--") => &args[1..],
        _ => &args[..],
    };
    if cmd.is_empty() {
        eprintln!("usage: pg-run -- <command> [args]");
        std::process::exit(2);
    }
    if let Err(error) = coverage::ephemeral_postgres::run_command(cmd) {
        eprintln!("pg-run: {error:#}");
        std::process::exit(1);
    }
}
