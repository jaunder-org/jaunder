//! Validates the checked-in issue #58 inventory and its delivery ledger.
//!
//! This deliberately parses only the narrow Markdown contract below. It is not
//! a Rust-source scanner and never turns lexical error-handling spellings into an
//! allowlist.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::result::{CommandResult, StepResult};

const STEP: &str = "error-swallowing-inventory";
const INVENTORY: &str = "docs/superpowers/specs/2026-08-13-issue-58-error-swallowing-inventory.md";
const BASELINE_COUNT: usize = 88;
const KEY_SEPARATOR: &str = " ␟ ";

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Key {
    path: String,
    symbol: String,
    expression: String,
}

impl Key {
    fn display(&self) -> String {
        format!(
            "{}{}{}{}{}",
            self.path, KEY_SEPARATOR, self.symbol, KEY_SEPARATOR, self.expression
        )
    }
}

fn section<'a>(markdown: &'a str, heading: &str) -> Result<&'a str, String> {
    let start = markdown
        .find(heading)
        .ok_or_else(|| format!("missing `{heading}` section"))?;
    let body = &markdown[start + heading.len()..];
    let end = body.find("\n## ").map_or(body.len(), |offset| offset + 1);
    Ok(&body[..end])
}

fn table(section: &str, label: &str) -> Result<Vec<Vec<String>>, String> {
    let mut lines = section
        .lines()
        .filter(|line| line.trim_start().starts_with('|'));
    let header = lines
        .next()
        .ok_or_else(|| format!("{label}: missing Markdown table"))?;
    let width = cells(header).len();
    let separator = lines
        .next()
        .ok_or_else(|| format!("{label}: missing Markdown table separator"))?;
    if cells(separator).len() != width {
        return Err(format!("{label}: malformed Markdown table separator"));
    }

    lines
        .map(|line| {
            let row = cells(line);
            if row.len() != width {
                Err(format!(
                    "{label}: table row has {} cells; expected {width}: {line}",
                    row.len()
                ))
            } else {
                Ok(row)
            }
        })
        .collect()
}

fn cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_owned())
        .collect()
}

fn code_text(cell: &str) -> String {
    cell.trim()
        .strip_prefix("<code>")
        .and_then(|value| value.strip_suffix("</code>"))
        .unwrap_or(cell.trim())
        .to_owned()
}

fn source_path(cell: &str) -> String {
    let source = code_text(cell);
    source
        .rsplit_once(':')
        .filter(|(_, suffix)| {
            !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit() || ch == ',')
        })
        .map_or(source.clone(), |(path, _)| path.to_owned())
}

fn key_from_cells(row: &[String], offset: usize) -> Result<Key, String> {
    let fields = row
        .get(offset..offset + 3)
        .ok_or_else(|| "row is missing key fields".to_owned())?;
    Ok(Key {
        path: if offset == 1 {
            source_path(&fields[0])
        } else {
            code_text(&fields[0])
        },
        symbol: code_text(&fields[1]),
        expression: code_text(&fields[2]),
    })
}

fn blank(value: &str) -> bool {
    let value = value.trim();
    let value = value
        .strip_prefix("<code>")
        .and_then(|value| value.strip_suffix("</code>"))
        .unwrap_or(value)
        .trim()
        .trim_matches('*')
        .trim();
    value.is_empty() || matches!(value, "—" | "-")
}

fn disposition(value: &str) -> &str {
    value.trim().trim_matches('*')
}

fn inventory_rows(markdown: &str) -> Result<HashMap<Key, Vec<String>>, String> {
    let mut rows = HashMap::new();
    for (heading, label) in [
        ("## Classified recipe union", "classified recipe union"),
        (
            "## Manual high-risk flow conclusions",
            "manual high-risk flow conclusions",
        ),
    ] {
        for row in table(section(markdown, heading)?, label)? {
            if row.len() != 12 {
                return Err(format!("{label}: expected 12 columns, found {}", row.len()));
            }
            let key = key_from_cells(&row, 1)?;
            if rows.insert(key.clone(), row).is_some() {
                return Err(format!("duplicate final inventory key `{}`", key.display()));
            }
        }
    }
    Ok(rows)
}

