use std::{io, path::PathBuf};

/// Writes a file whose final path component is not valid UTF-8, when the host
/// filesystem can represent one.
///
/// Some developer hosts (notably common macOS/APFS configurations) reject such
/// byte sequences before application code can observe them. Tests for non-UTF-8
/// path handling should use this fixture helper to keep platform differences
/// out of individual product tests.
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

/// Runs `test` with a non-UTF-8 path fixture when the host filesystem supports
/// creating one.
///
/// # Errors
///
/// Returns filesystem errors other than the host rejecting the requested raw
/// filename bytes.
pub fn with_non_utf8_filename_fixture<T>(
    parent: &std::path::Path,
    bytes: &[u8],
    contents: &[u8],
    test: impl FnOnce(PathBuf) -> T,
) -> io::Result<Option<T>> {
    Ok(write_non_utf8_filename_fixture(parent, bytes, contents)?.map(test))
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

#[cfg(test)]
mod tests {
    use super::{with_non_utf8_filename_fixture, write_non_utf8_filename_fixture};
    use std::path::PathBuf;

    #[test]
    fn reports_none_when_host_rejects_raw_filename_bytes() {
        let parent = temp_dir("rejected-bytes");

        let path = write_non_utf8_filename_fixture(&parent, b"bad\0name", b"contents")
            .unwrap_or_else(|error| panic!("fixture result: {error}"));

        assert!(path.is_none());
    }

    #[test]
    fn runs_closure_when_host_accepts_raw_filename_bytes() {
        let parent = temp_dir("accepted-bytes");
        let mut closure_ran = false;

        let path = with_non_utf8_filename_fixture(&parent, b"valid-name", b"contents", |path| {
            closure_ran = true;
            assert_eq!(
                std::fs::read(&path).unwrap_or_else(|error| panic!("contents: {error}")),
                b"contents"
            );
            path
        })
        .unwrap_or_else(|error| panic!("fixture result: {error}"));

        if let Some(path) = path {
            assert!(closure_ran);
            assert_eq!(
                path.file_name().and_then(|name| name.to_str()),
                Some("valid-name")
            );
        } else {
            assert!(!closure_ran);
        }
    }

    #[test]
    fn skips_closure_when_host_rejects_raw_filename_bytes() {
        let parent = temp_dir("rejected-with-closure");

        let result = with_non_utf8_filename_fixture(&parent, b"bad\0name", b"contents", |_| {
            panic!("closure should not run when fixture cannot be created");
        })
        .unwrap_or_else(|error| panic!("fixture result: {error}"));

        assert!(result.is_none());
    }

    #[test]
    fn propagates_filesystem_errors_other_than_rejected_filename_bytes() {
        let parent = temp_dir("propagates-errors");
        let file_parent = parent.join("file-parent");
        std::fs::write(&file_parent, b"file")
            .unwrap_or_else(|error| panic!("file parent: {error}"));

        match write_non_utf8_filename_fixture(&file_parent, b"name", b"contents") {
            Err(error) => assert_eq!(error.kind(), std::io::ErrorKind::NotADirectory),
            Ok(None) => {}
            Ok(Some(path)) => panic!("fixture unexpectedly wrote under file parent: {path:?}"),
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "jaunder-common-filesystem-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir(&path).unwrap_or_else(|error| panic!("temp dir: {error}"));
        path
    }
}
