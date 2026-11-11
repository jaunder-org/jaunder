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
//! None mutates the tree; resolution is a guided manual fix (format), the
//! selected Python package's `sync-readme` (README parity), or writing prose
//! (view parity — there is no mechanical recovery).

use std::path::Path;

use crate::adr_readme;
use crate::result::StepResult;

pub(crate) fn readme_parity_step_at(root: &Path) -> StepResult {
    parity_step(adr_readme::parity_report(root))
}

pub(crate) fn parity_step(report: anyhow::Result<Vec<String>>) -> StepResult {
    const RECOVERY: &str = "  recovery: nix run --accept-flake-config --no-update-lock-file --no-write-lock-file .#adr-promoter -- sync-readme";
    match report {
        Ok(problems) if problems.is_empty() => StepResult::ok("adr-readme-parity"),
        Ok(problems) => StepResult::fail("adr-readme-parity")
            .detail(format!("{}\n{RECOVERY}", problems.join("\n"))),
        Err(e) => StepResult::fail("adr-readme-parity").detail(format!("{e:#}\n{RECOVERY}")),
    }
}

pub(crate) fn view_parity_step_at(root: &Path) -> StepResult {
    const RECOVERY: &str = "  recovery: none mechanical — describe each ADR above in the \
        relevant section of docs/ARCHITECTURE.md and cite it there, as a link to \
        `adr/NNNN-<slug>.md` or a bare `ADR-NNNN` token. There is no exemption list.";
    match adr_readme::view_parity_problems(root) {
        Ok(problems) if problems.is_empty() => StepResult::ok("adr-view-parity"),
        Ok(problems) => StepResult::fail("adr-view-parity")
            .detail(format!("{}\n{RECOVERY}", problems.join("\n"))),
        Err(e) => StepResult::fail("adr-view-parity").detail(format!("{e:#}\n{RECOVERY}")),
    }
}
