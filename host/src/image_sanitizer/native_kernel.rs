//! Native container editing preserves encoded frames and rendering signals.
//!
//! Privacy is an explicit metadata allowlist, not successful write status. Each
//! output is reparsed and checked before the worker can report success.
use exiftool_rs::{
    ExifTool, FileType, Tag,
    file_type::detect_from_magic,
    metadata::exif::ByteOrderMark,
    writer::{
        exif_writer::{self, ExifFormat, IfdEntry},
        jpeg_writer, webp_writer,
    },
};
use gif::streaming_decoder::{Block, Decoded, OutputBuffer, StreamingDecoder};
use img_parts::{
    ImageEXIF,
    jpeg::Jpeg,
    png::{Png, PngChunk},
    webp::WebP,
};
use little_exif::{HeifMetadataRole, metadata::Metadata};
use std::path::Path;

use super::native::{EditedBytes as Outcome, NativeImageError as Failure};

fn tags(bytes: &[u8]) -> Result<Vec<Tag>, Failure> {
    let tags = ExifTool::new().extract_info_from_bytes(bytes, Path::new("private"))?;
    if tags
        .iter()
        .any(|tag| matches!(tag.name.as_str(), "Warning" | "Error"))
    {
        return Err("native metadata reader reported invalid input".into());
    }
    Ok(tags)
}
fn orientation_exif(tags: &[Tag]) -> Result<Vec<u8>, Failure> {
    // Thumbnail and secondary IFD orientation describe discarded images, not
    // the primary raster. Only primary EXIF or its document-level XMP carrier
    // may become a rendering instruction in the regenerated primary directory.
    let orientation = tags
        .iter()
        .find(|tag| tag.name == "Orientation" && tag.group.family1 == "IFD0")
        .or_else(|| {
            tags.iter()
                .find(|tag| tag.name == "Orientation" && tag.group.family1 == "XMP-tiff")
        });
    let orientation = orientation
        .map(|tag| tag.raw_value.to_display_string().parse::<u16>())
        .transpose()?;
    if orientation.is_some_and(|value| !(1..=8).contains(&value)) {
        return Err("invalid orientation".into());
    }
    let entries: Vec<_> = orientation
        .into_iter()
        .map(|value| IfdEntry {
            tag: 0x0112,
            format: ExifFormat::Short,
            data: value.to_le_bytes().to_vec(),
        })
        .collect();
    Ok(exif_writer::build_exif(
        &entries,
        &[],
        &[],
        ByteOrderMark::LittleEndian,
    )?)
}
const RENDERING_EXIF: &[&str] = &[
    "Orientation",
    "ColorSpace",
    "Gamma",
    "WhitePoint",
    "PrimaryChromaticities",
    "TransferFunction",
    "YCbCrCoefficients",
    "YCbCrPositioning",
    "ReferenceBlackWhite",
    "InteropIndex",
    "InteropVersion",
];

fn rendering_exif(before: &[Tag], sources: &[Vec<u8>]) -> Result<Vec<u8>, Failure> {
    use little_exif::{filetype::FileExtension, ifd::ExifTagGroup};
    let mut clean = Metadata::new_from_vec(&orientation_exif(before)?, FileExtension::TIFF)?;
    let selected = [
        (
            ExifTagGroup::GENERIC,
            &[0x012d, 0x013e, 0x013f, 0x0211, 0x0213, 0x0214][..],
        ),
        (ExifTagGroup::EXIF, &[0xa001, 0xa500][..]),
        (ExifTagGroup::INTEROP, &[0x0001, 0x0002][..]),
    ];
    for bytes in sources {
        let source = Metadata::new_from_vec(bytes, FileExtension::TIFF)?;
        for (group, ids) in &selected {
            if let Some(ifd) = source.get_ifd(*group, 0) {
                for tag in ifd
                    .get_tags()
                    .iter()
                    .filter(|tag| ids.contains(&tag.as_u16()))
                {
                    clean.set_tag(tag.clone());
                }
            }
        }
    }
    Ok(clean.encode()?)
}
fn verify_clean(bytes: &[u8]) -> Result<(), Failure> {
    const PRIVATE: &[&str] = &[
        "Make",
        "Model",
        "Artist",
        "DateTimeOriginal",
        "CreateDate",
        "ModifyDate",
        "SerialNumber",
        "BodySerialNumber",
        "LensSerialNumber",
        "CameraOwnerName",
        "ImageUniqueID",
        "Comment",
        "Description",
        "ThumbnailImage",
        "PreviewImage",
        "GPSLatitude",
        "GPSLongitude",
    ];
    if tags(bytes)?.iter().any(|tag| {
        if tag.group.family0 == "ICC_Profile" {
            return false;
        }
        PRIVATE.contains(&tag.name.as_str())
            || matches!(tag.group.family0.as_str(), "XMP" | "IPTC") && tag.name != "XMPToolkit"
            || tag.group.family0 == "EXIF" && !RENDERING_EXIF.contains(&tag.name.as_str())
    }) {
        return Err("native post-edit privacy verification failed".into());
    }
    Ok(())
}

