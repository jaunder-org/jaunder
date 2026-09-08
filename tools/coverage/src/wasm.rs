//! Versioned wire types exchanged by the browser coverage producer and host gate.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct BrowserStatus {
    pub version: String,
    pub requested_browser: String,
    pub actual_browser: String,
    pub csr_structural: Stage,
    pub diagnostic_export: Stage,
    pub source_mapping: Stage,
    pub module_signature: Option<String>,
    pub toolchain_identity: Option<Value>,
    pub served_module: Option<ServedModule>,
    pub artifacts: BTreeMap<String, Artifact>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Stage {
    pub outcome: Outcome,
    pub blocker: Option<String>,
}

impl Stage {
    pub fn passed() -> Self {
        Self {
            outcome: Outcome::Passed,
            blocker: None,
        }
    }

    pub fn failed(blocker: impl Into<String>) -> Self {
        Self {
            outcome: Outcome::Failed,
            blocker: Some(blocker.into()),
        }
    }

    pub fn not_run(blocker: impl Into<String>) -> Self {
        Self {
            outcome: Outcome::NotRun,
            blocker: Some(blocker.into()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Passed,
    Failed,
    NotRun,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Artifact {
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ServedModule {
    pub path: String,
    pub sha256: String,
}
