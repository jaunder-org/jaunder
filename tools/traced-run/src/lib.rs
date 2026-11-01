//! Shared command runner for `traced-run` and the temporary `devtool run`
//! compatibility route. It executes one program without a shell and parks output under `.xtask/run/`.
//! The runner exits with the child's exit code, so callers get an honest
//! pass/fail signal without shell scaffolding (`; echo $?`, `2>&1 | tail`) — the
//! latter silently overwrites the exit status with the last pipe stage's.

use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use processkit::{Command, Outcome, RunningProcess};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::runtime::{Builder, Runtime};

#[cfg(unix)]
mod terminal;

const RUN_DIR: &str = ".xtask/run";
const RUN_HISTORY_LIMIT: usize = 50;
const TRACE_GUARD: &str = "JAUNDER_CARGO_XTASK_INNER";
const TRACE_ENV: &str = "JAUNDER_TRACED_RUN_CONTEXT";
static NEXT_RUN: AtomicU64 = AtomicU64::new(1);

fn monotonic_ns() -> u64 {
    let time = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    (time.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(time.tv_nsec as u64)
}

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timed_out: Option<bool>,
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
    format!(
        "{millis}-{}-{}",
        std::process::id(),
        NEXT_RUN.fetch_add(1, Ordering::Relaxed)
    )
}

/// Keep only the newest history files. Enumeration/stat/removal failures are
/// aggregated into one stderr warning and never change the child result.
fn prune(dir: &Path) {
    prune_with(dir, |path| fs::remove_file(path), &mut std::io::stderr());
}

fn prune_with(
    dir: &Path,
    mut remove: impl FnMut(&Path) -> std::io::Result<()>,
    stderr: &mut impl Write,
) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => {
            report_cleanup_failure(true, "devtool.run.history_prune", stderr);
            return;
        }
    };
    let population = entries.filter_map(|entry| match entry {
        Ok(entry) => {
            let path = entry.path();
            if !matches!(
                path.extension().and_then(|extension| extension.to_str()),
                Some("out" | "err")
            ) {
                return None;
            }
            let modified = entry.metadata().and_then(|metadata| metadata.modified());
            Some(Ok((path, modified)))
        }
        Err(error) => Some(Err(error)),
    });
    prune_entries_with(population, &mut remove, stderr);
}

fn prune_entries_with(
    entries: impl IntoIterator<
        Item = std::io::Result<(PathBuf, std::io::Result<std::time::SystemTime>)>,
    >,
    mut remove: impl FnMut(&Path) -> std::io::Result<()>,
    stderr: &mut impl Write,
) {
    let mut failed = false;
    let mut files = Vec::new();
    for entry in entries {
        let (path, modified) = match entry {
            Ok(entry) => entry,
            Err(_) => {
                failed = true;
                continue;
            }
        };
        match modified {
            Ok(modified)
                if matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("out" | "err")
                ) =>
            {
                files.push((modified, path))
            }
            Ok(_) => continue,
            Err(_) => failed = true,
        }
    }
    files.sort_by_key(|file| std::cmp::Reverse(file.0));
    for (_, path) in files.into_iter().skip(RUN_HISTORY_LIMIT) {
        failed |= remove(&path).is_err();
    }
    report_cleanup_failure(failed, "devtool.run.history_prune", stderr);
}

fn report_cleanup_failure(failed: bool, key: &str, stderr: &mut impl Write) {
    if failed {
        let _ = writeln!(
            stderr,
            "devtool: warning: {key}: ignored failure during runner cleanup"
        );
    }
}

/// Count `\n` bytes (wc -l semantics) by streaming, so a huge output file never
/// lands in memory all at once.
fn count_lines(path: &Path) -> std::io::Result<u64> {
    let f = File::open(path)?;
    let mut reader = BufReader::new(f);
    let mut buf = [0u8; 64 * 1024];
    let mut count = 0u64;
    loop {
        match reader.read(&mut buf)? {
            0 => break,
            n => count += buf[..n].iter().filter(|&&b| b == b'\n').count() as u64,
        }
    }
    Ok(count)
}

