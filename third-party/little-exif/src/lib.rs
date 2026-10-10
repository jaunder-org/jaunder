// Copyright © 2022-2026 Tobias J. Prisching <tobias.prisching@icloud.com> and CONTRIBUTORS
// See https://github.com/TechnikTobi/little_exif#license for licensing details

#![allow(clippy::needless_return)]
#![allow(clippy::upper_case_acronyms)]
#![allow(clippy::identity_op)]
#![allow(clippy::redundant_field_names)]
#![warn(clippy::unwrap_used)]
#![allow(unused_parens)]

//! A small crate for reading and writing (some) EXIF data, written entirely in Rust. Currently supports
//! - JPEG / JPG
//! - JXL
//! - HEIF / HEIC / HIF / AVIF
//! - PNG
//! - TIFF
//! - WebP (only lossless and extended)
//!
//! files and a few dozen tags in IFD0 and ExifIFD.
//! Interaction is done via the [`Metadata`](metadata/struct.Metadata.html)
//! struct and the [`ExifTag`](exif_tag/enum.ExifTag.html) enum.
//!
//! # Usage
//! ## Write EXIF data
//! ```
//! use little_exif::metadata::Metadata;
//! use little_exif::exif_tag::ExifTag;
//!
//! # let directory = tempfile::tempdir().unwrap();
//! # let image_path = directory.path().join("image.png");
//! # std::fs::write(&image_path, include_bytes!("../../../host/src/image_sanitizer_fixtures/png-original.png")).unwrap();
//! let mut metadata = Metadata::new();
//! metadata.set_tag(
//!     ExifTag::ImageDescription("Hello World!".to_string())
//! );
//! metadata.write_to_file(&image_path).unwrap();
//! ```

#![forbid(unsafe_code)]
#![crate_type = "lib"]
#![crate_name = "little_exif"]

mod general_file_io;
pub mod ifd;

mod heif;
mod jpg;
mod jxl;
mod png;
mod tiff;
mod util;
mod webp;
mod xmp;

pub mod endian;
pub mod exif_tag;
pub mod exif_tag_format;
pub mod filetype;
pub mod metadata;
pub mod rational;
pub mod u8conversion;
pub use heif::metadata_view::HeifMetadataRole;
