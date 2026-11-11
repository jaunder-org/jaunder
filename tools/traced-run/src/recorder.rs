//! Passive observations share named-trace fragments and publication, never launcher context.
use super::{alloc_id, monotonic_ns, publish_trace_records, run_dir};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

fn invalid() -> io::Error {
    io::Error::other("invalid or unavailable recorder context")
}

// The boot identity fences the host-wide CLOCK_MONOTONIC origin across reboots;
// elapsed time alone cannot distinguish a new boot whose counter overtook start.
#[cfg(target_os = "linux")]
fn clock_identity() -> io::Result<String> {
    parse_clock_identity(&fs::read("/proc/sys/kernel/random/boot_id")?)
}

#[cfg(target_os = "macos")]
fn clock_identity() -> io::Result<String> {
    // Read only fixed recorder metadata, never caller-selected commands or workload
    // state. The OS utility avoids unsafe FFI and does not join launcher context.
    sysctl_identity(
        std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "kern.bootsessionuuid"])
            .stdin(std::process::Stdio::null())
            .output(),
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn clock_identity() -> io::Result<String> {
    Err(io::Error::other(
        "boot clock identity unavailable on this platform",
    ))
}

#[cfg(any(target_os = "macos", test))]
fn sysctl_identity(output: io::Result<std::process::Output>) -> io::Result<String> {
    let output = output?;
    if !output.status.success() {
        return Err(io::Error::other("boot identity metadata request failed"));
    }
    parse_clock_identity(&output.stdout)
}

fn parse_clock_identity(bytes: &[u8]) -> io::Result<String> {
    if bytes.len() > 38 {
        return Err(invalid());
    }
    let identity = std::str::from_utf8(bytes).map_err(|_| invalid())?.trim();
    if identity.len() != 36
        || !identity.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    {
        return Err(invalid());
    }
    Ok(identity.to_owned())
}

fn private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)
}

fn start(cwd: &Path, mode: &str) -> io::Result<String> {
    let clock = clock_identity()?;
    fs::create_dir_all(run_dir(cwd))?;
    let (handle, dir) = loop {
        let handle = alloc_id();
        let dir = run_dir(cwd).join(format!("{handle}.trace.d"));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&dir) {
            Ok(()) => break (handle, dir),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let origin = monotonic_ns();
    let pid = std::process::id();
    let event = json!({
        "name":"tsc-request", "cat":"process", "ph":"B", "pid":pid, "tid":pid, "ts":0,
        "args": {"run_id":handle, "mode":mode, "origin_ns":origin,
            "clock":"CLOCK_MONOTONIC", "clock_identity":clock,
            "outcome":"unknown", "boundary":"unknown", "source_identity":"unknown",
            "tool_identity":"unknown", "recorder_version":env!("CARGO_PKG_VERSION"),
            "envelope":"owner request between record-start and record-finish; pre-entry work unknown",
            "output":"raw output not archived"}
    });
    private_file(&dir.join("start.jsonl"), format!("{event}\n").as_bytes())?;
    Ok(handle)
}

fn finish(cwd: &Path, handle: &str, code: u8, boundary: &str) -> io::Result<()> {
    // Only generated identities can select a leaf of this checkout's run store.
    let parts: Vec<_> = handle.split('-').collect();
    if handle.len() > 80
        || parts.len() != 3
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(invalid());
    }
    let dir = run_dir(cwd).join(format!("{handle}.trace.d"));
    if !fs::symlink_metadata(&dir)?.file_type().is_dir() {
        return Err(invalid());
    }
    let path = dir.join("start.jsonl");
    if !fs::symlink_metadata(&path)?.file_type().is_file() {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(8193).read_to_end(&mut bytes)?;
    if bytes.len() > 8192 {
        return Err(invalid());
    }
    let begin: Value = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let args = &begin["args"];
    let clock = clock_identity()?;
    let now = monotonic_ns();
    let origin = args["origin_ns"].as_u64().ok_or_else(invalid)?;
    let pid = begin["pid"].as_u64().ok_or_else(invalid)?;
    if begin["name"] != "tsc-request"
        || begin["ph"] != "B"
        || begin["ts"] != 0
        || begin["tid"] != pid
        || args["run_id"] != handle
        || !matches!(args["mode"].as_str(), Some("verify" | "fix"))
        || args["clock"] != "CLOCK_MONOTONIC"
        || args["clock_identity"] != clock
        || origin == 0
        || now < origin
    {
        return Err(invalid());
    }
    // Exclusive claim serializes finishers. Failure leaves start evidence intact;
    // it is never permission for a second finisher to replace prior evidence.
    private_file(&dir.join("finish-claim"), b"finish attempted\n")?;
    let event = json!({"name":"tsc-request", "cat":"process", "ph":"E",
        "pid":pid, "tid":pid, "ts":(now-origin)/1000,
        "args":{"run_id":handle, "exit_code":code, "boundary":boundary,
            "outcome":if code == 0 {"success"} else {"failure"}}});
    private_file(&dir.join("finish.jsonl"), format!("{event}\n").as_bytes())?;
    publish_trace_records(&dir)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};

    const BOOT: &str = "01234567-89ab-cdef-0123-456789abcdef";

    #[test]
    fn boot_identity_requires_one_bounded_uuid() {
        assert_eq!(
            parse_clock_identity(format!("{BOOT}\n").as_bytes()).unwrap(),
            BOOT
        );
        for bytes in [
            b"".as_slice(),
            b"not a boot identity",
            b"0123456789abcdef0123456789abcdef012345",
            b"01234567-89ab-cdef-0123-456789abcdeg",
            b"01234567-89ab-cdef-0123-456789abcdef extra",
            b"\xff",
        ] {
            assert!(parse_clock_identity(bytes).is_err(), "{bytes:?}");
        }
    }

    #[test]
    fn macos_metadata_failures_are_refused_without_archiving_utility_output() {
        let output = |code, stdout: &[u8]| Output {
            status: ExitStatus::from_raw(code),
            stdout: stdout.to_vec(),
            stderr: b"must not be archived".to_vec(),
        };
        assert_eq!(
            sysctl_identity(Ok(output(0, BOOT.as_bytes()))).unwrap(),
            BOOT
        );
        for result in [
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "utility unavailable",
            )),
            Ok(output(256, BOOT.as_bytes())),
            Ok(output(0, b"malformed")),
            Ok(output(0, b"\xff")),
        ] {
            let error = sysctl_identity(result).unwrap_err();
            assert!(!error.to_string().contains("must not be archived"));
        }
    }
}

pub(super) fn cli(args: &[String], cwd: &Path) -> io::Result<Option<String>> {
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["record-start", "tsc-request", mode @ ("verify" | "fix")] => start(cwd, mode).map(Some),
        [
            "record-finish",
            handle,
            code,
            boundary @ ("none" | "setup" | "request"),
        ] => {
            let code = code.parse::<u8>().map_err(|_| invalid())?;
            if (code == 0) != (*boundary == "none") {
                return Err(invalid());
            }
            finish(cwd, handle, code, boundary).map(|()| None)
        }
        _ => Err(io::Error::other(
            "expected record-start tsc-request verify|fix or record-finish HANDLE EXIT-CODE none|setup|request",
        )),
    }
}
