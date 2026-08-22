//! The `gh` boundary — the only file in `pr/` that runs a subprocess.
//!
//! `gh api` collapses every failure into exit 1: a 404, a rate limit, a dead network,
//! and a malformed GraphQL query are indistinguishable by exit code. The
//! discriminating information is always in the *body* — which is why [`classify`] is
//! a pure function over `(exit, stdout, stderr)` and the subprocess wrapper around it
//! is five lines. That split is what lets every transport failure be tested offline.

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct GitError {
    pub operation: &'static str,
    pub path: PathBuf,
    pub source: Arc<dyn std::error::Error + Send + Sync>,
}

impl PartialEq for GitError {
    fn eq(&self, other: &Self) -> bool {
        self.operation == other.operation
            && self.path == other.path
            && format!("{:#}", self.source) == format!("{:#}", other.source)
    }
}

impl Eq for GitError {}

impl std::fmt::Display for GitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} at {}: {:#}",
            self.operation,
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for GitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// The `gh` binary is not on PATH.
    GhMissing,
    Unauthenticated,
    /// The repo, PR, or endpoint does not exist.
    NotFound,
    /// `reset_unix` is filled in by [`enrich_rate_limit`], not by `classify` — `gh`
    /// does not print response headers, so the body alone cannot carry it.
    RateLimited {
        reset_unix: Option<u64>,
    },
    Transport(String),
    Malformed(String),
    GraphQlErrors(String),
    Git(GitError),
}

impl ApiError {
    /// Whether the watch loop should absorb this against its strike budget.
    ///
    /// Rate-limiting is deliberately **not** transient: GitHub tells us when it
    /// clears, so the loop waits for the reset rather than spending five strikes over
    /// two minutes on a condition known to last twelve.
    pub fn is_transient(&self) -> bool {
        matches!(self, ApiError::Transport(_) | ApiError::Malformed(_))
    }

    pub fn detail(&self) -> String {
        match self {
            ApiError::GhMissing => "`gh` is not on PATH".into(),
            ApiError::Unauthenticated => "`gh` is not authenticated".into(),
            ApiError::NotFound => "not found".into(),
            ApiError::RateLimited {
                reset_unix: Some(r),
            } => {
                format!("rate limited until {r} (unix)")
            }
            ApiError::RateLimited { reset_unix: None } => "rate limited".into(),
            ApiError::Transport(m) => format!("transport: {m}"),
            ApiError::Malformed(m) => format!("malformed response: {m}"),
            ApiError::GraphQlErrors(m) => format!("graphql: {m}"),
            ApiError::Git(error) => format!("git: {error}"),
        }
    }
}