fn stream_of(path: &Path) -> Stream {
    stream_of_with(
        path,
        || fs::metadata(path).map(|metadata| metadata.len()),
        || count_lines(path),
        &mut std::io::stderr(),
    )
}

fn stream_of_with(
    path: &Path,
    metadata: impl FnOnce() -> std::io::Result<u64>,
    count: impl FnOnce() -> std::io::Result<u64>,
    stderr: &mut impl Write,
) -> Stream {
    let mut failed = false;
    let bytes = metadata().unwrap_or_else(|_| {
        failed = true;
        0
    });
    let lines = count().unwrap_or_else(|_| {
        failed = true;
        0
    });
    if failed {
        let _ = writeln!(
            stderr,
            "devtool: warning: devtool.run.stream_summary: ignored failure while summarizing captured output"
        );
    }
    Stream {
        path: path.display().to_string(),
        bytes,
        lines,
    }
}

struct Capture {
    exit_code: Option<i32>,
    signal: Option<i32>,
    timed_out: bool,
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

struct Process {
    running: Option<RunningProcess>,
    runtime: Runtime,
}

impl Process {
    fn start(command: Command, argv: &[String]) -> Result<Self, RunError> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|error| RunError {
                message: format!("creating process runtime: {error}"),
                kind: "spawn",
            })?;
        let running = runtime
            .block_on(command.start())
            .map_err(|error| RunError {
                message: format!("spawning {argv:?}: {error}"),
                kind: "spawn",
            })?;
        Ok(Self {
            running: Some(running),
            runtime,
        })
    }

    fn wait(mut self, argv: &[String]) -> Result<Outcome, RunError> {
        let running = self
            .running
            .take()
            .expect("a newly started process is still running");
        self.runtime
            .block_on(running.wait())
            .map_err(|error| RunError {
                message: format!("waiting on {argv:?}: {error}"),
                kind: "spawn",
            })
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // The process owns kill-on-drop containment; release it before its
        // executor so unwind cannot strand processkit's teardown work.
        std::mem::drop(self.running.take());
    }
}

#[cfg(unix)]
fn timeout_signal() -> Option<i32> {
    Some(9)
}

#[cfg(not(unix))]
fn timeout_signal() -> Option<i32> {
    None
}

