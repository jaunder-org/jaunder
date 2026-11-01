//! Bounded synchronous `test-local` host capture, scoped to one dispatcher.
//!
//! Only the closed `xtask_trace` target enters the adapter; arguments, environment,
//! paths, result details and subprocess output are never fields. Threaded B/E
//! events measure entered same-thread wall intervals, not child internals or CPU.
//! The in-memory writer permits preparation spans before checkout resolution.
//! Guard join precedes validation, timing projection, fallible persistence and all
//! existing result reporting. Capture failure is ancillary to the command verdict.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tracing_subscriber::{filter::filter_fn, prelude::*};

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);
impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("trace buffer poisoned"))?
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) struct Capture {
    buffer: Buffer,
    dispatch: tracing::Dispatch,
    guard: tracing_chrome::FlushGuard,
}
impl Default for Capture {
    fn default() -> Self {
        Self::new()
    }
}
impl Capture {
    pub fn new() -> Self {
        let buffer = Buffer::default();
        let (layer, guard) = tracing_chrome::ChromeLayerBuilder::new()
            .writer(buffer.clone())
            .trace_style(tracing_chrome::TraceStyle::Threaded)
            .include_args(true)
            .include_locations(false)
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(layer.with_filter(filter_fn(|metadata| metadata.target() == "xtask_trace")));
        Self {
            buffer,
            dispatch: tracing::Dispatch::new(subscriber),
            guard,
        }
    }
    pub fn with_default<T>(&self, f: impl FnOnce() -> T) -> T {
        tracing::dispatcher::with_default(&self.dispatch, f)
    }
    pub fn finish(self) -> Result<Trace, &'static str> {
        // Join the adapter writer before inspecting bytes or process::exit.
        drop(self.guard);
        let bytes = self.buffer.0.lock().map_err(|_| "buffer-poisoned")?.clone();
        let durations = intervals(&bytes)?;
        Ok(Trace { bytes, durations })
    }
}

/// Finalize capture and project timing before any result reporting or explicit exit.
pub fn execute(cli: crate::Cli) -> anyhow::Result<crate::CommandResult> {
    let capture = matches!(cli.command, crate::Command::TestLocal { .. }).then(Capture::new);
    let mut root = None;
    let invoke = || {
        let outer = capture.as_ref().map(|_| phase("xtask.command"));
        eprintln!("xtask-start: checkout-prepare");
        let checkout = capture.as_ref().map(|_| phase("xtask.checkout"));
        if capture.is_some() {
            root = std::env::current_dir()
                .ok()
                .and_then(|cwd| crate::git::toplevel(&cwd).ok())
                .map(PathBuf::from);
        }
        let prepared = crate::prepare_checkout();
        if let Some(checkout) = checkout {
            checkout.finish(prepared.is_ok());
        }
        let result = prepared.and_then(|()| {
            eprintln!("xtask-start: hook-setup");
            crate::ensure_hooks_installed();
            eprintln!("xtask-start: dispatch");
            let dispatch = capture.as_ref().map(|_| phase("xtask.dispatch"));
            let result = crate::run(cli);
            if let Some(dispatch) = dispatch {
                dispatch.finish(result.as_ref().is_ok_and(|result| result.ok));
            }
            result
        });
        if let Some(outer) = outer {
            let (ok, exit, override_code) = match &result {
                Ok(result) => (result.ok, result.exit_code(), result.exit_override),
                Err(_) => (false, 2, None),
            };
            command_outcome(ok, exit, override_code);
            outer.finish(ok);
        }
        result
    };
    let mut result = match &capture {
        Some(capture) => capture.with_default(invoke),
        None => invoke(),
    };
    if let Some(capture) = capture {
        let finalized = capture.finish().and_then(|trace| {
            if let Ok(result) = &mut result {
                trace.project(result);
            }
            trace.persist(root.as_deref().ok_or("checkout-unavailable")?)
        });
        match finalized {
            Ok(path) => eprintln!("xtask-trace: status=complete path={}", path.display()),
            Err(reason) => eprintln!("xtask-trace: status=unavailable reason={reason}"),
        }
    }
    result
}