fn baseline_keys(markdown: &str) -> Result<Vec<Key>, String> {
    let rows = table(
        section(markdown, "## Frozen baseline remediation keys")?,
        "frozen baseline remediation keys",
    )?;
    rows.into_iter()
        .map(|row| {
            if row.len() != 3 {
                return Err(format!(
                    "frozen baseline remediation keys: expected 3 columns, found {}",
                    row.len()
                ));
            }
            key_from_cells(&row, 0)
        })
        .collect()
}

fn validate(markdown: &str, expected_baseline_count: usize) -> Vec<String> {
    let mut problems = Vec::new();
    if markdown.contains("controller amendment required") {
        problems.push("pending controller-amendment marker remains".to_owned());
    }

    let final_rows = match inventory_rows(markdown) {
        Ok(rows) => rows,
        Err(error) => {
            problems.push(error);
            return problems;
        }
    };
    for (key, row) in &final_rows {
        if disposition(&row[5]) == "continued" {
            for (index, field) in [
                (7, "static context"),
                (8, "reporting site"),
                (9, "continued-result rationale"),
                (10, "behavioral proof"),
                (11, "primary result"),
            ] {
                if blank(&row[index]) {
                    problems.push(format!(
                        "final continued row `{}` has blank {field}",
                        key.display()
                    ));
                }
            }
        }
    }

    let baseline = match baseline_keys(markdown) {
        Ok(keys) => keys,
        Err(error) => {
            problems.push(error);
            return problems;
        }
    };
    if baseline.len() != expected_baseline_count {
        problems.push(format!(
            "frozen baseline contains {} keys; expected {expected_baseline_count}",
            baseline.len()
        ));
    }
    let mut baseline_set = HashSet::new();
    for key in baseline {
        if !baseline_set.insert(key.clone()) {
            problems.push(format!("duplicate frozen baseline key `{}`", key.display()));
        }
    }

    let ledger = match table(
        match section(markdown, "## Delivered remediation ledger") {
            Ok(section) => section,
            Err(error) => {
                problems.push(error);
                return problems;
            }
        },
        "delivered remediation ledger",
    ) {
        Ok(rows) => rows,
        Err(error) => {
            problems.push(error);
            return problems;
        }
    };

    let mut delivered = HashSet::new();
    for row in ledger {
        if row.len() != 11 {
            problems.push(format!(
                "delivered remediation ledger: expected 11 columns, found {}",
                row.len()
            ));
            continue;
        }
        let key = match key_from_cells(&row, 0) {
            Ok(key) => key,
            Err(error) => {
                problems.push(error);
                continue;
            }
        };
        if !baseline_set.contains(&key) {
            problems.push(format!("unknown baseline key `{}`", key.display()));
        }
        if !delivered.insert(key.clone()) {
            problems.push(format!(
                "duplicate delivered baseline key `{}`",
                key.display()
            ));
        }

        for (index, field) in [
            (3, "final disposition"),
            (4, "owning task"),
            (5, "commit"),
            (6, "behavioral test"),
            (7, "command"),
            (8, "final outcome"),
        ] {
            if blank(&row[index]) {
                problems.push(format!(
                    "delivered baseline key `{}` has blank {field}",
                    key.display()
                ));
            }
        }

        let outcome = row[8].trim();
        if let Some(final_key) = outcome.strip_prefix("row:") {
            if !final_rows
                .keys()
                .any(|key| key.display() == final_key.trim())
            {
                problems.push(format!(
                    "delivered baseline key `{}` has dangling final row `{}`",
                    key.display(),
                    final_key.trim()
                ));
            }
        } else if outcome
            .strip_prefix("removed:")
            .is_none_or(|reason| reason.trim().is_empty())
        {
            problems.push(format!(
                "delivered baseline key `{}` needs `row:<final key>` or `removed:<reason>`",
                key.display()
            ));
        }

        if disposition(&row[3]) == "continued" {
            for (index, field) in [(9, "final reporter"), (10, "primary result")] {
                if blank(&row[index]) {
                    problems.push(format!(
                        "continued delivered baseline key `{}` has blank {field}",
                        key.display()
                    ));
                }
            }
        }
    }

    for missing in baseline_set.difference(&delivered) {
        problems.push(format!(
            "frozen baseline key `{}` is absent from the delivered ledger",
            missing.display()
        ));
    }
    problems
}

