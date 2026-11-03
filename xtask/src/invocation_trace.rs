//! Bounded synchronous `test-local` host capture, scoped to one dispatcher.
//!
//! Only the closed `xtask_trace` target enters the adapter; arguments, environment,
//! paths, result details and subprocess output are never fields. B/E events use the
//! host monotonic clock shared with traced-run, measure entered same-thread wall
//! intervals, and are emitted as a runner fragment when a runner context exists.
//! The in-memory buffer permits preparation spans before checkout resolution.
//! Capture failure is ancillary to the command verdict.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tracing_subscriber::{filter::filter_fn, prelude::*};

#[derive(Clone, Default)]
struct Buffer {
    events: Arc<Mutex<Vec<serde_json::Value>>>,
    tracks: Arc<Mutex<BTreeSet<(u32, u64)>>>,
}
impl Buffer {
    fn push(&self, event: serde_json::Value) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
    }
    fn track(&self, pid: u32, tid: u64) {
        let Ok(mut tracks) = self.tracks.lock() else {
            return;
        };
        if !tracks.insert((pid, tid)) {
            return;
        }
        drop(tracks);
        self.push(
            serde_json::json!({"name":"process_name","ph":"M","pid":pid,"args":{"name":"xtask"}}),
        );
        self.push(serde_json::json!({"name":"thread_name","ph":"M","pid":pid,"tid":tid,"args":{"name":"xtask"}}));
    }
    fn bytes(&self) -> Result<Vec<u8>, &'static str> {
        serde_json::to_vec(&*self.events.lock().map_err(|_| "buffer-poisoned")?)
            .map_err(|_| "malformed-json")
    }
}

fn monotonic_ns() -> u64 {
    let time = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    (time.tv_sec as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(time.tv_nsec as u64)
}
// Trace only the parsed test-local identity, never the raw argv. Selector syntax
// is bounded for display but is not guaranteed secret-free; callers should use
// ordinary test identifiers rather than putting sensitive text in selectors.
fn safe_test_local_args(args: &[String]) -> serde_json::Map<String, serde_json::Value> {
    let mut fields = serde_json::Map::new();
    fields.insert("subcommand".into(), "test-local".into());
    let package = match args {
        [flag, package, selector] if flag == "-p" || flag == "--package" => {
            Some((package.as_str(), Some(selector.as_str())))
        }
        [flag, package] if flag == "-p" || flag == "--package" => Some((package.as_str(), None)),
        _ => None,
    };
    if let Some((package, selector)) = package
        && matches!(package, "storage" | "jaunder" | "common" | "web")
    {
        fields.insert("package".into(), package.into());
        if let Some(selector) = selector.filter(|value| {
            !value.is_empty()
                && value.len() <= 96
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_:.-".contains(&byte))
        }) {
            fields.insert("test_selector".into(), selector.into());
        }
    }
    fields
}

fn thread_id() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::thread::current().id().hash(&mut hasher);
    hasher.finish()
}

