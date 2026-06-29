//! `devtool run` — run exactly one program with no shell, capturing stdout and
//! stderr to files under `.xtask/run/` and returning a structured JSON result.
//! The runner exits with the child's exit code, so callers get an honest
//! pass/fail signal without shell scaffolding (`; echo $?`, `2>&1 | tail`) — the
//! latter silently overwrites the exit status with the last pipe stage's.

use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use serde::Serialize;

const RUN_DIR: &str = ".xtask/run";

#[derive(Serialize)]
pub struct Stream {
    pub path: String,
    pub bytes: u64,
    pub lines: u64,
}

#[derive(Serialize)]
pub struct RunResult {
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub ok: bool,
    pub signal: Option<String>,
    pub duration_ms: u128,
    pub stdout: Stream,
    pub stderr: Stream,
}

/// A runner-level failure (not a child failure). Serialized as `{error, kind}`.
#[derive(Debug)]
pub struct RunError {
    pub message: String,
    pub kind: &'static str, // "usage" | "shell_refused" | "spawn"
}

fn run_dir(cwd: &Path) -> PathBuf {
    cwd.join(RUN_DIR)
}

fn alloc_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{millis}-{}", std::process::id())
}

/// Count `\n` bytes (wc -l semantics) by streaming, so a huge output file never
/// lands in memory all at once.
fn count_lines(path: &Path) -> u64 {
    let Ok(f) = File::open(path) else {
        return 0;
    };
    let mut reader = BufReader::new(f);
    let mut buf = [0u8; 64 * 1024];
    let mut count = 0u64;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => count += buf[..n].iter().filter(|&&b| b == b'\n').count() as u64,
            Err(_) => break,
        }
    }
    count
}

fn stream_of(path: &Path) -> Stream {
    let bytes = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Stream {
        path: path.display().to_string(),
        bytes,
        lines: count_lines(path),
    }
}

struct Capture {
    exit_code: Option<i32>,
    signal: Option<i32>,
    duration_ms: u128,
}

#[cfg(unix)]
fn signal_name(sig: i32) -> String {
    match sig {
        2 => "SIGINT".into(),
        6 => "SIGABRT".into(),
        9 => "SIGKILL".into(),
        15 => "SIGTERM".into(),
        n => format!("SIG{n}"),
    }
}

#[cfg(not(unix))]
fn signal_name(sig: i32) -> String {
    format!("SIG{sig}")
}

/// Split a finished status into (exit_code, signal). On unix a signal death has
/// no exit code, so we report the signal instead.
fn interpret_status(status: &std::process::ExitStatus) -> (Option<i32>, Option<i32>) {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return (None, Some(sig));
        }
    }
    (status.code(), None)
}

fn exec_capture(
    argv: &[String],
    cwd: &Path,
    out_path: &Path,
    err_path: &Path,
) -> Result<Capture, RunError> {
    let mk = |p: &Path| -> Result<File, RunError> {
        File::create(p).map_err(|e| RunError {
            message: format!("creating {}: {e}", p.display()),
            kind: "spawn",
        })
    };
    let out_file = mk(out_path)?;
    let err_file = mk(err_path)?;

    let start = Instant::now();
    let status = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file))
        .spawn()
        .map_err(|e| RunError {
            message: format!("spawning {argv:?}: {e}"),
            kind: "spawn",
        })?
        .wait()
        .map_err(|e| RunError {
            message: format!("waiting on {argv:?}: {e}"),
            kind: "spawn",
        })?;
    let duration_ms = start.elapsed().as_millis();

    let (exit_code, signal) = interpret_status(&status);
    Ok(Capture {
        exit_code,
        signal,
        duration_ms,
    })
}

const REFUSED_SHELLS: &[&str] = &["bash", "sh", "zsh", "fish", "dash", "ash", "eval"];

/// Refuse argv that re-opens a shell or the `nix develop` wrapper — the whole
/// point of the runner is that there is no shell, so allowlisting
/// `devtool run *` stays narrower than `bash *`. Everything else runs.
/// `env VAR=x cmd` is deliberately allowed (a per-command env idiom, not a
/// shell); `xargs` is allowed (neutered here by the `/dev/null` stdin).
pub fn validate_argv(argv: &[String]) -> Result<(), RunError> {
    let first = argv.first().ok_or_else(|| RunError {
        message: "no command given after `--`".into(),
        kind: "usage",
    })?;
    let base = Path::new(first)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(first);
    if REFUSED_SHELLS.contains(&base) {
        return Err(RunError {
            message: format!("refusing to run a shell (`{base}`); invoke the program directly"),
            kind: "shell_refused",
        });
    }
    if base == "nix" && argv.get(1).map(String::as_str) == Some("develop") {
        return Err(RunError {
            message: "refusing `nix develop`; the toolchain is already on PATH via direnv".into(),
            kind: "shell_refused",
        });
    }
    Ok(())
}

