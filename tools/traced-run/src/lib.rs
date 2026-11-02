//! Standalone command runner for `traced-run`.
//! It executes one program without a shell and parks output under `.xtask/run/`.
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
/// `traced-run *` stays narrower than `bash *`. Everything else runs.
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

/// Parse the standalone runner's deliberately small option surface.
#[derive(Clone, Serialize, Deserialize)]
struct TraceContext {
    run_id: String,
    trace_dir: String,
    origin_ns: u64,
    parent_span_id: String,
    parent_pid: u32,
    parent_started_ns: u64,
    #[serde(default)]
    parent_flow_id: Option<u64>,
    #[serde(default)]
    parent_name: Option<String>,
}

struct TraceSpan {
    name: String,
    span_id: String,
    flow_id: u64,
    context: TraceContext,
    root: bool,
    started_ns: u64,
}

static NEXT_SPAN: AtomicU64 = AtomicU64::new(1);
static NEXT_FLOW: AtomicU64 = AtomicU64::new(1);

// Mirror only the admitted fields of the command selected by its causal flow.
// The selector is syntax/length bounded, not guaranteed secret-free.
fn workload_metadata(
    command: &serde_json::Value,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let args = command.get("args")?.as_object()?;
    if args.get("subcommand")?.as_str()? != "test-local" {
        return None;
    }
    let package = args.get("package")?.as_str()?;
    if !matches!(package, "storage" | "jaunder" | "common" | "web") {
        return None;
    }
    let mut metadata = serde_json::Map::new();
    metadata.insert("subcommand".into(), "test-local".into());
    metadata.insert("package".into(), package.into());
    if let Some(selector) = args
        .get("test_selector")
        .and_then(serde_json::Value::as_str)
        && !selector.is_empty()
        && selector.len() <= 96
        && selector
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_:.-".contains(&byte))
    {
        metadata.insert("test_selector".into(), selector.into());
    }
    // Only terminal status fields are mirrored, never arbitrary event fields.
    for key in ["outcome", "exit_code", "exit_override"] {
        if let Some(value) = args.get(key).and_then(serde_json::Value::as_str)
            && match key {
                "outcome" => matches!(value, "success" | "failure" | "skipped"),
                "exit_code" => value.parse::<i32>().is_ok(),
                _ => valid_exit_override(value),
            }
        {
            metadata.insert(key.into(), value.into());
        }
    }
    Some(metadata)
}

fn valid_exit_override(value: &str) -> bool {
    value == "None"
        || value
            .strip_prefix("Some(")
            .and_then(|code| code.strip_suffix(')'))
            .is_some_and(|code| code.parse::<i32>().is_ok())
}

// Resolve outcomes before logical track folding: an entered B/E interval on
// the original (pid, tid) is the identity, not a globally searched phase name.
// Only completed intervals consume instants; unmatched/error events stay visible.
fn present_phase_outcomes(events: &mut Vec<serde_json::Value>) {
    let mut order: Vec<_> = (0..events.len())
        .filter(|index| events[*index]["cat"] == "xtask_trace")
        .collect();
    order.sort_by(|a, b| {
        events[*a]["ts"]
            .as_f64()
            .unwrap_or_default()
            .total_cmp(&events[*b]["ts"].as_f64().unwrap_or_default())
    });
    let mut stacks: std::collections::BTreeMap<(u64, u64), Vec<usize>> =
        std::collections::BTreeMap::new();
    let mut completed = vec![false; events.len()];
    let mut attachments = Vec::new();
    for index in order {
        let event = &events[index];
        let (Some(pid), Some(tid)) = (event["pid"].as_u64(), event["tid"].as_u64()) else {
            continue;
        };
        let stack = stacks.entry((pid, tid)).or_default();
        match event["ph"].as_str() {
            Some("B") => stack.push(index),
            Some("E") => {
                if let Some(begin) = stack.pop()
                    && events[begin]["name"] == event["name"]
                {
                    completed[begin] = true;
                }
            }
            Some("i") => {
                let Some(&begin) = stack.last() else { continue };
                let command = event["name"] == "xtask.command.outcome"
                    && events[begin]["name"] == "xtask.command";
                let phase = event["name"] == "xtask.phase.outcome"
                    && event["args"]["phase"] == events[begin]["name"];
                if !(command || phase)
                    || !matches!(
                        event["args"]["outcome"].as_str(),
                        Some("success" | "failure" | "skipped")
                    )
                {
                    continue;
                }
                let mut fields = serde_json::Map::new();
                fields.insert("outcome".into(), event["args"]["outcome"].clone());
                if command {
                    if let Some(code) = event["args"]["exit_code"].as_str()
                        && code.parse::<i32>().is_ok()
                    {
                        fields.insert("exit_code".into(), code.into());
                    }
                    if let Some(value) = event["args"]["exit_override"].as_str()
                        && valid_exit_override(value)
                    {
                        fields.insert("exit_override".into(), value.into());
                    }
                }
                attachments.push((index, begin, fields));
            }
            _ => {}
        }
    }
    let mut consumed = vec![false; events.len()];
    for (instant, begin, fields) in attachments {
        if completed[begin] {
            if !events[begin]["args"].is_object() {
                events[begin]["args"] = serde_json::json!({});
            }
            events[begin]["args"]
                .as_object_mut()
                .unwrap()
                .extend(fields);
            consumed[instant] = true;
        }
    }
    let mut index = 0;
    events.retain(|_| {
        let keep = !consumed[index];
        index += 1;
        keep
    });
}