struct NativeTraceLayer {
    buffer: Buffer,
    origin_ns: u64,
    command_args: serde_json::Map<String, serde_json::Value>,
}
impl<S> tracing_subscriber::Layer<S> for NativeTraceLayer
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_enter(&self, id: &tracing::span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let pid = std::process::id();
            let tid = thread_id();
            self.buffer.track(pid, tid);
            let name = span.metadata().name();
            let args = if name == "xtask.command" {
                self.command_args.clone()
            } else {
                serde_json::Map::new()
            };
            self.buffer.push(serde_json::json!({"name":name,"cat":"xtask_trace","ph":"B","pid":pid,"tid":tid,"ts":monotonic_ns().saturating_sub(self.origin_ns) as f64 / 1000.0,"args":args}));
        }
    }
    fn on_exit(&self, id: &tracing::span::Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            self.buffer.push(serde_json::json!({"name":span.metadata().name(),"cat":"xtask_trace","ph":"E","pid":std::process::id(),"tid":thread_id(),"ts":monotonic_ns().saturating_sub(self.origin_ns) as f64 / 1000.0}));
        }
    }
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let name = event.metadata().name();
        if !matches!(name, "xtask.phase.outcome" | "xtask.command.outcome") {
            return;
        }
        let mut args = EventFields::default();
        event.record(&mut args);
        self.buffer.push(serde_json::json!({"name":name,"cat":"xtask_trace","ph":"i","s":"t","pid":std::process::id(),"tid":thread_id(),"ts":monotonic_ns().saturating_sub(self.origin_ns) as f64 / 1000.0,"args":args.0}));
    }
}
#[derive(Default)]
struct EventFields(serde_json::Map<String, serde_json::Value>);
impl tracing::field::Visit for EventFields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let value = format!("{value:?}");
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(&value);
        self.0.insert(field.name().into(), value.into());
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.0.insert(field.name().into(), value.to_string().into());
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.0.insert(field.name().into(), value.to_string().into());
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.0.insert(field.name().into(), value.into());
    }
}

pub(crate) struct Capture {
    buffer: Buffer,
    dispatch: tracing::Dispatch,
    origin_ns: u64,
}
impl Default for Capture {
    fn default() -> Self {
        Self::new()
    }
}
impl Capture {
    pub fn new() -> Self {
        Self::with_command_args(serde_json::Map::new())
    }
    fn with_command_args(command_args: serde_json::Map<String, serde_json::Value>) -> Self {
        let buffer = Buffer::default();
        let origin_ns = monotonic_ns();
        let layer = NativeTraceLayer {
            buffer: buffer.clone(),
            origin_ns,
            command_args,
        };
        let subscriber = tracing_subscriber::registry()
            .with(layer.with_filter(filter_fn(|metadata| metadata.target() == "xtask_trace")));
        Self {
            buffer,
            dispatch: tracing::Dispatch::new(subscriber),
            origin_ns,
        }
    }
    pub fn with_default<T>(&self, f: impl FnOnce() -> T) -> T {
        tracing::dispatcher::with_default(&self.dispatch, f)
    }
    pub fn finish(self) -> Result<Trace, &'static str> {
        let bytes = self.buffer.bytes()?;
        let durations = intervals(&bytes)?;
        Ok(Trace {
            bytes,
            durations,
            origin_ns: self.origin_ns,
        })
    }
}

