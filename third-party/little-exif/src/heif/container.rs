// Copyright © 2025-2026 Tobias J. Prisching <tobias.prisching@icloud.com> and CONTRIBUTORS
// See https://github.com/TechnikTobi/little_exif#license for licensing details

use std::io::Cursor;
use std::io::Read;
use std::io::Seek;

use crate::general_file_io::EXIF_HEADER;
use crate::general_file_io::io_error;
use crate::heif::box_type::BoxType;
use crate::heif::boxes::item_location::ItemConstructionMethod;
use crate::heif::boxes::item_reference::ItemReferenceBox;
use crate::heif::boxes::meta::MetaBox;
use crate::heif::read_next_box;

use crate::metadata::Metadata;
use crate::util::insert_multiple_at;
use crate::util::range_remove;
use crate::util::read_be_u32;

use super::boxes::GenericIsoBox;
use super::boxes::item_info::ItemInfoBox;
use super::boxes::item_location::ItemLocationBox;

pub struct HeifContainer {
    boxes: Vec<Box<dyn GenericIsoBox>>,
}

// General structure of ISO container
//
// ┏━━━━━━┓
// ┃ ftyp ┃
// ┣━━━━━━┫
// ┃ meta ┃ ─> ┏━━━━━━┓
// ┗━━━━━━┛    ┃ hdlr ┃
//             ┣━━━━━━┫
//             ┃ pitm ┃
//             ┣━━━━━━┫
//             ┃ iinf ┃ ─> ┏━━━━━━┳━━━━━━━━━┓
//             ┗━━━━━━┛    ┃ infe ┃ ID|Exif ┃
//                         ┣━━━━━━╋━━━━━━━━━┫
//                         ┃ infe ┃ ID|XMP  ┃
//                         ┗━━━━━━┻━━━━━━━━━┛
//
//             ┏━━━━━━┳━━━┳━━━━━━━━━━━━━━━┓┉┉┉┏━━━━━━━━━━━━━┓
//             ┃ iloc ┃ n ┃ ID|from|lenID ┃   ┃ ID|from|len ┃
//             ┗━━━━━━┻━━━┻━━━━━━━━━━━━━━━┛┉┉┉┗━━━━━━━━━━━━━┛
//          ┌────────────────────┘
//          │  ┏━━━━━━┓
//          │  ┃ iref ┃
//          │  ┣━━━━━━┫
//          │  ┃ idat ┃
//          │  ┣━━━━━━┫
//          │  ┃ iprp ┃ ─> ┏━━━━━━┓
//          │  ┗━━━━━━┛    ┃ ipco ┃ ─> ┏━━━━━━┓
//          │              ┗━━━━━━┛    ┃ ispe ┃
//          │                          ┣━━━━━━╋━━━━━━━━━━━━━┓
//          │                          ┃ colr ┃ ICC profile ┃
//          │              ┏━━━━━━┓    ┗━━━━━━┻━━━━━━━━━━━━━┛
//          │              ┃ ipma ┃
//          │              ┗━━━━━━┛
//          └─>┏━━━━━━━━━━━━━━━━━━━━━━━┓
//             ┃ II*\0 ...             ┃
//             ┣━━━━━━━━━━━━━━━┳━━━━━━━┛
//             ┃ <?xpacket ... ┃
//             ┗━━━━━━━━━━━━━━━┛
//
// ┏━━━━━━┳━━━━━━━━━━┳━━━━━━━━━━┓
// ┃ mdat ┃ length64 ┃ data ... ┃
// ┗━━━━━━┻━━━━━━━━━━┻━━━━━━━━━━┛

