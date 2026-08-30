use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The fixed producer schema. A different value is never interpreted leniently.
pub const STATUS_SCHEMA: &str = "elisp-coverage-v1";

/// Producer-owned status and census handed to the stateless host consumer.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProducerStatus {
    pub schema: String,
    pub outcome: ProducerOutcome,
    pub modules: Vec<ModuleCensus>,
}

/// Controlled producer outcomes. A non-success outcome is itself the verdict.
#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProducerOutcome {
    Success,
    ErtFailure,
    InstrumentationFailure,
    InvalidReport,
}

/// One production module and its producer-time top-level form census.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleCensus {
    pub path: String,
    pub forms: Vec<FormCensus>,
}

/// A source identity for one top-level form.
#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FormCensus {
    pub start_line: u32,
    pub kind: String,
    pub points: Vec<PointCensus>,
}

/// One Edebug stop point, or the synthetic opening-line point for a zero-stop form.
#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PointCensus {
    pub line: u32,
    pub kind: PointKind,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PointKind {
    Ordinary,
    Synthetic,
}

/// A concise, structured rejection suitable for a later xtask orchestration layer.
#[derive(Debug, PartialEq, Eq)]
pub enum CoverageError {
    Artifact { path: PathBuf, message: String },
    Status { message: String },
    Source { path: PathBuf, message: String },
    Census { message: String },
    Lcov { message: String },
    Verdict { failures: Vec<CoverageFailure> },
}

#[derive(Debug, PartialEq, Eq)]
pub struct CoverageFailure {
    pub path: String,
    pub line: u32,
    pub message: String,
}

/// The successful reconciliation summary. Failures are returned as `CoverageError`.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct CoverageReport {
    pub covered_points: usize,
    pub ignored_points: usize,
}