/// Finalize capture and project timing before any result reporting or explicit exit.
pub fn execute(cli: crate::Cli) -> anyhow::Result<crate::CommandResult> {
    let command_args = match &cli.command {
        crate::Command::TestLocal { nextest_args } => safe_test_local_args(nextest_args),
        _ => serde_json::Map::new(),
    };
    let capture = matches!(&cli.command, crate::Command::TestLocal { .. })
        .then(|| Capture::with_command_args(command_args));
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
            Ok((path, contributed)) => {
                let status = if contributed {
                    "contributed"
                } else {
                    "complete"
                };
                eprintln!("xtask-trace: status={status} path={}", path.display());
            }
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
        "xtask.csr-manifest-check",
        "xtask.csr-inventory-check",
        "xtask.csr-shell-check",
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
    origin_ns: u64,
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
    pub fn persist(&self, root: &Path) -> Result<(PathBuf, bool), &'static str> {
        match self.persist_runner_fragment_from_env() {
            Ok(Some(path)) => return Ok((path, true)),
            Ok(None) => {}
            Err(reason) => {
                eprintln!("xtask-trace: warning=runner-fragment-unavailable reason={reason}")
            }
        }
        self.persist_standalone(root).map(|path| (path, false))
    }
    fn persist_standalone(&self, root: &Path) -> Result<PathBuf, &'static str> {
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
    fn persist_runner_fragment_from_env(&self) -> Result<Option<PathBuf>, &'static str> {
        let Ok(context) = std::env::var("JAUNDER_TRACED_RUN_CONTEXT") else {
            return Ok(None);
        };
        let context: serde_json::Value =
            serde_json::from_str(&context).map_err(|_| "runner-context")?;
        self.persist_runner_fragment(&context).map(Some)
    }
    fn persist_runner_fragment(
        &self,
        context: &serde_json::Value,
    ) -> Result<PathBuf, &'static str> {
        let Some(directory) = context["trace_dir"].as_str() else {
            return Err("runner-context");
        };
        let Some(origin) = context["origin_ns"].as_u64() else {
            return Err("runner-context");
        };
        let offset_us = self.origin_ns.saturating_sub(origin) as f64 / 1000.0;
        let mut events: Vec<serde_json::Value> =
            serde_json::from_slice(&self.bytes).map_err(|_| "malformed-json")?;
        let root_ts = events
            .iter()
            .find(|event| event["name"] == "xtask.command" && event["ph"] == "B")
            .and_then(|event| event["ts"].as_f64())
            .ok_or("missing-command-root")?;
        for event in &mut events {
            if event["ph"] == "M" {
                continue;
            }
            let Some(ts) = event["ts"].as_f64() else {
                return Err("invalid-timestamp");
            };
            event["ts"] = (ts + offset_us).into();
        }
        if context["parent_name"] == "xtask-execute" {
            let flow_id = context["parent_flow_id"].as_u64().ok_or("runner-context")?;
            let parent_pid = context["parent_pid"].as_u64().ok_or("runner-context")?;
            let timestamp = root_ts + offset_us;
            let flow_id = format!("{flow_id:016x}");
            events.push(serde_json::json!({
                "name":"recorded child entry", "cat":"xtask_trace", "ph":"i", "s":"t",
                "pid":parent_pid, "tid":parent_pid, "ts":timestamp,
                "args":{"run_id":context["run_id"], "parent_span_id":context["parent_span_id"], "child_name":"xtask.command"}
            }));
            events.push(serde_json::json!({
                "name":"xtask-execute-to-command", "cat":"xtask_trace", "ph":"s",
                "pid":parent_pid, "tid":parent_pid, "ts":timestamp, "id":flow_id,
                "args":{"parent_span_id":context["parent_span_id"]}
            }));
            events.push(serde_json::json!({
                "name":"xtask-execute-to-command", "cat":"xtask_trace", "ph":"f",
                "pid":std::process::id(), "tid":thread_id(), "ts":timestamp, "id":flow_id,
                "bp":"e"
            }));
        }
        let path = Path::new(directory).join(format!("fragment-{}.jsonl", std::process::id()));
        let body = events
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(path, format!("{body}\n")).map_err(|_| "fragment-write")?;
        Ok(Path::new(directory).with_extension("json"))
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
    fn command_metadata_is_allowlisted_and_selector_is_bounded() {
        let fields = safe_test_local_args(&[
            "-p".into(),
            "storage".into(),
            "site_config_primitives_round_trip".into(),
        ]);
        assert_eq!(fields["subcommand"], "test-local");
        assert_eq!(fields["package"], "storage");
        assert_eq!(fields["test_selector"], "site_config_primitives_round_trip");
        assert!(
            safe_test_local_args(&["-p".into(), "unknown".into(), "token-secret".into()])
                .get("package")
                .is_none()
        );
        assert!(
            safe_test_local_args(&["-p".into(), "storage".into(), "secret/value".into()])
                .get("test_selector")
                .is_none()
        );
    }

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
        let trace = Trace {
            bytes,
            durations,
            origin_ns: monotonic_ns(),
        };
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
    fn runner_fragment_uses_the_runner_monotonic_epoch() {
        let directory = tempfile::tempdir().unwrap();
        let root_origin = monotonic_ns().saturating_sub(2_000_000);
        let capture = Capture::new();
        capture.with_default(|| {
            let outer = phase("xtask.command");
            let dispatch = phase("xtask.dispatch");
            dispatch.finish(true);
            command_outcome(true, 0, None);
            outer.finish(true);
        });
        let trace = capture.finish().unwrap();
        trace
            .persist_runner_fragment(&serde_json::json!({
                "trace_dir": directory.path(), "origin_ns": root_origin
            }))
            .unwrap();
        let fragment = std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let events: Vec<serde_json::Value> = std::fs::read_to_string(fragment)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(
            events
                .iter()
                .any(|event| event["name"] == "thread_name" && event["args"]["name"] == "xtask")
        );
        let event = events.iter().find(|event| event["ts"].is_number()).unwrap();
        let runner_ts = event["ts"].as_f64().unwrap();
        assert!(
            runner_ts >= 2_000.0,
            "timestamp must include measured root offset: {runner_ts}"
        );
        assert!(
            runner_ts < 10_000_000.0,
            "timestamp unexpectedly far from runner origin: {runner_ts}"
        );
    }

    #[test]
    fn runner_context_links_execute_to_nested_command_root_only() {
        let directory = tempfile::tempdir().unwrap();
        let origin = monotonic_ns().saturating_sub(2_000_000);
        let capture = Capture::new();
        capture.with_default(|| {
            let command = phase("xtask.command");
            let dispatch = phase("xtask.dispatch");
            dispatch.finish(true);
            command_outcome(true, 0, None);
            command.finish(true);
        });
        let trace = capture.finish().unwrap();
        trace
            .persist_runner_fragment(&serde_json::json!({
                "trace_dir": directory.path(), "origin_ns": origin,
                "parent_name":"xtask-execute", "parent_span_id":"execute-id",
                "parent_pid":1234, "parent_flow_id":987654321
            }))
            .unwrap();
        let fragment = std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let events: Vec<serde_json::Value> = std::fs::read_to_string(fragment)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let begins: Vec<_> = events
            .iter()
            .filter(|event| event["name"] == "xtask.command" && event["ph"] == "B")
            .collect();
        let phase = events
            .iter()
            .find(|event| event["name"] == "xtask.dispatch" && event["ph"] == "B")
            .unwrap();
        assert_eq!(begins.len(), 1);
        assert_eq!(begins[0]["pid"], phase["pid"]);
        assert_eq!(begins[0]["tid"], phase["tid"]);
        let flows: Vec<_> = events
            .iter()
            .filter(|event| event["ph"] == "s" || event["ph"] == "f")
            .collect();
        assert_eq!(
            flows.len(),
            2,
            "only the command root receives cross-process flow"
        );
        let marker_index = events
            .iter()
            .position(|event| event["name"] == "recorded child entry")
            .unwrap();
        assert_eq!(events[marker_index]["ph"], "i");
        assert_eq!(events[marker_index]["s"], "t");
        assert_eq!(events[marker_index]["pid"], 1234);
        assert_eq!(events[marker_index]["tid"], 1234);
        assert_eq!(events[marker_index]["args"]["parent_span_id"], "execute-id");
        assert_eq!(events[marker_index]["args"]["child_name"], "xtask.command");
        assert_eq!(&events[marker_index + 1], flows[0]);
        assert_eq!(events[marker_index]["ts"], flows[0]["ts"]);
        assert_eq!(flows[0]["id"], "000000003ade68b1");
        assert_eq!(flows[1]["id"], flows[0]["id"]);
        assert_eq!(flows[0]["pid"], 1234);
        assert_eq!(flows[0]["args"]["parent_span_id"], "execute-id");
        assert_eq!(flows[1]["name"], "xtask-execute-to-command");
        assert_eq!(flows[1]["ts"], begins[0]["ts"]);
    }

    #[test]
    fn persistence_retains_each_run_and_reports_io_failures() {
        use std::io;
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
            origin_ns: monotonic_ns(),
        };
        let first = trace.persist_standalone(root.path()).unwrap();
        let second = trace.persist_standalone(root.path()).unwrap();
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
        assert_eq!(trace.persist_standalone(&blocked), Err("directory-create"));
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
