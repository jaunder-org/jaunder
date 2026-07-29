//! `doc-links` — every relative Markdown link in the gated file set resolves on
//! disk (#682).
//!
//! Read-only. Unlike `adr-readme-parity` there is no `recovery:` line: the intended
//! target of a dead link is unknowable to the tool, so resolution is always a manual
//! fix.

use std::path::Path;

use crate::doc_links;
use crate::result::{CommandResult, StepResult};

/// Push the `doc-links` step.
pub fn run(result: &mut CommandResult) {
    result.push(match doc_links::problems(Path::new(".")) {
        Ok(problems) if problems.is_empty() => StepResult::ok("doc-links"),
        Ok(problems) => StepResult::fail("doc-links").detail(problems.join("\n")),
        Err(e) => StepResult::fail("doc-links").detail(format!("{e:#}")),
    });
}
