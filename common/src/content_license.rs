//! The closed publication-wide rights choices available to a User.

/// A User's current publication-wide Content License.
///
/// Its string representation is the persisted and wire token. The type is the
/// sole authority for the public rights label and Creative Commons metadata.
#[macros::text_enum(
    sqlx,
    error = InvalidContentLicense,
    message = "content license must be an approved rights token"
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, strum::VariantArray)]
pub enum ContentLicense {
    /// The default rights statement, which has no SPDX identifier or URL.
    #[default]
    #[strum(serialize = "all-rights-reserved")]
    AllRightsReserved,
    #[strum(serialize = "CC0-1.0")]
    Cc0_1_0,
    #[strum(serialize = "CC-BY-4.0")]
    CcBy4_0,
    #[strum(serialize = "CC-BY-SA-4.0")]
    CcBySa4_0,
    #[strum(serialize = "CC-BY-ND-4.0")]
    CcByNd4_0,
    #[strum(serialize = "CC-BY-NC-4.0")]
    CcByNc4_0,
    #[strum(serialize = "CC-BY-NC-SA-4.0")]
    CcByNcSa4_0,
    #[strum(serialize = "CC-BY-NC-ND-4.0")]
    CcByNcNd4_0,
}

/// Static public metadata for a [`ContentLicense`] choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContentLicenseDescriptor {
    /// Exact public label.
    pub label: &'static str,
    /// SPDX identifier, absent for a rights statement rather than a license.
    pub spdx_id: Option<&'static str>,
    /// Canonical Creative Commons deed URL.
    pub canonical_url: Option<&'static str>,
    /// Official Creative Commons 88-by-31 license icon URL.
    pub icon_url: Option<&'static str>,
}

impl ContentLicense {
    /// All static public metadata for this rights choice.
    #[must_use]
    pub const fn descriptor(self) -> ContentLicenseDescriptor {
        match self {
            Self::AllRightsReserved => ContentLicenseDescriptor {
                label: "All Rights Reserved",
                spdx_id: None,
                canonical_url: None,
                icon_url: None,
            },
            Self::Cc0_1_0 => ContentLicenseDescriptor {
                label: "CC0 1.0",
                spdx_id: Some("CC0-1.0"),
                canonical_url: Some("https://creativecommons.org/publicdomain/zero/1.0/"),
                icon_url: Some("https://licensebuttons.net/p/zero/1.0/88x31.png"),
            },
            Self::CcBy4_0 => ContentLicenseDescriptor {
                label: "CC BY 4.0",
                spdx_id: Some("CC-BY-4.0"),
                canonical_url: Some("https://creativecommons.org/licenses/by/4.0/"),
                icon_url: Some("https://licensebuttons.net/l/by/4.0/88x31.png"),
            },
            Self::CcBySa4_0 => ContentLicenseDescriptor {
                label: "CC BY-SA 4.0",
                spdx_id: Some("CC-BY-SA-4.0"),
                canonical_url: Some("https://creativecommons.org/licenses/by-sa/4.0/"),
                icon_url: Some("https://licensebuttons.net/l/by-sa/4.0/88x31.png"),
            },
            Self::CcByNd4_0 => ContentLicenseDescriptor {
                label: "CC BY-ND 4.0",
                spdx_id: Some("CC-BY-ND-4.0"),
                canonical_url: Some("https://creativecommons.org/licenses/by-nd/4.0/"),
                icon_url: Some("https://licensebuttons.net/l/by-nd/4.0/88x31.png"),
            },
            Self::CcByNc4_0 => ContentLicenseDescriptor {
                label: "CC BY-NC 4.0",
                spdx_id: Some("CC-BY-NC-4.0"),
                canonical_url: Some("https://creativecommons.org/licenses/by-nc/4.0/"),
                icon_url: Some("https://licensebuttons.net/l/by-nc/4.0/88x31.png"),
            },
            Self::CcByNcSa4_0 => ContentLicenseDescriptor {
                label: "CC BY-NC-SA 4.0",
                spdx_id: Some("CC-BY-NC-SA-4.0"),
                canonical_url: Some("https://creativecommons.org/licenses/by-nc-sa/4.0/"),
                icon_url: Some("https://licensebuttons.net/l/by-nc-sa/4.0/88x31.png"),
            },
            Self::CcByNcNd4_0 => ContentLicenseDescriptor {
                label: "CC BY-NC-ND 4.0",
                spdx_id: Some("CC-BY-NC-ND-4.0"),
                canonical_url: Some("https://creativecommons.org/licenses/by-nc-nd/4.0/"),
                icon_url: Some("https://licensebuttons.net/l/by-nc-nd/4.0/88x31.png"),
            },
        }
    }

