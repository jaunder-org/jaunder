use super::{platform, run_fixture, with_non_utf8_filename_fixture};
use std::{io, path::PathBuf};

#[test]
fn successful_write_invokes_assertions_with_the_written_path() {
    let path = PathBuf::from("fixture");
    let result = run_fixture(
        path.clone(),
        b"asset",
        |written_path, contents| {
            assert_eq!(written_path, path);
            assert_eq!(contents, b"asset");
            Ok(())
        },
        |_| panic!("successful write must not classify errors"),
        |written_path| {
            assert_eq!(written_path, path);
            42
        },
    )
    .expect("successful fixture");
    assert_eq!(result, Some(42));
}

#[test]
fn filename_rejection_does_not_invoke_assertions() {
    let result = run_fixture(
        PathBuf::from("fixture"),
        b"asset",
        |_, _| Err(io::Error::from_raw_os_error(92)),
        |error| {
            assert_eq!(error.raw_os_error(), Some(92));
            true
        },
        |_| panic!("rejected filename must not invoke assertions"),
    )
    .expect("filename rejection");
    assert!(result.is_none());
}

#[test]
fn ordinary_io_errors_are_preserved_and_do_not_invoke_assertions() {
    for kind in [
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::NotFound,
        io::ErrorKind::NotADirectory,
        io::ErrorKind::Unsupported,
        io::ErrorKind::InvalidInput,
    ] {
        let error = run_fixture(
            PathBuf::from("fixture"),
            b"asset",
            |_, _| Err(io::Error::new(kind, "original failure")),
            platform::rejects_filename,
            |_| panic!("failed write must not invoke assertions"),
        )
        .expect_err("I/O failure");
        assert_eq!(error.kind(), kind);
        assert_eq!(error.to_string(), "original failure");
    }
}

#[test]
fn filename_rejection_is_specific_to_the_platform_error_code() {
    assert_eq!(
        platform::rejects_filename(&io::Error::from_raw_os_error(92)),
        std::env::consts::OS == "macos"
    );
    assert_eq!(
        platform::rejects_filename(&io::Error::from_raw_os_error(123)),
        std::env::consts::OS == "windows"
    );
}

#[test]
fn native_filename_is_not_unicode_and_has_one_component() {
    let name = platform::filename();
    assert!(name.to_str().is_none());
    assert_eq!(std::path::Path::new(&name).components().count(), 1);
}

#[test]
fn real_fixture_writes_contents_or_reports_filename_rejection() {
    let parent = tempfile::tempdir().expect("fixture directory");
    let mut invoked = false;
    let result = with_non_utf8_filename_fixture(parent.path(), b"asset", |path| {
        invoked = true;
        assert!(path.file_name().is_some_and(|name| name.to_str().is_none()));
        assert_eq!(std::fs::read(path).expect("written fixture"), b"asset");
    })
    .expect("real fixture");
    assert_eq!(invoked, result.is_some());
}

#[test]
fn real_fixture_propagates_a_missing_parent() {
    let parent = tempfile::tempdir().expect("fixture directory");
    let error = with_non_utf8_filename_fixture(&parent.path().join("missing"), b"asset", |_| {
        panic!("missing parent must not invoke assertions");
    })
    .expect_err("missing parent");
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
}
