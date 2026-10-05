//! Linux process identity based on `/proc/<pid>/stat` start-time tokens.

use std::path::Path;

use super::proc_stat;

pub(crate) fn current_process_start_time() -> anyhow::Result<u64> {
    process_start_time(std::process::id())?
        .ok_or_else(|| anyhow::anyhow!("cannot read own process start-time"))
}

pub(crate) fn process_start_time(pid: u32) -> std::io::Result<Option<u64>> {
    proc_stat::read_start_time_at(Path::new(&format!("/proc/{pid}/stat")))
}
