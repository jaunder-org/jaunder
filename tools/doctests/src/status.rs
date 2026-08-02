//! The in-sandbox doctest sentinel: what `devtool doctests emit` can report from
//! inside the Nix producer derivation.
//!
//! Written to `$out/status.json`; read by both the `doctests-gate` consumer
//! derivation and the host `xtask` step. Mirrors `coverage::status` deliberately —
//! same producer-always-succeeds shape, same kebab-case wire spelling — but with
//! its own file and its own vocabulary, so a doctest failure never has to be
//! squeezed into a field that means "nextest test ids".

use serde::{Deserialize, Serialize};

use crate::check::Violation;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StatusCategory {
    /// The tree and the run agree, and every doctest passed.
    Ok,
    /// At least one violation — see `violations`.
    Violations,
    /// The emit could not produce a verdict (could not spawn the runner, etc.).
    Infra,
}

/// Everything the gate needs to decide, and to say why.
///
/// There is deliberately no separate `failed_doctests` list: a failing doctest is
/// a [`Violation`] with `kind: failed`, so the consumer has one list to render and
/// cannot report a failure in a shape the host renderer does not understand.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctestStatus {
    pub category: StatusCategory,
    #[serde(default)]
    pub violations: Vec<Violation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infra_detail: Option<String>,
}

impl DoctestStatus {
    /// `Ok` when `violations` is empty, `Violations` otherwise — so the category
    /// can never disagree with the list it summarizes.
    pub fn from_violations(violations: Vec<Violation>) -> Self {
        let category = if violations.is_empty() {
            StatusCategory::Ok
        } else {
            StatusCategory::Violations
        };
        Self {
            category,
            violations,
            infra_detail: None,
        }
    }

    /// The emit could not run at all.
    pub fn infra(detail: impl Into<String>) -> Self {
        Self {
            category: StatusCategory::Infra,
            violations: Vec::new(),
            infra_detail: Some(detail.into()),
        }
    }

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
    use crate::check::Kind;

    fn violation(kind: Kind) -> Violation {
        Violation {
            file: "a.rs".to_string(),
            line: 7,
            kind,
            detail: "d".to_string(),
        }
    }

    #[test]
    fn roundtrips_through_json() {
        let s = DoctestStatus::from_violations(vec![violation(Kind::NotRun)]);
        assert_eq!(DoctestStatus::from_json(&s.to_json()).unwrap(), s);
    }

    #[test]
    fn category_serializes_kebab_case() {
        let s = DoctestStatus::from_violations(Vec::new());
        assert!(s.to_json().contains("\"ok\""), "{}", s.to_json());
    }

    #[test]
    fn violation_kind_serializes_kebab_case() {
        // The gate derivation's jq prints `\(.kind)` straight into the failure
        // message, and the host renderer matches on the same spelling.
        let s = DoctestStatus::from_violations(vec![violation(Kind::NotRun)]);
        assert!(s.to_json().contains("\"not-run\""), "{}", s.to_json());
    }

    #[test]
    fn the_category_cannot_disagree_with_the_violation_list() {
        assert_eq!(
            DoctestStatus::from_violations(Vec::new()).category,
            StatusCategory::Ok
        );
        assert_eq!(
            DoctestStatus::from_violations(vec![violation(Kind::Failed)]).category,
            StatusCategory::Violations
        );
    }

    #[test]
    fn an_infra_status_carries_its_detail_and_no_violations() {
        let s = DoctestStatus::infra("could not spawn cargo");
        assert_eq!(s.category, StatusCategory::Infra);
        assert!(s.violations.is_empty());
        assert_eq!(s.infra_detail.as_deref(), Some("could not spawn cargo"));
    }
}
