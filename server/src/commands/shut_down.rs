//! Identity-verified local graceful-shutdown command.

#[cfg(target_os = "linux")]
#[path = "shut_down/linux.rs"]
mod imp;
#[cfg(not(target_os = "linux"))]
#[path = "shut_down/non_linux.rs"]
mod imp;

pub(super) use imp::cmd_shut_down;