impl HeifContainer {
    pub(super) fn construct_from_cursor_unboxed<T: Seek + Read>(
        cursor: &mut T,
    ) -> Result<Self, std::io::Error> {
        let mut boxes = Vec::new();

        let start = cursor.stream_position()?;
        let end = cursor.seek(std::io::SeekFrom::End(0))?;
        cursor.seek(std::io::SeekFrom::Start(start))?;
        while cursor.stream_position()? < end {
            let before = cursor.stream_position()?;
            let next_box = read_next_box(cursor)?;
            let size = next_box.get_header().get_box_size();
            let expected = before.checked_add(size).ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "container size overflow")
            })?;
            if size < next_box.get_header().get_header_size()
                || expected > end
                || cursor.stream_position()? != expected
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "incomplete native container box",
                ));
            }
            boxes.push(next_box);
        }

        return Ok(Self { boxes });
    }

    pub(super) fn metadata_regions(
        &self,
        file_len: usize,
    ) -> Result<Vec<(crate::HeifMetadataRole, Vec<std::ops::Range<usize>>)>, std::io::Error> {
        use std::io::{Error, ErrorKind};
        let invalid = || Error::new(ErrorKind::InvalidData, "invalid native metadata extent");
        let unsupported = || {
            Error::new(
                ErrorKind::Unsupported,
                "unsupported native metadata location",
            )
        };
        let meta = self.get_meta_box()?;
        let infos = meta.get_item_info_box()?;
        let locations = meta.get_item_location_box()?;
        if self
            .boxes
            .iter()
            .filter(|item| item.get_header().get_box_type() == BoxType::meta)
            .count()
            != 1
            || meta
                .other_boxes
                .iter()
                .filter(|item| item.get_header().get_box_type() == BoxType::iinf)
                .count()
                != 1
            || meta
                .other_boxes
                .iter()
                .filter(|item| item.get_header().get_box_type() == BoxType::iloc)
                .count()
                != 1
        {
            return Err(unsupported());
        }
        let mut mdat_ranges = Vec::new();
        let mut uuid_regions = Vec::new();
        let xmp_uuid = [
            0xbe, 0x7a, 0xcf, 0xcb, 0x97, 0xa9, 0x42, 0xe8, 0x9c, 0x71, 0x99, 0x94, 0x91, 0xe3,
            0xaf, 0xac,
        ];
        let mut meta_body_start = 0;
        let mut meta_end = 0;
        let mut offset = 0u64;
        for item in &self.boxes {
            let header = item.get_header();
            let end = offset
                .checked_add(header.get_box_size())
                .ok_or_else(invalid)?;
            if header.get_box_type() == BoxType::mdat {
                let start = offset
                    .checked_add(header.get_header_size())
                    .ok_or_else(invalid)?;
                mdat_ranges.push(start..end);
            }
            if header.get_box_type() == (BoxType::uuid { usertype: xmp_uuid }) {
                let start = offset
                    .checked_add(header.get_header_size())
                    .ok_or_else(invalid)?;
                uuid_regions.push((
                    crate::HeifMetadataRole::Xmp,
                    vec![
                        usize::try_from(start).map_err(|_| invalid())?
                            ..usize::try_from(end).map_err(|_| invalid())?,
                    ],
                ));
            }
            if header.get_box_type() == BoxType::meta {
                meta_body_start = offset
                    .checked_add(header.get_header_size())
                    .ok_or_else(invalid)?;
                meta_end = end;
            }
            offset = end;
        }
        if offset != file_len as u64 {
            return Err(invalid());
        }
        let mut idat_ranges = Vec::new();
        let mut sub_offset = meta_body_start
            .checked_add(meta.handler_size())
            .ok_or_else(invalid)?;
        for item in &meta.other_boxes {
            let header = item.get_header();
            let end = sub_offset
                .checked_add(header.get_box_size())
                .ok_or_else(invalid)?;
            if end > meta_end {
                return Err(invalid());
            }
            if header.get_box_type() == BoxType::idat {
                let start = sub_offset
                    .checked_add(header.get_header_size())
                    .ok_or_else(invalid)?;
                idat_ranges.push(start..end);
            }
            if header.get_box_type() == (BoxType::uuid { usertype: xmp_uuid }) {
                let start = sub_offset
                    .checked_add(header.get_header_size())
                    .ok_or_else(invalid)?;
                uuid_regions.push((
                    crate::HeifMetadataRole::Xmp,
                    vec![
                        usize::try_from(start).map_err(|_| invalid())?
                            ..usize::try_from(end).map_err(|_| invalid())?,
                    ],
                ));
            }
            sub_offset = end;
        }
        if idat_ranges.len() > 1 || sub_offset != meta_end {
            return Err(unsupported());
        }
        let mut ids = std::collections::HashSet::new();
        for info in &infos.items {
            if info.header_version() != 2 {
                return Err(unsupported());
            }
            if !ids.insert(info.item_id) {
                return Err(invalid());
            }
        }
        let mut location_ids = std::collections::HashSet::new();
        if locations.item_count as usize != locations.items.len()
            || infos.item_count != infos.items.len()
            || locations
                .items
                .iter()
                .any(|item| !location_ids.insert(item.item_id))
        {
            return Err(invalid());
        }
        let mut result = uuid_regions;
        for info in &infos.items {
            let kind = if info.item_name.starts_with("Exif") {
                crate::HeifMetadataRole::Exif
            } else if info.item_name.starts_with("mime") {
                crate::HeifMetadataRole::Xmp
            } else {
                continue;
            };
            if kind == crate::HeifMetadataRole::Xmp {
                let strings: Vec<_> = info.additional_data.split(|b| *b == 0).collect();
                if strings.len() < 2 {
                    return Err(Error::new(
                        ErrorKind::InvalidData,
                        "truncated native MIME descriptor",
                    ));
                }
                if strings[0] != b"application/rdf+xml" || !strings[1].is_empty() {
                    return Err(unsupported());
                }
            }
            if info.item_protection_index != 0 {
                return Err(unsupported());
            }
            let item = locations.get_item_location_entry(info.item_id)?;
            let (origin, allowed) = match item.get_construction_method() {
                ItemConstructionMethod::FILE => (0, &mdat_ranges),
                ItemConstructionMethod::IDAT => (
                    idat_ranges.first().ok_or_else(unsupported)?.start,
                    &idat_ranges,
                ),
                _ => return Err(unsupported()),
            };
            if item.reserved_and_construction_method >> 4 != 0
                || item.data_reference_index != 0
                || item.extents.is_empty()
                || item.extent_count as usize != item.extents.len()
            {
                return Err(unsupported());
            }
            let mut regions = Vec::new();
            for extent in &item.extents {
                if extent.extent_length == 0 {
                    continue;
                }
                let start = origin
                    .checked_add(item.base_offset)
                    .and_then(|base| base.checked_add(extent.extent_offset))
                    .ok_or_else(invalid)?;
                let end = start
                    .checked_add(extent.extent_length)
                    .ok_or_else(invalid)?;
                if start >= end
                    || end > file_len as u64
                    || !allowed
                        .iter()
                        .any(|range| range.start <= start && end <= range.end)
                {
                    return Err(invalid());
                }
                for other in &locations.items {
                    if other.item_id == u32::from(info.item_id) {
                        continue;
                    }
                    if other.data_reference_index != 0 {
                        continue;
                    }
                    let other_origin = match other.get_construction_method() {
                        ItemConstructionMethod::FILE => 0,
                        ItemConstructionMethod::IDAT => {
                            idat_ranges.first().ok_or_else(unsupported)?.start
                        }
                        _ => return Err(unsupported()),
                    };
                    for extent in &other.extents {
                        let a = other_origin
                            .checked_add(other.base_offset)
                            .and_then(|base| base.checked_add(extent.extent_offset))
                            .ok_or_else(invalid)?;
                        let b = a.checked_add(extent.extent_length).ok_or_else(invalid)?;
                        if a < b && start < b && a < end {
                            return Err(invalid());
                        }
                    }
                }
                regions.push(
                    usize::try_from(start).map_err(|_| invalid())?
                        ..usize::try_from(end).map_err(|_| invalid())?,
                );
            }
            if !regions.is_empty() {
                result.push((kind, regions));
            }
            if !info.name_span.is_empty() {
                let start = meta_body_start
                    .checked_add(info.name_span.start)
                    .ok_or_else(invalid)?;
                let end = meta_body_start
                    .checked_add(info.name_span.end)
                    .ok_or_else(invalid)?;
                if start > end || end > meta_end {
                    return Err(invalid());
                }
                result.push((
                    crate::HeifMetadataRole::Name,
                    vec![
                        usize::try_from(start).map_err(|_| invalid())?
                            ..usize::try_from(end).map_err(|_| invalid())?,
                    ],
                ));
            }
        }
        // No overlapping metadata extents may change the same physical bytes.
        let mut ordered: Vec<_> = result
            .iter()
            .flat_map(|(_, regions)| regions)
            .cloned()
            .collect();
        ordered.sort_by_key(|r| r.start);
        if ordered.windows(2).any(|w| w[0].end > w[1].start) {
            return Err(invalid());
        }
        Ok(result)
    }

    fn get_meta_box(&self) -> Result<&MetaBox, std::io::Error> {
        match self
            .boxes
            .iter()
            .find(|b| b.get_header().get_box_type() == BoxType::meta)
        {
            Some(b) => match b.as_any().downcast_ref::<MetaBox>() {
                Some(unboxed) => Ok(unboxed),
                None => io_error!(Other, "Found meta box but could not downcast to MetaBox"),
            },
            None => io_error!(Other, "No meta box found in HEIF container"),
        }
    }

    fn get_meta_box_mut(&mut self) -> Result<&mut MetaBox, std::io::Error> {
        match self
            .boxes
            .iter_mut()
            .find(|b| b.get_header().get_box_type() == BoxType::meta)
        {
            Some(b) => match b.as_any_mut().downcast_mut::<MetaBox>() {
                Some(unboxed) => Ok(unboxed),
                None => io_error!(
                    Other,
                    "Found meta box but could not downcast to MetaBox (mut)"
                ),
            },
            None => io_error!(Other, "No meta box found in HEIF container"),
        }
    }

    fn get_item_id_exif_data(&self) -> Result<u16, std::io::Error> {
        if let Ok(meta) = self.get_meta_box() {
            if let Some(item) = meta.get_item_info_box()?.get_exif_item() {
                return Ok(item.item_id);
            }
        }

        return io_error!(Other, "No EXIF item found!");
    }

    fn get_exif_data_pos_and_len(&self, exif_item_id: u16) -> Result<(u64, u64), std::io::Error> {
        let exif_item = match self.get_meta_box() {
            Ok(meta) => meta
                .get_item_location_box()?
                .get_item_location_entry(exif_item_id)?,
            Err(e) => return Err(e),
        };

        let exif_extents = &exif_item.extents;
        if exif_extents.len() != 1 {
            return io_error!(Other, "Expected exactly one EXIF extent info entry");
        }

        match exif_item.get_construction_method() {
            super::boxes::item_location::ItemConstructionMethod::FILE => {
                if let Some(first) = exif_extents.first() {
                    return Ok((
                        first.extent_offset + exif_item.base_offset,
                        first.extent_length,
                    ));
                } else {
                    return io_error!(Other, "Expected one EXIF extent");
                }
            }

            super::boxes::item_location::ItemConstructionMethod::IDAT => {
                return io_error!(
                    Other,
                    "HEIF: item constr. method 'IDAT' currently not supported. Please create a new ticket at https://github.com/TechnikTobi/little_exif with an example image file"
                );
            }

            super::boxes::item_location::ItemConstructionMethod::ITEM => {
                return io_error!(
                    Other,
                    "HEIF: item constr. method 'ITEM' currently not supported. Please create a new ticket at https://github.com/TechnikTobi/little_exif with an example image file"
                );
            }
        }
    }

    pub(super) fn get_exif_data<T: Seek + Read>(
        &self,
        cursor: &mut T,
    ) -> Result<Vec<u8>, std::io::Error> {
        // Locate exif data
        let exif_item_id = self.get_item_id_exif_data()?;
        let (start, length) = self.get_exif_data_pos_and_len(exif_item_id)?;

        // Reset cursor to start of exif data
        cursor.seek(std::io::SeekFrom::Start(start))?;

        // Read in the first 4 bytes, which gives the offset to the start
        // of the TIFF header and seek to that
        let exif_tiff_header_offset = read_be_u32(cursor)? as usize;

        cursor.seek(std::io::SeekFrom::Current(exif_tiff_header_offset as i64))?;

        if length < 4 + exif_tiff_header_offset as u64 {
            return io_error!(
                InvalidData,
                format!(
                    "EXIF data length ({}) is smaller than expected minimum size ({})",
                    length,
                    4 + exif_tiff_header_offset as u64
                )
            );
        }

        // Read in the remaining bytes
        let mut exif_buffer = vec![0u8;
            length as usize
            - 4                       // the 4 bytes that store the offset
            - exif_tiff_header_offset // the actual offset
        ];
        cursor.read_exact(&mut exif_buffer)?;

        // Stick a EXIF_HEADER in the front
        let mut full_exif_data = EXIF_HEADER.to_vec();
        full_exif_data.append(&mut exif_buffer);

        return Ok(full_exif_data);
    }

    /// Constructs a new version of the exif data area of the HEIF file
    /// the i64 tells us the delta in bytes. If negative, the new area is
    /// shorter than the old one, positive if longer
    #[allow(unused_assignments)]
    fn construct_new_exif_data_area<T: Seek + Read>(
        &self,
        cursor: &mut T,
        metadata: &Metadata,
    ) -> Result<(Vec<u8>, i64), std::io::Error> {
        // The buffer containing the new metadata that gets returned
        let mut new_exif_buffer;

        // Try to locate the old exif data.
        let exif_item_id = self.get_item_id_exif_data()?;

        // Determine the start and length of the previous exif data area
        let (start, length) = self.get_exif_data_pos_and_len(exif_item_id)?;

        // If the length is zero, we assume that this is a previously newly
        // created exif data area, which requires special handling.
        // If the length is non-zero, there has been exif data before:
        if length > 0 {
            // Reset cursor to start of exif data
            cursor.seek(std::io::SeekFrom::Start(start))?;

            // Read in all of this area
            let mut exif_buffer = vec![0u8; length as usize];
            cursor.read_exact(&mut exif_buffer)?;

            // Decode the first 4 bytes, which tells us where to cut off the old
            // data and replace with the new one
            let mut local_cursor = Cursor::new(exif_buffer[0..4].to_vec());
            let exif_tiff_header_offset = read_be_u32(&mut local_cursor)?;

            // Cut off data, starting at the old TIFF header and replace with new
            new_exif_buffer = exif_buffer[0..exif_tiff_header_offset as usize + 4].to_vec();
        } else {
            // Create a new exif header, starting with an empty TIFF header.
            // new_exif_buffer = 0_u32.to_be_bytes().to_vec();
            new_exif_buffer = [0x00, 0x00, 0x00, 0x06].to_vec();
            new_exif_buffer.extend(EXIF_HEADER.to_vec());
        }

        // Append the encoded metadata to the old/newly created TIFF header and
        // compute the delta in length
        if !metadata.get_ifds().is_empty() {
            new_exif_buffer.append(&mut metadata.encode()?);
        }
        let delta = new_exif_buffer.len() as i64 - length as i64;

        return Ok((new_exif_buffer, delta));
    }

    fn get_start_address_for_new_exif_area(&self) -> u64 {
        // Assumes that the new exif area that gets created should start at the
        // end of the mdat area

        let mut byte_count = 0;

        for b in &self.boxes {
            byte_count += b.get_header().get_box_size();

            if b.get_header().get_box_type() == BoxType::mdat {
                return byte_count as u64;
            }
        }

        return u64::MAX;
    }

    pub(super) fn generic_write_metadata(
        &mut self,
        file_buffer: &mut Vec<u8>,
        metadata: &Metadata,
    ) -> Result<(), std::io::Error> {
        // Find out where old exif is located, needed to determine which iloc
        // entries need to be updated
        let mut id = self.get_item_id_exif_data();

        // Check that the ID is okay - if not, there is no exif area yet and
        // we need to create one!
        if id.is_err() {
            // What we need to do at this point is
            // - Create a new item location entry that points to the EXIF data
            // - Create an item information entry that tells us that the iloc
            //   entry points to EXIF data
            // - Create an item reference entry that links the EXIF data to the
            //   image/iloc ID #1 -> is this always #1?

            // Where to put the new exif area
            let new_exif_start = self.get_start_address_for_new_exif_area();

            // If there is no iref box yet, create one so we can find one,
            // and get the size delta of the new box for extents
            let mut iref_size_delta = self
                .get_meta_box_mut()?
                .create_new_item_reference_box_if_none_exists_yet();

            // Acquire the item location, the item information and the item
            // reference boxes that are inside the meta box. For some reason,
            // this is not trivial - using e.g. get_item_location_box_mut()
            // does not work due to (according to the borrow checker) multiple
            // mutable usages of self
            let mut iloc_opt = None;
            let mut iinf_opt = None;
            let mut iref_opt = None;

            let meta_mut_ref = self.get_meta_box_mut()?;
            for other_box in &mut meta_mut_ref.other_boxes {
                if other_box.get_header().get_box_type() == BoxType::iloc {
                    iloc_opt = other_box.as_any_mut().downcast_mut::<ItemLocationBox>();
                } else if other_box.get_header().get_box_type() == BoxType::iinf {
                    iinf_opt = other_box.as_any_mut().downcast_mut::<ItemInfoBox>();
                } else if other_box.get_header().get_box_type() == BoxType::iref {
                    iref_opt = other_box.as_any_mut().downcast_mut::<ItemReferenceBox>();
                }
            }

            let iloc = match iloc_opt {
                Some(v) => v,
                None => return io_error!(Other, "iloc box should exist"),
            };
            let iinf = match iinf_opt {
                Some(v) => v,
                None => return io_error!(Other, "iinf box should exist"),
            };
            let iref = match iref_opt {
                Some(v) => v,
                None => return io_error!(Other, "iref box should exist"),
            };

            // Note that the given `new_exif_start` value is based on old
            // length values (which change due to adding a new item to both the
            // iloc and iinf boxes) - but this does not matter as this will
            // be updated anyway later by `add_to_extents`
            // This way, we don't need any exception during the update procedure
            let (new_iloc_id, iloc_size_delta) =
                iloc.create_new_item_location_entry(new_exif_start, 0);
            let iinf_size_delta = iinf.create_new_item_info_entry(new_iloc_id, "Exif");
            iref_size_delta += iref.create_new_single_item_reference_box(
                "cdsc", // TODO: Check if this is always this type?
                new_iloc_id,
                vec![1], // TODO: Check if this is always item #1?
            );

            // Fix the extents in the iloc box
            iloc.add_to_extents((iloc_size_delta + iinf_size_delta + iref_size_delta) as i64);

            // Fix up the size of the meta box as well
            let new_box_size = self.get_meta_box()?.serialize().len() as u64;
            self.get_meta_box_mut()?
                .get_header_mut()
                .set_box_size(new_box_size);

            // No change to the mdat data at this point as we set up the
            // iloc item so that the exif area currently has a length of zero

            // Now we have a valid exif area with an iloc ID!
            id = Ok(new_iloc_id as u16);
        }

        // Get position and length of current exif area
        let (old_exif_pos, old_exif_len) = match &id {
            Ok(idv) => self.get_exif_data_pos_and_len(*idv)?,
            Err(_) => (0, 0),
        };

        // Get cursor for file
        let mut cursor = Cursor::new(file_buffer);

        // Construct new exif data area
        let (mut new_exif_area, delta) =
            self.construct_new_exif_data_area(&mut cursor, metadata)?;

        let meta_mut = self.get_meta_box_mut()?;
        for item in &mut meta_mut.get_item_location_box_mut()?.items {
            // First, check if any extent of this item has the same offset as
            // the old exif data area. In that case, there must be only one
            // extent - other cases can't be handled right now
            if item
                .extents
                .iter()
                .any(|extent| item.base_offset + extent.extent_offset == old_exif_pos)
            {
                assert!(
                    item.extents.len() == 1,
                    "Expect to have exactly one extent info for EXIF!"
                );

                // In case of the EXIF extent information we need to update
                // the length information, not the offset!
                let first_extent = match item.extents.first() {
                    Some(f) => f,
                    None => return io_error!(Other, "Expected one extent for EXIF"),
                };
                let new_ext_len = (first_extent.extent_length as i64 + delta) as u64;
                match item.extents.first_mut() {
                    Some(fm) => fm.extent_length = new_ext_len,
                    None => return io_error!(Other, "Expected one extent for EXIF (mut)"),
                }

                continue;
            }

            if item.get_construction_method() == ItemConstructionMethod::IDAT {
                // In this case the offset information is relative to the
                // position of an idat box -> not affected by change in length
                // of another box
                continue;
            }

            if item.get_construction_method() == ItemConstructionMethod::ITEM {
                // Offset is relative to another item's extent
                // Also nothing to do here (for now...)
                continue;
            }

            if item.data_reference_index != 0 {
                // A value other than 0 implies that this extent refers to
                // another file, not this one, so we can also skip this
                // See ISO/IEC 14496-12:2015 § 8.11.3.1, p. 78
                continue;
            }

            if item.base_offset > delta.unsigned_abs() {
                // Potentially modify the entire base offset
                // however, we can only do that if all complete offsets
                // point to an area after the exif data area
                // So we need to check that first:
                if item
                    .extents
                    .iter()
                    .all(|extent| item.base_offset + extent.extent_offset >= old_exif_pos)
                {
                    item.base_offset = (item.base_offset as i64 + delta) as u64;
                    continue;
                }
            }

            // At this point we have no option left but to modify all
            // individual extent offsets
            for extent in &mut item.extents {
                let complete_offset = item.base_offset + extent.extent_offset;

                if complete_offset > old_exif_pos {
                    extent.extent_offset = (extent.extent_offset as i64 + delta) as u64;
                }
            }
        }

        // Now we clear the vec and write the boxes to it
        // Keep track of how many bytes were written so we know when to
        // replace old exif data with new
        cursor.get_mut().clear();

        let mut written_bytes = 0usize;
        let mut new_exif_written = false;
        let end_of_old_exif = (old_exif_pos + old_exif_len) as usize;

        for iso_box in &mut self.boxes {
            let mut serialized = iso_box.serialize();

            // If this box encompasses the exif data area, update its size and
            // serialize it again
            // TODO: As this is not the cleanest approach (e.g. what if the
            // exif area is not in this top level box but some nested box?
            // -> requires update of size fields of all boxes "downward") some
            // other solution needs to be found for this
            // In the meantime, this should work for the majority of HEIFs
            if written_bytes + serialized.len() >= end_of_old_exif && !new_exif_written {
                let new_size = (iso_box.get_header().get_box_size() as i64 + delta) as u64;
                iso_box.get_header_mut().set_box_size(new_size);
                serialized = iso_box.serialize();

                // Write the serialized box with the OLD exif data
                cursor.get_mut().extend(&serialized);

                // Remove old exif data
                range_remove(
                    cursor.get_mut(),
                    old_exif_pos as usize,
                    (old_exif_pos + old_exif_len) as usize,
                );

                // Insert new exif data
                insert_multiple_at(cursor.get_mut(), old_exif_pos as usize, &mut new_exif_area);

                new_exif_written = true;
            } else {
                // Just extend with the serialized box contents
                cursor.get_mut().extend(&serialized);
            }

            written_bytes += serialized.len();
        }

        return Ok(());
    }

    pub(super) fn generic_clear_metadata(
        &mut self,
        file_buffer: &mut Vec<u8>,
    ) -> Result<(), std::io::Error> {
        // Instead of truly clearing the metadata, just write an empty
        // exif data area
        // Based on what the macOS shortcut is doing, only keeps the tags
        // 0x0112: Orientation
        // 0x011A: XResolution
        // 0x011B: YResolution
        // 0x0128: ResolutionUnit

        // Note: It is up for debate whether keeping this information is wanted
        // or not/this should write a truly empty exif area

        // Create cursor
        let mut cursor = Cursor::new(file_buffer);

        // Read original metadata
        let orig_metadata = Metadata::general_decoding_wrapper(self.get_exif_data(&mut cursor))?;

        // Construct new metadata that only contains the above tags
        let mut new_metadata = Metadata::new();

        // 0x0112: Orientation
        if let Some(tag) = orig_metadata.get_tag_by_hex(0x0112, None).next() {
            new_metadata.set_tag(tag.clone());
        }

        // 0x011A: XResolution
        if let Some(tag) = orig_metadata.get_tag_by_hex(0x011A, None).next() {
            new_metadata.set_tag(tag.clone());
        }

        // 0x011A: YResolution
        if let Some(tag) = orig_metadata.get_tag_by_hex(0x011B, None).next() {
            new_metadata.set_tag(tag.clone());
        }

        // 0x0128: ResolutionUnit
        if let Some(tag) = orig_metadata.get_tag_by_hex(0x0128, None).next() {
            new_metadata.set_tag(tag.clone());
        }

        return self.generic_write_metadata(cursor.get_mut(), &new_metadata);
    }
}