fn exec_capture(
    argv: &[String],
    cwd: &Path,
    timeout: Option<u64>,
    out_path: &Path,
    err_path: &Path,
    trace_context: Option<&str>,
) -> Result<Capture, RunError> {
    let command = Command::new(&argv[0])
        .args(&argv[1..])
        .current_dir(cwd)
        .stdout_file(out_path)
        .stderr_file(err_path)
        .timeout_opt(timeout.map(Duration::from_secs));
    let command = if let Some(context) = trace_context {
        command.env(TRACE_ENV, context)
    } else {
        command
    };

    let start = Instant::now();
    let outcome = Process::start(command, argv)?.wait(argv)?;
    let duration_ms = start.elapsed().as_millis();
    let timed_out = outcome.timed_out();
    let signal = if timed_out {
        timeout_signal()
    } else {
        outcome.signal()
    };

    Ok(Capture {
        exit_code: outcome.code(),
        signal,
        timed_out,
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
pub fn execute(
    argv: &[String],
    cwd: &Path,
    timeout: Option<u64>,
) -> Result<(RunResult, i32), RunError> {
    execute_with_trace(argv, cwd, timeout, None)
}

fn execute_with_trace(
    argv: &[String],
    cwd: &Path,
    timeout: Option<u64>,
    trace_context: Option<&str>,
) -> Result<(RunResult, i32), RunError> {
    validate_argv(argv)?;
    let dir = run_dir(cwd);
    fs::create_dir_all(&dir).map_err(|e| RunError {
        message: format!("creating {}: {e}", dir.display()),
        kind: "spawn",
    })?;
    let id = alloc_id();
    let out_path = dir.join(format!("{id}.out"));
    let err_path = dir.join(format!("{id}.err"));

    let cap = exec_capture(argv, cwd, timeout, &out_path, &err_path, trace_context)?;
    prune(&dir);

    let process_code = if cap.timed_out {
        124
    } else {
        match (cap.exit_code, cap.signal) {
            (Some(code), _) => code,
            (None, Some(sig)) => 128 + sig,
            (None, None) => 1,
        }
    };
    let result = RunResult {
        command: argv.to_vec(),
        exit_code: cap.exit_code,
        ok: cap.exit_code == Some(0),
        signal: cap.signal.map(signal_name),
        duration_ms: cap.duration_ms,
        timed_out: if cap.timed_out { Some(true) } else { None },
        stdout: stream_of(&out_path),
        stderr: stream_of(&err_path),
    };
    Ok((result, process_code))
}

/// Compatibility entry point retained until the externally-owned route is migrated.
pub fn run_compat(argv: &[String], cwd: &Path, timeout: Option<u64>) -> ! {
    run(argv, Some(cwd.to_owned()), timeout)
}

/// Parse the standalone runner's deliberately small option surface.
#[derive(Clone, Serialize, Deserialize)]
struct TraceContext {
    run_id: String,
    trace_dir: String,
    origin_ns: u64,
    parent_span_id: String,
}

struct TraceSpan {
    name: String,
    span_id: String,
    context: TraceContext,
    root: bool,
    started_ns: u64,
}

static NEXT_SPAN: AtomicU64 = AtomicU64::new(1);

fn prune_traces(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        eprintln!("traced-run: warning: trace pruning unavailable");
        return;
    };
    let mut completed = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".trace.json"))
            && let Ok(modified) = entry.metadata().and_then(|metadata| metadata.modified())
        {
            completed.push((modified, path));
        }
    }
    completed.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    for (_, trace) in completed.into_iter().skip(RUN_HISTORY_LIMIT) {
        let stem = trace
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .trim_end_matches(".trace.json");
        if fs::remove_file(&trace).is_err() {
            eprintln!("traced-run: warning: completed trace pruning failed");
            continue;
        }
        let records = dir.join(format!("{stem}.trace.d"));
        if records.exists() && fs::remove_dir_all(records).is_err() {
            eprintln!("traced-run: warning: completed trace record pruning failed");
        }
    }
}

