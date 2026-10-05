//! Fallback for platforms without a reuse-safe process start-time reader.

pub(crate) fn current_process_start_time() -> anyhow::Result<u64> {
    process_start_time(std::process::id())?
        .ok_or_else(|| anyhow::anyhow!("cannot read own process start-time"))
}

pub(crate) fn process_start_time(_pid: u32) -> std::io::Result<Option<u64>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "process start-time identity is only implemented on Linux and macOS",
    ))
}
