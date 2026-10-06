use std::{ffi::OsString, io, os::windows::ffi::OsStringExt};

pub(super) fn filename() -> OsString {
    OsString::from_wide(&[0x0069, 0x006e, 0x0076, 0xd800])
}

pub(super) fn rejects_filename(error: &io::Error) -> bool {
    // ERROR_INVALID_NAME: the filesystem cannot represent our unpaired surrogate.
    error.raw_os_error() == Some(123)
}
