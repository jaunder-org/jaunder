//! OS-dependent lease safety, owner identity, and evidence permissions.
//! Selection happens here; workflow code uses one platform interface.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(super) use unix::*;

#[cfg(not(unix))]
mod non_unix;
#[cfg(not(unix))]
pub(super) use non_unix::*;

#[cfg(any(target_os = "linux", target_os = "android"))]
mod procfs;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub(super) use procfs::process_start_ticks;

#[cfg(not(any(target_os = "linux", target_os = "android")))]
mod process_identity;
#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub(super) use process_identity::process_start_ticks;
