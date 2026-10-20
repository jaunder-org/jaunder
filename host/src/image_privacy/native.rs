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
    if let Some(succeeded) = super::test_control::worker_override(spool)? {
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
    if get_file_type(&output).map_err(PreparationError::Native)? != detected
        || after.iter().any(private_tag)
    {
        return Err(PreparationError::Verification);
    }
    // The native editor may drop EXIF directories containing only mandatory
    // bookkeeping (ExifVersion, ComponentsConfiguration, resolution defaults).
    // Those are not JPEG display dimensions or the intact ICC profile. Protect
    // the actual display controls rather than requiring bookkeeping to persist.
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
    Ok(Some(detected.mime_type().into()))
}