pub(crate) fn active() -> bool {
    tracing::enabled!(target: "xtask_trace", tracing::Level::INFO)
}

/// An entered interval is synchronous and must stay on the entering thread.
pub(crate) struct Phase {
    entered: tracing::span::EnteredSpan,
    name: &'static str,
}
pub(crate) fn phase(name: &'static str) -> Phase {
    macro_rules! span { ($($name:literal),+) => { match name {
        $($name => tracing::info_span!(target: "xtask_trace", $name),)+
        _ => unreachable!("closed phase vocabulary"),
    } }; }
    let span = span!(
        "xtask.command",
        "xtask.checkout",
        "xtask.hooks",
        "xtask.dispatch",
        "xtask.artifact-lock-wait",
        "test-local-csr-bundle",
        "xtask.csr-admission",
        "xtask.csr-build",
        "test-local-csr-prerequisite",
        "test-local-image-worker",
        "test-local"
    );
    Phase {
        entered: span.entered(),
        name,
    }
}
impl Phase {
    pub fn finish(self, ok: bool) {
        tracing::event!(name: "xtask.phase.outcome", target: "xtask_trace", tracing::Level::INFO, phase = %self.name, outcome = %(if ok { "success" } else { "failure" }));
        drop(self.entered);
    }
}
pub(crate) fn command_outcome(ok: bool, exit_code: i32, exit_override: Option<i32>) {
    tracing::event!(name: "xtask.command.outcome", target: "xtask_trace", tracing::Level::INFO,
        command = %"test-local", outcome = %(if ok { "success" } else { "failure" }), exit_code, exit_override = ?exit_override);
}

pub(crate) struct Trace {
    pub bytes: Vec<u8>,
    pub durations: BTreeMap<String, u128>,
}
impl Trace {
    fn duration(&self, name: &str) -> u128 {
        match self.durations.get(name) {
            Some(duration) => *duration,
            None => {
                eprintln!("xtask-trace: warning=missing-interval phase={name}");
                0
            }
        }
    }
    pub fn project(&self, result: &mut crate::CommandResult) {
        result.duration_ms = self.duration("xtask.dispatch");
        for step in &mut result.steps {
            if matches!(
                step.name.as_str(),
                "test-local-csr-bundle"
                    | "test-local-csr-prerequisite"
                    | "test-local-image-worker"
                    | "test-local"
            ) {
                step.duration_ms = self.duration(&step.name);
            }
        }
    }
    pub fn persist(&self, root: &Path) -> Result<PathBuf, &'static str> {
        let parent = root.join(".xtask/traces");
        std::fs::create_dir_all(&parent).map_err(|_| "directory-create")?;
        // tempfile's random leaf prevents overwrite even for simultaneous runs.
        let directory = tempfile::Builder::new()
            .prefix("run-")
            .tempdir_in(parent)
            .map_err(|_| "directory-create")?
            .keep();
        let path = directory.join("trace.json");
        persist_file(&directory, &path, &self.bytes)?;
        Ok(path)
    }
}
fn persist_file(directory: &Path, path: &Path, bytes: &[u8]) -> Result<(), &'static str> {
    let mut file = tempfile::NamedTempFile::new_in(directory).map_err(|_| "file-create")?;
    write_trace(&mut file, bytes)?;
    file.persist_noclobber(path).map_err(|_| "file-rename")?;
    Ok(())
}
fn write_trace(writer: &mut impl Write, bytes: &[u8]) -> Result<(), &'static str> {
    writer.write_all(bytes).map_err(|_| "file-write")?;
    writer.flush().map_err(|_| "file-flush")
}