/// Run `argv` in `cwd`, parking output under `.xtask/run/`, and return the result
/// plus the process exit code the caller should exit with. No printing, no
/// `exit` — so it is unit-testable.
pub fn execute(argv: &[String], cwd: &Path) -> Result<(RunResult, i32), RunError> {
    validate_argv(argv)?;
    let dir = run_dir(cwd);
    fs::create_dir_all(&dir).map_err(|e| RunError {
        message: format!("creating {}: {e}", dir.display()),
        kind: "spawn",
    })?;
    let id = alloc_id();
    let out_path = dir.join(format!("{id}.out"));
    let err_path = dir.join(format!("{id}.err"));

    let cap = exec_capture(argv, cwd, &out_path, &err_path)?;

    let process_code = match (cap.exit_code, cap.signal) {
        (Some(code), _) => code,
        (None, Some(sig)) => 128 + sig,
        (None, None) => 1,
    };
    let result = RunResult {
        command: argv.to_vec(),
        exit_code: cap.exit_code,
        ok: cap.exit_code == Some(0),
        signal: cap.signal.map(signal_name),
        duration_ms: cap.duration_ms,
        stdout: stream_of(&out_path),
        stderr: stream_of(&err_path),
    };
    Ok((result, process_code))
}

/// CLI entry: run the command, print the JSON result, and exit with the child's
/// code (or 64 for a runner-level failure).
pub fn run(argv: &[String], cwd: Option<PathBuf>) -> ! {
    let cwd = cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    match execute(argv, &cwd) {
        Ok((result, code)) => {
            println!("{}", serde_json::to_string_pretty(&result).unwrap());
            std::process::exit(code);
        }
        Err(e) => {
            println!(
                "{}",
                serde_json::json!({ "error": e.message, "kind": e.kind })
            );
            std::process::exit(64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        tempfile::Builder::new()
            .prefix("devtool-run-test.")
            .tempdir()
            .unwrap()
            .keep()
    }

    #[test]
    fn true_succeeds_with_empty_streams() {
        let cwd = tmp();
        let (r, code) = execute(&["true".into()], &cwd).unwrap();
        assert_eq!(r.exit_code, Some(0));
        assert!(r.ok);
        assert_eq!(code, 0);
        assert_eq!(r.stdout.bytes, 0);
        assert_eq!(r.stderr.bytes, 0);
        assert!(Path::new(&r.stdout.path).exists());
    }

    #[test]
    fn false_fails_with_exit_one() {
        let cwd = tmp();
        let (r, code) = execute(&["false".into()], &cwd).unwrap();
        assert_eq!(r.exit_code, Some(1));
        assert!(!r.ok);
        assert_eq!(code, 1);
    }

    #[test]
    fn stdout_bytes_and_lines_counted() {
        let cwd = tmp();
        let (r, _) = execute(&["printf".into(), "a\nb\n".into()], &cwd).unwrap();
        assert_eq!(r.stdout.bytes, 4);
        assert_eq!(r.stdout.lines, 2);
        assert_eq!(r.stderr.bytes, 0);
    }

    #[test]
    fn stderr_captured_separately() {
        let cwd = tmp();
        let (r, _) = execute(&["ls".into(), "/no/such/path/xyzzy".into()], &cwd).unwrap();
        assert!(!r.ok);
        assert!(r.stderr.bytes > 0);
        assert_eq!(r.stdout.bytes, 0);
    }

    #[test]
    fn accepts_normal_programs() {
        for ok in [
            vec!["cargo".to_string(), "build".into()],
            vec!["git".into(), "status".into()],
            vec!["gh".into(), "pr".into(), "view".into()],
            vec!["rg".into(), "foo".into()],
            vec!["nix".into(), "build".into()],
            vec!["emacs".into(), "--batch".into()],
            vec!["prettier".into(), "--check".into()],
            vec!["/usr/bin/printf".into(), "x".into()],
        ] {
            assert!(validate_argv(&ok).is_ok(), "{ok:?} should be accepted");
        }
    }

    #[test]
    fn refuses_shells_and_nix_develop() {
        for bad in [
            vec!["bash".to_string(), "-c".into(), "echo hi".into()],
            vec!["sh".into()],
            vec!["zsh".into()],
            vec!["fish".into()],
            vec!["dash".into()],
            vec!["ash".into()],
            vec!["eval".into()],
            vec!["/bin/bash".into(), "-c".into(), "x".into()],
            vec!["nix".into(), "develop".into(), "-c".into(), "cargo".into()],
        ] {
            let err = validate_argv(&bad).unwrap_err();
            assert_eq!(err.kind, "shell_refused", "{bad:?}");
        }
    }

    #[test]
    fn refuses_empty_argv() {
        let err = validate_argv(&[]).unwrap_err();
        assert_eq!(err.kind, "usage");
    }
}
