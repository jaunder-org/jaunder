//! Compatibility entry points; Python owns every promotion mutation.

use std::path::Path;

use anyhow::Result;

use super::delegate::{Action, output};
use crate::result::StepResult;

pub fn promote() -> StepResult {
    match run_promote(Path::new(".")) {
        Ok(summary) => StepResult::ok("adr-promote").detail(summary),
        Err(error) => StepResult::fail("adr-promote").detail(format!("{error:#}")),
    }
}

/// The existing Rust controller uses the same fixed Python action as the CLI.
pub(crate) fn run_promote(repo: &Path) -> Result<String> {
    output(repo, Action::Promote)
}
