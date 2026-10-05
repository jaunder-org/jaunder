//! Linux `/proc/<pid>/stat` process start-time parsing.

use std::path::Path;

/// Field 22 (start-time, jiffies since boot) of a `/proc/<pid>/stat` line;
/// `InvalidData` if malformed — a malformed stat is a hard failure in every caller.
/// Field 2 (`comm`) is paren-wrapped and may contain spaces and `)`, so parse from
/// the **last** `)` (via `rsplit_once`, not slice-indexing, so it can never panic on
/// a char boundary); after it, `split_whitespace` coalesces the leading space and
/// start-time is index 19 (the 20th field after `comm`).
pub(crate) fn parse_stat_start_time(stat: &str) -> std::io::Result<u64> {
    stat.rsplit_once(')')
        .and_then(|(_, after)| after.split_whitespace().nth(19))
        .and_then(|field| field.parse().ok())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "unparseable /proc stat")
        })
}

/// Reads a start-time from `path`. `Ok(Some)` when it reads and parses; `Ok(None)`
/// when it does not exist (`NotFound` — a dead pid for `/proc/<pid>/stat`); `Err`
/// on any other I/O error **or** an unparseable read (the `/proc` mechanism is
/// unusable → the caller hard-fails). Path is a parameter so tests exercise every
/// arm with planted files.
pub(crate) fn read_start_time_at(path: &Path) -> std::io::Result<Option<u64>> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(Some(parse_stat_start_time(&s)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Reads a **required** start-time from a caller-supplied proc-stat path. This
/// Linux parser remains testable directly; startup uses `current_process_start_time`
/// so non-Linux platforms can provide their own process identity source.
#[cfg(test)]
pub(crate) fn require_start_time_at(path: &Path) -> anyhow::Result<u64> {
    read_start_time_at(path)?
        .ok_or_else(|| anyhow::anyhow!("cannot read own start-time from {}", path.display()))
}
