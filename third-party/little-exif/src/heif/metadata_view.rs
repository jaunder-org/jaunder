// Jaunder additions; SPDX-License-Identifier: GPL-3.0-only
// Upstream little_exif grants and notices are preserved in this dependency.

use super::container::HeifContainer;
use std::io::Cursor;

/// The meaning of a checked, replaceable HEIF metadata extent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeifMetadataRole {
    /// An EXIF offset prefix and TIFF metadata payload.
    Exif,
    /// An Adobe XMP packet in a MIME item or UUID box.
    Xmp,
    /// A descriptive item name, excluding its type and terminating NUL.
    Name,
}

pub(crate) fn metadata_regions(
    data: &[u8],
) -> Result<Vec<(HeifMetadataRole, Vec<std::ops::Range<usize>>)>, std::io::Error> {
    let mut cursor = Cursor::new(data);
    let container = HeifContainer::construct_from_cursor_unboxed(&mut cursor)?;
    container.metadata_regions(data.len())
}