fn rendering_sources(
    data: &[u8],
    kind: FileType,
    before: &mut Vec<Tag>,
) -> Result<Vec<Vec<u8>>, Failure> {
    let mut sources = Vec::new();
    let container_exif = match kind {
        FileType::Jpeg => Jpeg::from_bytes(data.to_vec().into())?.exif(),
        FileType::Png => Png::from_bytes(data.to_vec().into())?.exif(),
        FileType::WebP => WebP::from_bytes(data.to_vec().into())?.exif(),
        _ => None,
    };
    if let Some(bytes) = container_exif {
        sources.push(bytes.to_vec());
    }
    if kind == FileType::Heif {
        // Locate TIFF through checked native extents: rendering fields may live
        // in named or IDAT carriers, not only the reader's default EXIF item.
        for (role, ranges) in Metadata::heif_metadata_regions(data)? {
            if role != HeifMetadataRole::Exif {
                continue;
            }
            let payload: Vec<_> = ranges
                .iter()
                .flat_map(|range| data[range.clone()].iter().copied())
                .collect();
            let prefix: [u8; 4] = payload
                .get(..4)
                .ok_or("truncated HEIF EXIF offset")?
                .try_into()?;
            let start = usize::try_from(u32::from_be_bytes(prefix))?
                .checked_add(4)
                .ok_or("EXIF offset overflow")?;
            let tiff = payload.get(start..).ok_or("invalid HEIF EXIF offset")?;
            before.extend(tags(tiff)?);
            sources.push(tiff.to_vec());
        }
    }
    Ok(sources)
}

pub(super) fn sanitize(
    data: &[u8],
    claims_raster: bool,
    max_bytes: usize,
) -> Result<Outcome, Failure> {
    if data.len() > max_bytes {
        return Err(Failure::FileTooLarge);
    }
    let Some(kind) = detect_from_magic(data) else {
        return if claims_raster {
            Err("unrecognized claimed raster".into())
        } else {
            Ok(Outcome::Passthrough(data.to_vec()))
        };
    };
    // SVG is explicitly outside raster metadata editing; byte detection wins
    // over a misleading raster filename or MIME claim at shared ingress.
    if kind == FileType::Svg {
        return Ok(Outcome::Passthrough(data.to_vec()));
    }
    if !matches!(
        kind,
        FileType::Jpeg | FileType::Png | FileType::Gif | FileType::WebP | FileType::Heif
    ) {
        if claims_raster
            || kind.mime_type().starts_with("image/") && kind.mime_type() != "image/svg+xml"
        {
            return Err("unsupported raster".into());
        }
        return Ok(Outcome::Passthrough(data.to_vec()));
    }
    let mut before = tags(data)?;
    let sources = rendering_sources(data, kind, &mut before)?;
    let exif = rendering_exif(&before, &sources)?;
    let mime = edited_mime(kind, &before);
    let output = edit_container(data, kind, exif)?;
    checked_output(output, kind, mime, max_bytes)
}