    /// The exact public label for this rights choice.
    #[must_use]
    pub const fn label(self) -> &'static str {
        self.descriptor().label
    }

    /// The SPDX identifier, when this choice is a license rather than a rights statement.
    #[must_use]
    pub const fn spdx_id(self) -> Option<&'static str> {
        self.descriptor().spdx_id
    }

    /// The canonical Creative Commons URL, when this choice has one.
    #[must_use]
    pub const fn canonical_url(self) -> Option<&'static str> {
        self.descriptor().canonical_url
    }

    /// The official Creative Commons license icon URL, when this choice has one.
    #[must_use]
    pub const fn icon_url(self) -> Option<&'static str> {
        self.descriptor().icon_url
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use strum::VariantArray as _;

    use super::*;

    #[test]
    fn every_choice_has_the_approved_token_and_metadata() {
        let cases = [
            (
                ContentLicense::AllRightsReserved,
                "all-rights-reserved",
                "All Rights Reserved",
                None,
                None,
                None,
            ),
            (
                ContentLicense::Cc0_1_0,
                "CC0-1.0",
                "CC0 1.0",
                Some("CC0-1.0"),
                Some("https://creativecommons.org/publicdomain/zero/1.0/"),
                Some("https://licensebuttons.net/p/zero/1.0/88x31.png"),
            ),
            (
                ContentLicense::CcBy4_0,
                "CC-BY-4.0",
                "CC BY 4.0",
                Some("CC-BY-4.0"),
                Some("https://creativecommons.org/licenses/by/4.0/"),
                Some("https://licensebuttons.net/l/by/4.0/88x31.png"),
            ),
            (
                ContentLicense::CcBySa4_0,
                "CC-BY-SA-4.0",
                "CC BY-SA 4.0",
                Some("CC-BY-SA-4.0"),
                Some("https://creativecommons.org/licenses/by-sa/4.0/"),
                Some("https://licensebuttons.net/l/by-sa/4.0/88x31.png"),
            ),
            (
                ContentLicense::CcByNd4_0,
                "CC-BY-ND-4.0",
                "CC BY-ND 4.0",
                Some("CC-BY-ND-4.0"),
                Some("https://creativecommons.org/licenses/by-nd/4.0/"),
                Some("https://licensebuttons.net/l/by-nd/4.0/88x31.png"),
            ),
            (
                ContentLicense::CcByNc4_0,
                "CC-BY-NC-4.0",
                "CC BY-NC 4.0",
                Some("CC-BY-NC-4.0"),
                Some("https://creativecommons.org/licenses/by-nc/4.0/"),
                Some("https://licensebuttons.net/l/by-nc/4.0/88x31.png"),
            ),
            (
                ContentLicense::CcByNcSa4_0,
                "CC-BY-NC-SA-4.0",
                "CC BY-NC-SA 4.0",
                Some("CC-BY-NC-SA-4.0"),
                Some("https://creativecommons.org/licenses/by-nc-sa/4.0/"),
                Some("https://licensebuttons.net/l/by-nc-sa/4.0/88x31.png"),
            ),
            (
                ContentLicense::CcByNcNd4_0,
                "CC-BY-NC-ND-4.0",
                "CC BY-NC-ND 4.0",
                Some("CC-BY-NC-ND-4.0"),
                Some("https://creativecommons.org/licenses/by-nc-nd/4.0/"),
                Some("https://licensebuttons.net/l/by-nc-nd/4.0/88x31.png"),
            ),
        ];
        assert_eq!(ContentLicense::VARIANTS.len(), cases.len());
        for (license, token, label, spdx_id, canonical_url, icon_url) in cases {
            assert_eq!(license.as_ref(), token);
            assert_eq!(ContentLicense::from_str(token).ok(), Some(license));
            assert_eq!(
                license.descriptor(),
                ContentLicenseDescriptor {
                    label,
                    spdx_id,
                    canonical_url,
                    icon_url,
                }
            );
            assert_eq!(license.label(), label);
            assert_eq!(license.spdx_id(), spdx_id);
            assert_eq!(license.canonical_url(), canonical_url);
            assert_eq!(license.icon_url(), icon_url);
        }
    }

    #[test]
    fn rejects_values_outside_the_closed_catalog() {
        for value in ["cc-by-4.0", "CC-BY-3.0", "MIT", ""] {
            assert!(
                ContentLicense::from_str(value).is_err(),
                "{value:?} must reject"
            );
        }
    }
}
