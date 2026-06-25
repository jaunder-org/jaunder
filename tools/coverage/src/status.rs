//! The in-sandbox coverage sentinel: what `devtool coverage emit` can know
//! without git/baseline context (only test pass/fail + infrastructure health).
//! Written to `$out/status.json`; read by both the Nix consumer derivation and
//! the host `xtask` gate.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatusCategory {
    TestsOk,
    TestFailure,
    Infra,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageStatus {
    pub category: StatusCategory,
    #[serde(default)]
    pub failed_tests: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infra_detail: Option<String>,
}

impl CoverageStatus {
    pub fn to_json(&self) -> String {
        format!(
            "{}\n",
            serde_json::to_string_pretty(self).expect("serialize status")
        )
    }

    pub fn from_json(s: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(s)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips_through_json() {
        let s = CoverageStatus {
            category: StatusCategory::TestFailure,
            failed_tests: vec!["web_posts::case_3".into()],
            infra_detail: None,
        };
        let back = CoverageStatus::from_json(&s.to_json()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn category_serializes_kebab_case() {
        let s = CoverageStatus {
            category: StatusCategory::Infra,
            failed_tests: vec![],
            infra_detail: Some("ENOSPC".into()),
        };
        assert!(s.to_json().contains("\"infra\""));
    }
}