fn edit_container(data: &[u8], kind: FileType, exif: Vec<u8>) -> Result<Vec<u8>, Failure> {
    // Editing preserves encoded raster data; validation of the resulting
    // identity, size and privacy remains a separate mandatory phase.
    Ok(match kind {
        FileType::Jpeg => {
            let jpeg = Jpeg::from_bytes(data.to_vec().into())?;
            if jpeg.segment_by_marker(0xda).is_none() || !data.ends_with(&[0xff, 0xd9]) {
                return Err("incomplete JPEG image".into());
            }
            jpeg_writer::write_jpeg(data, Some(&exif), None, None, None, false, true, true, true)?
        }
        FileType::Png => {
            let mut png = Png::from_bytes(data.to_vec().into())?;
            if png.chunk_by_type(*b"IHDR").is_none()
                || png.chunk_by_type(*b"IDAT").is_none()
                || png.chunk_by_type(*b"IEND").is_none()
            {
                return Err("incomplete PNG image".into());
            }
            png.chunks_mut().retain(|chunk| {
                !matches!(
                    &chunk.kind(),
                    b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt" | b"tIME"
                )
            });
            png.chunks_mut()
                .insert(1, PngChunk::new(*b"eXIf", exif.into()));
            png.encoder().bytes().to_vec()
        }
        FileType::WebP => {
            let _container = WebP::from_bytes(data.to_vec().into())?;
            webp_writer::write_webp(data, Some(&exif), None, false, true)?
        }
        FileType::Gif => edit_gif(data)?,
        FileType::Heif => edit_heif(data, &exif)?,
        _ => return Err("unreachable covered format dispatch".into()),
    })
}

fn checked_output(
    output: Vec<u8>,
    kind: FileType,
    mime: &'static str,
    max_bytes: usize,
) -> Result<Outcome, Failure> {
    if output.len() > max_bytes {
        return Err(Failure::FileTooLarge);
    }
    if detect_from_magic(&output) != Some(kind) {
        return Err("edited MIME changed".into());
    }
    verify_clean(&output)?;
    Ok(Outcome::Image {
        bytes: output,
        mime,
    })
}
fn edited_mime(kind: FileType, before: &[Tag]) -> &'static str {
    // HEIF container identity alone does not imply HEIC. Compatible codec
    // brands refine the canonical MIME without overriding other raster kinds.
    if kind == FileType::Heif
        && before.iter().any(|tag| {
            tag.name == "CompatibleBrands"
                && matches!(&tag.raw_value, exiftool_rs::Value::List(values) if values.iter().any(|value|
                    matches!(value, exiftool_rs::Value::String(brand) if matches!(brand.as_str(), "heic" | "heix" | "hevc" | "hevx"))))
        })
    {
        "image/heic"
    } else {
        kind.mime_type()
    }
}

