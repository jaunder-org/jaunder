//! Validated, compiled custom-theme fixture for storage tests and browser seeding.

use std::{collections::BTreeMap, sync::Arc};

use common::{MutationOutcome, ids::ThemeId};
use host::{
    theme_package,
    theme_package::{CompiledThemeRevision, ThemePackageLimits},
};

use crate::{ThemeDraft, ThemeDraftAsset, ThemeOwner, ThemeQuotaLimits, ThemeStorage, WriteScope};

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

fn digest_to_lowercase_hex(digest: &[u8]) -> String {
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = std::fmt::Write::write_fmt(&mut hex, format_args!("{byte:02x}"));
    }
    hex
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn stored_theme_package() -> anyhow::Result<Vec<u8>> {
    let entries = [
        (
            "theme.json",
            br#"{"schema":1,"name":"Test","style_contract":1,"assets":{"assets/pixel.png":"image/png"},"defaults":{"logo":"assets/pixel.png","header":["assets/pixel.png"]}}"#
                .as_slice(),
        ),
        (
            "style.css",
            br":root {
  outline-color: rgb(1 2 3);
  position: fixed;
  inset: 0;
  z-index: 2147483647;
  transform: translateZ(0);
  filter: contrast(1);
  overflow: visible;
}
.j-post {
  position: absolute;
  top: -100vh;
  right: -100vw;
  bottom: -100vh;
  left: -100vw;
  z-index: 2147483647;
}"
                .as_slice(),
        ),
        (
            "assets/pixel.png",
            b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x01\x00\x00\x00\x01\x08\x04\x00\x00\x00\xb5\x1c\x0c\x02\x00\x00\x00\x0b\x49\x44\x41\x54\x78\xda\x63\x64\xf8\x0f\x00\x01\x05\x01\x01\x27\x18\xe3\x66\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82"
                .as_slice(),
        ),
    ];
    let mut archive = Vec::new();
    let mut central = Vec::new();
    for (name, content) in entries {
        let offset = u32::try_from(archive.len())?;
        push_u32(&mut archive, 0x0403_4b50);
        push_u16(&mut archive, 20);
        push_u16(&mut archive, 0);
        push_u16(&mut archive, 0);
        push_u16(&mut archive, 0);
        push_u16(&mut archive, 0);
        push_u32(&mut archive, crc32(content));
        push_u32(&mut archive, u32::try_from(content.len())?);
        push_u32(&mut archive, u32::try_from(content.len())?);
        push_u16(&mut archive, u16::try_from(name.len())?);
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
        push_u32(&mut central, u32::try_from(content.len())?);
        push_u32(&mut central, u32::try_from(content.len())?);
        push_u16(&mut central, u16::try_from(name.len())?);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u32(&mut central, 0);
        push_u32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let central_offset = u32::try_from(archive.len())?;
    archive.extend_from_slice(&central);
    push_u32(&mut archive, 0x0605_4b50);
    push_u16(&mut archive, 0);
    push_u16(&mut archive, 0);
    push_u16(&mut archive, 3);
    push_u16(&mut archive, 3);
    push_u32(&mut archive, u32::try_from(central.len())?);
    push_u32(&mut archive, central_offset);
    push_u16(&mut archive, 0);
    Ok(archive)
}

/// Returns a valid compiled theme with an immutable stylesheet and PNG asset.
///
/// # Errors
///
/// Returns an error if the static fixture no longer satisfies the Theme Package contract.
pub fn try_compiled_theme_fixture() -> anyhow::Result<CompiledThemeRevision> {
    let validated = theme_package::validate_theme_package(
        &stored_theme_package()?,
        ThemePackageLimits::default(),
    )?; // cov:ignore
    Ok(validated.compile(&BTreeMap::new(), ThemePackageLimits::default())?)
}

/// Returns one-theme quota limits with room for both the mutable fixture draft
/// and `bytes` of immutable content.
///
/// The fixture's canonical manifest, stylesheet, and asset bytes remain charged
/// while its revision is published, so the limit reserves conservative draft
/// headroom rather than modeling published content alone.
#[must_use]
pub fn theme_quota_limits(bytes: i64) -> ThemeQuotaLimits {
    let total_bytes = bytes.saturating_mul(4);
    ThemeQuotaLimits {
        active_themes: 1,
        retained_revisions: 1,
        logical_bytes: total_bytes,
        site_retained_revisions: 1,
        site_physical_bytes: total_bytes,
    }
}

/// Creates one catalog entry with the compiled fixture as its mutable draft.
///
/// # Errors
///
/// Returns an error if the draft cannot be created or its commit is indeterminate.
pub async fn try_create_theme(
    themes: Arc<dyn ThemeStorage>,
    scope: WriteScope,
    owner: ThemeOwner,
    compiled: &CompiledThemeRevision,
) -> anyhow::Result<ThemeId> {
    let hex = digest_to_lowercase_hex(&compiled.source_digest());
    let assets = compiled
        .assets()
        .map(|(path, mime, bytes, digest)| {
            let hex = digest_to_lowercase_hex(&digest);
            Ok(ThemeDraftAsset {
                path: path.to_owned(),
                mime: mime.to_owned(),
                bytes: bytes.to_vec(),
                digest: hex
                    .parse()
                    .map_err(|_| anyhow::anyhow!("fixture asset digest was invalid"))?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let draft = ThemeDraft {
        theme_id: ThemeId::from(0),
        manifest: compiled.canonical_manifest().to_vec(),
        stylesheet: compiled.css().bytes().to_vec(),
        source_digest: hex
            .parse()
            .map_err(|_| anyhow::anyhow!("fixture source digest was invalid"))?,
        assets,
    };
    let outcome = scope
        .run(move |transaction| {
            Box::pin(async move {
                themes
                    .create_theme(
                        transaction,
                        owner,
                        "Test",
                        &draft,
                        theme_quota_limits(i64::MAX),
                    )
                    .await
            })
        })
        .await?;
    match outcome {
        MutationOutcome::Confirmed(theme_id) => Ok(theme_id),
        MutationOutcome::CommitIndeterminate(_) => {
            Err(anyhow::anyhow!("fixture theme commit was indeterminate"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_seed_fixture_remains_a_valid_theme_package() {
        super::try_compiled_theme_fixture().expect("compile browser seed fixture");
    }
    // guard:no-backend — mocked theme storage and injected acknowledgement loss isolate the outcome mapping
    #[tokio::test]
    async fn indeterminate_fixture_theme_commit_is_reported_as_an_error() {
        let mut themes = crate::MockThemeStorage::new();
        themes
            .expect_create_theme()
            .once()
            .returning(|_, _, _, _, _| Ok(ThemeId::from(1)));
        let compiled = super::try_compiled_theme_fixture().expect("compile fixture");

        let error = super::try_create_theme(
            Arc::new(themes),
            WriteScope::mock().with_commit_acknowledgement_loss_after_commit_for_test(),
            ThemeOwner::Site,
            &compiled,
        )
        .await
        .expect_err("lost commit acknowledgement is indeterminate");

        assert!(error.to_string().contains("commit was indeterminate"));
    }
}
