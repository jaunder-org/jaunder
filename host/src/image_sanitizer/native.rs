//! Linked metadata editing for the private same-executable worker.
//!
//! This boundary preserves encoded image data and rendering metadata. It never
//! performs executable discovery, starts a helper or falls back to a covered
//! image's original bytes after a processing failure.

use thiserror::Error;

/// A private edited raster, or an unchanged non-raster attachment.
#[derive(Debug)]
pub enum EditedBytes {
    /// The byte-detected image type describes the edited bytes.
    Image { bytes: Vec<u8>, mime: &'static str },
    /// The bytes neither claim nor detect as an unsupported raster.
    Passthrough(Vec<u8>),
}

/// Native failures retain their sources inside the worker boundary.
#[derive(Debug, Error)]
pub enum NativeImageError {
    /// Covers both received bytes and an expanded edited container.
    #[error("image exceeds the configured file-size limit")]
    FileTooLarge,
    /// Expected rejection carries only a fixed, metadata-free explanation.
    #[error("invalid image: {0}")]
    InvalidImage(&'static str),
    /// Preserve the concrete native metadata reader/writer failure.
    #[error("native metadata processing failed")]
    Metadata(#[from] exiftool_rs::Error),
    /// Container parsing fails closed rather than becoming passthrough.
    #[error("native image container processing failed")]
    Container(#[from] img_parts::Error),
    /// Native TIFF and HEIF parsing/editing retain the actual I/O error.
    #[error("native metadata I/O failed")]
    Io(#[from] std::io::Error),
    /// Encoded GIF boundaries must be successfully read before editing.
    #[error("native GIF processing failed")]
    Gif(#[from] gif::DecodingError),
    /// Rendering fields cannot silently default after a parse error.
    #[error("invalid image rendering value")]
    Number(#[from] std::num::ParseIntError),
    /// Container extents must fit the platform's address range.
    #[error("invalid image extent")]
    Integer(#[from] std::num::TryFromIntError),
    /// Truncated fixed-width container fields are rejected.
    #[error("truncated image field")]
    Field(#[from] std::array::TryFromSliceError),
}

impl From<&'static str> for NativeImageError {
    fn from(reason: &'static str) -> Self {
        Self::InvalidImage(reason)
    }
}

/// Sanitize byte-detected covered images before any durable identity is made.
///
/// # Errors
/// Returns a typed rejection for malformed/unsupported rasters or size limits.
/// Non-raster attachments are eligible for passthrough only without a raster claim.
pub fn sanitize(
    data: &[u8],
    claims_raster: bool,
    max_bytes: usize,
) -> Result<EditedBytes, NativeImageError> {
    super::native_kernel::sanitize(data, claims_raster, max_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expect_image(outcome: EditedBytes) -> (Vec<u8>, &'static str) {
        match outcome {
            EditedBytes::Image { bytes, mime } => (bytes, mime),
            EditedBytes::Passthrough(_) => panic!("expected an edited raster, not passthrough"),
        }
    }

    fn expect_passthrough(outcome: EditedBytes) -> Vec<u8> {
        match outcome {
            EditedBytes::Passthrough(bytes) => bytes,
            EditedBytes::Image { .. } => panic!("expected passthrough, not an edited raster"),
        }
    }

    #[test]
    fn outcome_assertions_reject_the_opposite_real_sanitization_result() {
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        let passthrough = sanitize(svg, true, 16_777_216).expect("excluded SVG");
        assert!(std::panic::catch_unwind(|| expect_image(passthrough)).is_err());
        let image = sanitize(
            include_bytes!("../image_sanitizer_fixtures/png-original.png"),
            false,
            16_777_216,
        )
        .expect("covered PNG");
        assert!(std::panic::catch_unwind(|| expect_passthrough(image)).is_err());
    }

    #[test]
    fn document_xmp_orientation_is_preserved_as_primary_exif() {
        use std::path::Path;
        let mut png = img_parts::png::Png::from_bytes(
            include_bytes!("../image_sanitizer_fixtures/png-original.png")
                .as_slice()
                .into(),
        )
        .expect("owned PNG");
        png.chunks_mut()
            .retain(|chunk| !matches!(&chunk.kind(), b"eXIf" | b"iTXt"));
        let mut packet = b"XML:com.adobe.xmp\0\0\0\0\0".to_vec();
        packet.extend_from_slice(br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:tiff="http://ns.adobe.com/tiff/1.0/" tiff:Orientation="6"/></rdf:RDF></x:xmpmeta>"#);
        png.chunks_mut()
            .insert(1, img_parts::png::PngChunk::new(*b"iTXt", packet.into()));
        let input = png.encoder().bytes();
        let reader = exiftool_rs::ExifTool::new();
        let before = reader
            .extract_info_from_bytes(&input, Path::new("private"))
            .expect("XMP control");
        assert!(before.iter().any(|tag| tag.name == "Orientation"
            && tag.group.family1 == "XMP-tiff"
            && tag.raw_value.to_display_string() == "6"));
        assert!(
            !before
                .iter()
                .any(|tag| tag.name == "Orientation" && tag.group.family1 == "IFD0")
        );
        let (bytes, _) =
            expect_image(sanitize(&input, true, 16_777_216).expect("primary XMP orientation"));
        let after = reader
            .extract_info_from_bytes(&bytes, Path::new("private"))
            .expect("edited orientation");
        assert!(after.iter().any(|tag| tag.name == "Orientation"
            && tag.group.family1 == "IFD0"
            && tag.raw_value.to_display_string() == "6"));
        assert!(!after.iter().any(|tag| tag.group.family0 == "XMP"));
    }

    #[test]
    fn thumbnail_orientation_does_not_rotate_the_primary_image() {
        use little_exif::{exif_tag::ExifTag, ifd::ExifTagGroup, metadata::Metadata};
        use std::path::Path;

        let mut metadata = Metadata::new();
        metadata.set_tag(ExifTag::Artist("private-thumbnail-control".to_owned()));
        metadata
            .get_ifd_mut(ExifTagGroup::GENERIC, 1)
            .set_tag(ExifTag::Orientation(vec![6]));
        let exif = metadata.encode().expect("thumbnail-only EXIF control");
        let input = exiftool_rs::writer::jpeg_writer::write_jpeg(
            include_bytes!("../image_sanitizer_fixtures/jpeg-original.jpg"),
            Some(&exif),
            None,
            None,
            None,
            false,
            true,
            true,
            true,
        )
        .expect("owned JPEG with thumbnail-only orientation");
        let tags = exiftool_rs::ExifTool::new()
            .extract_info_from_bytes(&input, Path::new("private"))
            .expect("control metadata");
        let orientation_groups = tags
            .iter()
            .filter(|tag| tag.name == "Orientation")
            .map(|tag| &tag.group.family1)
            .collect::<Vec<_>>();
        assert!(
            tags.iter()
                .any(|tag| tag.name == "Orientation" && tag.group.family1 == "IFD1"),
            "control orientation groups: {orientation_groups:?}"
        );
        assert!(
            !tags
                .iter()
                .any(|tag| tag.name == "Orientation" && tag.group.family1 == "IFD0")
        );
        let (bytes, _) = expect_image(
            sanitize(&input, true, 16_777_216).expect("sanitize thumbnail-only orientation"),
        );
        let tags = exiftool_rs::ExifTool::new()
            .extract_info_from_bytes(&bytes, Path::new("private"))
            .expect("edited metadata");
        assert!(
            !tags.iter().any(|tag| tag.name == "Orientation"),
            "thumbnail orientation must not become a primary rendering instruction"
        );
    }

    #[test]
    fn truncated_images_and_missing_terminal_blocks_are_rejected() {
        for original in [
            include_bytes!("../image_sanitizer_fixtures/jpeg-original.jpg").as_slice(),
            include_bytes!("../image_sanitizer_fixtures/png-original.png").as_slice(),
            include_bytes!("../image_sanitizer_fixtures/gif-animation-original.gif").as_slice(),
            include_bytes!("../image_sanitizer_fixtures/webp-static-original.webp").as_slice(),
            include_bytes!("../image_sanitizer_fixtures/heic-original.heic").as_slice(),
        ] {
            assert!(sanitize(&original[..original.len() / 2], true, 16_777_216).is_err());
        }
        for original in [
            include_bytes!("../image_sanitizer_fixtures/jpeg-original.jpg").as_slice(),
            include_bytes!("../image_sanitizer_fixtures/png-original.png").as_slice(),
            include_bytes!("../image_sanitizer_fixtures/gif-animation-original.gif").as_slice(),
        ] {
            assert!(sanitize(&original[..original.len() - 1], true, 16_777_216).is_err());
        }
    }

    #[test]
    fn malformed_and_unsupported_rasters_never_become_passthrough() {
        for data in [
            b"\xff\xd8\xff\xd9".as_slice(),
            b"GIF89a".as_slice(),
            b"RIFF\0\0\0\0WEBP".as_slice(),
            b"\x89PNG\r\n\x1a\n".as_slice(),
            b"MM\0*\0\0\0\0".as_slice(),
            b"not-an-image".as_slice(),
        ] {
            assert!(sanitize(data, true, 16_777_216).is_err());
        }
    }

    #[test]
    fn byte_detected_svg_remains_passthrough_despite_a_raster_claim() {
        let data = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
        let actual = expect_passthrough(
            sanitize(data, true, 16_777_216).expect("SVG is excluded from raster editing"),
        );
        assert_eq!(actual, data);
    }

    #[test]
    fn svg_and_non_raster_attachments_remain_byte_identical() {
        for data in [
            b"ordinary attachment".as_slice(),
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".as_slice(),
        ] {
            let actual = expect_passthrough(
                sanitize(data, false, 16_777_216).expect("non-raster attachment"),
            );
            assert_eq!(actual, data);
        }
    }

    #[test]
    fn input_and_expanded_output_limits_keep_their_typed_rejection() {
        let original = include_bytes!("../image_sanitizer_fixtures/png-original.png");
        assert!(matches!(
            sanitize(original, true, original.len() - 1),
            Err(NativeImageError::FileTooLarge)
        ));
        let (bytes, _) = expect_image(
            sanitize(
                include_bytes!("../image_sanitizer_fixtures/hdr-signal.png"),
                true,
                16_777_216,
            )
            .expect("HDR control"),
        );
        let mut tiny = img_parts::png::Png::from_bytes(bytes.into()).expect("edited PNG container");
        tiny.chunks_mut().retain(|chunk| chunk.kind() != *b"eXIf");
        let tiny = tiny.encoder().bytes();
        assert!(matches!(
            sanitize(&tiny, true, tiny.len()),
            Err(NativeImageError::FileTooLarge)
        ));
    }

    #[test]
    fn hostile_heif_length_cannot_produce_public_bytes() {
        let mut data = include_bytes!("../image_sanitizer_fixtures/heic-original.heic").to_vec();
        data[..4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(sanitize(&data, true, 16_777_216).is_err());
    }

    #[test]
    fn owned_format_corpus_is_edited_with_stable_reupload_identity() {
        // Nine owned format descriptors plus the separate representative HDR control.
        let fixtures: &[(&[u8], &str); 10] = &[
            (
                include_bytes!("../image_sanitizer_fixtures/jpeg-original.jpg"),
                "image/jpeg",
            ),
            (
                include_bytes!("../image_sanitizer_fixtures/png-original.png"),
                "image/png",
            ),
            (
                include_bytes!("../image_sanitizer_fixtures/apng-first-original.png"),
                "image/png",
            ),
            (
                include_bytes!("../image_sanitizer_fixtures/apng-default-original.png"),
                "image/png",
            ),
            (
                include_bytes!("../image_sanitizer_fixtures/gif-static-original.gif"),
                "image/gif",
            ),
            (
                include_bytes!("../image_sanitizer_fixtures/gif-animation-original.gif"),
                "image/gif",
            ),
            (
                include_bytes!("../image_sanitizer_fixtures/webp-static-original.webp"),
                "image/webp",
            ),
            (
                include_bytes!("../image_sanitizer_fixtures/webp-animation-original.webp"),
                "image/webp",
            ),
            (
                include_bytes!("../image_sanitizer_fixtures/heic-original.heic"),
                "image/heic",
            ),
            (
                include_bytes!("../image_sanitizer_fixtures/hdr-signal.png"),
                "image/png",
            ),
        ];
        for (original, expected_mime) in fixtures {
            let (bytes, mime) =
                expect_image(sanitize(original, false, 16_777_216).expect("owned native image"));
            assert_eq!(mime, *expected_mime);
            let (second, second_mime) =
                expect_image(sanitize(&bytes, false, 16_777_216).expect("sanitized reupload"));
            assert_eq!(second_mime, mime);
            assert_eq!(second, bytes, "reupload identity changed for {mime}");
        }
    }

    #[test]
    fn recognized_non_raster_preserves_bytes_unless_claimed_as_raster() {
        // Recognized attachment formats must not enter raster editing, but a
        // false raster claim must still fail closed rather than pass through.
        let pdf = b"%PDF-1.7\nattachment bytes owner@example.test\n%%EOF\n";
        assert!(matches!(
            sanitize(pdf, false, pdf.len()),
            Ok(EditedBytes::Passthrough(bytes)) if bytes == pdf
        ));
        assert!(matches!(
            sanitize(pdf, true, pdf.len()),
            Err(NativeImageError::InvalidImage("unsupported raster"))
        ));
    }

    #[test]
    fn edited_container_expansion_respects_the_same_size_limit() {
        // A valid received image at the limit can grow when rendering EXIF is
        // regenerated. Expanded output must be rejected before durable identity.
        let mut png = img_parts::png::Png::from_bytes(
            include_bytes!("../image_sanitizer_fixtures/png-original.png")
                .as_slice()
                .into(),
        )
        .expect("owned PNG");
        png.chunks_mut().retain(|chunk| {
            !matches!(
                &chunk.kind(),
                b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt" | b"tIME"
            )
        });
        let input = png.encoder().bytes();
        assert!(matches!(
            sanitize(&input, false, input.len()),
            Err(NativeImageError::FileTooLarge)
        ));
        assert!(matches!(
            sanitize(&input, false, input.len() + 1024),
            Ok(EditedBytes::Image { bytes, mime: "image/png" }) if bytes.len() > input.len()
        ));
    }

    #[test]
    fn byte_detected_png_is_sanitized_without_a_raster_claim() {
        // Ingress filenames and MIME claims must not let private raster metadata
        // bypass editing. The owned fixture contains conventional private tags.
        let original = include_bytes!("../image_sanitizer_fixtures/png-original.png");
        let (bytes, mime) =
            expect_image(sanitize(original, false, 1_048_576).expect("covered PNG"));
        assert_eq!(mime, "image/png");
        assert_ne!(bytes, original);
        let (second, second_mime) =
            expect_image(sanitize(&bytes, false, 1_048_576).expect("sanitized reupload"));
        assert_eq!(second_mime, mime);
        assert_eq!(second, bytes);
    }
}
