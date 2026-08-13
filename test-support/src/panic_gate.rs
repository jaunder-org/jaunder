//! Shared e2e zero-panic verifier.

use std::io::ErrorKind;
use std::path::Path;

use anyhow::Context;

const PANIC_MARKER: &[u8] = b"panicked at";

// Default-deny: a future proven-benign exception must be documented here so
// the host and VM gates change together. No runtime seam may widen this list.
const ALLOWED_PANICS: &[&[u8]] = &[];

struct PanicReport {
    key: Vec<u8>,
    line: Vec<u8>,
}

/// Fail if either the scoped diagnostic stream or required server log records
/// a Rust panic.
///
/// `capture_dir` is the ADR-0057 directory, not a diagnostic-file path: this
/// module resolves [`host::capture::Stream::Diag`] so its filename remains
/// defined in one place. The diagnostic stream is optional because the panic
/// hook may not have installed yet; the server log is the required fallback.
///
/// # Errors
///
/// Returns an infrastructure error when a present diagnostic stream or the
/// required server log cannot be read. Returns a zero-panic-gate error naming
/// every distinct report when either input contains the raw panic marker.
pub fn verify_no_panics(capture_dir: &Path, server_log: &Path) -> anyhow::Result<()> {
    let diag_path = capture_dir.join(host::capture::Stream::Diag.filename());
    let diag = match std::fs::read(&diag_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read diagnostic log {}", diag_path.display()));
        }
    };
    let server = std::fs::read(server_log).with_context(|| {
        format!(
            "failed to read required server log {}",
            server_log.display()
        )
    })?;

    let mut reports = Vec::new();
    collect_reports(&diag, &mut reports);
    collect_reports(&server, &mut reports);

    if reports.is_empty() {
        return Ok(());
    }

    let mut message = String::from("e2e zero-panic gate: server logged Rust panic(s):");
    for report in reports {
        message.push('\n');
        message.push_str(&String::from_utf8_lossy(&report.line));
    }
    anyhow::bail!(message)
}

fn collect_reports(input: &[u8], reports: &mut Vec<PanicReport>) {
    for line in input.split(|byte| *byte == b'\n') {
        let Some(marker_at) = find_bytes(line, PANIC_MARKER).filter(|_| {
            !ALLOWED_PANICS
                .iter()
                .any(|allowed| !allowed.is_empty() && find_bytes(line, allowed).is_some())
        }) else {
            continue;
        };

        let key = report_key(line, marker_at);
        if reports.iter().all(|report| report.key != key) {
            reports.push(PanicReport {
                key,
                line: line.to_vec(),
            });
        }
    }
}

fn report_key(line: &[u8], marker_at: usize) -> Vec<u8> {
    let after_marker = &line[marker_at + PANIC_MARKER.len()..];
    let location = after_marker
        .strip_prefix(b" ")
        .map(<[u8]>::trim_ascii_start)
        .and_then(|rest| {
            let end = rest
                .iter()
                .position(u8::is_ascii_whitespace)
                .unwrap_or(rest.len());
            let mut token = &rest[..end];
            while let Some(stripped) = token.strip_suffix(b":") {
                token = stripped;
            }
            (!token.is_empty()).then_some(token)
        });

    let mut key = Vec::with_capacity(1 + location.map_or(line.len(), <[u8]>::len));
    if let Some(location) = location {
        key.push(0);
        key.extend_from_slice(location);
    } else {
        key.push(1);
        key.extend_from_slice(line);
    }
    key
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::write(path, bytes).expect("write fixture");
    }

    fn verify(diag: Option<&[u8]>, server: &[u8]) -> anyhow::Result<()> {
        let dir = tempfile::tempdir().expect("tempdir");
        let capture = dir.path().join("capture");
        std::fs::create_dir_all(&capture).expect("capture dir");
        if let Some(bytes) = diag {
            write(&capture.join(host::capture::Stream::Diag.filename()), bytes);
        }
        let server_log = dir.path().join("server.log");
        write(&server_log, server);
        verify_no_panics(&capture, &server_log)
    }

    #[test]
    fn clean_non_json_and_invalid_utf8_without_marker_pass() {
        verify(Some(b"not json\n\xff warning\n"), b"ordinary stderr\n")
            .expect("non-panic bytes are clean");
    }

    #[test]
    fn absent_optional_diag_passes() {
        verify(None, b"ordinary stderr\n").expect("missing diag is empty");
    }

    #[test]
    fn server_only_raw_panic_fails() {
        let error = verify(None, b"thread panicked at src/server.rs:7:9:\nboom\n")
            .expect_err("server panic must fail")
            .to_string();
        assert!(error.contains("src/server.rs:7:9"), "{error}");
    }

    #[test]
    fn diag_only_torn_invalid_utf8_panic_fails() {
        let error = verify(
            Some(b"\xff{torn panicked at src/diag.rs:4:2: boom\n"),
            b"clean\n",
        )
        .expect_err("raw marker must fail without JSON or UTF-8")
        .to_string();
        assert!(error.contains("src/diag.rs:4:2"), "{error}");
    }

    #[test]
    fn marker_without_location_still_fails() {
        let error = verify(Some(b"torn panicked at\n"), b"clean\n")
            .expect_err("marker-only line must fail")
            .to_string();
        assert!(error.contains("torn panicked at"), "{error}");
    }

    #[test]
    fn same_location_is_reported_once_with_diag_preferred() {
        let error = verify(
            Some(b"scoped panicked at src/shared.rs:12:5: scoped payload\n"),
            b"journal panicked at src/shared.rs:12:5:\nlegacy payload\n",
        )
        .expect_err("duplicate panic must fail")
        .to_string();
        assert_eq!(error.matches("src/shared.rs:12:5").count(), 1, "{error}");
        assert!(error.contains("scoped payload"), "{error}");
        assert!(!error.contains("legacy payload"), "{error}");
    }

    #[test]
    fn distinct_locations_are_all_reported() {
        let error = verify(
            Some(b"panicked at src/a.rs:1:2: a\n"),
            b"panicked at src/b.rs:3:4: b\n",
        )
        .expect_err("both panics must fail")
        .to_string();
        assert!(error.contains("src/a.rs:1:2"), "{error}");
        assert!(error.contains("src/b.rs:3:4"), "{error}");
    }

    #[test]
    fn unreadable_present_diag_is_infrastructure_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let capture = dir.path().join("capture");
        let diag = capture.join(host::capture::Stream::Diag.filename());
        std::fs::create_dir_all(&diag).expect("directory at diagnostic file path");
        let server = dir.path().join("server.log");
        write(&server, b"clean\n");

        let error = verify_no_panics(&capture, &server)
            .expect_err("present diagnostic stream must be readable")
            .to_string();
        assert!(error.contains("diagnostic log"), "{error}");
        assert!(
            error.contains(host::capture::Stream::Diag.filename()),
            "{error}"
        );
    }

    #[test]
    fn required_server_log_read_failure_is_infrastructure_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let capture = dir.path().join("capture");
        std::fs::create_dir_all(&capture).expect("capture dir");
        let missing = dir.path().join("missing-server.log");
        let error = verify_no_panics(&capture, &missing)
            .expect_err("required server log must be readable")
            .to_string();
        assert!(error.contains("server log"), "{error}");
        assert!(error.contains("missing-server.log"), "{error}");
    }
}
