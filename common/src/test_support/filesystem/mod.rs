use std::{io, path::PathBuf};

// Platform selection lives only at the module boundary. The fixture lifecycle
// and its tests do not depend on platform-specific compilation.
#[cfg(unix)]
#[path = "unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;
#[cfg(not(any(unix, windows)))]
compile_error!("non-Unicode filename fixtures require Unix or Windows path encoding");

/// Writes a platform-native filename that cannot be converted to a Rust `str`,
/// then runs `test` with its path.
///
/// Unix uses an invalid UTF-8 byte; Windows uses an unpaired UTF-16 surrogate.
/// The caller owns the parent directory and cleanup. If the filesystem rejects
/// this particular filename, returns `Ok(None)` and reports the skipped fixture.
/// A successful write always invokes the assertion closure.
///
/// # Errors
///
/// Propagates ordinary I/O failures (including missing parents and permissions).
/// Only filename-representation rejection
/// permits skipping the assertion closure.
pub fn with_non_utf8_filename_fixture<T>(
    parent: &std::path::Path,
    contents: &[u8],
    test: impl FnOnce(PathBuf) -> T,
) -> io::Result<Option<T>> {
    let path = parent.join(platform::filename());
    run_fixture(
        path,
        contents,
        |path, contents| std::fs::write(path, contents),
        platform::rejects_filename,
        test,
    )
}

fn run_fixture<T>(
    path: PathBuf,
    contents: &[u8],
    write: impl FnOnce(&std::path::Path, &[u8]) -> io::Result<()>,
    rejects_filename: impl FnOnce(&io::Error) -> bool,
    test: impl FnOnce(PathBuf) -> T,
) -> io::Result<Option<T>> {
    match write(&path, contents) {
        Ok(()) => Ok(Some(test(path))),
        Err(error) if rejects_filename(&error) => {
            eprintln!(
                "non-Unicode filename fixture unavailable at {}: {error}",
                path.display()
            );
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests;