impl TraceSpan {
    fn start(name: &str, cwd: &Path) -> Result<Self, RunError> {
        if name.is_empty()
            || name.len() > 80
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err(RunError {
                message:
                    "trace name must be 1-80 ASCII letters, digits, dot, underscore, or hyphen"
                        .into(),
                kind: "usage",
            });
        }
        let inherited = std::env::var(TRACE_ENV)
            .ok()
            .and_then(|value| serde_json::from_str::<TraceContext>(&value).ok());
        let root = inherited.is_none();
        let context = if let Some(context) = inherited {
            context
        } else {
            let run_id = alloc_id();
            let trace_dir = run_dir(cwd).join(format!("{run_id}.trace.d"));
            if fs::create_dir_all(&trace_dir).is_err() {
                eprintln!("traced-run: warning: trace directory could not be created");
            }
            TraceContext {
                run_id,
                trace_dir: trace_dir.display().to_string(),
                origin_ns: monotonic_ns(),
                parent_span_id: String::new(),
            }
        };
        let span_id = format!(
            "{}-{}",
            std::process::id(),
            NEXT_SPAN.fetch_add(1, Ordering::Relaxed)
        );
        let started_ns = if root {
            context.origin_ns
        } else {
            monotonic_ns()
        };
        let active = PathBuf::from(&context.trace_dir).join(format!("active-{span_id}"));
        if fs::write(&active, b"unfinished\n").is_err() {
            eprintln!(
                "traced-run: warning: participant registration unavailable; trace will be incomplete"
            );
            let _ = fs::write(
                PathBuf::from(&context.trace_dir).join("incomplete"),
                b"participant registration failed\n",
            );
        }
        Ok(Self {
            name: name.to_owned(),
            span_id,
            context,
            root,
            started_ns,
        })
    }

    fn child_context(&self) -> TraceContext {
        let mut context = self.context.clone();
        context.parent_span_id = self.span_id.clone();
        context
    }

    fn finish(self) {
        let dir = PathBuf::from(&self.context.trace_dir);
        let end_ns = monotonic_ns();
        let event = serde_json::json!({
            "name": self.name, "cat":"process", "ph":"X", "pid":std::process::id(), "tid":std::process::id(),
            "ts":self.started_ns.saturating_sub(self.context.origin_ns) / 1000, "dur":end_ns.saturating_sub(self.started_ns) / 1000, "args":{"run_id":self.context.run_id,"span_id":self.span_id,
            "parent_span_id":if self.root { serde_json::Value::Null } else { self.context.parent_span_id.clone().into() }}
        });
        let file = dir.join(format!("span-{}.jsonl", self.span_id));
        let temporary = file.with_extension("tmp");
        if fs::write(&temporary, format!("{}\n", event)).is_err()
            || fs::rename(&temporary, &file).is_err()
        {
            eprintln!("traced-run: warning: trace record could not be published");
            let _ = fs::write(dir.join("incomplete"), b"a span record was not published\n");
            return;
        }
        let active = dir.join(format!("active-{}", self.span_id));
        if fs::remove_file(active).is_err() {
            let _ = fs::write(
                dir.join("incomplete"),
                b"participant registration could not be cleared\n",
            );
        }
        if self.root {
            self.publish_root(&dir);
        }
    }

    fn publish_root(&self, dir: &Path) {
        let mut events = Vec::new();
        if dir.join("incomplete").exists()
            || fs::read_dir(dir).is_ok_and(|entries| {
                entries
                    .flatten()
                    .any(|entry| entry.file_name().to_string_lossy().starts_with("active-"))
            })
        {
            eprintln!(
                "traced-run: warning: incomplete span records or unfinished participants; merged trace withheld"
            );
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            eprintln!("traced-run: warning: trace records unavailable");
            return;
        };
        let mut complete = true;
        for entry in entries {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "jsonl")
            {
                let Ok(bytes) = fs::read(entry.path()) else {
                    complete = false;
                    continue;
                };
                for line in bytes
                    .split(|byte| *byte == b'\n')
                    .filter(|line| !line.is_empty())
                {
                    match serde_json::from_slice::<serde_json::Value>(line) {
                        Ok(event) => events.push(event),
                        Err(_) => complete = false,
                    }
                }
            }
        }
        if !complete {
            eprintln!(
                "traced-run: warning: span records could not be fully read; merged trace withheld"
            );
            return;
        }
        let destination = dir.with_extension("json");
        let temporary = destination.with_extension("tmp");
        let document = serde_json::json!({"traceEvents":events,"displayTimeUnit":"ms"});
        if fs::write(&temporary, document.to_string()).is_err()
            || fs::rename(&temporary, &destination).is_err()
        {
            eprintln!("traced-run: warning: final trace could not be published");
        } else {
            prune_traces(dir.parent().unwrap_or(dir));
        }
    }
}

