//! Read-only ADR gates (ADR-0036 addendum, #196), siblings of
//! `identifier-collisions`:
//!
//! - **`adr-format`** — every `docs/adr/NNNN-*.md` matches the canonical heading
//!   (`# ADR-NNNN: <title>`) and status (`- Status: <token>`) style, with the
//!   token one of `NUMBERED_STATUS_VOCAB`. `proposed` is rejected outright: a
//!   numbered ADR has been accepted by construction, so a decision still under
//!   consideration belongs in `docs/adr/drafts/`. Logically upstream of parity: a
//!   malformed ADR can't be projected into a table row.
//! - **`adr-readme-parity`** — the README table's number/link/status cells and
//!   row set match `docs/adr/`. Titles are hand-owned and not compared.
//! - **`adr-view-parity`** — every accepted ADR is cited at least once in
//!   `docs/ARCHITECTURE.md`. No allowlist, no exemptions: an ADR nobody can
//!   describe in the view is an ADR the view is lying about.
//!
//! None mutates the tree; resolution is a guided manual fix (format), `cargo
//! xtask adr sync-readme` (README parity), or writing prose (view parity — there
//! is no mechanical recovery).

use std::path::Path;

use crate::adr_readme;
use crate::result::{CommandResult, StepResult};

/// Push the `adr-format`, `adr-readme-parity` and `adr-view-parity` steps.
pub fn run(result: &mut CommandResult) {
    result.push(format_step());
    result.push(parity_step());
    result.push(view_parity_step());
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

fn view_parity_step() -> StepResult {
    const RECOVERY: &str = "  recovery: none mechanical — describe each ADR above in the \
        relevant section of docs/ARCHITECTURE.md and cite it there, as a link to \
        `adr/NNNN-<slug>.md` or a bare `ADR-NNNN` token. There is no exemption list.";
    match adr_readme::view_parity_problems(Path::new(".")) {
        Ok(problems) if problems.is_empty() => StepResult::ok("adr-view-parity"),
        Ok(problems) => StepResult::fail("adr-view-parity")
            .detail(format!("{}\n{RECOVERY}", problems.join("\n"))),
        Err(e) => StepResult::fail("adr-view-parity").detail(format!("{e:#}\n{RECOVERY}")),
    }
}
