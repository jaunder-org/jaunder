//! Read-only ADR gates (ADR-0036 addendum, #196), siblings of
//! `identifier-collisions`:
//!
//! - **`adr-format`** — every `docs/adr/NNNN-*.md` matches the canonical heading
//!   (`# ADR-NNNN: <title>`) and status (`- Status: <token>`) style. Logically
//!   upstream of parity: a malformed ADR can't be projected into a table row.
//! - **`adr-readme-parity`** — the README table's number/link/status cells and
//!   row set match `docs/adr/`. Titles are hand-owned and not compared.
//!
//! Neither mutates the tree; resolution is a guided manual fix (format) or
//! `cargo xtask adr sync-readme` (parity).

use std::path::Path;

use crate::adr_readme;
use crate::result::{CommandResult, StepResult};

/// Push the `adr-format` and `adr-readme-parity` steps.
pub fn run(result: &mut CommandResult) {
    result.push(format_step());
    result.push(parity_step());
}

fn format_step() -> StepResult {
    let problems = adr_readme::format_problems(Path::new("."));
    if problems.is_empty() {
        StepResult::ok("adr-format")
    } else {
        StepResult::fail("adr-format").detail(problems.join("\n"))
    }
}

fn parity_step() -> StepResult {
    const RECOVERY: &str = "  recovery: cargo xtask adr sync-readme";
    match adr_readme::parity_report(Path::new(".")) {
        Ok(problems) if problems.is_empty() => StepResult::ok("adr-readme-parity"),
        Ok(problems) => StepResult::fail("adr-readme-parity")
            .detail(format!("{}\n{RECOVERY}", problems.join("\n"))),
        Err(e) => StepResult::fail("adr-readme-parity").detail(format!("{e:#}\n{RECOVERY}")),
    }
}