struct MatchedCommand {
    root_id: String,
    root_pid: u64,
    execute_id: String,
    command_pid: u64,
    command_tid: u64,
    command_start: f64,
    command_end: f64,
    metadata: serde_json::Map<String, serde_json::Value>,
}

fn command_for_execute(
    events: &[serde_json::Value],
    root_id: &str,
    execute: &serde_json::Value,
) -> Option<MatchedCommand> {
    let execute_id = execute["args"]["span_id"].as_str()?;
    let root = events
        .iter()
        .find(|event| event["name"] == "cargo-xtask" && event["args"]["span_id"] == root_id)?;
    let root_pid = root["pid"].as_u64()?;
    let flow = events.iter().enumerate().find(|(index, event)| {
        event["name"] == "xtask-execute-to-command"
            && event["ph"] == "s"
            && event["args"]["parent_span_id"] == execute_id
            && index
                .checked_sub(1)
                .and_then(|prior| events.get(prior))
                .is_some_and(|marker| {
                    marker["name"] == "recorded child entry"
                        && marker["args"]["parent_span_id"] == execute_id
                        && marker["args"]["child_name"] == "xtask.command"
                })
    })?;
    let flow_id = flow.1["id"].as_str()?.to_owned();
    let finish = events
        .iter()
        .find(|event| event["ph"] == "f" && event["id"] == flow_id)?;
    let command_pid = finish["pid"].as_u64()?;
    let command_tid = finish["tid"].as_u64()?;
    let command_start = finish["ts"].as_f64()?;
    let begin_index = events.iter().position(|event| {
        event["name"] == "xtask.command"
            && event["ph"] == "B"
            && event["pid"].as_u64() == Some(command_pid)
            && event["tid"].as_u64() == Some(command_tid)
            && event["ts"].as_f64() == Some(command_start)
    })?;
    let begin = &events[begin_index];
    let mut depth = 0_u32;
    let command_end = events.iter().skip(begin_index).find_map(|event| {
        if event["name"] != "xtask.command"
            || event["pid"].as_u64() != Some(command_pid)
            || event["tid"].as_u64() != Some(command_tid)
        {
            return None;
        }
        match event["ph"].as_str()? {
            "B" => depth += 1,
            "E" => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return event["ts"].as_f64();
                }
            }
            _ => {}
        }
        None
    })?;
    Some(MatchedCommand {
        root_id: root_id.to_owned(),
        root_pid,
        execute_id: execute_id.to_owned(),
        command_pid,
        command_tid,
        command_start,
        command_end,
        metadata: workload_metadata(begin)?,
    })
}

