//! Closed ZIP/manifest/media boundary for portable public Theme Packages.
//!
//! CSS AST transformation is intentionally delegated to the sibling `css`
//! module. This module owns the archive, canonical manifest, decoded assets,
//! and non-circular digest framing shared with storage.

mod css;

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read},
};

use image::{ImageFormat, ImageReader};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use zip::ZipArchive;

pub use css::{CompiledCss, compile_stylesheet};

const SOURCE_DOMAIN: &[u8] = b"jaunder-theme-source-v1";
const REVISION_DOMAIN: &[u8] = b"jaunder-theme-revision-v1";

#[derive(Clone, Copy, Debug)]
pub struct ThemePackageLimits {
    pub max_archive_bytes: usize,
    pub max_compressed_bytes: u64,
    pub max_expanded_bytes: usize,
    pub max_files: usize,
    pub max_file_bytes: usize,
    pub max_css_rules: usize,
    pub max_css_nesting: usize,
    pub max_image_width: u32,
    pub max_image_height: u32,
    pub max_image_frames: usize,
    pub max_image_pixels: u64,
    pub max_font_bytes: usize,
}
impl Default for ThemePackageLimits {
    fn default() -> Self {
        Self {
            max_archive_bytes: 16 * 1024 * 1024,
            max_compressed_bytes: 8 * 1024 * 1024,
            max_expanded_bytes: 32 * 1024 * 1024,
            max_files: 128,
            max_file_bytes: 8 * 1024 * 1024,
            max_css_rules: 10_000,
            max_css_nesting: 4,
            max_image_width: 10_000,
            max_image_height: 10_000,
            max_image_frames: 1,
            max_image_pixels: 40_000_000,
            max_font_bytes: 16 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ThemePackageError {
    #[error("archive is invalid: {0}")]
    Archive(String),
    #[error("archive exceeds {limit}")]
    LimitExceeded { limit: &'static str },
    #[error("invalid archive member `{0}")]
    Member(String),
    #[error("manifest is invalid: {0}")]
    Manifest(String),
    #[error("asset `{path}` does not match declared MIME type `{declared}")]
    Mime { path: String, declared: String },
    #[error("asset `{0}` is not declared by the manifest")]
    UndeclaredAsset(String),
    #[error("CSS is invalid: {0}")]
    Css(String),
    #[error("CSS URL `{0}` is not a declared package asset")]
    Url(String),
}

/// Source bytes that passed the closed package validator. Only this module can mint it.
#[derive(Debug)]
pub struct ValidatedThemePackage {
    manifest: Vec<u8>,
    css: Vec<u8>,
    assets: BTreeMap<String, ThemeAsset>,
    source_digest: [u8; 32],
}
/// Immutable compiler result. Only the compiler can mint it.
#[derive(Debug)]
pub struct CompiledThemeRevision {
    manifest: Vec<u8>,
    css: CompiledCss,
    assets: BTreeMap<String, ThemeAsset>,
    source_digest: [u8; 32],
    revision_digest: [u8; 32],
}
#[derive(Debug)]
pub struct ThemeAsset {
    mime: AssetMime,
    bytes: Vec<u8>,
    digest: [u8; 32],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AssetMime {
    Woff2,
    Png,
    Jpeg,
    Webp,
    Avif,
}
impl AssetMime {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "font/woff2" => Some(Self::Woff2),
            "image/png" => Some(Self::Png),
            "image/jpeg" => Some(Self::Jpeg),
            "image/webp" => Some(Self::Webp),
            "image/avif" => Some(Self::Avif),
            _ => None,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Woff2 => "font/woff2",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
            Self::Avif => "image/avif",
        }
    }
    fn image_format(self) -> Option<ImageFormat> {
        match self {
            Self::Png => Some(ImageFormat::Png),
            Self::Jpeg => Some(ImageFormat::Jpeg),
            Self::Webp => Some(ImageFormat::WebP),
            Self::Avif => Some(ImageFormat::Avif),
            Self::Woff2 => None,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u8,
    name: String,
    style_contract: u8,
    assets: BTreeMap<String, String>,
    defaults: Defaults,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Defaults {
    logo: Option<String>,
    header: Option<Vec<String>>,
}

/// Streams a ZIP archive without extracting an archive-selected filesystem path.
///
/// # Errors
///
/// Returns [`ThemePackageError`] when the archive, manifest, assets, or limits
/// violate the closed Theme Package format.
pub fn validate_theme_package(
    input: &[u8],
    limits: ThemePackageLimits,
) -> Result<ValidatedThemePackage, ThemePackageError> {
    if input.len() > limits.max_archive_bytes {
        return Err(limit("archive bytes"));
    }
    let mut entries = read_archive_entries(input, limits)?;
    let (raw_manifest, css) = take_required_entries(&mut entries)?;
    let (manifest, canonical) = parse_manifest(&raw_manifest)?;
    let assets = validate_declared_assets(&manifest, &mut entries, limits)?;
    validate_defaults(&manifest.defaults, &assets)?;
    if let Some((path, _)) = entries.into_iter().next() {
        return Err(ThemePackageError::UndeclaredAsset(path));
    }
    let source_digest = source_digest(&canonical, &css, &assets);
    Ok(ValidatedThemePackage {
        manifest: canonical,
        css,
        assets,
        source_digest,
    })
}

fn read_archive_entries(
    input: &[u8],
    limits: ThemePackageLimits,
) -> Result<BTreeMap<String, Vec<u8>>, ThemePackageError> {
    validate_central_directory_names(input)?;
    let mut archive = ZipArchive::new(Cursor::new(input)).map_err(|error| zip_error(&error))?;
    if archive.len() > limits.max_files {
        return Err(limit("file count"));
    }
    let mut entries = BTreeMap::new();
    let mut compressed = 0_u64;
    let mut expanded = 0_usize;
    for index in 0..archive.len() {
        let file = archive.by_index(index).map_err(|error| zip_error(&error))?;
        let path = file.name().to_owned();
        validate_member_name(&path)?;
        if !file.is_file()
            || file.encrypted()
            || file
                .unix_mode()
                .is_some_and(|mode| mode & 0o170_000 != 0o100_000)
        {
            return Err(ThemePackageError::Member(path));
        }
        let header_start = usize::try_from(file.header_start()).map_err(|_| {
            ThemePackageError::Archive("local header offset exceeds platform limit".into())
        })?;
        validate_local_header(
            input,
            header_start,
            &path,
            file.compression(),
            file.crc32(),
            file.compressed_size(),
            file.size(),
        )?;
        compressed = compressed
            .checked_add(file.compressed_size())
            .ok_or_else(|| limit("compressed bytes"))?;
        let file_size = usize::try_from(file.size()).map_err(|_| limit("per-file bytes"))?;
        if compressed > limits.max_compressed_bytes || file_size > limits.max_file_bytes {
            return Err(limit("compressed or per-file bytes"));
        }
        let mut bytes = Vec::with_capacity(file_size);
        let read_limit = u64::try_from(limits.max_file_bytes)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        file.take(read_limit)
            .read_to_end(&mut bytes)
            .map_err(|error| ThemePackageError::Archive(error.to_string()))?;
        if bytes.len() > limits.max_file_bytes {
            return Err(limit("per-file bytes"));
        }
        expanded = expanded
            .checked_add(bytes.len())
            .ok_or_else(|| limit("expanded bytes"))?;
        if expanded > limits.max_expanded_bytes {
            return Err(limit("expanded bytes"));
        }
        if entries.insert(path.clone(), bytes).is_some() {
            return Err(ThemePackageError::Member(path));
        }
    }
    Ok(entries)
}

fn take_required_entries(
    entries: &mut BTreeMap<String, Vec<u8>>,
) -> Result<(Vec<u8>, Vec<u8>), ThemePackageError> {
    let manifest = entries
        .remove("theme.json")
        .ok_or_else(|| ThemePackageError::Member("missing theme.json".into()))?;
    let css = entries
        .remove("style.css")
        .ok_or_else(|| ThemePackageError::Member("missing style.css".into()))?;
    Ok((manifest, css))
}

fn parse_manifest(raw_manifest: &[u8]) -> Result<(Manifest, Vec<u8>), ThemePackageError> {
    let manifest: Manifest = serde_json::from_slice(raw_manifest)
        .map_err(|error| ThemePackageError::Manifest(error.to_string()))?;
    if manifest.schema != 1 || manifest.style_contract != 1 || manifest.name.trim().is_empty() {
        return Err(ThemePackageError::Manifest(
            "schema, style_contract, and name must be version 1 and non-empty".into(),
        ));
    }
    let value = serde_json::from_slice::<serde_json::Value>(raw_manifest)
        .map_err(|error| ThemePackageError::Manifest(error.to_string()))?;
    let canonical = serde_jcs::to_vec(&value)
        .map_err(|error| ThemePackageError::Manifest(error.to_string()))?;
    Ok((manifest, canonical))
}

fn validate_declared_assets(
    manifest: &Manifest,
    entries: &mut BTreeMap<String, Vec<u8>>,
    limits: ThemePackageLimits,
) -> Result<BTreeMap<String, ThemeAsset>, ThemePackageError> {
    let mut assets = BTreeMap::new();
    for (path, declared) in &manifest.assets {
        validate_asset_path(path)?;
        let mime = AssetMime::parse(declared).ok_or_else(|| {
            ThemePackageError::Manifest(format!("unsupported MIME type `{declared}`"))
        })?;
        let bytes = entries
            .remove(path)
            .ok_or_else(|| ThemePackageError::UndeclaredAsset(path.clone()))?;
        validate_asset(path, mime, &bytes, limits)?;
        assets.insert(
            path.clone(),
            ThemeAsset {
                mime,
                digest: sha256(&bytes),
                bytes,
            },
        );
    }
    Ok(assets)
}

fn validate_defaults(
    defaults: &Defaults,
    assets: &BTreeMap<String, ThemeAsset>,
) -> Result<(), ThemePackageError> {
    if let Some(path) = &defaults.logo {
        require_image_default(assets, path)?;
    }
    if let Some(paths) = &defaults.header {
        if paths.is_empty() {
            return Err(ThemePackageError::Manifest(
                "default header must not be empty".into(),
            ));
        }
        let mut distinct = BTreeSet::new();
        for path in paths {
            if !distinct.insert(path) {
                return Err(ThemePackageError::Manifest(
                    "default header contains a duplicate".into(),
                ));
            }
            require_image_default(assets, path)?;
        }
    }
    Ok(())
}
impl ValidatedThemePackage {
    #[must_use]
    pub fn source_digest(&self) -> [u8; 32] {
        self.source_digest
    }
    #[must_use]
    pub fn canonical_manifest(&self) -> &[u8] {
        &self.manifest
    }
    #[must_use]
    pub fn authored_css(&self) -> &[u8] {
        &self.css
    }
    #[must_use]
    pub fn asset(&self, path: &str) -> Option<(&str, &[u8], [u8; 32])> {
        self.assets
            .get(path)
            .map(|asset| (asset.mime.as_str(), asset.bytes.as_slice(), asset.digest))
    }
    /// Compiles the validated source with storage-chosen immutable asset URLs.
    ///
    /// The URL map is explicit because this host-only compiler does not own
    /// persistence or routing; CSS must nevertheless bind every URL before the
    /// revision digest is minted.
    ///
    /// # Errors
    ///
    /// Returns [`ThemePackageError`] when the stylesheet cannot be compiled
    /// against the declared immutable asset URLs.
    pub fn compile(
        self,
        asset_urls: &BTreeMap<String, String>,
        limits: ThemePackageLimits,
    ) -> Result<CompiledThemeRevision, ThemePackageError> {
        let css = compile_stylesheet(
            &self.css,
            &self.manifest,
            self.source_digest,
            asset_urls,
            limits,
        )?;
        let revision_digest = revision_digest(&self.manifest, css.digest(), &self.assets);
        Ok(CompiledThemeRevision {
            manifest: self.manifest,
            css,
            assets: self.assets,
            source_digest: self.source_digest,
            revision_digest,
        })
    }
}
impl CompiledThemeRevision {
    #[must_use]
    pub fn source_digest(&self) -> [u8; 32] {
        self.source_digest
    }
    #[must_use]
    pub fn revision_digest(&self) -> [u8; 32] {
        self.revision_digest
    }
    #[must_use]
    pub fn canonical_manifest(&self) -> &[u8] {
        &self.manifest
    }
    #[must_use]
    pub fn css(&self) -> &CompiledCss {
        &self.css
    }
    #[must_use]
    pub fn asset(&self, path: &str) -> Option<(&str, &[u8], [u8; 32])> {
        self.assets
            .get(path)
            .map(|asset| (asset.mime.as_str(), asset.bytes.as_slice(), asset.digest))
    }
}

fn validate_asset(
    path: &str,
    mime: AssetMime,
    bytes: &[u8],
    limits: ThemePackageLimits,
) -> Result<(), ThemePackageError> {
    if mime == AssetMime::Woff2 {
        if bytes.len() > limits.max_font_bytes {
            return Err(limit("font bytes"));
        }
        let decoded = wuff::decompress_woff2(bytes).map_err(|_| ThemePackageError::Mime {
            path: path.into(),
            declared: mime.as_str().into(),
        })?;
        if decoded.len() > limits.max_font_bytes {
            return Err(limit("font decoded bytes"));
        }
        return Ok(());
    }
    let format = mime.image_format().ok_or_else(|| ThemePackageError::Mime {
        path: path.into(),
        declared: mime.as_str().into(),
    })?;
    let image = ImageReader::with_format(Cursor::new(bytes), format)
        .decode()
        .map_err(|_| ThemePackageError::Mime {
            path: path.into(),
            declared: mime.as_str().into(),
        })?;
    let (width, height) = (image.width(), image.height());
    if width > limits.max_image_width || height > limits.max_image_height {
        return Err(limit("image dimensions"));
    }
    let frames = animation_frames(mime, bytes)?;
    if frames > limits.max_image_frames {
        return Err(limit("image frames"));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(frames as u64))
        .ok_or_else(|| limit("image pixels"))?;

    if pixels > limits.max_image_pixels {
        return Err(limit("image pixels"));
    }
    Ok(())
}

fn animation_frames(mime: AssetMime, bytes: &[u8]) -> Result<usize, ThemePackageError> {
    match mime {
        AssetMime::Png => {
            let mut offset = 8;
            while offset + 12 <= bytes.len() {
                let length =
                    u32::from_be_bytes(bytes[offset..offset + 4].try_into().map_err(|_| {
                        ThemePackageError::Mime {
                            path: "image".into(),
                            declared: "image/png".into(),
                        }
                    })?) as usize;
                if &bytes[offset + 4..offset + 8] == b"acTL" {
                    return Ok(u32::from_be_bytes(
                        bytes
                            .get(offset + 8..offset + 12)
                            .ok_or_else(|| ThemePackageError::Mime {
                                path: "image".into(),
                                declared: "image/png".into(),
                            })?
                            .try_into()
                            .map_err(|_| ThemePackageError::Mime {
                                path: "image".into(),
                                declared: "image/png".into(),
                            })?,
                    ) as usize);
                }
                offset =
                    offset
                        .checked_add(12 + length)
                        .ok_or_else(|| ThemePackageError::Mime {
                            path: "image".into(),
                            declared: "image/png".into(),
                        })?;
            }
            Ok(1)
        }
        AssetMime::Webp => Ok(bytes
            .windows(4)
            .filter(|chunk| *chunk == b"ANMF")
            .count()
            .max(1)),
        AssetMime::Avif | AssetMime::Jpeg => Ok(1),
        AssetMime::Woff2 => Ok(0),
    }
}
fn validate_member_name(path: &str) -> Result<(), ThemePackageError> {
    if path.is_empty()
        || path.contains('\0')
        || path.contains('\\')
        || path.starts_with('/')
        || path.get(1..2) == Some(":")
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(ThemePackageError::Member(path.into()));
    }
    if path != "theme.json" && path != "style.css" && !path.starts_with("assets/") {
        return Err(ThemePackageError::Member(path.into()));
    }
    Ok(())
}
fn validate_asset_path(path: &str) -> Result<(), ThemePackageError> {
    validate_member_name(path)?;
    if !path.starts_with("assets/") {
        return Err(ThemePackageError::Manifest(
            "asset path must be below assets/".into(),
        ));
    }
    Ok(())
}
fn require_image_default(
    assets: &BTreeMap<String, ThemeAsset>,
    path: &str,
) -> Result<(), ThemePackageError> {
    if !assets
        .get(path)
        .is_some_and(|asset| asset.mime != AssetMime::Woff2)
    {
        return Err(ThemePackageError::Manifest(format!(
            "default `{path}` is not a declared image"
        )));
    }
    Ok(())
}
fn validate_local_header(
    input: &[u8],
    start: usize,
    expected_name: &str,
    expected_method: zip::CompressionMethod,
    expected_crc: u32,
    expected_compressed: u64,
    expected_expanded: u64,
) -> Result<(), ThemePackageError> {
    let header = input
        .get(start..start + 30)
        .ok_or_else(|| ThemePackageError::Archive("truncated local header".into()))?;
    if &header[..4] != b"PK\x03\x04" {
        return Err(ThemePackageError::Archive(
            "local header disagreement".into(),
        ));
    }
    let flags = u16::from_le_bytes([header[6], header[7]]);
    let method = u16::from_le_bytes([header[8], header[9]]);
    let crc = u32::from_le_bytes([header[14], header[15], header[16], header[17]]);
    let compressed = u64::from(u32::from_le_bytes([
        header[18], header[19], header[20], header[21],
    ]));
    let expanded = u64::from(u32::from_le_bytes([
        header[22], header[23], header[24], header[25],
    ]));
    let name_len = usize::from(u16::from_le_bytes([header[26], header[27]]));
    let local_name = input
        .get(start + 30..start + 30 + name_len)
        .ok_or_else(|| ThemePackageError::Archive("truncated local filename".into()))?;
    let method_matches = match expected_method {
        zip::CompressionMethod::Stored => method == 0,
        zip::CompressionMethod::Deflated => method == 8,
        _ => false,
    };
    if flags & 1 != 0
        || !method_matches
        || std::str::from_utf8(local_name).ok() != Some(expected_name)
        || (flags & 8 == 0
            && (crc != expected_crc
                || compressed != expected_compressed
                || expanded != expected_expanded))
    {
        return Err(ThemePackageError::Archive(
            "local/central header disagreement".into(),
        ));
    }
    Ok(())
}

/// ZIP readers may index duplicate names by one entry; reject them from the
/// central directory before the reader has a chance to collapse that distinction.
fn validate_central_directory_names(input: &[u8]) -> Result<(), ThemePackageError> {
    let end = input
        .windows(4)
        .rposition(|window| window == b"PK\x05\x06")
        .ok_or_else(|| ThemePackageError::Archive("missing end of central directory".into()))?;
    let eocd = input
        .get(end..end + 22)
        .ok_or_else(|| ThemePackageError::Archive("truncated end of central directory".into()))?;
    let entries = u16::from_le_bytes([eocd[10], eocd[11]]) as usize;
    let central_start = u32::from_le_bytes([eocd[16], eocd[17], eocd[18], eocd[19]]) as usize;
    let mut offset = central_start;
    let mut names = BTreeSet::new();
    for _ in 0..entries {
        let header = input
            .get(offset..offset + 46)
            .ok_or_else(|| ThemePackageError::Archive("truncated central header".into()))?;
        if &header[..4] != b"PK\x01\x02" {
            return Err(ThemePackageError::Archive(
                "invalid central header signature".into(),
            ));
        }
        let name_len = u16::from_le_bytes([header[28], header[29]]) as usize;
        let extra_len = u16::from_le_bytes([header[30], header[31]]) as usize;
        let comment_len = u16::from_le_bytes([header[32], header[33]]) as usize;
        let name = input
            .get(offset + 46..offset + 46 + name_len)
            .ok_or_else(|| ThemePackageError::Archive("truncated central filename".into()))?;
        let name = std::str::from_utf8(name)
            .map_err(|_| ThemePackageError::Member("non-UTF-8 member name".into()))?;
        validate_member_name(name)?;
        if !names.insert(name) {
            return Err(ThemePackageError::Member(name.into()));
        }
        offset = offset
            .checked_add(46 + name_len + extra_len + comment_len)
            .ok_or_else(|| ThemePackageError::Archive("central directory overflow".into()))?;
    }
    Ok(())
}
fn source_digest(manifest: &[u8], css: &[u8], assets: &BTreeMap<String, ThemeAsset>) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(SOURCE_DOMAIN);
    hash.update((assets.len() as u64).to_be_bytes());
    frame(&mut hash, manifest);
    frame(&mut hash, css);
    for (path, asset) in assets {
        frame(&mut hash, path.as_bytes());
        frame(&mut hash, asset.mime.as_str().as_bytes());
        frame(&mut hash, &asset.bytes);
    }
    hash.finalize().into()
}
fn revision_digest(
    manifest: &[u8],
    css_digest: [u8; 32],
    assets: &BTreeMap<String, ThemeAsset>,
) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(REVISION_DOMAIN);
    hash.update((assets.len() as u64).to_be_bytes());
    frame(&mut hash, manifest);
    hash.update(css_digest);
    for (path, asset) in assets {
        frame(&mut hash, path.as_bytes());
        frame(&mut hash, asset.mime.as_str().as_bytes());
        hash.update(asset.digest);
    }
    hash.finalize().into()
}
fn frame(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}
fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn zip_error(error: &zip::result::ZipError) -> ThemePackageError {
    ThemePackageError::Archive(error.to_string())
}
fn limit(limit: &'static str) -> ThemePackageError {
    ThemePackageError::LimitExceeded { limit }
}

#[cfg(test)]
mod tests {
    use image::{DynamicImage, ImageBuffer, Rgb};

    use super::*;
    use std::io::Write;

    use zip::{ZipWriter, write::SimpleFileOptions};

    fn package(manifest: &str, css: &str) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("theme.json", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(manifest.as_bytes()).unwrap();
        writer
            .start_file("style.css", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(css.as_bytes()).unwrap();
        writer.finish().unwrap().into_inner()
    }

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (path, bytes) in entries {
            writer
                .start_file(*path, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn raster(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let image = DynamicImage::ImageRgb8(ImageBuffer::<Rgb<u8>, _>::from_pixel(
            width,
            height,
            Rgb([0x12, 0x34, 0x56]),
        ));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
        bytes.into_inner()
    }

    fn apng_with_frame_count(frames: u32) -> Vec<u8> {
        let mut png = raster(ImageFormat::Png, 1, 1);
        let mut chunk = Vec::from([0, 0, 0, 8, b'a', b'c', b'T', b'L']);
        chunk.extend_from_slice(&frames.to_be_bytes());
        chunk.extend_from_slice(&0_u32.to_be_bytes());
        let crc = chunk[4..].iter().fold(0xffff_ffff_u32, |crc, byte| {
            (0..8).fold(crc ^ u32::from(*byte), |crc, _| {
                if crc & 1 == 0 {
                    crc >> 1
                } else {
                    (crc >> 1) ^ 0xedb8_8320
                }
            })
        }) ^ 0xffff_ffff;
        chunk.extend_from_slice(&crc.to_be_bytes());
        png.splice(33..33, chunk);
        png
    }

    fn single_asset_package(path: &str, mime: &str, bytes: &[u8]) -> Vec<u8> {
        let manifest = format!(
            r#"{{"schema":1,"name":"Paper","style_contract":1,"assets":{{"{path}":"{mime}"}},"defaults":{{}}}}"#
        );
        archive(&[
            ("theme.json", manifest.as_bytes()),
            ("style.css", b"body {}"),
            (path, bytes),
        ])
    }

    #[test]
    fn canonicalizes_closed_manifest_before_source_digesting() {
        let package = package(
            r#"{"style_contract":1,"defaults":{},"name":"Paper","assets":{},"schema":1}"#,
            "body { color: black; }",
        );
        let validated = validate_theme_package(&package, ThemePackageLimits::default()).unwrap();
        assert_eq!(
            validated.canonical_manifest(),
            br#"{"assets":{},"defaults":{},"name":"Paper","schema":1,"style_contract":1}"#
        );
    }

    #[test]
    fn rejects_unknown_manifest_fields_before_any_result_is_minted() {
        let package = package(
            r#"{"schema":1,"name":"Paper","style_contract":1,"assets":{},"defaults":{},"unexpected":true}"#,
            "body {}",
        );
        assert!(matches!(
            validate_theme_package(&package, ThemePackageLimits::default()),
            Err(ThemePackageError::Manifest(_))
        ));
    }

    #[test]
    fn rejects_unix_symlink_members_before_manifest_processing() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("theme.json", SimpleFileOptions::default())
            .unwrap();
        writer
            .write_all(
                br#"{"schema":1,"name":"Paper","style_contract":1,"assets":{},"defaults":{}}"#,
            )
            .unwrap();
        writer
            .start_file("style.css", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"body {}").unwrap();
        writer
            .add_symlink("assets/link", "target", SimpleFileOptions::default())
            .unwrap();
        let archive = writer.finish().unwrap().into_inner();
        assert!(matches!(
            validate_theme_package(&archive, ThemePackageLimits::default()),
            Err(ThemePackageError::Member(path)) if path == "assets/link"
        ));
    }

    #[test]
    fn rejects_archive_byte_limit_and_illegal_member_paths() {
        let limits = ThemePackageLimits {
            max_archive_bytes: 1,
            ..ThemePackageLimits::default()
        };
        assert!(matches!(
            validate_theme_package(b"PK", limits),
            Err(ThemePackageError::LimitExceeded {
                limit: "archive bytes"
            })
        ));
        for path in [
            "",
            "../x",
            "assets/../x",
            "assets\\x",
            "/assets/x",
            "C:assets/x",
            "assets//x",
            "assets/./x",
            "unexpected",
        ] {
            assert!(validate_member_name(path).is_err(), "{path}");
        }
    }

    #[test]
    fn frame_encoding_distinguishes_field_boundaries_and_asset_order_is_canonical() {
        let asset_a = ThemeAsset {
            mime: AssetMime::Png,
            bytes: b"bc".to_vec(),
            digest: sha256(b"bc"),
        };
        let asset_b = ThemeAsset {
            mime: AssetMime::Png,
            bytes: b"c".to_vec(),
            digest: sha256(b"c"),
        };
        let mut first = BTreeMap::new();
        first.insert("assets/a".to_owned(), asset_a);
        first.insert("assets/ab".to_owned(), asset_b);
        let mut second = BTreeMap::new();
        second.insert(
            "assets/ab".to_owned(),
            ThemeAsset {
                mime: AssetMime::Png,
                bytes: b"c".to_vec(),
                digest: sha256(b"c"),
            },
        );
        second.insert(
            "assets/a".to_owned(),
            ThemeAsset {
                mime: AssetMime::Png,
                bytes: b"bc".to_vec(),
                digest: sha256(b"bc"),
            },
        );
        assert_eq!(
            source_digest(b"{}", b"x", &first),
            source_digest(b"{}", b"x", &second)
        );
        assert_ne!(
            source_digest(b"{}", b"abc", &BTreeMap::new()),
            source_digest(b"{}", b"bc", &BTreeMap::new())
        );
    }

    #[test]
    fn counts_declared_animation_frames_without_substring_css_logic() {
        assert_eq!(
            animation_frames(AssetMime::Webp, b"ANMFxxxxANMF").unwrap(),
            2
        );
        let mut apng = vec![0; 20];
        apng[8..12].copy_from_slice(&4_u32.to_be_bytes());
        apng[12..16].copy_from_slice(b"acTL");
        apng[16..20].copy_from_slice(&3_u32.to_be_bytes());
        assert_eq!(animation_frames(AssetMime::Png, &apng).unwrap(), 3);
    }
    #[test]
    fn rejects_file_count_and_missing_required_members() {
        let package = archive(&[
            ("theme.json", b"{}"),
            ("style.css", b""),
            ("assets/x", b"x"),
        ]);
        assert!(matches!(
            validate_theme_package(
                &package,
                ThemePackageLimits {
                    max_files: 2,
                    ..ThemePackageLimits::default()
                }
            ),
            Err(ThemePackageError::LimitExceeded {
                limit: "file count"
            })
        ));
        assert!(matches!(
            validate_theme_package(&archive(&[("theme.json", b"{}")]), ThemePackageLimits::default()),
            Err(ThemePackageError::Member(path)) if path == "missing style.css"
        ));
    }

    #[test]
    fn rejects_non_image_defaults_and_malformed_woff2() {
        let mut assets = BTreeMap::new();
        assets.insert(
            "assets/font.woff2".to_owned(),
            ThemeAsset {
                mime: AssetMime::Woff2,
                bytes: Vec::new(),
                digest: sha256(b""),
            },
        );
        assert!(require_image_default(&assets, "assets/font.woff2").is_err());
        assert!(matches!(
            validate_asset(
                "assets/font.woff2",
                AssetMime::Woff2,
                b"not a woff2",
                ThemePackageLimits::default()
            ),
            Err(ThemePackageError::Mime { .. })
        ));
    }

    #[test]
    fn rejects_local_header_member_disagreement() {
        let mut header = vec![0_u8; 31];
        header[..4].copy_from_slice(b"PK\x03\x04");
        header[26..28].copy_from_slice(&1_u16.to_le_bytes());
        header[30] = b'x';
        assert!(
            validate_local_header(&header, 0, "y", zip::CompressionMethod::Stored, 0, 0, 0,)
                .is_err()
        );
    }
    #[test]
    fn rejects_manifest_version_and_unexpected_asset_members() {
        let invalid_version = package(
            r#"{"schema":2,"name":"Paper","style_contract":1,"assets":{},"defaults":{}}"#,
            "body {}",
        );
        assert!(matches!(
            validate_theme_package(&invalid_version, ThemePackageLimits::default()),
            Err(ThemePackageError::Manifest(_))
        ));
        let unexpected = archive(&[
            (
                "theme.json",
                br#"{"schema":1,"name":"Paper","style_contract":1,"assets":{},"defaults":{}}"#,
            ),
            ("style.css", b"body {}"),
            ("assets/undeclared.png", b"x"),
        ]);
        assert!(matches!(
            validate_theme_package(&unexpected, ThemePackageLimits::default()),
            Err(ThemePackageError::UndeclaredAsset(path)) if path == "assets/undeclared.png"
        ));
    }
    #[test]
    fn rejects_per_file_and_expanded_limits_before_manifest_use() {
        let package = archive(&[("theme.json", b"{}"), ("style.css", b"0123456789")]);
        assert!(matches!(
            validate_theme_package(
                &package,
                ThemePackageLimits {
                    max_file_bytes: 1,
                    ..ThemePackageLimits::default()
                }
            ),
            Err(ThemePackageError::LimitExceeded { .. })
        ));
        assert!(matches!(
            validate_theme_package(
                &package,
                ThemePackageLimits {
                    max_expanded_bytes: 1,
                    ..ThemePackageLimits::default()
                }
            ),
            Err(ThemePackageError::LimitExceeded {
                limit: "expanded bytes"
            })
        ));
    }
    #[test]
    fn rejects_empty_or_duplicate_header_defaults() {
        for defaults in [r#"{"header":[]}"#, r#"{"header":["assets/a","assets/a"]}"#] {
            let manifest = format!(
                r#"{{"schema":1,"name":"Paper","style_contract":1,"assets":{{}},"defaults":{defaults}}}"#
            );
            assert!(matches!(
                validate_theme_package(
                    &package(&manifest, "body {}"),
                    ThemePackageLimits::default()
                ),
                Err(ThemePackageError::Manifest(_))
            ));
        }
    }
    #[test]
    fn rejects_missing_manifest_fields_empty_name_and_unsupported_mime() {
        for manifest in [
            r#"{"schema":1,"style_contract":1,"assets":{},"defaults":{}}"#,
            r#"{"schema":1,"name":"","style_contract":1,"assets":{},"defaults":{}}"#,
            r#"{"schema":1,"name":"Paper","style_contract":2,"assets":{},"defaults":{}}"#,
            r#"{"schema":1,"name":"Paper","style_contract":1,"assets":{"assets/x":"image/svg+xml"},"defaults":{}}"#,
        ] {
            assert!(
                validate_theme_package(
                    &package(manifest, "body {}"),
                    ThemePackageLimits::default()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn rejects_directory_members_and_missing_declared_assets() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("theme.json", SimpleFileOptions::default())
            .unwrap();
        writer
            .write_all(
                br#"{"schema":1,"name":"Paper","style_contract":1,"assets":{"assets/missing.png":"image/png"},"defaults":{}}"#,
            )
            .unwrap();
        writer
            .start_file("style.css", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"body {}").unwrap();
        writer
            .add_directory("assets/empty/", SimpleFileOptions::default())
            .unwrap();
        let archive = writer.finish().unwrap().into_inner();
        assert!(matches!(
            validate_theme_package(&archive, ThemePackageLimits::default()),
            Err(ThemePackageError::Member(path)) if path == "assets/empty/"
        ));
        let missing = package(
            r#"{"schema":1,"name":"Paper","style_contract":1,"assets":{"assets/missing.png":"image/png"},"defaults":{}}"#,
            "body {}",
        );
        assert!(matches!(
            validate_theme_package(&missing, ThemePackageLimits::default()),
            Err(ThemePackageError::UndeclaredAsset(path)) if path == "assets/missing.png"
        ));
    }

    #[test]
    fn accepts_each_allowlisted_raster_and_rejects_declared_detected_mismatches() {
        let formats = [
            ("assets/image.png", "image/png", ImageFormat::Png),
            ("assets/image.jpeg", "image/jpeg", ImageFormat::Jpeg),
            ("assets/image.webp", "image/webp", ImageFormat::WebP),
        ];
        for (path, mime, format) in formats {
            let bytes = raster(format, 1, 1);
            validate_theme_package(
                &single_asset_package(path, mime, &bytes),
                ThemePackageLimits::default(),
            )
            .unwrap();
            for (other_path, other_mime, _) in formats {
                if other_mime == mime {
                    continue;
                }
                assert!(matches!(
                    validate_theme_package(
                        &single_asset_package(other_path, other_mime, &bytes),
                        ThemePackageLimits::default(),
                    ),
                    Err(ThemePackageError::Mime { .. })
                ));
            }
        }
    }

    #[test]
    fn accepts_decoded_avif_and_rejects_corrupt_av1_payload() {
        let avif = include_bytes!("theme_package/fixtures/one-pixel.avif").to_vec();
        let package = single_asset_package("assets/image.avif", "image/avif", &avif);
        validate_theme_package(&package, ThemePackageLimits::default()).unwrap();

        let mdat = avif
            .windows(4)
            .position(|window| window == b"mdat")
            .expect("AVIF fixture has an mdat box");
        let box_start = mdat.checked_sub(4).unwrap();
        let box_size = usize::try_from(u32::from_be_bytes(
            avif[box_start..mdat].try_into().unwrap(),
        ))
        .unwrap();
        let mut corrupt = avif;
        corrupt[mdat + 4..box_start + box_size].fill(0xff);

        assert!(matches!(
            validate_theme_package(
                &single_asset_package("assets/image.avif", "image/avif", &corrupt),
                ThemePackageLimits::default(),
            ),
            Err(ThemePackageError::Mime { .. })
        ));
    }

    #[test]
    fn accepts_valid_woff2_and_bounds_decoded_expansion() {
        let font = include_bytes!("theme_package/fixtures/roboto-regular.woff2");
        let decoded = wuff::decompress_woff2(font).unwrap();
        assert!(decoded.len() > font.len());
        validate_asset(
            "assets/font.woff2",
            AssetMime::Woff2,
            font,
            ThemePackageLimits::default(),
        )
        .unwrap();
        assert!(matches!(
            validate_asset(
                "assets/font.woff2",
                AssetMime::Woff2,
                font,
                ThemePackageLimits {
                    max_font_bytes: decoded.len() - 1,
                    ..ThemePackageLimits::default()
                }
            ),
            Err(ThemePackageError::LimitExceeded {
                limit: "font decoded bytes"
            })
        ));
    }

    #[test]
    fn rejects_a_decodable_animated_png_at_the_frame_limit() {
        let png = apng_with_frame_count(2);
        assert!(
            ImageReader::with_format(Cursor::new(&png), ImageFormat::Png)
                .decode()
                .is_ok()
        );
        assert!(matches!(
            validate_theme_package(
                &single_asset_package("assets/animated.png", "image/png", &png),
                ThemePackageLimits {
                    max_image_frames: 1,
                    ..ThemePackageLimits::default()
                }
            ),
            Err(ThemePackageError::LimitExceeded {
                limit: "image frames"
            })
        ));
    }

    #[test]
    fn rejects_valid_rasters_at_dimension_and_pixel_limits() {
        let bytes = raster(ImageFormat::Png, 2, 3);
        let package = single_asset_package("assets/image.png", "image/png", &bytes);
        assert!(matches!(
            validate_theme_package(
                &package,
                ThemePackageLimits {
                    max_image_width: 1,
                    ..ThemePackageLimits::default()
                }
            ),
            Err(ThemePackageError::LimitExceeded {
                limit: "image dimensions"
            })
        ));
        assert!(matches!(
            validate_theme_package(
                &package,
                ThemePackageLimits {
                    max_image_pixels: 5,
                    ..ThemePackageLimits::default()
                }
            ),
            Err(ThemePackageError::LimitExceeded {
                limit: "image pixels"
            })
        ));
    }

    #[test]
    fn rejects_duplicate_encrypted_and_header_disagreeing_zip_members() {
        let manifest =
            br#"{"schema":1,"name":"Paper","style_contract":1,"assets":{},"defaults":{}}"#;
        let mut duplicate = archive(&[("theme.json", manifest), ("style.css", b"body {}")]);
        let end_of_central_directory = duplicate
            .windows(4)
            .rposition(|window| window == b"PK\x05\x06")
            .unwrap();
        let style_central_header = duplicate[..end_of_central_directory]
            .windows(4)
            .rposition(|window| window == b"PK\x01\x02")
            .unwrap();
        let duplicate_central_entry =
            duplicate[style_central_header..end_of_central_directory].to_vec();
        duplicate.splice(
            end_of_central_directory..end_of_central_directory,
            duplicate_central_entry.iter().copied(),
        );
        let end_of_central_directory = end_of_central_directory + duplicate_central_entry.len();
        duplicate[end_of_central_directory + 8..end_of_central_directory + 10]
            .copy_from_slice(&3_u16.to_le_bytes());
        duplicate[end_of_central_directory + 10..end_of_central_directory + 12]
            .copy_from_slice(&3_u16.to_le_bytes());
        let central_size = u32::from_le_bytes(
            duplicate[end_of_central_directory + 12..end_of_central_directory + 16]
                .try_into()
                .unwrap(),
        );
        let duplicate_central_size = central_size
            .checked_add(u32::try_from(duplicate_central_entry.len()).unwrap())
            .unwrap();
        duplicate[end_of_central_directory + 12..end_of_central_directory + 16]
            .copy_from_slice(&duplicate_central_size.to_le_bytes());
        assert!(matches!(
            validate_theme_package(&duplicate, ThemePackageLimits::default()),
            Err(ThemePackageError::Member(path)) if path == "style.css"
        ));

        let mut encrypted = archive(&[("theme.json", manifest), ("style.css", b"body {}")]);
        encrypted[6] |= 1;
        let central = encrypted
            .windows(4)
            .position(|window| window == b"PK\x01\x02")
            .unwrap();
        encrypted[central + 8] |= 1;
        assert!(validate_theme_package(&encrypted, ThemePackageLimits::default()).is_err());

        let mut disagreement = archive(&[("theme.json", manifest), ("style.css", b"body {}")]);
        let local_name = disagreement
            .windows(b"style.css".len())
            .position(|window| window == b"style.css")
            .unwrap();
        disagreement[local_name] = b'x';
        assert!(validate_theme_package(&disagreement, ThemePackageLimits::default()).is_err());
    }

    #[test]
    fn pins_published_revision_digest_encoding() {
        let manifest = br#"{"assets":{"assets/a.png":"image/png"},"defaults":{},"name":"Paper","schema":1,"style_contract":1}"#;
        let mut assets = BTreeMap::new();
        assets.insert(
            "assets/a.png".to_owned(),
            ThemeAsset {
                mime: AssetMime::Png,
                bytes: b"asset bytes".to_vec(),
                digest: sha256(b"asset bytes"),
            },
        );
        assert_eq!(
            revision_digest(manifest, sha256(b"compiled css"), &assets),
            [
                0x37, 0x9a, 0x58, 0xca, 0x67, 0xca, 0x81, 0x5f, 0xc3, 0xc2, 0xd2, 0x46, 0x46, 0x50,
                0xff, 0xd6, 0x99, 0x73, 0x9d, 0x09, 0x0d, 0x18, 0xbd, 0x44, 0xe2, 0x4b, 0xba, 0x5a,
                0xf1, 0x0e, 0x1f, 0x7a,
            ]
        );
    }
    #[test]
    fn rejects_compressed_byte_limit_from_central_metadata() {
        let package = archive(&[("theme.json", b"{}"), ("style.css", b"body {}")]);
        assert!(matches!(
            validate_theme_package(
                &package,
                ThemePackageLimits {
                    max_compressed_bytes: 1,
                    ..ThemePackageLimits::default()
                }
            ),
            Err(ThemePackageError::LimitExceeded { .. })
        ));
    }
}
