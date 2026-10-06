use std::{ffi::OsString, io, os::unix::ffi::OsStringExt};

pub(super) fn filename() -> OsString {
    OsString::from_vec(b"invalid-\xff.asset".to_vec())
}

pub(super) fn rejects_filename(error: &io::Error) -> bool {
    // Darwin reports EILSEQ (92) when APFS rejects an invalid UTF-8 filename.
    // On Linux, errno 92 means ENOPROTOOPT and must not be swallowed.
    std::env::consts::OS == "macos" && error.raw_os_error() == Some(92)
}
