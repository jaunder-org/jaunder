//! Pure shared GitHub Actions failure-signature policy.
//!
//! This module deliberately has no transport or clock dependency.  It turns raw log
//! text and already-observed run/job metadata into the versioned annotation emitted
//! by `pr watch`.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sha2::{Digest, Sha256};

/// The selected failed Actions check, retained internally until enrichment can use it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectFailure {
    pub workflow_run_id: u64,
    pub check_run_id: u64,
    pub name: String,
}

/// The serialized v1 annotation for an identical failure on another eligible head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SharedFailure {
    pub version: u8,
    pub algorithm: String,
    pub digest_sha256: String,
    pub excerpt: String,
    pub matches: Vec<SharedFailureMatch>,
}

/// One other failed job with the same complete normalized error block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SharedFailureMatch {
    pub source: SharedFailureSource,
    pub head_sha: String,
    pub run: SharedFailureRun,
    pub job: SharedFailureJob,
}

/// The eligible source head that produced a matching failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SharedFailureSource {
    PullRequest { number: u64, url: String },
    Main,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SharedFailureRun {
    pub id: u64,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SharedFailureJob {
    pub id: u64,
    pub name: String,
    pub url: String,
}

/// A primary-CI run observed by the transport. `None` means it has not completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRun {
    pub id: u64,
    pub url: String,
    pub head_sha: String,
    pub created_at: String,
    pub conclusion: Option<String>,
}

/// A failed job and its raw log from a run selected for comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateJob {
    pub id: u64,
    pub name: String,
    pub url: String,
    pub log: String,
}

/// An eligible head captured before candidate-run selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EligibleHead {
    pub sha: String,
    pub source: SharedFailureSource,
}

/// A selected completed candidate run together with the source that made it eligible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedRun {
    pub run: CandidateRun,
    pub source: SharedFailureSource,
}

/// Transport evidence ready for the pure signature policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedFailureEvidence {
    pub subject_log: String,
    pub selected_runs: Vec<SelectedRun>,
    pub jobs: Vec<(u64, Vec<CandidateJob>)>,
}

const ALGORITHM: &str = "github-actions-error-block-v1";
const MAX_BLOCK_LINES: usize = 8;
const MAX_BLOCK_BYTES: usize = 4 * 1024;
const MAX_MATCHES: usize = 10;

/// Remove presentation noise while preserving all substantive error text.
pub fn normalize_log(log: &str) -> String {
    let no_ansi = strip_ansi(log);
    let mut normalized = String::new();
    let mut previous_blank = false;
    for line in no_ansi.replace("\r\n", "\n").replace('\r', "\n").lines() {
        let line = strip_actions_prefix(line).trim_end();
        let blank = line.is_empty();
        if blank && previous_blank {
            continue;
        }
        if !normalized.is_empty() {
            normalized.push('\n');
        }
        normalized.push_str(line);
        previous_blank = blank;
    }
    normalized
}

/// Extract the final qualifying bounded error block from an Actions log.
pub fn extract_error_block(log: &str) -> Option<String> {
    let normalized = normalize_log(log);
    let lines = normalized.lines().collect::<Vec<_>>();
    let anchor = lines.iter().rposition(is_anchor)?;
    let mut block = vec![lines[anchor]];
    for line in &lines[anchor + 1..] {
        let trimmed = line.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
        if line.is_empty() || trimmed.starts_with("Caused by:") || starts_indented(line) {
            block.push(line);
        } else {
            break;
        }
    }
    let line_count = block.len();
    let block = block.join("\n");
    (line_count <= MAX_BLOCK_LINES && block.len() <= MAX_BLOCK_BYTES).then_some(block)
}