pub fn traced_cli(args: Vec<String>) -> ! {
    #[cfg(unix)]
    if args.first().is_some_and(|arg| arg == terminal::REENTRY) {
        terminal::exec(&args[1..]);
    }
    let mut args = args.into_iter();
    let mut name = None;
    let mut timeout = None;
    let mut launcher = false;
    let mut command = Vec::new();
    let mut separator = false;
    let mut usage_error = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "Usage: traced-run [--name LABEL] [--timeout SECONDS] [--launcher] -- PROGRAM [ARG ...]"
                );
                std::process::exit(0);
            }
            "--" => {
                command.extend(args);
                separator = true;
                break;
            }
            "--name" => match args.next() {
                Some(value) => name = Some(value),
                None => usage_error = Some("--name requires a label"),
            },
            "--timeout" => match args.next().and_then(|value| value.parse::<u64>().ok()) {
                Some(value) => timeout = Some(value),
                None => usage_error = Some("--timeout requires a whole number of seconds"),
            },
            "--launcher" => launcher = true,
            _ => {
                usage_error = Some("expected options followed by `-- PROGRAM [ARG ...]`");
                break;
            }
        }
    }
    if !separator || command.is_empty() {
        usage_error.get_or_insert("a program is required after `--`");
    }
    if launcher && name.is_none() {
        usage_error.get_or_insert("--launcher requires --name");
    }
    if let Some(error) = usage_error {
        if launcher {
            eprintln!("traced-run: {error}");
        } else {
            println!("{}", serde_json::json!({"error":error,"kind":"usage"}));
        }
        std::process::exit(64);
    }
    if let Some(label) = name {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        match TraceSpan::start(&label, &cwd) {
            Ok(span) => {
                let child_context = span.child_context();
                let serialized = serde_json::to_string(&child_context).unwrap_or_default();
                let (code, summary) = if launcher {
                    (execute_launcher(&command, &cwd, timeout, &serialized), None)
                } else {
                    match execute_with_trace(&command, &cwd, timeout, Some(&serialized)) {
                        Ok((result, code)) => (code, Some(result)),
                        Err(error) => {
                            println!(
                                "{}",
                                serde_json::json!({"error":error.message,"kind":error.kind})
                            );
                            (64, None)
                        }
                    }
                };
                span.finish();
                if let Some(summary) = summary {
                    println!("{}", serde_json::to_string_pretty(&summary).unwrap());
                }
                std::process::exit(code);
            }
            Err(error) => {
                let diagnostic = serde_json::json!({"error":error.message,"kind":error.kind});
                if launcher {
                    eprintln!("{diagnostic}");
                } else {
                    println!("{diagnostic}");
                }
                std::process::exit(64);
            }
        }
    }
    run(&command, None, timeout)
}

fn execute_launcher(argv: &[String], cwd: &Path, timeout: Option<u64>, trace_context: &str) -> i32 {
    if let Err(error) = validate_argv(argv) {
        eprintln!("traced-run: {}", error.message);
        return 64;
    }
    #[cfg(unix)]
    let group = match processkit::ProcessGroup::new() {
        Ok(group) => group,
        Err(error) => {
            eprintln!("traced-run: {error}");
            return 64;
        }
    };
    #[cfg(unix)]
    let foreground = terminal::Foreground::acquire(&group);
    #[cfg(unix)]
    let helper_argv = if let Some(foreground) = &foreground {
        let executable = match std::env::current_exe() {
            Ok(executable) => executable,
            Err(error) => {
                eprintln!("traced-run: {error}");
                return 64;
            }
        };
        let mut helper = vec![
            executable.into_os_string(),
            terminal::REENTRY.into(),
            foreground.group().into(),
        ];
        helper.extend(argv.iter().map(std::ffi::OsString::from));
        helper
    } else {
        argv.iter().map(std::ffi::OsString::from).collect()
    };
    #[cfg(unix)]
    let launch_argv = &helper_argv;
    #[cfg(not(unix))]
    let launch_argv = argv;
    let command = Command::new(&launch_argv[0])
        .args(&launch_argv[1..])
        .current_dir(cwd)
        .env(TRACE_ENV, trace_context)
        .inherit_stdin()
        .stdout(processkit::StdioMode::Inherit)
        .stderr(processkit::StdioMode::Inherit)
        .timeout_opt(timeout.map(Duration::from_secs));
    let is_cargo_xtask = Path::new(&argv[0])
        .file_name()
        .is_some_and(|name| name == "cargo-xtask");
    let command = if is_cargo_xtask {
        command.env(TRACE_GUARD, "1")
    } else {
        command
    };
    #[cfg(unix)]
    let result = wait_launcher(command, argv, group);
    #[cfg(not(unix))]
    let result = wait_launcher(command, argv);
    match result {
        Ok(outcome) if outcome.timed_out() => 124,
        Ok(outcome) => match (outcome.code(), outcome.signal()) {
            (Some(code), _) => code,
            (None, Some(sig)) => 128 + sig,
            _ => 1,
        },
        Err(error) => {
            eprintln!("traced-run: {}", error.message);
            64
        }
    }
}

