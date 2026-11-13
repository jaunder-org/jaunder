use anyhow::Result;

// This is diagnostic metadata, not lock ownership. The kernel file lock remains
// authoritative on hosts without Linux's /proc process-start counter.
pub fn process_start_ticks(_pid: u32) -> Result<u64> {
    Ok(0)
}
