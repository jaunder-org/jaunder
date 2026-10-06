use anyhow::{Context, Result};
use std::fs;

pub fn process_start_ticks(pid: u32) -> Result<u64> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat"))
        .with_context(|| format!("reading owner process {pid}"))?;
    let close = text.rfind(')').context("malformed /proc stat")?;
    text[close + 2..]
        .split_whitespace()
        .nth(19)
        .context("missing /proc process start time")?
        .parse()
        .context("parsing /proc process start time")
}
