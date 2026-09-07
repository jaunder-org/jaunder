//! Validated, compiled custom-theme fixtures shared by storage tests.

use std::{collections::BTreeMap, fmt::Write as _, sync::Arc};

use common::ids::ThemeId;
use host::theme_package::{CompiledThemeRevision, ThemePackageLimits, validate_theme_package};

use crate::{
    ThemeDraft, ThemeOwner, ThemeQuotaLimits, ThemeStorage, WriteScope, test_support::confirmed,
};

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn stored_theme_package() -> Vec<u8> {
    let entries = [
        (
            "theme.json",
            br#"{"schema":1,"name":"Test","style_contract":1,"assets":{"assets/pixel.png":"image/png"},"defaults":{"logo":"assets/pixel.png"}}"#
                .as_slice(),
        ),
        ("style.css", b"body { color: black; }".as_slice()),
        (
            "assets/pixel.png",
            b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x01\x00\x00\x00\x01\x08\x04\x00\x00\x00\xb5\x1c\x0c\x02\x00\x00\x00\x0b\x49\x44\x41\x54\x78\xda\x63\x64\xf8\x0f\x00\x01\x05\x01\x01\x27\x18\xe3\x66\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82"
                .as_slice(),
        ),
    ];
    let mut archive = Vec::new();
    let mut central = Vec::new();
    for (name, content) in entries {
        let offset = u32::try_from(archive.len()).expect("fixture archive offset fits");
        push_u32(&mut archive, 0x0403_4b50);
        push_u16(&mut archive, 20);
        push_u16(&mut archive, 0);
        push_u16(&mut archive, 0);
        push_u16(&mut archive, 0);
        push_u16(&mut archive, 0);
        push_u32(&mut archive, crc32(content));
        push_u32(
            &mut archive,
            u32::try_from(content.len()).expect("fixture entry fits"),
        );
        push_u32(
            &mut archive,
            u32::try_from(content.len()).expect("fixture entry fits"),
        );
        push_u16(
            &mut archive,
            u16::try_from(name.len()).expect("fixture name fits"),
        );
        push_u16(&mut archive, 0);
        archive.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(content);

        push_u32(&mut central, 0x0201_4b50);
        push_u16(&mut central, 20);
        push_u16(&mut central, 20);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u32(&mut central, crc32(content));
        push_u32(
            &mut central,
            u32::try_from(content.len()).expect("fixture entry fits"),
        );
        push_u32(
            &mut central,
            u32::try_from(content.len()).expect("fixture entry fits"),
        );
        push_u16(
            &mut central,
            u16::try_from(name.len()).expect("fixture name fits"),
        );
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u32(&mut central, 0);
        push_u32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let central_offset = u32::try_from(archive.len()).expect("fixture archive fits");
    archive.extend_from_slice(&central);
    push_u32(&mut archive, 0x0605_4b50);
    push_u16(&mut archive, 0);
    push_u16(&mut archive, 0);
    push_u16(&mut archive, 3);
    push_u16(&mut archive, 3);
    push_u32(
        &mut archive,
        u32::try_from(central.len()).expect("fixture archive fits"),
    );
    push_u32(&mut archive, central_offset);
    push_u16(&mut archive, 0);
    archive
}

/// Returns a valid compiled theme with an immutable stylesheet and PNG asset.
///
/// # Panics
///
/// Panics if the static fixture no longer satisfies the Theme Package contract.
#[must_use]
pub fn compiled_theme_fixture() -> CompiledThemeRevision {
    validate_theme_package(&stored_theme_package(), ThemePackageLimits::default())
        .expect("valid theme fixture")
        .compile(&BTreeMap::new(), ThemePackageLimits::default())
        .expect("compile fixture")
}

/// Returns one-theme quota limits for `bytes` of immutable content.
///
#[must_use]
pub fn theme_quota_limits(bytes: i64) -> ThemeQuotaLimits {
    ThemeQuotaLimits {
        active_themes: 1,
        retained_revisions: 1,
        logical_bytes: bytes,
        site_retained_revisions: 1,
        site_physical_bytes: bytes,
    }
}

/// Creates one site catalog entry with the compiled fixture as its mutable draft.
///
/// # Panics
///
/// Panics if the fixture transaction does not commit successfully.
pub async fn create_site_theme(
    themes: Arc<dyn ThemeStorage>,
    scope: WriteScope,
    compiled: &CompiledThemeRevision,
) -> ThemeId {
    let draft = ThemeDraft {
        theme_id: ThemeId::from(0),
        manifest: compiled.canonical_manifest().to_vec(),
        stylesheet: b"body { color: black; }".to_vec(),
        source_digest: {
            let digest = compiled.source_digest();
            let mut hex = String::with_capacity(digest.len() * 2);
            for byte in digest {
                let _ = write!(hex, "{byte:02x}");
            }
            hex.parse().expect("valid source digest")
        },
    };
    confirmed(
        scope
            .run(move |transaction| {
                Box::pin(async move {
                    themes
                        .create_theme(
                            transaction,
                            ThemeOwner::Site,
                            "Test",
                            &draft,
                            theme_quota_limits(i64::MAX),
                        )
                        .await
                })
            })
            .await
            .expect("create theme"),
    )
}
