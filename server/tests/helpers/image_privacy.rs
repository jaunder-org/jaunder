//! Independent TEST-only image observations shared by the real upload routes.
//! Values stay in memory; assertion diagnostics never expose private contents.
use std::io::Cursor;

use image::AnimationDecoder;
use tempfile::TempDir;

/// Decode only formats already enabled in the pinned application dependency.
/// GIF's encoded transparency controls are tested separately; no new codec is enabled.
pub fn rgba_frames(bytes: &[u8], mime: &str) -> Option<Vec<image::Frame>> {
    match mime {
        "image/png" => {
            let decoder = image::codecs::png::PngDecoder::new(Cursor::new(bytes))
                .expect("independent PNG decoder");
            if decoder.is_apng().expect("PNG animation detection") {
                Some(
                    decoder
                        .apng()
                        .expect("APNG decoder")
                        .into_frames()
                        .collect_frames()
                        .expect("independent APNG frames"),
                )
            } else {
                Some(vec![image::Frame::new(
                    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
                        .expect("independent PNG decode")
                        .into_rgba8(),
                )])
            }
        }
        "image/webp" => Some(
            image::codecs::webp::WebPDecoder::new(Cursor::new(bytes))
                .expect("independent WebP decoder")
                .into_frames()
                .collect_frames()
                .expect("independent WebP frames"),
        ),
        _ => None,
    }
}

pub fn jpeg_privacy_tags(bytes: &[u8]) -> serde_json::Map<String, serde_json::Value> {
    oracle_tags(bytes, &["-EXIF:all", "-XMP:all", "-IPTC:all", "-Comment"])
}

pub fn oracle_tags(bytes: &[u8], arguments: &[&str]) -> serde_json::Map<String, serde_json::Value> {
    let mut options = vec!["-j", "-G1", "-n", "-a"];
    options.extend_from_slice(arguments);
    let mut documents: Vec<serde_json::Map<String, serde_json::Value>> =
        serde_json::from_slice(&jpeg_oracle_bytes(bytes, &options)).expect("oracle JSON");
    documents.remove(0)
}

pub fn jpeg_oracle_bytes(bytes: &[u8], arguments: &[&str]) -> Vec<u8> {
    let directory = TempDir::new().expect("oracle directory");
    let path = directory.path().join("image");
    std::fs::write(&path, bytes).expect("oracle input");
    let output = std::process::Command::new("exiftool")
        .args(arguments)
        .arg(path)
        .output()
        .expect("pinned test-only ExifTool");
    assert!(
        output.status.success(),
        "independent image observation failed"
    );
    output.stdout
}
