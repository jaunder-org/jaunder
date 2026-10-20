//! Native worker entry: controlled extensionless paths, supported metadata edits
//! and a fresh ordinary read. It never initializes logging, storage or networking.
use std::io::{self, Write};
use std::path::Path;

use exiftool_rs::{ExifTool, FileType, Options, Tag, get_file_type};

use super::wire::{PreparationError, Status};

pub const WORKER_ARGUMENT: &str = "--internal-image-privacy-worker";

/// Executes only a private metadata job. The response contains no input paths,
/// metadata values or unreviewed native error descriptions.
///
/// # Errors
///
/// Returns an I/O error if test controls cannot be read, or if the response cannot
/// be serialized or flushed to stdout.
pub fn run_worker(spool: &Path, supplied_mime: &str) -> io::Result<bool> {
    #[cfg(any(test, feature = "test-utils"))]
    if let Some(succeeded) = super::test_control::worker_override(spool, &mut io::stdout().lock())?
    {
        return Ok(succeeded);
    }
    let result = prepare(spool, supplied_mime);
    let succeeded = result.is_ok();
    let status = Status {
        version: 1,
        result: result.map_err(Into::into),
    };
    serde_json::to_writer(io::stdout().lock(), &status)?;
    io::stdout().flush()?;
    Ok(succeeded)
}

fn reader() -> ExifTool {
    ExifTool::with_options(Options {
        duplicates: true,
        print_conv: false,
        ..Options::default()
    })
}

// Rendering fields stay untouched. Deleting EXIF and recreating selected tags
// would needlessly convert values and lose fields the native setter cannot write.
// XMP is edited selectively so HDR namespaces are not indiscriminately erased.
fn rendering(tag: &Tag) -> bool {
    matches!(
        tag.name.as_str(),
        "Orientation"
            | "ColorSpace"
            | "YCbCrPositioning"
            | "XResolution"
            | "YResolution"
            | "ResolutionUnit"
            | "ExifImageWidth"
            | "ExifImageHeight"
            | "Gamma"
            | "WhitePoint"
            | "PrimaryChromaticities"
            | "YCbCrCoefficients"
            | "ReferenceBlackWhite"
            | "ComponentsConfiguration"
            | "ExifVersion"
            | "FlashpixVersion"
    )
}

fn private_xmp(tag: &Tag) -> bool {
    tag.group.family0 == "XMP"
        && (matches!(
            tag.group.family1.as_str(),
            "XMP-dc" | "XMP-iptcCore" | "XMP-iptcExt"
        ) || matches!(
            tag.group.family2.as_str(),
            "Location" | "Author" | "Time" | "Camera"
        ) || tag.name.starts_with("GPS")
            || matches!(
                tag.name.as_str(),
                "Make"
                    | "Model"
                    | "SerialNumber"
                    | "LensSerialNumber"
                    | "OwnerName"
                    | "CameraOwnerName"
                    | "CreatorTool"
                    | "History"
                    | "MetadataDate"
                    | "CreateDate"
                    | "ModifyDate"
                    | "DateTimeOriginal"
                    | "Description"
                    | "Title"
                    | "Label"
                    | "Rating"
                    | "Comment"
                    | "UserComment"
                    | "Thumbnails"
            ))
}

fn private_tag(tag: &Tag) -> bool {
    (tag.group.family0 == "EXIF" && !rendering(tag))
        || tag.group.family1 == "IFD1"
        || tag.group.family0 == "IPTC"
        || private_xmp(tag)
        || (tag.group.family0 == "PNG"
            && matches!(
                tag.name.as_str(),
                "Author"
                    | "Description"
                    | "Copyright"
                    | "CreationTime"
                    | "ModifyDate"
                    | "Software"
                    | "Title"
                    | "XMLcomadobexmp"
            ))
        || (tag.name == "Comment" && tag.group.family0 != "ICC_Profile")
}

fn png_rendering(tag: &Tag) -> bool {
    tag.group.family0 == "PNG" && matches!(tag.name.as_str(), "Gamma" | "SRGBRendering")
}

// The native reader exposes decoding gamma numerically and sRGB intent by name.
// Keep those supported representations, not the independent oracle's numeric intent.
fn png_rendering_value(tag: &Tag) -> Result<String, PreparationError> {
    let value = if tag.name == "Gamma" {
        tag.raw_value
            .as_f64()
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|value| value.to_string())
    } else {
        tag.raw_value
            .as_str()
            .filter(|value| {
                matches!(
                    *value,
                    "Perceptual" | "Relative Colorimetric" | "Saturation" | "Absolute Colorimetric"
                )
            })
            .map(str::to_owned)
    };
    value.ok_or_else(|| {
        PreparationError::Native(exiftool_rs::Error::ConversionError(
            "PNG rendering value cannot be restored".into(),
        ))
    })
}

