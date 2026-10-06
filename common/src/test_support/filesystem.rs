use std::{io, path::PathBuf};

/// Writes a file whose final path component is not valid UTF-8, when the host
/// filesystem can represent one.
///
/// Some developer hosts (notably common macOS/APFS configurations) reject such
/// byte sequences before application code can observe them. Tests for non-UTF-8
/// path handling should call this fixture helper and return early when it yields
/// `Ok(None)` rather than scattering platform cfgs across product modules.
///
/// # Errors
///
/// Returns filesystem errors other than the host rejecting the requested raw
/// filename bytes.
pub fn write_non_utf8_filename_fixture(
    parent: &std::path::Path,
    bytes: &[u8],
    contents: &[u8],
) -> io::Result<Option<PathBuf>> {
    platform::write(parent, bytes, contents)
}

#[cfg(unix)]
mod platform {
    use super::{PathBuf, io};
    use std::os::unix::ffi::OsStringExt;

    pub(super) fn write(
        parent: &std::path::Path,
        bytes: &[u8],
        contents: &[u8],
    ) -> io::Result<Option<PathBuf>> {
        let path = parent.join(std::ffi::OsString::from_vec(bytes.to_vec()));
        match std::fs::write(&path, contents) {
            Ok(()) => Ok(Some(path)),
            Err(error) if filesystem_rejected_filename_bytes(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn filesystem_rejected_filename_bytes(error: &io::Error) -> bool {
        matches!(
            error.kind(),
            io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
        ) || error.raw_os_error() == Some(92)
    }
}

#[cfg(not(unix))]
mod platform {
    use super::{PathBuf, io};

    pub(super) fn write(
        _parent: &std::path::Path,
        _bytes: &[u8],
        _contents: &[u8],
    ) -> io::Result<Option<PathBuf>> {
        Ok(None)
    }
}