/// Supervise launcher interruption without doing process work in a signal handler.
/// The explicit group owns the entire child tree, including children remaining
/// after the direct child exits. A second signal skips the two-second grace.
#[cfg(unix)]
fn wait_launcher(
    command: Command,
    argv: &[String],
    group: processkit::ProcessGroup,
) -> Result<Outcome, RunError> {
    use signal_hook::consts::{SIGINT, SIGTERM};
    let failure = |error: String| RunError {
        message: format!("supervising {argv:?}: {error}"),
        kind: "spawn",
    };
    // Register before spawning so an early interruption cannot bypass teardown.
    let mut signals = signal_hook::iterator::Signals::new([SIGINT, SIGTERM])
        .map_err(|error| failure(error.to_string()))?;
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .map_err(|error| failure(error.to_string()))?;
    let running = runtime
        .block_on(group.start(&command))
        .map_err(|error| failure(error.to_string()))?;
    let wait = runtime.spawn(running.wait());
    let mut deadline = None;
    while !wait.is_finished() {
        for signal in signals.pending() {
            if deadline.is_some() {
                group
                    .kill_all()
                    .map_err(|error| failure(error.to_string()))?;
            } else {
                group
                    .signal(processkit::Signal::Other(signal))
                    .map_err(|error| failure(error.to_string()))?;
                deadline = Some(Instant::now() + Duration::from_secs(2));
            }
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            group
                .kill_all()
                .map_err(|error| failure(error.to_string()))?;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let outcome = runtime
        .block_on(wait)
        .map_err(|error| failure(error.to_string()))?
        .map_err(|error| failure(error.to_string()));
    // Kill remaining descendants before releasing the runtime, even when the
    // direct child handled the forwarded signal and exited successfully.
    if let Err(error) = group.kill_all() {
        eprintln!("traced-run: warning: child-tree cleanup failed: {error}");
    }
    drop(group);
    outcome
}

#[cfg(not(unix))]
fn wait_launcher(command: Command, argv: &[String]) -> Result<Outcome, RunError> {
    Process::start(command, argv)?.wait(argv)
}

/// CLI entry: run the command, print the JSON result, and exit with the child's
/// code (or 64 for a runner-level failure).
pub fn run(argv: &[String], cwd: Option<PathBuf>, timeout: Option<u64>) -> ! {
    let cwd = cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    match execute(argv, &cwd, timeout) {
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

    // Keep the owner through every dependent assertion so captures share the
    // fixture's ordinary scope-exit and panic-unwind cleanup.
    fn tmp() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("devtool-run-test.")
            .tempdir()
            .unwrap()
    }

    #[test]
    fn true_succeeds_with_empty_streams() {
        let fixture_path = {
            let fixture = tmp();
            let cwd = fixture.path();
            let (r, code) = execute(&["true".into()], cwd, None).unwrap();
            assert_eq!(r.exit_code, Some(0));
            assert!(r.ok);
            assert_eq!(code, 0);
            assert_eq!(r.stdout.bytes, 0);
            assert_eq!(r.stderr.bytes, 0);
            assert!(cwd.is_dir());
            assert!(Path::new(&r.stdout.path).is_file());
            assert!(Path::new(&r.stderr.path).is_file());
            cwd.to_owned()
        };
        assert!(
            !fixture_path.exists(),
            "fixture must be removed at scope exit"
        );
    }

    #[test]
    fn false_fails_with_exit_one() {
        let fixture_path = {
            let fixture = tmp();
            let cwd = fixture.path();
            let (r, code) = execute(&["false".into()], cwd, None).unwrap();
            assert_eq!(r.exit_code, Some(1));
            assert!(!r.ok);
            assert_eq!(code, 1);
            assert!(cwd.is_dir());
            assert!(Path::new(&r.stdout.path).is_file());
            assert!(Path::new(&r.stderr.path).is_file());
            cwd.to_owned()
        };
        assert!(
            !fixture_path.exists(),
            "fixture must be removed at scope exit"
        );
    }

    #[test]
    fn fixture_is_removed_on_panic_unwind() {
        let mut fixture_path = None;
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // The owner must be created inside the unwinding scope.
            let fixture = tmp();
            let cwd = fixture.path();
            fixture_path = Some(cwd.to_owned());
            let (r, code) = execute(&["printf".into(), "captured\n".into()], cwd, None).unwrap();
            assert!(r.ok);
            assert_eq!(code, 0);
            assert!(cwd.is_dir());
            assert_eq!(fs::read_to_string(&r.stdout.path).unwrap(), "captured\n");
            assert!(Path::new(&r.stderr.path).is_file());
            std::panic::panic_any("fixture cleanup unwind probe");
        }))
        .expect_err("the cleanup probe must unwind");
        assert_eq!(
            panic.downcast_ref::<&str>(),
            Some(&"fixture cleanup unwind probe")
        );
        assert!(
            !fixture_path.unwrap().exists(),
            "unwinding must remove the fixture"
        );
    }

    #[cfg(unix)]
    #[test]
    fn signalled_child_reports_signal_and_shell_exit_code() {
        let fixture = tmp();
        let cwd = fixture.path();
        let executable = std::env::current_exe().unwrap();
        let argv = vec![
            executable.to_string_lossy().into_owned(),
            "--exact".into(),
            "tests::exit_via_sigterm".into(),
            "--ignored".into(),
        ];
        let (r, code) = execute(&argv, cwd, None).unwrap();
        assert_eq!(r.exit_code, None);
        assert!(!r.ok);
        assert_eq!(r.signal.as_deref(), Some("SIGTERM"));
        assert_eq!(r.timed_out, None);
        assert_eq!(code, 143);
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "subprocess fixture: terminates its test process with SIGTERM"]
    fn exit_via_sigterm() {
        signal_hook::low_level::raise(signal_hook::consts::SIGTERM).unwrap();
        unreachable!("SIGTERM should terminate the subprocess");
    }

    #[test]
    fn stdout_bytes_and_lines_counted() {
        let fixture = tmp();
        let cwd = fixture.path();
        let (r, _) = execute(&["printf".into(), "a\nb\n".into()], cwd, None).unwrap();
        assert_eq!(r.stdout.bytes, 4);
        assert_eq!(r.stdout.lines, 2);
        assert_eq!(r.stderr.bytes, 0);
    }

    #[test]
    fn stderr_captured_separately() {
        let fixture = tmp();
        let cwd = fixture.path();
        let (r, _) = execute(&["ls".into(), "/no/such/path/xyzzy".into()], cwd, None).unwrap();
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

    #[test]
    fn timeout_kills_and_reports() {
        let fixture = tmp();
        let cwd = fixture.path();
        let (r, code) = execute(&["sleep".into(), "10".into()], cwd, Some(1)).unwrap();
        assert_eq!(r.exit_code, None);
        assert!(!r.ok);
        #[cfg(unix)]
        assert_eq!(r.signal.as_deref(), Some("SIGKILL"));
        assert_eq!(r.timed_out, Some(true));
        assert_eq!(code, 124);
        assert!(Path::new(&r.stdout.path).exists());
        assert!(Path::new(&r.stderr.path).exists());
    }

    // Set mtime without a dependency: `File::set_modified` is stable since 1.75.
    fn set_mtime(p: &Path, t: std::time::SystemTime) {
        std::fs::OpenOptions::new()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn prune_keeps_newest_limit() {
        let fixture = tmp();
        let dir = fixture.path();
        for i in 0..(RUN_HISTORY_LIMIT + 10) {
            let p = dir.join(format!("{i:04}.out"));
            std::fs::write(&p, b"x").unwrap();
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(i as u64);
            set_mtime(&p, t);
        }
        prune(dir);
        let remaining = std::fs::read_dir(dir).unwrap().count();
        assert_eq!(remaining, RUN_HISTORY_LIMIT);
        assert!(!dir.join("0000.out").exists(), "oldest should be pruned");
        assert!(
            dir.join(format!("{:04}.out", RUN_HISTORY_LIMIT + 9))
                .exists(),
            "newest should remain"
        );
    }

    #[test]
    fn ancillary_history_warning_preserves_primary_code() {
        let fixture = tmp();
        let dir = fixture.path();
        for i in 0..=RUN_HISTORY_LIMIT {
            let path = dir.join(format!("{i:04}.out"));
            std::fs::write(&path, b"x").unwrap();
            set_mtime(
                &path,
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(i as u64),
            );
        }
        let primary_code = 124;
        let mut stderr = Vec::new();
        prune_with(
            dir,
            |_| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "sensitive",
                ))
            },
            &mut stderr,
        );
        assert_eq!(primary_code, 124);
        let warning = String::from_utf8(stderr).unwrap();
        assert_eq!(warning.matches("devtool.run.history_prune").count(), 1);
        assert_eq!(warning.lines().count(), 1);
        assert!(!warning.contains("sensitive"));
    }
    #[test]
    fn ancillary_warning_history_population_failures_warn_once_per_attempt() {
        let fixture = tmp();
        let missing = fixture.path().join("missing");
        let mut stderr = Vec::new();
        prune_with(
            &missing,
            |_| unreachable!("an unreadable population has no removal candidates"),
            &mut stderr,
        );
        let warning = String::from_utf8(stderr).unwrap();
        assert_eq!(warning.matches("devtool.run.history_prune").count(), 1);
        assert_eq!(warning.lines().count(), 1);

        for population in [
            vec![Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "sensitive directory entry",
            ))],
            vec![Ok((
                PathBuf::from("sensitive"),
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "sensitive metadata or modified time",
                )),
            ))],
        ] {
            let mut stderr = Vec::new();
            prune_entries_with(
                population,
                |_| unreachable!("failed population members are never removed"),
                &mut stderr,
            );
            let warning = String::from_utf8(stderr).unwrap();
            assert_eq!(warning.matches("devtool.run.history_prune").count(), 1);
            assert_eq!(warning.lines().count(), 1);
            assert!(!warning.contains("sensitive"));
        }
    }

    #[test]
    fn ancillary_warning_stream_summary_failures_preserve_fallback_and_primary_code() {
        for metadata_fails in [true, false] {
            let primary_code = 17;
            let mut stderr = Vec::new();
            let stream = stream_of_with(
                Path::new("/sensitive/output"),
                || {
                    if metadata_fails {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::PermissionDenied,
                            "sensitive metadata",
                        ))
                    } else {
                        Ok(7)
                    }
                },
                || {
                    if metadata_fails {
                        Ok(9)
                    } else {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "sensitive read",
                        ))
                    }
                },
                &mut stderr,
            );
            assert_eq!(primary_code, 17);
            assert_eq!(stream.bytes, if metadata_fails { 0 } else { 7 });
            assert_eq!(stream.lines, if metadata_fails { 9 } else { 0 });
            let warning = String::from_utf8(stderr).unwrap();
            assert_eq!(warning.matches("devtool.run.stream_summary").count(), 1);
            assert_eq!(warning.lines().count(), 1);
            assert!(!warning.contains("sensitive"));
        }
        let fixture = tmp();
        let missing = fixture.path().join("missing-output");
        let mut stderr = Vec::new();
        let stream = stream_of_with(
            &missing,
            || fs::metadata(&missing).map(|metadata| metadata.len()),
            || count_lines(&missing),
            &mut stderr,
        );
        assert_eq!((stream.bytes, stream.lines), (0, 0));
        let warning = String::from_utf8(stderr).unwrap();
        assert_eq!(warning.matches("devtool.run.stream_summary").count(), 1);
        assert_eq!(warning.lines().count(), 1);

        let unreadable_fixture = tmp();
        let unreadable = unreadable_fixture.path();
        let mut stderr = Vec::new();
        let stream = stream_of_with(
            unreadable,
            || fs::metadata(unreadable).map(|metadata| metadata.len()),
            || count_lines(unreadable),
            &mut stderr,
        );
        assert_eq!(stream.lines, 0);
        let warning = String::from_utf8(stderr).unwrap();
        assert_eq!(warning.matches("devtool.run.stream_summary").count(), 1);
        assert_eq!(warning.lines().count(), 1);
    }
}
