//! Target-independent contract for the diagnostic LLVM profile export.
//!
//! Browser glue belongs in `client`; these names and result classifications are
//! intentionally dual-target so the capture protocol remains host-testable.

/// JavaScript export that returns the current module's raw LLVM profile bytes.
pub const PROFILE_EXPORT: &str = "jaunderCoverageProfile";
/// JavaScript export that identifies the module that produced a raw profile.
pub const MODULE_SIGNATURE_EXPORT: &str = "jaunderCoverageModuleSignature";
/// LLVM custom sections required for source-based coverage mapping.
pub const REQUIRED_COVERAGE_SECTIONS: [&str; 2] = ["__llvm_covfun", "__llvm_covmap"];

/// Truthful result of asking a browser-loaded diagnostic module for a profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileDumpState {
    /// The runtime was not built with the diagnostic coverage export.
    Unavailable,
    /// The export ran but emitted no raw-profile bytes.
    Empty,
    /// The export emitted raw-profile bytes ready for host-side collection.
    Captured { bytes: usize },
}

/// Classify the browser-visible profile result without interpreting profile data.
#[must_use]
pub fn profile_dump_state(profile: Option<&[u8]>) -> ProfileDumpState {
    match profile {
        None => ProfileDumpState::Unavailable,
        Some([]) => ProfileDumpState::Empty,
        Some(bytes) => ProfileDumpState::Captured { bytes: bytes.len() },
    }
}

/// Whether a processed wasm module still contains every required LLVM mapping
/// section. A missing section is a diagnostic result, never synthetic success.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoverageMetadataStatus {
    /// The module contains both `__llvm_covfun` and `__llvm_covmap`.
    Preserved,
    /// At least one mapping section was removed by the build or bundling path.
    Missing,
}

/// Determine whether the supplied custom-section names retain coverage mapping.
#[must_use]
pub fn coverage_metadata_status<'a>(
    sections: impl IntoIterator<Item = &'a str>,
) -> CoverageMetadataStatus {
    let mut found = [false; REQUIRED_COVERAGE_SECTIONS.len()];
    for section in sections {
        for (index, required) in REQUIRED_COVERAGE_SECTIONS.iter().enumerate() {
            found[index] |= section == *required;
        }
    }
    if found.into_iter().all(core::convert::identity) {
        CoverageMetadataStatus::Preserved
    } else {
        CoverageMetadataStatus::Missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_dump_state_distinguishes_unavailable_empty_and_captured_profiles() {
        assert_eq!(profile_dump_state(None), ProfileDumpState::Unavailable);
        assert_eq!(profile_dump_state(Some(&[])), ProfileDumpState::Empty);
        assert_eq!(
            profile_dump_state(Some(&[0_u8, 1, 2])),
            ProfileDumpState::Captured { bytes: 3 }
        );
    }

    #[test]
    fn browser_export_names_are_stable_capture_contracts() {
        assert_eq!(PROFILE_EXPORT, "jaunderCoverageProfile");
        assert_eq!(MODULE_SIGNATURE_EXPORT, "jaunderCoverageModuleSignature");
    }

    #[test]
    fn metadata_status_requires_both_llvm_mapping_sections() {
        assert_eq!(
            coverage_metadata_status(["name", "__llvm_covfun"]),
            CoverageMetadataStatus::Missing
        );
        assert_eq!(
            coverage_metadata_status(["__llvm_covmap", "__llvm_covfun"]),
            CoverageMetadataStatus::Preserved
        );
    }
}
