//! Read explicit local originals without checkout admission, hooks, or server access.
use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
};

fn main() -> ExitCode {
    // This domain-only interface accepts paths, not discovery or inventory options.
    let mut files = Vec::new();
    let mut paths_only = false;
    for argument in std::env::args_os().skip(1) {
        if !paths_only && argument == "--" {
            paths_only = true;
        } else if !paths_only && argument.as_encoded_bytes().starts_with(b"-") {
            eprintln!("usage: org-timezone-manifest [--] <file>...");
            return ExitCode::from(2);
        } else {
            files.push(PathBuf::from(argument));
        }
    }
    if files.is_empty() {
        eprintln!("usage: org-timezone-manifest [--] <file>...");
        return ExitCode::from(2);
    }
    let report = repository_policy::org_timezone_manifest::extract(&files);
    let output = io::stdout();
    let mut output = output.lock();
    // Treat encoding and output errors as invocation/output failure, never partial success.
    if let Err(error) = serde_json::to_writer_pretty(&mut output, &report)
        .map_err(io::Error::other)
        .and_then(|()| writeln!(output))
        .and_then(|()| output.flush())
    {
        eprintln!("org-timezone-manifest: {error}");
        return ExitCode::from(2);
    }
    ExitCode::from(u8::from(!report.diagnostics.is_empty()))
}