fn prepare(spool: &Path, supplied_mime: &str) -> Result<Option<String>, PreparationError> {
    let input = spool.join("input");
    let output = spool.join("output");
    let detected = match get_file_type(&input) {
        Ok(kind) => kind,
        Err(exiftool_rs::Error::UnsupportedFileType(_)) if !supplied_mime.starts_with("image/") => {
            std::fs::copy(&input, &output)?;
            return Ok(None);
        }
        Err(exiftool_rs::Error::UnsupportedFileType(_)) => {
            return Err(PreparationError::InvalidImage);
        }
        Err(error) => return Err(PreparationError::Native(error)),
    };
    if !matches!(
        detected,
        FileType::Jpeg | FileType::Png | FileType::Gif | FileType::WebP | FileType::Heif
    ) {
        // A recognized non-covered type is outside this policy. Its supplied
        // MIME retains existing semantics; only a detection miss above uses an
        // image label to fail closed.
        std::fs::copy(&input, &output)?;
        return Ok(None);
    }
    let before = reader()
        .extract_info(&input)
        .map_err(PreparationError::Native)?;
    let mut editor = reader();
    editor.set_new_value("IPTC:all", None);
    editor.set_new_value("Comment", None);
    if detected == FileType::Png {
        // Group removal clears ordinary text (including language-coded tags)
        // and tIME. Restore only present color controls in this same write;
        // the native writer inserts replacements before image data.
        editor.set_new_value("PNG:all", None);
        for tag in before.iter().filter(|tag| png_rendering(tag)) {
            editor.set_new_value(
                &format!("PNG:{}", tag.name),
                Some(&png_rendering_value(tag)?),
            );
        }
    }
    for tag in &before {
        if tag.group.family0 == "EXIF" && private_tag(tag) {
            editor.set_new_value(&format!("EXIF:{}", tag.name), None);
        } else if private_xmp(tag) {
            editor.set_new_value(&format!("{}:{}", tag.group.family1, tag.name), None);
        }
    }
    // Ok(0) is a valid already-clean result, never a privacy certificate.
    editor
        .write_info(&input, &output)
        .map_err(PreparationError::Native)?;
    let after = reader()
        .extract_info(&output)
        .map_err(PreparationError::Native)?;
    let output_type = get_file_type(&output).map_err(PreparationError::Native)?;
    verify_output(detected, output_type, &before, &after)?;
    Ok(Some(detected.mime_type().into()))
}

