//! Non-Linux boundary for the identity-verified local graceful-shutdown command.

use std::time::Duration;

use anyhow::{Result, anyhow};

use crate::cli::StorageArgs;

pub(crate) fn cmd_shut_down(_storage: &StorageArgs, timeout: Duration) -> Result<()> {
    if timeout.is_zero() {
        return Err(anyhow!("timeout must be positive"));
    }
    Err(anyhow!(
        "graceful shutdown command is currently Linux-only because it requires pidfd process handles"
    ))
}