/// Chrome Threaded B/E events nest per (pid, tid), not by global event order.
fn intervals(bytes: &[u8]) -> Result<BTreeMap<String, u128>, &'static str> {
    let events: Vec<serde_json::Value> =
        serde_json::from_slice(bytes).map_err(|_| "malformed-json")?;
    let mut stacks: BTreeMap<(u64, u64), Vec<(String, f64)>> = BTreeMap::new();
    let mut durations = BTreeMap::new();
    let mut outcomes = BTreeSet::new();
    let mut terminal = 0;
    for event in events {
        let name = event["name"].as_str().unwrap_or("");
        match event["ph"].as_str() {
            Some("B" | "E") => {
                let track = (
                    event["pid"].as_u64().ok_or("invalid-track")?,
                    event["tid"].as_u64().ok_or("invalid-track")?,
                );
                let ts = event["ts"]
                    .as_f64()
                    .filter(|ts| ts.is_finite())
                    .ok_or("invalid-timestamp")?;
                let stack = stacks.entry(track).or_default();
                if event["ph"] == "B" {
                    stack.push((name.to_owned(), ts));
                } else {
                    let (begin, start) = stack.pop().ok_or("unmatched-interval")?;
                    if begin != name || ts < start {
                        return Err("unmatched-interval");
                    }
                    *durations.entry(begin).or_insert(0) += ((ts - start) / 1000.0).floor() as u128;
                }
            }
            Some("i") if name == "xtask.phase.outcome" => {
                if !matches!(
                    event["args"]["outcome"].as_str(),
                    Some("success" | "failure")
                ) {
                    return Err("missing-outcome");
                }
                outcomes.insert(
                    event["args"]["phase"]
                        .as_str()
                        .ok_or("missing-outcome")?
                        .to_owned(),
                );
            }
            Some("i") if name == "xtask.command.outcome" => {
                if event["args"]["command"] != "test-local"
                    || !matches!(
                        event["args"]["outcome"].as_str(),
                        Some("success" | "failure")
                    )
                    || event["args"]["exit_code"]
                        .as_str()
                        .and_then(|code| code.parse::<i32>().ok())
                        .is_none()
                    || !event["args"]["exit_override"].is_string()
                {
                    return Err("missing-outcome");
                }
                terminal += 1;
            }
            _ => {}
        }
    }
    if stacks.values().any(|stack| !stack.is_empty()) {
        return Err("unmatched-interval");
    }
    if terminal != 1
        || !durations.contains_key("xtask.command")
        || !durations.keys().all(|name| outcomes.contains(name))
        || !outcomes.iter().all(|name| durations.contains_key(name))
    {
        return Err("missing-outcome-or-interval");
    }
    Ok(durations)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separate_threads_have_independent_nested_tracks_and_no_payload_leaks() {
        let capture = Capture::new();
        capture.with_default(|| {
            let outer = phase("xtask.command");
            let dispatch = tracing::dispatcher::get_default(Clone::clone);
            let barrier = Arc::new(std::sync::Barrier::new(2));
            std::thread::scope(|scope| {
                for _ in 0..2 {
                    let dispatch = dispatch.clone();
                    let barrier = barrier.clone();
                    scope.spawn(move || {
                        tracing::dispatcher::with_default(&dispatch, || {
                            let wait = phase("xtask.artifact-lock-wait");
                            barrier.wait();
                            tracing::info!(target: "other", secret = "do-not-export");
                            wait.finish(true);
                        })
                    });
                }
            });
            command_outcome(false, 37, Some(37));
            outer.finish(false);
        });
        let trace = capture.finish().unwrap();
        let events: Vec<serde_json::Value> = serde_json::from_slice(&trace.bytes).unwrap();
        let tracks: BTreeSet<_> = events
            .iter()
            .filter(|event| event["ph"] == "B" && event["name"] == "xtask.artifact-lock-wait")
            .map(|event| event["tid"].as_u64().unwrap())
            .collect();
        assert_eq!(tracks.len(), 2);
        let outcome = events
            .iter()
            .find(|event| event["name"] == "xtask.command.outcome")
            .unwrap();
        assert_eq!(outcome["args"]["exit_code"], "37");
        assert_eq!(outcome["args"]["exit_override"], "Some(37)");
        let mut result = crate::CommandResult::new("test-local");
        result.ok = false;
        result.exit_override = Some(37);
        trace.project(&mut result);
        assert!(!result.ok);
        assert_eq!(result.exit_code(), 37);
        assert!(
            !String::from_utf8(trace.bytes)
                .unwrap()
                .contains("do-not-export")
        );
    }

    #[test]
    fn intervals_round_down_and_reject_missing_malformed_or_unbalanced_capture() {
        let events = serde_json::json!([
            {"ph":"B","pid":1,"tid":1,"name":"xtask.command","ts":0.25},
            {"ph":"B","pid":1,"tid":1,"name":"xtask.dispatch","ts":100.5},
            {"ph":"E","pid":1,"tid":1,"name":"xtask.dispatch","ts":2099.5},
            {"ph":"i","name":"xtask.phase.outcome","args":{"phase":"xtask.dispatch","outcome":"success"}},
            {"ph":"i","name":"xtask.command.outcome","args":{"command":"test-local","outcome":"success","exit_code":"0","exit_override":"None"}},
            {"ph":"i","name":"xtask.phase.outcome","args":{"phase":"xtask.command","outcome":"success"}},
            {"ph":"E","pid":1,"tid":1,"name":"xtask.command","ts":3100.25}
        ]);
        let bytes = serde_json::to_vec(&events).unwrap();
        let durations = intervals(&bytes).unwrap();
        assert_eq!(durations["xtask.dispatch"], 1);
        assert_eq!(durations["xtask.command"], 3);
        let trace = Trace { bytes, durations };
        let mut result = crate::CommandResult::new("test-local");
        result.push(crate::StepResult::ok("test-local"));
        trace.project(&mut result);
        assert_eq!(result.duration_ms, 1);
        assert_eq!(result.steps[0].duration_ms, 0);
        assert_eq!(intervals(b"not json"), Err("malformed-json"));
        assert_eq!(intervals(b"[]"), Err("missing-outcome-or-interval"));
        let mut missing_end = events.as_array().unwrap().clone();
        missing_end.pop();
        assert_eq!(
            intervals(&serde_json::to_vec(&missing_end).unwrap()),
            Err("unmatched-interval")
        );
        let mut malformed_outcome = events.clone();
        malformed_outcome[4]["args"] = serde_json::json!({});
        assert_eq!(
            intervals(&serde_json::to_vec(&malformed_outcome).unwrap()),
            Err("missing-outcome")
        );
        let mut missing_terminal = events.as_array().unwrap().clone();
        missing_terminal.remove(4);
        assert_eq!(
            intervals(&serde_json::to_vec(&missing_terminal).unwrap()),
            Err("missing-outcome-or-interval")
        );
    }

    #[test]
    fn persistence_retains_each_run_and_reports_io_failures() {
        struct Broken {
            flush: bool,
        }
        impl Write for Broken {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.flush {
                    Ok(bytes.len())
                } else {
                    Err(io::Error::other("write"))
                }
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::other("flush"))
            }
        }
        assert_eq!(
            write_trace(&mut Broken { flush: false }, b"[]"),
            Err("file-write")
        );
        assert_eq!(
            write_trace(&mut Broken { flush: true }, b"[]"),
            Err("file-flush")
        );
        let root = tempfile::tempdir().unwrap();
        let trace = Trace {
            bytes: b"[]".to_vec(),
            durations: BTreeMap::new(),
        };
        let first = trace.persist(root.path()).unwrap();
        let second = trace.persist(root.path()).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), b"[]");
        assert_eq!(
            persist_file(root.path(), &first, b"overwrite"),
            Err("file-rename")
        );
        assert_eq!(std::fs::read(&first).unwrap(), b"[]");
        assert_eq!(
            persist_file(&root.path().join("missing"), &first, b"[]"),
            Err("file-create")
        );
        let blocked = root.path().join("not-a-directory");
        std::fs::write(&blocked, "blocked").unwrap();
        assert_eq!(trace.persist(&blocked), Err("directory-create"));
    }

    #[test]
    fn finalized_capture_matches_nested_intervals_and_terminal_outcome() {
        let capture = Capture::new();
        capture.with_default(|| {
            let outer = phase("xtask.command");
            let inner = phase("xtask.dispatch");
            inner.finish(true);
            command_outcome(true, 0, None);
            outer.finish(true);
        });
        let trace = capture.finish().unwrap();
        assert!(trace.durations.contains_key("xtask.dispatch"));
        assert!(trace.durations["xtask.command"] >= trace.durations["xtask.dispatch"]);
        assert!(serde_json::from_slice::<Vec<serde_json::Value>>(&trace.bytes).is_ok());
    }
}
