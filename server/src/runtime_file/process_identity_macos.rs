//! macOS process identity based on processkit's `proc_pidinfo` reader.

pub(crate) fn current_process_start_time() -> anyhow::Result<u64> {
    process_start_time(std::process::id())?
        .ok_or_else(|| anyhow::anyhow!("cannot read own process start-time"))
}

pub(crate) fn process_start_time(pid: u32) -> std::io::Result<Option<u64>> {
    let Some(info) = processkit::process_info(pid).map_err(std::io::Error::other)? else {
        return Ok(None);
    };
    info.start_time().map(Some).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "macOS process lookup did not return a start-time token",
        )
    })
}