fn step_for(markdown: &str, expected_baseline_count: usize) -> StepResult {
    let problems = validate(markdown, expected_baseline_count);
    if problems.is_empty() {
        StepResult::ok(STEP)
    } else {
        StepResult::fail(STEP).detail(format!(
            "{}\n  recovery: reconcile `{INVENTORY}`; do not weaken the frozen baseline",
            problems.join("\n")
        ))
    }
}

/// Validate the checked-in classified inventory and delivery ledger.
pub fn run(result: &mut CommandResult) {
    let step = match std::fs::read_to_string(Path::new(INVENTORY)) {
        Ok(markdown) => step_for(&markdown, BASELINE_COUNT),
        Err(error) => StepResult::fail(STEP).detail(format!("cannot read {INVENTORY}: {error}")),
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "src/example.rs";
    const SYMBOL: &str = "example";
    const EXPR: &str = "operation()?";

    fn fixture(disposition: &str, outcome: &str) -> String {
        format!(
            "# fixture\n\
             \n\
             ## Classified recipe union\n\
             \n\
             | ID | Source | Containing symbol | Normalized expression | Recipe membership | Disposition | Classification rationale | Static context | Current reporting site | Continued-result rationale | Behavioral proof | Primary result preserved |\n\
             | -- | -- | -- | -- | -- | -- | -- | -- | -- | -- | -- | -- |\n\
             | R001 | <code>{PATH}:1</code> | <code>{SYMBOL}</code> | <code>{EXPR}</code> | map-err | **{disposition}** | rationale | context | reporter | continuation | exact_test | primary |\n\
             \n\
             ## Manual high-risk flow conclusions\n\
             \n\
             | ID | Source | Containing symbol | Normalized expression | Evidence family | Disposition | Classification rationale | Static context | Current reporting site | Continued-result rationale | Behavioral proof | Primary result preserved |\n\
             | -- | -- | -- | -- | -- | -- | -- | -- | -- | -- | -- | -- |\n\
             \n\
             ## Frozen baseline remediation keys\n\
             \n\
             | Path | Containing symbol | Normalized expression |\n\
             | -- | -- | -- |\n\
             | <code>{PATH}</code> | <code>{SYMBOL}</code> | <code>{EXPR}</code> |\n\
             \n\
             ## Delivered remediation ledger\n\
             \n\
             | Baseline path | Baseline containing symbol | Baseline normalized expression | Final disposition | Owning task | Commit | Behavioral test | Command | Final outcome | Final reporter | Primary result |\n\
             | -- | -- | -- | -- | -- | -- | -- | -- | -- | -- | -- |\n\
             | <code>{PATH}</code> | <code>{SYMBOL}</code> | <code>{EXPR}</code> | **{disposition}** | Task 2 | <code>abc123</code> | <code>exact_test</code> | <code>cargo test exact_test</code> | {outcome} | reporter | primary |\n"
        )
    }

    fn row_outcome() -> String {
        format!("row:{PATH}{KEY_SEPARATOR}{SYMBOL}{KEY_SEPARATOR}{EXPR}")
    }

    #[test]
    fn error_swallowing_inventory_rejects_missing_baseline_key() {
        let markdown = fixture("propagated", &row_outcome()).replace(
            &format!(
                "| <code>{PATH}</code> | <code>{SYMBOL}</code> | <code>{EXPR}</code> | **propagated** | Task 2 | <code>abc123</code> | <code>exact_test</code> | <code>cargo test exact_test</code> | {} | reporter | primary |\n",
                row_outcome()
            ),
            "",
        );
        assert!(
            validate(&markdown, 1)
                .iter()
                .any(|problem| problem.contains("absent from the delivered ledger"))
        );
    }

    #[test]
    fn error_swallowing_inventory_rejects_duplicate_baseline_key() {
        let row = format!(
            "| <code>{PATH}</code> | <code>{SYMBOL}</code> | <code>{EXPR}</code> | **propagated** | Task 2 | <code>abc123</code> | <code>exact_test</code> | <code>cargo test exact_test</code> | {} | reporter | primary |\n",
            row_outcome()
        );
        let markdown = fixture("propagated", &row_outcome()).replace(&row, &(row.clone() + &row));
        assert!(
            validate(&markdown, 1)
                .iter()
                .any(|problem| problem.contains("duplicate delivered baseline key"))
        );
    }

    #[test]
    fn error_swallowing_inventory_rejects_unknown_baseline_key() {
        let markdown = fixture("propagated", &row_outcome()).replace(
            &format!("<code>{EXPR}</code> | **propagated** | Task 2"),
            "<code>unknown()</code> | **propagated** | Task 2",
        );
        assert!(
            validate(&markdown, 1)
                .iter()
                .any(|problem| problem.contains("unknown baseline key"))
        );
    }

    #[test]
    fn error_swallowing_inventory_rejects_blank_delivery_fields() {
        let outcome = row_outcome();
        for (value, field) in [
            ("Task 2", "owning task"),
            ("abc123", "commit"),
            ("exact_test", "behavioral test"),
            ("cargo test exact_test", "command"),
            (outcome.as_str(), "final outcome"),
        ] {
            let markdown = fixture("propagated", &outcome).replace(value, "—");
            assert!(
                validate(&markdown, 1)
                    .iter()
                    .any(|problem| problem.contains(&format!("blank {field}"))),
                "field {field}"
            );
        }
    }

    #[test]
    fn error_swallowing_inventory_rejects_dangling_row_reference() {
        let markdown = fixture(
            "propagated",
            &format!("row:elsewhere{KEY_SEPARATOR}other{KEY_SEPARATOR}missing()"),
        );
        assert!(
            validate(&markdown, 1)
                .iter()
                .any(|problem| problem.contains("dangling final row"))
        );
    }

    #[test]
    fn error_swallowing_inventory_rejects_continued_delivery_proof_gaps() {
        for field in ["reporter", "primary"] {
            let markdown = fixture("continued", &row_outcome()).replace(field, "—");
            assert!(validate(&markdown, 1).iter().any(|problem| {
                problem.contains("continued delivered baseline key") && problem.contains("blank")
            }));
        }
    }

    #[test]
    fn error_swallowing_inventory_rejects_final_continued_proof_gap() {
        let markdown = fixture("continued", &row_outcome()).replacen(
            "| rationale | context | reporter | continuation | exact_test | primary |",
            "| rationale | context | — | continuation | exact_test | primary |",
            1,
        );
        assert!(
            validate(&markdown, 1)
                .iter()
                .any(|problem| problem.contains("final continued row")
                    && problem.contains("blank reporting site"))
        );
    }

    #[test]
    fn error_swallowing_inventory_rejects_surviving_amendment_marker() {
        let markdown = format!(
            "{}\ncontroller amendment required\n",
            fixture("propagated", &row_outcome())
        );
        assert!(
            validate(&markdown, 1)
                .iter()
                .any(|problem| problem.contains("pending controller-amendment marker"))
        );
    }

    #[test]
    fn error_swallowing_inventory_accepts_matching_row_outcome() {
        assert!(validate(&fixture("propagated", &row_outcome()), 1).is_empty());
    }

    #[test]
    fn error_swallowing_inventory_accepts_justified_removal_outcome() {
        assert!(
            validate(
                &fixture(
                    "propagated",
                    "removed:typed propagation removed the selected spelling"
                ),
                1
            )
            .is_empty()
        );
    }

    #[test]
    fn error_swallowing_inventory_invalid_document_produces_failed_step() {
        let step = step_for("not an inventory", 1);
        assert!(!step.ok);
        assert_eq!(step.name, STEP);
        assert!(step.detail.is_some());
    }
}