/// Turn one `gh` invocation's three outputs into a value or a typed error.
///
/// Invariant relied on by [`run_gh_raw`]: a non-zero exit **never** yields `Ok`.
pub fn classify(exit: i32, stdout: &str, stderr: &str) -> Result<Value, ApiError> {
    // First, because a missing binary also writes a `gh: `-prefixed line that the
    // GraphQL branch below would otherwise claim.
    if exit == 127 || stderr.contains("command not found") {
        return Err(ApiError::GhMissing);
    }

    let body: Option<Value> = serde_json::from_str(stdout.trim()).ok();

    // A GraphQL error arrives with HTTP 200 and exit 0, carrying `errors[]` beside a
    // null `data`. Checking the exit code alone would report it as success — this is
    // the "the tool lies" case, so the array is checked before anything else.
    if let Some(errors) = body
        .as_ref()
        .and_then(|b| b.get("errors"))
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
    {
        // GraphQL reports "no such PR" as a typed error inside a 200, not as a 404.
        // Without this it would read as a generic query failure — i.e. the tooling
        // being broken — rather than as the subject not existing.
        if errors
            .iter()
            .any(|e| e.get("type").and_then(Value::as_str) == Some("NOT_FOUND"))
        {
            return Err(ApiError::NotFound);
        }
        return Err(ApiError::GraphQlErrors(
            serde_json::Value::Array(errors.clone()).to_string(),
        ));
    }

    if exit == 0 {
        return match body {
            Some(v) => Ok(v),
            None => Err(ApiError::Malformed(truncate(stdout))),
        };
    }

    // Non-zero: the body's `status` is the only trustworthy discriminator.
    if let Some(status) = body
        .as_ref()
        .and_then(|b| b.get("status"))
        .and_then(Value::as_str)
    {
        let message = body
            .as_ref()
            .and_then(|b| b.get("message"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Err(from_status(status, message));
    }

    // No body to go on. `gh: …` means gh itself spoke (a rejected GraphQL document);
    // anything else is the transport failing underneath it.
    let trimmed = stderr.trim();
    if trimmed.starts_with("gh: ") {
        Err(ApiError::GraphQlErrors(
            trimmed.trim_start_matches("gh: ").to_string(),
        ))
    } else {
        Err(ApiError::Transport(truncate(trimmed)))
    }
}

/// A 403 is either a rate limit or a plain refusal, and only the message says which.
fn from_status(status: &str, message: &str) -> ApiError {
    match status {
        "401" => ApiError::Unauthenticated,
        "404" => ApiError::NotFound,
        "403" if message.to_ascii_lowercase().contains("rate limit") => {
            ApiError::RateLimited { reset_unix: None }
        }
        _ => ApiError::Transport(format!("HTTP {status}: {message}")),
    }
}

/// Fold a separately-fetched reset time into a rate-limit error. Any other error
/// passes through untouched.
pub fn enrich_rate_limit(err: ApiError, reset: Option<u64>) -> ApiError {
    match err {
        ApiError::RateLimited { .. } => ApiError::RateLimited { reset_unix: reset },
        other => other,
    }
}

/// Cap an error blob for display, cutting on a **character** boundary.
///
/// Byte slicing would panic on any multi-byte character straddling the cut — and this
/// runs on exactly the failure paths (`Malformed`, `Transport`) where the text is
/// least predictable: `gh` prints `…`/`✓`, proxies return UTF-8 HTML, GitHub messages
/// use curly quotes. A panic here would kill the process with no report at all, which
/// is the one thing this command must never do.
fn truncate(s: &str) -> String {
    let s = s.trim();
    match s.char_indices().nth(300) {
        None => s.to_string(),
        Some((cut, _)) => format!("{}…", &s[..cut]),
    }
}

/// Run `gh` and hand its three outputs to [`classify`].
fn spawn(args: &[&str]) -> Result<(i32, String, String), ApiError> {
    match Command::new("gh").args(args).output() {
        Ok(out) => Ok((
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ApiError::GhMissing),
        Err(e) => Err(ApiError::Transport(format!("could not run gh: {e}"))),
    }
}

/// Run `gh` with a complete stdin payload and hand its outputs to [`classify`].
pub fn run_gh_stdin(args: &[&str], stdin: &str) -> Result<Value, ApiError> {
    let mut child = match Command::new("gh")
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ApiError::GhMissing),
        Err(e) => return Err(ApiError::Transport(format!("could not run gh: {e}"))),
    };
    let Some(mut pipe) = child.stdin.take() else {
        return Err(ApiError::Transport("could not open gh stdin".into()));
    };
    pipe.write_all(stdin.as_bytes())
        .map_err(|e| ApiError::Transport(format!("could not write gh stdin: {e}")))?;
    drop(pipe);
    let out = child
        .wait_with_output()
        .map_err(|e| ApiError::Transport(format!("could not wait for gh: {e}")))?;
    classify(
        out.status.code().unwrap_or(-1),
        &String::from_utf8_lossy(&out.stdout),
        &String::from_utf8_lossy(&out.stderr),
    )
    .map_err(with_reset)
}

/// Look up the reset time only when it can matter. `enrich_rate_limit` discards it for
/// every other variant, so calling it eagerly would spawn a second `gh` on every 404
/// and every transport blip.
fn with_reset(err: ApiError) -> ApiError {
    match err {
        ApiError::RateLimited { .. } => enrich_rate_limit(err, rate_limit_reset()),
        other => other,
    }
}

/// A JSON-producing `gh api` call.
pub fn run_gh(args: &[&str]) -> Result<Value, ApiError> {
    let (exit, out, err) = spawn(args)?;
    classify(exit, &out, &err).map_err(with_reset)
}

/// A `gh` call whose stdout is prose, not JSON — `gh pr merge` prints a human
/// sentence. Parsing it as JSON would classify every *successful* arm as malformed,
/// so this path never looks at stdout: the arm is verified by the next snapshot.
pub fn run_gh_raw(args: &[&str]) -> Result<(), ApiError> {
    let (exit, out, err) = spawn(args)?;
    if exit == 0 {
        return Ok(());
    }
    // `classify` never returns Ok for a non-zero exit, so the fallback is unreachable
    // in practice; it exists so this cannot silently succeed on a failure.
    match classify(exit, &out, &err) {
        Ok(_) => Err(ApiError::Transport(format!("gh exited {exit}"))),
        Err(e) => Err(with_reset(e)),
    }
}

/// When the exhausted rate-limit bucket resets, via the REST `rate_limit` endpoint.
/// A failed best-effort lookup preserves the original [`ApiError::RateLimited`] and
/// emits one fixed, redacted warning.
pub fn rate_limit_reset() -> Option<u64> {
    rate_limit_reset_with(|| spawn(&["api", "rate_limit"]), &mut std::io::stderr())
}

fn rate_limit_reset_with(
    spawn_reset: impl FnOnce() -> Result<(i32, String, String), ApiError>,
    stderr: &mut impl Write,
) -> Option<u64> {
    let attempt = (|| -> Result<Option<u64>, ApiError> {
        let (exit, out, err) = spawn_reset()?;
        let body = classify(exit, &out, &err)?;
        let resources = body.get("resources").ok_or_else(|| {
            ApiError::Malformed("rate_limit response omitted resources".to_owned())
        })?;
        Ok(["graphql", "core"]
            .iter()
            .filter_map(|bucket| resources.get(bucket))
            .filter(|bucket| bucket.get("remaining").and_then(Value::as_u64) == Some(0))
            .filter_map(|bucket| bucket.get("reset").and_then(Value::as_u64))
            .max())
    })();
    match attempt {
        Ok(reset) => reset,
        Err(_) => {
            let _ = writeln!(
                stderr,
                "xtask: warning: xtask.pr.rate_limit_reset: ignored failure while looking up rate-limit reset"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_parses_the_body() {
        let v = classify(0, r#"{"number":731}"#, "").unwrap();
        assert_eq!(v["number"], 731);
    }

    #[test]
    fn rest_404_body_classifies_as_not_found() {
        // Captured verbatim from `gh api /repos/jaunder-org/jaunder/pulls/999999`.
        let out = r#"{"message":"Not Found","documentation_url":"https://docs.github.com/rest/pulls/pulls#get-a-pull-request","status":"404"}"#;
        assert_eq!(
            classify(1, out, "gh: Not Found (HTTP 404)").unwrap_err(),
            ApiError::NotFound
        );
    }

    #[test]
    fn graphql_schema_error_classifies_as_graphql_errors() {
        // Captured verbatim: gh writes this to stderr with empty stdout.
        let err = "gh: Field 'nosuchfield' doesn't exist on type 'Repository'\n";
        match classify(1, "", err).unwrap_err() {
            ApiError::GraphQlErrors(m) => assert!(m.contains("nosuchfield")),
            other => panic!("expected GraphQlErrors, got {other:?}"),
        }
    }

    #[test]
    fn graphql_errors_array_on_exit_zero_is_still_an_error() {
        // A 200 response carrying `errors[]` must NOT read as success.
        let out = r#"{"data":null,"errors":[{"message":"Something went wrong"}]}"#;
        match classify(0, out, "").unwrap_err() {
            ApiError::GraphQlErrors(m) => assert!(m.contains("Something went wrong")),
            other => panic!("expected GraphQlErrors, got {other:?}"),
        }
    }

    #[test]
    fn graphql_not_found_classifies_as_not_found_not_a_query_failure() {
        // Captured verbatim from `gh api graphql` for a nonexistent PR: GitHub
        // reports a missing subject as a typed GraphQL error, never as a 404.
        let out = r#"{"data":{"repository":{"pullRequest":null}},"errors":[{"type":"NOT_FOUND","path":["repository","pullRequest"],"message":"Could not resolve to a PullRequest with the number of 999999."}]}"#;
        assert_eq!(classify(1, out, "").unwrap_err(), ApiError::NotFound);
        // …and on the exit-0 form of the same response.
        assert_eq!(classify(0, out, "").unwrap_err(), ApiError::NotFound);
    }

    #[test]
    fn rate_limit_body_classifies_without_a_reset() {
        // gh does not print response headers, so `classify` can never see the reset.
        let out = r#"{"message":"API rate limit exceeded for user ID 1.","status":"403"}"#;
        assert_eq!(
            classify(1, out, "").unwrap_err(),
            ApiError::RateLimited { reset_unix: None }
        );
    }

    #[test]
    fn secondary_rate_limit_also_classifies_as_rate_limited() {
        let out = r#"{"message":"You have exceeded a secondary rate limit.","status":"403"}"#;
        assert_eq!(
            classify(1, out, "").unwrap_err(),
            ApiError::RateLimited { reset_unix: None }
        );
    }

    #[test]
    fn a_non_rate_limit_403_is_transport_not_rate_limited() {
        // The 403 split rule: only a rate-limit *message* means rate-limited.
        let out = r#"{"message":"Resource not accessible by integration","status":"403"}"#;
        assert!(matches!(
            classify(1, out, "").unwrap_err(),
            ApiError::Transport(_)
        ));
    }

    #[test]
    fn enrich_fills_the_reset_only_for_rate_limits() {
        assert_eq!(
            enrich_rate_limit(ApiError::RateLimited { reset_unix: None }, Some(600)),
            ApiError::RateLimited {
                reset_unix: Some(600)
            }
        );
        assert_eq!(
            enrich_rate_limit(ApiError::NotFound, Some(600)),
            ApiError::NotFound
        );
    }

    #[test]
    fn auth_failure_classifies_as_unauthenticated() {
        let out = r#"{"message":"Bad credentials","status":"401"}"#;
        assert_eq!(classify(1, out, "").unwrap_err(), ApiError::Unauthenticated);
    }

    #[test]
    fn missing_binary_classifies_as_gh_missing() {
        assert_eq!(
            classify(127, "", "gh: command not found").unwrap_err(),
            ApiError::GhMissing
        );
    }

    #[test]
    fn server_error_is_transport_and_transient() {
        let out = r#"{"message":"Server Error","status":"502"}"#;
        let e = classify(1, out, "").unwrap_err();
        assert!(matches!(e, ApiError::Transport(_)));
        assert!(e.is_transient());
    }

    #[test]
    fn exit_one_with_no_body_and_no_gh_prefix_is_transport() {
        // The empty-stdout split rule: `gh: …` on stderr means gh spoke (GraphQL);
        // anything else is the transport failing underneath it.
        assert!(matches!(
            classify(1, "", "connection reset by peer").unwrap_err(),
            ApiError::Transport(_)
        ));
    }

    #[test]
    fn unparseable_success_body_is_malformed() {
        assert!(matches!(
            classify(0, "not json", "").unwrap_err(),
            ApiError::Malformed(_)
        ));
    }

    #[test]
    fn a_long_multibyte_error_body_truncates_without_panicking() {
        // Byte-slicing at 300 would land mid-character and panic, taking the whole
        // process — and the report — with it. `…` is 3 bytes, so a 200-char string of
        // them is 600 bytes with no char boundary at 300.
        let blob = "…".repeat(200);
        match classify(1, "", &blob).unwrap_err() {
            ApiError::Transport(m) => assert!(m.ends_with('…')),
            other => panic!("expected Transport, got {other:?}"),
        }
        // And the same on the malformed-body path.
        assert!(matches!(
            classify(0, &blob, "").unwrap_err(),
            ApiError::Malformed(_)
        ));
    }

    #[test]
    fn transience_partitions_the_variants() {
        assert!(!ApiError::RateLimited { reset_unix: None }.is_transient());
        assert!(!ApiError::GhMissing.is_transient());
        assert!(!ApiError::Unauthenticated.is_transient());
        assert!(!ApiError::NotFound.is_transient());
        assert!(ApiError::Transport("x".into()).is_transient());
        assert!(ApiError::Malformed("x".into()).is_transient());
    }

    fn assert_reset_failure_warns_once(
        attempt: impl FnOnce() -> Result<(i32, String, String), ApiError>,
    ) {
        let original = ApiError::RateLimited { reset_unix: None };
        let mut stderr = Vec::new();
        let reset = rate_limit_reset_with(attempt, &mut stderr);
        assert_eq!(enrich_rate_limit(original.clone(), reset), original);
        let warning = String::from_utf8(stderr).unwrap();
        assert_eq!(warning.matches("xtask.pr.rate_limit_reset").count(), 1);
        assert_eq!(warning.lines().count(), 1);
        assert!(!warning.contains("sensitive"));
    }

    #[test]
    fn rate_limit_reset_spawn_failure_preserves_the_rate_limit() {
        assert_reset_failure_warns_once(|| Err(ApiError::GhMissing));
    }

    #[test]
    fn rate_limit_reset_nonzero_and_malformed_responses_warn_once_each() {
        assert_reset_failure_warns_once(|| {
            Ok((
                1,
                r#"{"message":"rate limit exceeded","status":"403"}"#.to_owned(),
                "sensitive transport text".to_owned(),
            ))
        });
        assert_reset_failure_warns_once(|| {
            Ok((0, "sensitive malformed body".to_owned(), String::new()))
        });
    }
}