fn present_xtask_workflow(events: &mut Vec<serde_json::Value>) {
    present_phase_outcomes(events);
    // Parent span IDs and the execute-to-command flow select the exact workflow;
    // timestamps are used only to bound that command's own same-thread phases.
    let roots: Vec<_> = events
        .iter()
        .filter(|event| event["name"] == "cargo-xtask")
        .filter_map(|event| {
            Some((
                event["args"]["span_id"].as_str()?.to_owned(),
                event["pid"].as_u64()?,
            ))
        })
        .collect();
    let mut matched = Vec::new();
    for (root_id, _) in &roots {
        let executes: Vec<_> = events
            .iter()
            .filter(|event| {
                event["name"] == "xtask-execute"
                    && event["args"]["parent_span_id"].as_str() == Some(root_id.as_str())
            })
            .cloned()
            .collect();
        for execute in executes {
            if let Some(command) = command_for_execute(events, root_id, &execute) {
                matched.push(command);
            }
        }
    }
    if matched.is_empty() {
        return;
    }

    let mut folded_spans = Vec::new();
    let mut folded_flows: Vec<(String, u64)> = Vec::new();
    let mut folded_processes = Vec::new();
    for workflow in &matched {
        let spans: Vec<_> = events
            .iter()
            .filter(|event| {
                event["args"]["parent_span_id"] == workflow.root_id
                    && (event["name"] == "xtask-acquire"
                        || (event["name"] == "xtask-execute"
                            && event["args"]["span_id"] == workflow.execute_id))
            })
            .filter_map(|event| event["args"]["span_id"].as_str().map(str::to_owned))
            .collect();
        folded_spans.extend(spans.iter().cloned().map(|span| (span, workflow.root_pid)));
        for (index, event) in events.iter().enumerate() {
            let folded_child = event["name"] == "recorded child entry"
                && event["args"]["parent_span_id"] == workflow.root_id
                && spans
                    .iter()
                    .any(|id| event["args"]["child_span_id"].as_str() == Some(id.as_str()));
            let command_entry = event["name"] == "recorded child entry"
                && event["args"]["parent_span_id"] == workflow.execute_id
                && event["args"]["child_name"] == "xtask.command";
            if (folded_child || command_entry)
                && let Some(id) = events
                    .get(index + 1)
                    .filter(|next| next["ph"] == "s")
                    .and_then(|next| next["id"].as_str())
            {
                folded_flows.push((id.to_owned(), workflow.root_pid));
            }
        }
        for span in spans {
            if let Some(event) = events
                .iter()
                .find(|event| event["ph"] == "X" && event["args"]["span_id"] == span)
                && let Some(pid) = event["pid"].as_u64()
            {
                folded_processes.push(pid);
            }
        }
        folded_processes.push(workflow.command_pid);
    }
    let folded_tracks: Vec<_> = events
        .iter()
        .filter(|event| event["ph"] == "X")
        .filter_map(|event| {
            let (_, root_pid) = folded_spans
                .iter()
                .find(|(id, _)| event["args"]["span_id"].as_str() == Some(id.as_str()))?;
            Some((event["pid"].as_u64()?, event["tid"].as_u64()?, *root_pid))
        })
        .collect();
    for event in events.iter_mut() {
        let direct_root = if event["ph"] == "X" {
            folded_spans
                .iter()
                .find(|(id, _)| event["args"]["span_id"].as_str() == Some(id.as_str()))
                .map(|(_, pid)| *pid)
        } else {
            None
        };
        let command_root = matched
            .iter()
            .find(|workflow| {
                event["cat"] == "xtask_trace"
                    && event["pid"].as_u64() == Some(workflow.command_pid)
                    && event["tid"].as_u64() == Some(workflow.command_tid)
                    && event["ts"].as_f64().is_some_and(|ts| {
                        ts >= workflow.command_start && ts <= workflow.command_end
                    })
            })
            .map(|workflow| workflow.root_pid);
        let flow_root = event["id"].as_str().and_then(|id| {
            folded_flows
                .iter()
                .find(|(flow, _)| flow == id)
                .map(|(_, pid)| *pid)
        });
        let endpoint_root = matches!(event["ph"].as_str(), Some("s" | "t" | "f"))
            .then(|| {
                folded_tracks
                    .iter()
                    .find(|(pid, tid, _)| {
                        event["pid"].as_u64() == Some(*pid) && event["tid"].as_u64() == Some(*tid)
                    })
                    .map(|(_, _, root)| *root)
            })
            .flatten();
        if let Some(root_pid) = direct_root.or(command_root).or(endpoint_root).or(flow_root) {
            event["pid"] = root_pid.into();
            event["tid"] = root_pid.into();
        }
    }
    // Grouping consumes the original causal edges above. Presentation needs no
    // arrows wholly within a folded logical lane, regardless of their names.
    // Keep complete cross-lane chains, including their intermediate endpoints.
    let redundant_flows: Vec<_> = events
        .iter()
        .filter(|event| event["ph"] == "s")
        .filter_map(|start| {
            let id = start["id"].as_str()?;
            let root = start["pid"].as_u64()?;
            if start["tid"].as_u64() != Some(root)
                || !matched.iter().any(|workflow| workflow.root_pid == root)
            {
                return None;
            }
            let endpoints: Vec<_> = events
                .iter()
                .filter(|event| {
                    event["id"].as_str() == Some(id)
                        && matches!(event["ph"].as_str(), Some("s" | "t" | "f"))
                })
                .collect();
            (endpoints.iter().any(|event| event["ph"] == "f")
                && endpoints.iter().all(|event| {
                    event["pid"].as_u64() == Some(root) && event["tid"].as_u64() == Some(root)
                }))
            .then(|| id.to_owned())
        })
        .collect();
    events.retain(|event| {
        let command_wrapper = matched.iter().any(|workflow| {
            event["name"] == "xtask.command"
                && event["pid"].as_u64() == Some(workflow.root_pid)
                && ((event["ph"] == "B" && event["ts"].as_f64() == Some(workflow.command_start))
                    || (event["ph"] == "E" && event["ts"].as_f64() == Some(workflow.command_end)))
        });
        let marker = event["name"] == "recorded child entry"
            && (matched.iter().any(|workflow| {
                event["args"]["parent_span_id"] == workflow.execute_id
                    && event["args"]["child_name"] == "xtask.command"
            }) || folded_spans
                .iter()
                .any(|(id, _)| event["args"]["child_span_id"].as_str() == Some(id.as_str())));
        !command_wrapper
            && !marker
            && !(matches!(event["ph"].as_str(), Some("s" | "t" | "f"))
                && event["id"]
                    .as_str()
                    .is_some_and(|id| redundant_flows.iter().any(|flow| flow == id)))
    });
    for workflow in &matched {
        let execute = events.iter_mut().find(|event| {
            event["name"] == "xtask-execute" && event["args"]["span_id"] == workflow.execute_id
        });
        if let Some(args) = execute.and_then(|event| event["args"].as_object_mut()) {
            args.extend(workflow.metadata.clone());
        }
        if let Some(root) = events.iter_mut().find(|event| {
            event["name"] == "cargo-xtask" && event["args"]["span_id"] == workflow.root_id
        }) && let Some(args) = root["args"].as_object_mut()
        {
            args.extend(workflow.metadata.clone());
        }
    }
    let unused_processes: Vec<_> = folded_processes
        .into_iter()
        .filter(|pid| {
            !events
                .iter()
                .any(|event| event["pid"].as_u64() == Some(*pid) && event["ph"] != "M")
        })
        .collect();
    events.retain(|event| {
        !(event["ph"] == "M"
            && unused_processes.contains(&event["pid"].as_u64().unwrap_or_default()))
    });
    let matched_roots: Vec<_> = matched.iter().map(|workflow| workflow.root_pid).collect();
    for root_pid in matched_roots {
        for event in events
            .iter_mut()
            .filter(|event| event["ph"] == "M" && event["pid"].as_u64() == Some(root_pid))
        {
            if event["name"] == "process_name" {
                event["args"]["name"] = "cargo-xtask workflow (logical)".into();
            } else if event["name"] == "thread_name" {
                event["args"]["name"] = "workflow lane (logical)".into();
            }
        }
    }
}