fn edit_heif(source: &[u8], tiff: &[u8]) -> Result<Vec<u8>, Failure> {
    let mut exif = vec![0, 0, 0, 0];
    exif.extend_from_slice(tiff);
    let packet = b"<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"/>";
    let mut output = source.to_vec();
    for (kind, group) in Metadata::heif_metadata_regions(source)? {
        let len = group.iter().try_fold(0usize, |sum, range| {
            sum.checked_add(range.len())
                .ok_or("metadata length overflow")
        })?;
        let replacement = match kind {
            HeifMetadataRole::Exif => &exif[..],
            HeifMetadataRole::Name => &[][..],
            HeifMetadataRole::Xmp => &packet[..],
        };
        if len < replacement.len() {
            return Err("metadata replacement does not fit extent".into());
        }
        let mut clean = vec![
            match kind {
                HeifMetadataRole::Exif => 0,
                HeifMetadataRole::Xmp | HeifMetadataRole::Name => b' ',
            };
            len
        ];
        clean[..replacement.len()].copy_from_slice(replacement);
        let mut offset = 0;
        for range in group {
            output[range.clone()].copy_from_slice(&clean[offset..offset + range.len()]);
            offset += range.len();
        }
    }
    Ok(output)
}
fn edit_gif(source: &[u8]) -> Result<Vec<u8>, Failure> {
    let mut decoder = StreamingDecoder::new();
    let mut pos = 0;
    let mut block_start = None;
    let mut application_allowed = false;
    let mut ranges = Vec::new();
    let mut pixels = vec![0_u8; 65536];
    let mut trailer = false;
    let mut frames = 0;
    while pos < source.len() {
        let (consumed, event) =
            decoder.update(&source[pos..], &mut OutputBuffer::Slice(&mut pixels))?;
        pos += consumed;
        match event {
            Decoded::BlockStart(Block::Extension) => {
                block_start = Some(pos.checked_sub(3).ok_or("missing extension introducer")?);
                application_allowed = false;
            }
            Decoded::SubBlock { ext, is_last } => {
                if ext.0 == 0xff
                    && !is_last
                    && matches!(
                        decoder.last_ext_sub_block(),
                        b"NETSCAPE2.0" | b"ANIMEXTS1.0" | b"ICCRGBG1012"
                    )
                {
                    application_allowed = true;
                }
                if is_last && (ext.0 == 0xfe || (ext.0 == 0xff && !application_allowed)) {
                    ranges.push((block_start.take().ok_or("missing extension start")?, pos));
                }
            }
            Decoded::DataEnd => frames += 1,
            Decoded::BlockStart(Block::Trailer) => {
                trailer = true;
                break;
            }
            _ => (),
        }
    }
    if !trailer || frames == 0 {
        return Err("incomplete GIF image".into());
    }
    let mut output = Vec::new();
    let mut start = 0;
    for (from, to) in ranges {
        output.extend_from_slice(&source[start..from]);
        start = to;
    }
    output.extend_from_slice(&source[start..pos]);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_dispatch_rejects_formats_outside_the_covered_raster_set() {
        // The editor's own boundary must reject accidental dispatch of a
        // passthrough format instead of rewriting or returning its bytes.
        assert!(matches!(
            edit_container(b"<svg/>", FileType::Svg, Vec::new()),
            Err(Failure::InvalidImage("unreachable covered format dispatch"))
        ));
    }

    #[test]
    fn post_edit_validation_rejects_changed_identity_before_privacy_check() {
        // A structurally valid PNG cannot certify a JPEG identity. Use the
        // original private fixture so MIME rejection cannot be confused with
        // the later, independently enforced privacy rejection.
        let png = include_bytes!("../image_sanitizer_fixtures/png-original.png").to_vec();
        let limit = png.len();
        assert!(matches!(
            checked_output(png, FileType::Jpeg, "image/jpeg", limit),
            Err(Failure::InvalidImage("edited MIME changed"))
        ));
    }

    #[test]
    fn canonical_mime_refinement_is_limited_to_heif_codec_brands() {
        // Keep generic HEIF distinct from HEIC without allowing unrelated
        // extracted metadata to override a byte-detected PNG identity.
        let heic_tags = tags(include_bytes!(
            "../image_sanitizer_fixtures/heic-original.heic"
        ))
        .expect("owned HEIC metadata");
        assert_eq!(edited_mime(FileType::Heif, &heic_tags), "image/heic");
        assert_eq!(edited_mime(FileType::Heif, &[]), "image/heif");
        assert_eq!(edited_mime(FileType::Png, &heic_tags), "image/png");
    }

    #[test]
    fn privacy_verification_rejects_original_private_metadata() {
        // Successful editing alone is insufficient: the independent post-edit
        // verifier must reject an original image still carrying private tags.
        assert!(matches!(
            verify_clean(include_bytes!(
                "../image_sanitizer_fixtures/png-original.png"
            )),
            Err(Failure::InvalidImage(
                "native post-edit privacy verification failed"
            ))
        ));
    }

    #[test]
    fn orientation_outside_rendering_domain_is_rejected() {
        // A primary orientation must be a real rendering instruction, not an
        // out-of-range value silently converted into the default orientation.
        for value in [0_u16, 9] {
            let tiff = exif_writer::build_exif(
                &[IfdEntry {
                    tag: 0x0112,
                    format: ExifFormat::Short,
                    data: value.to_le_bytes().to_vec(),
                }],
                &[],
                &[],
                ByteOrderMark::LittleEndian,
            )
            .expect("well-framed primary EXIF");
            let before = tags(&tiff).expect("read primary orientation");
            assert!(
                before
                    .iter()
                    .any(|tag| tag.name == "Orientation" && tag.group.family1 == "IFD0")
            );
            assert!(matches!(
                orientation_exif(&before),
                Err(Failure::InvalidImage("invalid orientation"))
            ));
        }
    }
}
