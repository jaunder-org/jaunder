//! Private binary embedding and verification for producer-staged system artifacts.

use std::collections::BTreeSet;

use host::system_theme::{SystemArtifactInventory, SystemArtifactLoadError, SystemArtifactSource};
use rust_embed::RustEmbed;

// cov:ignore-start: RustEmbed derive expansion is compiler-generated rather than handwritten runtime behavior.
#[derive(RustEmbed)]
// cov:ignore-stop
#[folder = "$OUT_DIR/system-artifacts/"]
struct EmbeddedSystemArtifacts;

struct EmbeddedSource;

impl SystemArtifactSource for EmbeddedSource {
    fn paths(&self) -> Result<BTreeSet<String>, SystemArtifactLoadError> {
        Ok(EmbeddedSystemArtifacts::iter()
            .map(std::borrow::Cow::into_owned)
            .collect())
    }

    fn read(&self, path: &str) -> Result<Vec<u8>, SystemArtifactLoadError> {
        EmbeddedSystemArtifacts::get(path)
            .map(|asset| asset.data.into_owned())
            .ok_or_else(|| {
                SystemArtifactLoadError::Invalid(format!("embedded artifact is missing: {path}"))
            })
    }
}

/// Replays the binary's private producer inventory before startup admission.
///
/// # Errors
///
/// Returns an error when the binary has no production inventory or any embedded
/// role fails the common source/compiled-content verification boundary.
pub(crate) fn load() -> Result<SystemArtifactInventory, SystemArtifactLoadError> {
    host::system_theme::load_system_artifact_inventory(&EmbeddedSource)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_embedded_member_fails_closed_with_its_path() {
        assert!(
            matches!(EmbeddedSource.read("missing"), Err(SystemArtifactLoadError::Invalid(message)) if message == "embedded artifact is missing: missing")
        );
    }

    /// Debug builds compile the inventory in rather than reading `$OUT_DIR` at
    /// run time; a cached lib from another checkout would otherwise read that
    /// checkout's `target/` (#2048).
    #[test]
    fn every_artifact_is_compiled_into_the_binary() {
        let paths: Vec<_> = EmbeddedSystemArtifacts::iter().collect();
        assert!(
            !paths.is_empty(),
            "the test build stages a system-artifact inventory"
        );
        for path in paths {
            let file = EmbeddedSystemArtifacts::get(&path).expect("listed artifact is embedded");
            assert!(
                matches!(file.data, std::borrow::Cow::Borrowed(_)),
                "{path} is read from disk at run time, not embedded"
            );
        }
    }
}