// The native editor may drop EXIF directories containing only mandatory
// bookkeeping. Verify only privacy and display controls that the upload contract
// promises to retain, using the same ordinary metadata reader as production.
fn verify_output(
    detected: FileType,
    output_type: FileType,
    before: &[Tag],
    after: &[Tag],
) -> Result<(), PreparationError> {
    if output_type != detected || after.iter().any(private_tag) {
        return Err(PreparationError::Verification);
    }
    for original in before.iter().filter(|tag| {
        tag.group.family0 == "ICC_Profile"
            || png_rendering(tag)
            || matches!(
                tag.name.as_str(),
                "Orientation" | "ImageWidth" | "ImageHeight"
            )
    }) {
        if !after.iter().any(|tag| {
            tag.group.family1 == original.group.family1
                && tag.name == original.name
                && tag.raw_value == original.raw_value
        }) {
            return Err(PreparationError::Verification);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        FileType, PreparationError, png_rendering, png_rendering_value, private_tag, private_xmp,
        verify_output,
    };
    use exiftool_rs::{Error as NativeError, Tag, TagGroup, TagId, Value};

    fn tag(family0: &str, family1: &str, family2: &str, name: &str, raw_value: Value) -> Tag {
        Tag {
            id: TagId::Text(name.into()),
            name: name.into(),
            description: name.into(),
            group: TagGroup {
                family0: family0.into(),
                family1: family1.into(),
                family2: family2.into(),
                family3: "Main".into(),
            },
            print_value: raw_value.to_display_string(),
            raw_value,
            priority: 0,
        }
    }

    fn text_tag(family0: &str, family1: &str, family2: &str, name: &str) -> Tag {
        tag(
            family0,
            family1,
            family2,
            name,
            Value::String("fixture value".into()),
        )
    }

    #[test]
    fn private_xmp_classifies_group_location_gps_and_named_private_fields() {
        assert!(private_xmp(&text_tag("XMP", "XMP-dc", "Other", "Keywords")));
        assert!(private_xmp(&text_tag("XMP", "Other", "Location", "City")));
        assert!(private_xmp(&text_tag(
            "XMP",
            "Other",
            "Other",
            "GPSLatitude"
        )));
        assert!(private_xmp(&text_tag(
            "XMP",
            "Other",
            "Other",
            "CreatorTool"
        )));
        assert!(!private_xmp(&text_tag(
            "EXIF",
            "Other",
            "Other",
            "CreatorTool"
        )));
        assert!(!private_xmp(&text_tag(
            "XMP",
            "Other",
            "Other",
            "Unrelated"
        )));
    }

    #[test]
    fn private_tag_preserves_icc_comment_and_rendering_controls() {
        assert!(private_tag(&text_tag("EXIF", "IFD0", "Image", "Make")));
        assert!(!private_tag(&text_tag(
            "EXIF",
            "IFD0",
            "Image",
            "Orientation"
        )));
        assert!(private_tag(&text_tag("PNG", "PNG", "Text", "Author")));
        assert!(!private_tag(&text_tag("PNG", "PNG", "Text", "Unrelated")));
        assert!(private_tag(&text_tag(
            "XMP",
            "Other",
            "Other",
            "CreatorTool"
        )));
        assert!(private_tag(&text_tag("EXIF", "IFD0", "Image", "Comment")));
        assert!(!private_tag(&text_tag(
            "ICC_Profile",
            "ICC_Profile",
            "Image",
            "Comment"
        )));
    }

    #[test]
    fn png_rendering_values_accept_only_supported_native_representations() {
        let gamma = tag("PNG", "PNG", "Image", "Gamma", Value::F64(0.45455));
        assert_eq!(png_rendering_value(&gamma).unwrap(), "0.45455");
        for intent in [
            "Perceptual",
            "Relative Colorimetric",
            "Saturation",
            "Absolute Colorimetric",
        ] {
            let rendering = text_tag("PNG", "PNG", "Image", "SRGBRendering");
            let rendering = Tag {
                raw_value: Value::String(intent.into()),
                print_value: intent.into(),
                ..rendering
            };
            assert_eq!(png_rendering_value(&rendering).unwrap(), intent);
            assert!(png_rendering(&rendering));
        }
        for invalid in [
            tag("PNG", "PNG", "Image", "Gamma", Value::F64(0.0)),
            tag("PNG", "PNG", "Image", "Gamma", Value::F64(f64::INFINITY)),
            text_tag("PNG", "PNG", "Image", "SRGBRendering"),
        ] {
            assert!(matches!(
                png_rendering_value(&invalid),
                Err(PreparationError::Native(NativeError::ConversionError(_)))
            ));
        }
    }

    #[test]
    fn output_verification_rejects_wrong_type_private_residue_and_lost_fidelity() {
        let orientation = tag("EXIF", "IFD0", "Image", "Orientation", Value::U16(6));
        let icc = text_tag("ICC_Profile", "ICC_Profile", "Image", "ProfileDescription");
        let gamma = tag("PNG", "PNG", "Image", "Gamma", Value::F64(0.45));
        let before = [orientation.clone(), icc.clone(), gamma.clone()];
        assert!(verify_output(FileType::Png, FileType::Png, &before, &before).is_ok());
        assert!(matches!(
            verify_output(FileType::Png, FileType::Jpeg, &before, &before),
            Err(PreparationError::Verification)
        ));
        let private = text_tag("PNG", "PNG", "Text", "Author");
        assert!(matches!(
            verify_output(FileType::Png, FileType::Png, &before, &[private]),
            Err(PreparationError::Verification)
        ));
        for missing in 0..before.len() {
            let after = before
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != missing)
                .map(|(_, tag)| tag.clone())
                .collect::<Vec<_>>();
            assert!(matches!(
                verify_output(FileType::Png, FileType::Png, &before, &after),
                Err(PreparationError::Verification)
            ));
        }
        let changed_gamma = tag("PNG", "PNG", "Image", "Gamma", Value::F64(0.46));
        assert!(matches!(
            verify_output(
                FileType::Png,
                FileType::Png,
                &before,
                &[orientation, icc, changed_gamma],
            ),
            Err(PreparationError::Verification)
        ));
    }
}