/// Build the versioned signature of one raw failed-job log.
pub fn signature(log: &str) -> Option<(String, String)> {
    let excerpt = extract_error_block(log)?;
    let digest_sha256 = Sha256::digest(excerpt.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Some((excerpt, digest_sha256))
}

/// Choose the newest completed run for each eligible head after bounding history.
///
/// `runs` must be the primary workflow's repository-scoped history. The caller
/// supplies exactly the preceding-24-hour history; the first 50 newest entries are
/// deliberately retained *before* eligibility filtering.
pub fn select_runs(
    runs: impl IntoIterator<Item = CandidateRun>,
    eligible_heads: &[EligibleHead],
    subject_run_id: u64,
    not_before: &str,
) -> Vec<SelectedRun> {
    let sources = eligible_heads
        .iter()
        .map(|head| (head.sha.as_str(), head.source.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut runs = runs.into_iter().collect::<Vec<_>>();
    runs.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    let mut chosen = BTreeSet::new();
    runs.into_iter()
        .filter(|run| run.created_at.as_str() >= not_before)
        .take(50)
        .filter_map(|run| {
            let source = sources.get(run.head_sha.as_str())?;
            (run.id != subject_run_id
                && run.conclusion.is_some()
                && chosen.insert(run.head_sha.clone()))
            .then(|| SelectedRun {
                run,
                source: source.clone(),
            })
        })
        .collect()
}

/// Compare failed-job logs exactly and return the capped, deterministic annotation.
pub fn shared_failure(
    subject_log: &str,
    selected_runs: impl IntoIterator<Item = SelectedRun>,
    jobs: impl IntoIterator<Item = (u64, Vec<CandidateJob>)>,
) -> Option<SharedFailure> {
    let (excerpt, digest_sha256) = signature(subject_log)?;
    let subject_excerpt = excerpt.as_str();
    let jobs = jobs.into_iter().collect::<BTreeMap<_, _>>();
    let mut matches = selected_runs
        .into_iter()
        .flat_map(|selected| {
            jobs.get(&selected.run.id)
                .into_iter()
                .flatten()
                .filter(|job| extract_error_block(&job.log).as_deref() == Some(subject_excerpt))
                .map(move |job| {
                    (
                        selected.run.created_at.clone(),
                        selected.run.id,
                        SharedFailureMatch {
                            source: selected.source.clone(),
                            head_sha: selected.run.head_sha.clone(),
                            run: SharedFailureRun {
                                id: selected.run.id,
                                url: selected.run.url.clone(),
                            },
                            job: SharedFailureJob {
                                id: job.id,
                                name: job.name.clone(),
                                url: job.url.clone(),
                            },
                        },
                    )
                })
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| right.1.cmp(&left.1))
            .then_with(|| left.2.job.id.cmp(&right.2.job.id))
    });
    let matches = matches
        .into_iter()
        .take(MAX_MATCHES)
        .map(|(_, _, value)| value)
        .collect::<Vec<_>>();
    (!matches.is_empty()).then_some(SharedFailure {
        version: 1,
        algorithm: ALGORITHM.into(),
        digest_sha256,
        excerpt,
        matches,
    })
}

fn starts_indented(line: &str) -> bool {
    line.chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_whitespace())
}

fn is_anchor(line: &&str) -> bool {
    let line = line.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
    (line.starts_with("error:") || line.starts_with("Error:") || line.starts_with("fatal:"))
        && line != "Error: The operation was canceled."
        && !line.starts_with("Error: Process completed with exit code")
}

fn strip_actions_prefix(line: &str) -> &str {
    let line = line.strip_prefix("##[error]").unwrap_or(line);
    let payload = if let Some((_, after_job)) = line.split_once('\t')
        && let Some((_, timed)) = after_job.split_once('\t')
        && let Some((timestamp, message)) = timed.split_once(' ')
        && is_timestamp(timestamp)
    {
        message
    } else {
        let (timestamp, message) = line.split_once(' ').unwrap_or((line, ""));
        if is_timestamp(timestamp) {
            message
        } else {
            line
        }
    };
    payload.strip_prefix("##[error]").unwrap_or(payload)
}

fn is_timestamp(value: &str) -> bool {
    value.contains('T')
        && value.bytes().all(|byte| {
            byte.is_ascii_digit() || matches!(byte, b':' | b'.' | b'Z' | b'T' | b'+' | b'-')
        })
}

fn strip_ansi(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < bytes.len() && !(0x40..=0x7e).contains(&bytes[index]) {
                index += 1;
            }
            index += usize::from(index < bytes.len());
        } else if let Some(ch) = input[index..].chars().next() {
            output.push(ch);
            index += ch.len_utf8();
        } else {
            break;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_cited_logs_to_the_same_complete_signature() {
        let first = "Validate (no e2e)\tValidate (static + clippy + Rust and Emacs coverage, via xtask)\t2026-09-12T18:47:55.2254816Z error: failed to download `jiff v0.2.35`\r\nValidate (no e2e)\tValidate (static + clippy + Rust and Emacs coverage, via xtask)\t2026-09-12T18:47:55.2255010Z \r\nValidate (no e2e)\tValidate (static + clippy + Rust and Emacs coverage, via xtask)\t2026-09-12T18:47:55.2255098Z Caused by:\r\nValidate (no e2e)\tValidate (static + clippy + Rust and Emacs coverage, via xtask)\t2026-09-12T18:47:55.2255381Z   attempting to make an HTTP request, but --offline was specified";
        let second = "\u{1b}[31mValidate (no e2e)\tValidate (static + clippy + Rust and Emacs coverage, via xtask)\t2026-09-12T18:49:09.002Z error: failed to download `jiff v0.2.35`\u{1b}[0m\nValidate (no e2e)\tValidate (static + clippy + Rust and Emacs coverage, via xtask)\t2026-09-12T18:49:12.400Z \nValidate (no e2e)\tValidate (static + clippy + Rust and Emacs coverage, via xtask)\t2026-09-12T18:49:13.100Z Caused by:\nValidate (no e2e)\tValidate (static + clippy + Rust and Emacs coverage, via xtask)\t2026-09-12T18:49:15.600Z   attempting to make an HTTP request, but --offline was specified";
        assert_eq!(extract_error_block(first), extract_error_block(second));
    }

    #[test]
    fn extraction_uses_last_real_anchor_and_only_its_continuation_grammar() {
        let log = "error: first\nother\n Error: Process completed with exit code 1.\nfatal: second\n\nCaused by:\n  nested\nnext";
        assert_eq!(
            extract_error_block(log).as_deref(),
            Some("fatal: second\n\nCaused by:\n  nested")
        );
        assert_eq!(
            extract_error_block("Error: The operation was canceled."),
            None
        );
        assert_eq!(extract_error_block("not an error"), None);
    }

    #[test]
    fn serializes_the_version_one_source_discriminant_without_null_fields() {
        let source = SharedFailureSource::PullRequest {
            number: 1437,
            url: "https://github.com/jaunder-org/jaunder/pull/1437".into(),
        };
        assert_eq!(
            serde_json::to_value(source).unwrap(),
            serde_json::json!({
                "kind": "pull-request",
                "number": 1437,
                "url": "https://github.com/jaunder-org/jaunder/pull/1437",
            })
        );
        assert_eq!(
            serde_json::to_value(SharedFailureSource::Main).unwrap(),
            serde_json::json!({ "kind": "main" })
        );
    }

    #[test]
    fn rejects_blocks_independently_at_line_and_byte_limits() {
        let nine_lines = format!("error: x\n{}", "  x\n".repeat(8));
        assert_eq!(extract_error_block(&nine_lines), None);
        assert_eq!(
            extract_error_block(&format!("error: x\n  {}", "x".repeat(4096))),
            None
        );
    }

    #[test]
    fn selects_bounded_history_and_latest_completed_eligible_head() {
        let eligible = vec![EligibleHead {
            sha: "a".into(),
            source: SharedFailureSource::Main,
        }];
        let mut runs = (0..51)
            .map(|id| CandidateRun {
                id,
                url: String::new(),
                head_sha: "ignored".into(),
                created_at: format!("{id:02}"),
                conclusion: Some("failure".into()),
            })
            .collect::<Vec<_>>();
        runs.push(CandidateRun {
            id: 99,
            url: String::new(),
            head_sha: "a".into(),
            created_at: "00".into(),
            conclusion: Some("failure".into()),
        });
        runs.push(CandidateRun {
            id: 98,
            url: String::new(),
            head_sha: "a".into(),
            created_at: "99".into(),
            conclusion: None,
        });
        assert!(
            select_runs(runs, &eligible, 0, "").is_empty(),
            "the pre-filter cap excludes the 51st eligible run"
        );
        let selected = select_runs(
            [
                CandidateRun {
                    id: 4,
                    url: String::new(),
                    head_sha: "a".into(),
                    created_at: "04".into(),
                    conclusion: None,
                },
                CandidateRun {
                    id: 3,
                    url: String::new(),
                    head_sha: "a".into(),
                    created_at: "03".into(),
                    conclusion: Some("success".into()),
                },
                CandidateRun {
                    id: 2,
                    url: String::new(),
                    head_sha: "a".into(),
                    created_at: "02".into(),
                    conclusion: Some("failure".into()),
                },
            ],
            &eligible,
            0,
            "",
        );
        assert_eq!(
            selected[0].run.id, 3,
            "a green latest completed run supersedes an older failure"
        );
        assert!(
            select_runs(
                [CandidateRun {
                    id: 5,
                    url: String::new(),
                    head_sha: "a".into(),
                    created_at: "2026-09-14T00:00:00Z".into(),
                    conclusion: Some("failure".into()),
                }],
                &eligible,
                0,
                "2026-09-14T00:00:01Z",
            )
            .is_empty(),
            "runs before the 24-hour cutoff are ineligible"
        );
    }

    #[test]
    fn matches_exact_text_and_orders_then_caps_jobs() {
        let run = |id: u64, created_at: &str| SelectedRun {
            run: CandidateRun {
                id,
                url: format!("run/{id}"),
                head_sha: format!("sha/{id}"),
                created_at: created_at.into(),
                conclusion: Some("failure".into()),
            },
            source: SharedFailureSource::Main,
        };
        let selected = vec![
            run(2, "2026-09-15T02:00:00Z"),
            run(1, "2026-09-15T01:00:00Z"),
        ];
        let matching = "error: preserved /a/v1/id\n  cause";
        let jobs = vec![
            (
                2,
                (0..11)
                    .map(|id| CandidateJob {
                        id,
                        name: "job".into(),
                        url: String::new(),
                        log: matching.into(),
                    })
                    .collect(),
            ),
            (
                1,
                vec![CandidateJob {
                    id: 99,
                    name: "different".into(),
                    url: String::new(),
                    log: "error: preserved /a/v2/id\n  cause".into(),
                }],
            ),
        ];
        let annotation = shared_failure(matching, selected, jobs).unwrap();
        assert_eq!(annotation.matches.len(), 10);
        assert_eq!(annotation.matches.first().unwrap().job.id, 0);
        assert!(
            annotation
                .digest_sha256
                .chars()
                .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
        );
    }
}