fn alloc_flow_id() -> u64 {
    (u64::from(std::process::id()) << 32)
        | (NEXT_FLOW.fetch_add(1, Ordering::Relaxed) & u64::from(u32::MAX))
}

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
                parent_pid: 0,
                parent_started_ns: 0,
                parent_flow_id: None,
                parent_name: None,
            }
        };
        let span_sequence = NEXT_SPAN.fetch_add(1, Ordering::Relaxed);
        let span_id = format!("{}-{span_sequence}", std::process::id());
        let flow_id = alloc_flow_id();
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
            flow_id,
            context,
            root,
            started_ns,
        })
    }

    fn child_context(&self) -> TraceContext {
        let mut context = self.context.clone();
        context.parent_span_id = self.span_id.clone();
        context.parent_pid = std::process::id();
        context.parent_started_ns = self.started_ns;
        context.parent_flow_id = Some(alloc_flow_id());
        context.parent_name = Some(self.name.clone());
        context
    }

    fn finish(self) {
        let dir = PathBuf::from(&self.context.trace_dir);
        let end_ns = monotonic_ns();
        let span_ts = self.started_ns.saturating_sub(self.context.origin_ns) / 1000;
        let span_dur = end_ns.saturating_sub(self.started_ns) / 1000;
        let pid = std::process::id();
        let mut events = vec![
            serde_json::json!({"name":"process_name","ph":"M","pid":pid,"args":{"name":"traced-run"}}),
            serde_json::json!({"name":"thread_name","ph":"M","pid":pid,"tid":pid,"args":{"name":"traced-run"}}),
        ];
        let span_event = serde_json::json!({
            "name": self.name, "cat":"process", "ph":"X", "pid":pid, "tid":pid,
            "ts":span_ts, "dur":span_dur, "args":{"runner":"traced-run","run_id":self.context.run_id,"span_id":self.span_id,
            "parent_span_id":if self.root { serde_json::Value::Null } else { self.context.parent_span_id.clone().into() }}
        });
        events.push(span_event);
        if !self.root {
            let flow_id = format!("{:016x}", self.flow_id);
            // Perfetto draws flows from the source slice's END, not the `s`
            // event timestamp. Bind to a thread instant inside the parent so
            // both visible anchors express recorded child participation entry.
            // This projects the child's observation; it is not an OS spawn time
            // or a separately measured parent action. Keep the instant before
            // `s` at the same timestamp so the importer binds to that instant.
            events.push(serde_json::json!({
                "name":"recorded child entry", "cat":"process", "ph":"i", "s":"t",
                "pid":self.context.parent_pid, "tid":self.context.parent_pid,
                "ts":span_ts, "args":{"run_id":self.context.run_id,
                "parent_span_id":self.context.parent_span_id, "child_span_id":self.span_id}
            }));
            events.push(serde_json::json!({
                "name":"traced-run parent-child", "cat":"process", "ph":"s",
                "pid":self.context.parent_pid, "tid":self.context.parent_pid,
                "ts":span_ts, "id":flow_id
            }));
            events.push(serde_json::json!({
                "name":"traced-run parent-child", "cat":"process", "ph":"f",
                "pid":std::process::id(), "tid":std::process::id(),
                "ts":span_ts, "id":flow_id, "bp":"e"
            }));
        }
        let file = dir.join(format!("span-{}.jsonl", self.span_id));
        let temporary = file.with_extension("tmp");
        let records = events
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        if fs::write(&temporary, format!("{records}\n")).is_err()
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
        present_xtask_workflow(&mut events);
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
    fn phase_outcomes_use_active_completed_slice_on_the_original_thread() {
        for outcome in ["success", "failure", "skipped"] {
            let mut events = vec![
                serde_json::json!({"name":"phase","cat":"xtask_trace","ph":"B","pid":1,"tid":1,"ts":0,"args":{}}),
                serde_json::json!({"name":"phase","cat":"xtask_trace","ph":"B","pid":2,"tid":1,"ts":0,"args":{}}),
                serde_json::json!({"name":"xtask.phase.outcome","cat":"xtask_trace","ph":"i","pid":1,"tid":1,"ts":1,"args":{"phase":"phase","outcome":outcome,"diagnostic":"not admitted"}}),
                serde_json::json!({"name":"xtask.phase.outcome","cat":"xtask_trace","ph":"i","pid":2,"tid":1,"ts":1,"args":{"phase":"other","outcome":"failure"}}),
                serde_json::json!({"name":"error","cat":"xtask_trace","ph":"i","pid":1,"tid":1,"ts":2,"args":{"error":"meaningful"}}),
                serde_json::json!({"name":"phase","cat":"xtask_trace","ph":"E","pid":1,"tid":1,"ts":3}),
                serde_json::json!({"name":"phase","cat":"xtask_trace","ph":"E","pid":2,"tid":1,"ts":3}),
                serde_json::json!({"name":"xtask.phase.outcome","cat":"xtask_trace","ph":"i","pid":1,"tid":1,"ts":4,"args":{"phase":"phase","outcome":"failure"}}),
            ];
            let original = events.clone();
            present_xtask_workflow(&mut events);
            assert_eq!(events.len(), original.len() - 1);
            assert_eq!(events[0]["args"], serde_json::json!({"outcome":outcome}));
            assert_eq!(
                events[1], original[1],
                "another process is not the same phase"
            );
            for index in [3, 4, 5, 6, 7] {
                assert!(
                    events.contains(&original[index]),
                    "unmatched/error events and measured ends survive"
                );
            }
        }
    }

    #[test]
    fn phase_outcomes_preserve_incomplete_and_nested_thread_identity() {
        let mut events = vec![
            serde_json::json!({"name":"phase","cat":"xtask_trace","ph":"B","pid":1,"tid":1,"ts":0}),
            serde_json::json!({"name":"phase","cat":"xtask_trace","ph":"B","pid":1,"tid":2,"ts":0}),
            serde_json::json!({"name":"phase","cat":"xtask_trace","ph":"B","pid":1,"tid":1,"ts":1}),
            serde_json::json!({"name":"xtask.phase.outcome","cat":"xtask_trace","ph":"i","pid":1,"tid":1,"ts":2,"args":{"phase":"phase","outcome":"failure"}}),
            serde_json::json!({"name":"phase","cat":"xtask_trace","ph":"E","pid":1,"tid":1,"ts":3}),
            serde_json::json!({"name":"xtask.phase.outcome","cat":"xtask_trace","ph":"i","pid":1,"tid":1,"ts":4,"args":{"phase":"phase","outcome":"success"}}),
            serde_json::json!({"name":"xtask.phase.outcome","cat":"xtask_trace","ph":"i","pid":1,"tid":2,"ts":4,"args":{"phase":"phase","outcome":"failure"}}),
            serde_json::json!({"name":"phase","cat":"xtask_trace","ph":"E","pid":1,"tid":1,"ts":5}),
        ];
        let incomplete = events[6].clone();
        present_xtask_workflow(&mut events);
        assert_eq!(events[0]["args"]["outcome"], "success");
        assert_eq!(events[2]["args"]["outcome"], "failure");
        assert!(events[1]["args"].is_null(), "other thread is incomplete");
        assert!(events.contains(&incomplete));
    }

    #[test]
    fn workflow_uses_causal_command_match_and_leaves_unrelated_capture_separate() {
        let mut events = vec![
            serde_json::json!({"name":"process_name","ph":"M","pid":10,"args":{"name":"traced-run"}}),
            serde_json::json!({"name":"thread_name","ph":"M","pid":10,"tid":10,"args":{"name":"traced-run"}}),
            serde_json::json!({"name":"process_name","ph":"M","pid":11,"args":{"name":"acquire-runner"}}),
            serde_json::json!({"name":"process_name","ph":"M","pid":12,"args":{"name":"execute-runner"}}),
            serde_json::json!({"name":"process_name","ph":"M","pid":14,"args":{"name":"unmatched-execute"}}),
            serde_json::json!({"name":"process_name","ph":"M","pid":20,"args":{"name":"unmatched-command"}}),
            serde_json::json!({"name":"process_name","ph":"M","pid":30,"args":{"name":"matched-command"}}),
            serde_json::json!({"name":"cargo-xtask","ph":"X","pid":10,"tid":10,"ts":0,"dur":100,"args":{"span_id":"root","parent_span_id":null}}),
            serde_json::json!({"name":"xtask-acquire","ph":"X","pid":11,"tid":11,"ts":10,"dur":20,"args":{"span_id":"acquire","parent_span_id":"root"}}),
            serde_json::json!({"name":"xtask-execute","ph":"X","pid":12,"tid":12,"ts":30,"dur":60,"args":{"span_id":"execute","parent_span_id":"root"}}),
            serde_json::json!({"name":"xtask-execute","ph":"X","pid":14,"tid":14,"ts":30,"dur":60,"args":{"span_id":"execute-unmatched","parent_span_id":"root"}}),
            // Deliberately enumerate the unrelated command first. It has no flow from this execute.
            serde_json::json!({"name":"xtask.command","cat":"xtask_trace","ph":"B","pid":20,"tid":20,"ts":31,"args":{"subcommand":"test-local","package":"jaunder","test_selector":"unmatched"}}),
            serde_json::json!({"name":"xtask.command","cat":"xtask_trace","ph":"E","pid":20,"tid":20,"ts":90}),
            serde_json::json!({"name":"xtask.command","cat":"xtask_trace","ph":"B","pid":30,"tid":30,"ts":51,"args":{"subcommand":"test-local","package":"storage","test_selector":"selected"}}),
            serde_json::json!({"name":"xtask.command","cat":"xtask_trace","ph":"E","pid":30,"tid":30,"ts":79}),
            serde_json::json!({"name":"xtask.checkout","cat":"xtask_trace","ph":"B","pid":30,"tid":30,"ts":52,"args":{}}),
            serde_json::json!({"name":"xtask.checkout","cat":"xtask_trace","ph":"E","pid":30,"tid":30,"ts":55}),
            serde_json::json!({"name":"xtask.command.outcome","cat":"xtask_trace","ph":"i","pid":30,"tid":30,"ts":78,"args":{"exit_code":"17","outcome":"failure"}}),
            serde_json::json!({"name":"parallel-work","cat":"xtask_trace","ph":"B","pid":30,"tid":99,"ts":53}),
            serde_json::json!({"name":"recorded child entry","ph":"i","pid":10,"tid":10,"ts":10,"args":{"parent_span_id":"root","child_span_id":"acquire"}}),
            serde_json::json!({"name":"root-acquire","ph":"s","pid":10,"tid":10,"ts":10,"id":"ra"}),
            serde_json::json!({"name":"root-acquire","ph":"f","pid":11,"tid":11,"ts":10,"id":"ra"}),
            serde_json::json!({"name":"recorded child entry","ph":"i","pid":10,"tid":10,"ts":30,"args":{"parent_span_id":"root","child_span_id":"execute"}}),
            serde_json::json!({"name":"root-execute","ph":"s","pid":10,"tid":10,"ts":30,"id":"re"}),
            serde_json::json!({"name":"root-execute","ph":"f","pid":12,"tid":12,"ts":30,"id":"re"}),
            serde_json::json!({"name":"recorded child entry","ph":"i","pid":12,"tid":12,"ts":51,"args":{"parent_span_id":"execute","child_name":"xtask.command"}}),
            serde_json::json!({"name":"xtask-execute-to-command","ph":"s","pid":12,"tid":12,"ts":51,"id":"ec","args":{"parent_span_id":"execute"}}),
            serde_json::json!({"name":"xtask-execute-to-command","ph":"f","pid":30,"tid":30,"ts":51,"id":"ec"}),
        ];
        // Additional causal edges are not necessarily the named entry flows.
        // Once folded, these endpoints share the hierarchy; concurrent work does not.
        events.extend([
            serde_json::json!({"name":"runner-link","cat":"process","ph":"s","pid":11,"tid":11,"ts":20,"id":"runner-same"}),
            serde_json::json!({"name":"runner-link","cat":"process","ph":"f","pid":12,"tid":12,"ts":30,"id":"runner-same"}),
            serde_json::json!({"name":"phase-link","cat":"xtask_trace","ph":"s","pid":30,"tid":30,"ts":52,"id":"same"}),
            serde_json::json!({"name":"phase-link","cat":"xtask_trace","ph":"t","pid":30,"tid":30,"ts":53,"id":"same"}),
            serde_json::json!({"name":"phase-link","cat":"xtask_trace","ph":"f","pid":30,"tid":30,"ts":55,"id":"same"}),
            serde_json::json!({"name":"concurrent-link","cat":"xtask_trace","ph":"s","pid":30,"tid":30,"ts":53,"id":"cross"}),
            serde_json::json!({"name":"concurrent-link","cat":"xtask_trace","ph":"f","pid":30,"tid":99,"ts":54,"id":"cross"}),
        ]);
        let mut success = events.clone();
        let terminal = success
            .iter_mut()
            .find(|event| event["name"] == "xtask.command.outcome")
            .unwrap();
        terminal["args"]["outcome"] = "success".into();
        terminal["args"]["exit_code"] = "0".into();
        present_xtask_workflow(&mut success);
        for name in ["cargo-xtask", "xtask-execute"] {
            let selected = success.iter().find(|event| event["name"] == name).unwrap();
            assert_eq!(selected["args"]["outcome"], "success");
            assert_eq!(selected["args"]["exit_code"], "0");
        }
        present_xtask_workflow(&mut events);
        let flows: Vec<_> = events
            .iter()
            .filter(|event| matches!(event["ph"].as_str(), Some("s" | "t" | "f")))
            .collect();
        assert_eq!(flows.len(), 2, "only the cross-lane flow remains");
        assert!(flows.iter().all(|event| event["id"] == "cross"));
        assert_eq!(
            (flows[0]["pid"].as_u64(), flows[0]["tid"].as_u64()),
            (Some(10), Some(10))
        );
        assert_eq!(
            (flows[1]["pid"].as_u64(), flows[1]["tid"].as_u64()),
            (Some(30), Some(99))
        );

        let slices: Vec<_> = events.iter().filter(|event| event["ph"] == "X").collect();
        assert_eq!(slices.len(), 4);
        for (name, ts, dur) in [
            ("cargo-xtask", 0, 100),
            ("xtask-acquire", 10, 20),
            ("xtask-execute", 30, 60),
        ] {
            let slice = slices
                .iter()
                .find(|event| {
                    event["name"] == name && event["args"]["span_id"] != "execute-unmatched"
                })
                .unwrap();
            assert_eq!(slice["ts"], ts);
            assert_eq!(slice["dur"], dur);
        }
        assert!(
            slices
                .iter()
                .filter(|event| event["args"]["span_id"] != "execute-unmatched")
                .all(|event| event["pid"] == 10 && event["tid"] == 10)
        );
        assert!(
            slices
                .iter()
                .any(|event| event["args"]["span_id"] == "execute-unmatched" && event["pid"] == 14)
        );
        assert_eq!(
            slices
                .iter()
                .filter(|event| event["args"]["span_id"] != "execute-unmatched")
                .map(|event| event["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["cargo-xtask", "xtask-acquire", "xtask-execute"]
        );
        let root = events
            .iter()
            .find(|event| event["name"] == "cargo-xtask")
            .unwrap();
        assert_eq!(root["args"]["test_selector"], "selected");
        let execute = events
            .iter()
            .find(|event| event["name"] == "xtask-execute")
            .unwrap();
        assert_eq!(execute["args"]["package"], "storage");
        assert!(
            !events
                .iter()
                .any(|event| event["name"] == "xtask.command" && event["pid"] == 10),
            "matched command wrapper is omitted"
        );
        assert!(
            events
                .iter()
                .any(|event| event["name"] == "xtask.command" && event["pid"] == 20),
            "unmatched command remains"
        );
        assert!(
            events
                .iter()
                .any(|event| event["name"] == "xtask.checkout" && event["pid"] == 10),
            "measured phase is folded, not removed"
        );
        for selected in [root, execute] {
            assert_eq!(selected["args"]["outcome"], "failure");
            assert_eq!(selected["args"]["exit_code"], "17");
        }
        assert!(
            !events
                .iter()
                .any(|event| event["name"] == "xtask.command.outcome")
        );
        let concurrent = events
            .iter()
            .find(|event| event["name"] == "parallel-work")
            .unwrap();
        assert_eq!(
            (concurrent["pid"].as_u64(), concurrent["tid"].as_u64()),
            (Some(30), Some(99))
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event["id"].as_str(), Some("ra" | "re" | "ec")))
        );
        assert!(
            !events
                .iter()
                .any(|event| event["name"] == "recorded child entry")
        );
        assert!(
            events
                .iter()
                .any(|event| event["name"] == "process_name" && event["pid"] == 20)
        );
        assert!(
            events
                .iter()
                .any(|event| event["name"] == "process_name" && event["pid"] == 14)
        );
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
