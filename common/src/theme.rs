//! Built-in and custom public presentation theme identities and content metadata.

/// A built-in public presentation theme.
///
/// `serialize_all = "snake_case"` supplies the durable configuration and wire tokens:
/// `"terminal"`, `"studio"`, and `"reader"`. The default keeps existing public
/// presentation unchanged until an operator chooses otherwise.
#[macros::text_enum(
    sqlx,
    error = InvalidTheme,
    message = "theme must be \"terminal\", \"studio\", or \"reader\""
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, strum::VariantArray)]
#[strum(serialize_all = "snake_case")]
pub enum Theme {
    Terminal,
    #[default]
    Studio,
    Reader,
}

use crate::{permalink_route::PermalinkRoute, tag::Tag, username::Username};

use sha2::{Digest, Sha256};
use std::str::FromStr;

use thiserror::Error;

/// A full SHA-256 digest used to address immutable public Theme content.
#[derive(Clone, Debug, PartialEq, Eq, Hash, macros::StrNewtype)]
pub struct ThemeContentDigest(String);

/// The input was not the canonical lowercase hexadecimal representation of a
/// SHA-256 digest.
#[derive(Debug, Error)]
#[error("theme content digest must be 64 lowercase hex characters")]
pub struct InvalidThemeContentDigest;

impl FromStr for ThemeContentDigest {
    type Err = InvalidThemeContentDigest;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if crate::media::is_valid_content_hash(value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidThemeContentDigest)
        }
    }
}

/// Canonical digest of validated editable Theme Package source bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, macros::StrNewtype)]
pub struct ThemeSourceDigest(String);

/// Canonical digest of one immutable published Theme revision.
#[derive(Clone, Debug, PartialEq, Eq, Hash, macros::StrNewtype)]
pub struct ThemeRevisionDigest(String);

/// Canonical digest of deterministic transformed Theme CSS bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, macros::StrNewtype)]
pub struct ThemeStylesheetDigest(String);

/// Canonical digest of an immutable package asset.
#[derive(Clone, Debug, PartialEq, Eq, Hash, macros::StrNewtype)]
pub struct ThemeAssetDigest(String);

/// Canonical digest of the sorted entries in an explicit header-image pool.
#[derive(Clone, Debug, PartialEq, Eq, Hash, macros::StrNewtype)]
pub struct ThemePoolRevisionDigest(String);

/// Input was not a canonical theme SHA-256 digest.
#[derive(Debug, Error)]
#[error("theme digest must be 64 lowercase hex characters")]
pub struct InvalidThemeDigest;

macro_rules! theme_digest_from_str {
    ($type:ident) => {
        impl FromStr for $type {
            type Err = InvalidThemeDigest;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                if crate::media::is_valid_content_hash(value) {
                    Ok(Self(value.to_owned()))
                } else {
                    Err(InvalidThemeDigest)
                }
            }
        }
    };
}

theme_digest_from_str!(ThemeSourceDigest);
theme_digest_from_str!(ThemeRevisionDigest);
theme_digest_from_str!(ThemeStylesheetDigest);
theme_digest_from_str!(ThemeAssetDigest);
theme_digest_from_str!(ThemePoolRevisionDigest);

/// A public selection is either one of the closed built-ins or an opaque custom
/// theme identity. The identity is intentionally stable across custom-theme renames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum PublicThemeSelection {
    BuiltIn(Theme),
    Custom(crate::ids::ThemeId),
}

/// The identity selected for a resolved public presentation.
///
/// Custom identities remain stable as their display names and published revisions change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum PublishedThemeIdentity {
    BuiltIn(Theme),
    Custom(crate::ids::ThemeId),
}

/// The complete theme information needed to render one public route.
///
/// This is deliberately wasm-safe: all serving and selection work is complete before this
/// DTO crosses the server boundary. A custom selection names its stable theme identity while
/// `revision` pins the immutable content currently presented for that identity.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PublishedThemePresentation {
    pub identity: PublishedThemeIdentity,
    pub revision: Option<ThemeRevisionDigest>,
    pub stylesheet_url: crate::root_relative_url::RootRelativeUrl,
    pub logo_url: Option<crate::root_relative_url::RootRelativeUrl>,
    pub header_url: Option<crate::root_relative_url::RootRelativeUrl>,
}

impl PublishedThemePresentation {
    /// The built-in fallback presentation used when no valid selection is available.
    #[must_use]
    pub fn built_in(theme: Theme) -> Self {
        Self {
            identity: PublishedThemeIdentity::BuiltIn(theme),
            revision: None,
            stylesheet_url: crate::root_relative_url::RootRelativeUrl::built_in_theme_stylesheet(),
            logo_url: None,
            header_url: None,
        }
    }

    /// Stable root attribute value understood by the built-in stylesheet.
    #[must_use]
    pub fn data_theme(&self) -> String {
        match self.identity {
            PublishedThemeIdentity::BuiltIn(theme) => theme.token().to_owned(),
            PublishedThemeIdentity::Custom(_) => "custom".to_owned(),
        }
    }
}

/// The two Style Contract image roles a Theme Package may supply.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeImageRole {
    Logo,
    Header,
}

impl ThemeImageRole {
    /// Stable relational token shared by both database backends.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Logo => "logo",
            Self::Header => "header",
        }
    }
}

/// The closed source of one presentation-image role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeImageBindingMode {
    PackagedDefault,
    ExplicitAbsent,
    PackageAsset,
    Media,
    HeaderPool,
}

impl ThemeImageBindingMode {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::PackagedDefault => "packaged_default",
            Self::ExplicitAbsent => "explicit_absent",
            Self::PackageAsset => "package_asset",
            Self::Media => "media",
            Self::HeaderPool => "pool",
        }
    }
}

/// Canonical identity of one public presentation route.
///
/// Construction is closed over the public router's typed route values, preventing
/// callers from inventing alternate spellings for deterministic header selection.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PublicThemeRoute(String);

impl PublicThemeRoute {
    #[must_use]
    pub fn site() -> Self {
        Self("/".to_owned())
    }

    #[must_use]
    pub fn site_tag(tag: &Tag) -> Self {
        Self(format!("/tags/{tag}"))
    }

    #[must_use]
    pub fn author(username: &Username) -> Self {
        Self(format!("/~{username}"))
    }

    #[must_use]
    pub fn author_tag(username: &Username, tag: &Tag) -> Self {
        Self(format!("/~{username}/tags/{tag}"))
    }

    #[must_use]
    pub fn permalink(route: &PermalinkRoute) -> Self {
        Self(format!(
            "/~{}/{}/{}",
            route.username,
            route.date.value().format("%Y/%m/%d"),
            route.slug
        ))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One member of an explicit header-image pool.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ThemePoolEntry {
    Package(String),
    Media {
        source: String,
        digest: ThemeContentDigest,
        filename: String,
    },
}

impl ThemePoolEntry {
    /// The versioned, unambiguous pool encoding.
    #[must_use]
    pub fn canonical_encoding(&self) -> Vec<u8> {
        let mut encoded = Vec::new();
        match self {
            Self::Package(path) => {
                encoded.push(0);
                push_length_prefixed(&mut encoded, path.as_bytes());
            }
            Self::Media {
                source,
                digest,
                filename,
            } => {
                encoded.push(1);
                push_length_prefixed(&mut encoded, source.as_bytes());
                let raw_digest = hex_digest(digest.as_ref());
                encoded.extend(raw_digest);
                push_length_prefixed(&mut encoded, filename.as_bytes());
            }
        }
        encoded
    }
}

/// Canonicalized explicit header pool, its stable revision digest, and selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeHeaderPool {
    entries: Vec<ThemePoolEntry>,
    revision: ThemePoolRevisionDigest,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InvalidThemeHeaderPool {
    #[error("theme header pool must not be empty")]
    Empty,
    #[error("theme header pool contains a duplicate canonical entry")]
    Duplicate,
    #[error("theme header pool digest could not be represented")]
    Digest,
}

impl ThemeHeaderPool {
    /// Canonicalizes an explicit header pool and derives its revision digest.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidThemeHeaderPool::Empty`] for an empty pool,
    /// [`InvalidThemeHeaderPool::Duplicate`] for duplicate canonical entries,
    /// or [`InvalidThemeHeaderPool::Digest`] if the derived digest cannot be
    /// represented by the validated digest type.
    pub fn new(mut entries: Vec<ThemePoolEntry>) -> Result<Self, InvalidThemeHeaderPool> {
        if entries.is_empty() {
            return Err(InvalidThemeHeaderPool::Empty);
        }
        entries.sort_by_key(ThemePoolEntry::canonical_encoding);
        if entries
            .windows(2)
            .any(|pair| pair[0].canonical_encoding() == pair[1].canonical_encoding())
        {
            return Err(InvalidThemeHeaderPool::Duplicate);
        }
        let mut hasher = Sha256::new();
        hasher.update(b"jaunder-theme-pool-v1");
        hasher.update((entries.len() as u64).to_be_bytes());
        for entry in &entries {
            let encoded = entry.canonical_encoding();
            push_length_prefixed_hash(&mut hasher, &encoded);
        }
        let revision = digest_hex(hasher.finalize().as_slice())
            .parse()
            .map_err(|_| InvalidThemeHeaderPool::Digest)?;
        Ok(Self { entries, revision })
    }

    #[must_use]
    pub fn entries(&self) -> &[ThemePoolEntry] {
        &self.entries
    }

    #[must_use]
    pub fn revision(&self) -> &ThemePoolRevisionDigest {
        &self.revision
    }

    #[must_use]
    pub fn select(
        &self,
        route: &PublicThemeRoute,
        published_revision: &ThemeRevisionDigest,
        shuffle_seed: &[u8; 32],
    ) -> &ThemePoolEntry {
        let mut hasher = Sha256::new();
        hasher.update(b"jaunder-theme-assignment-v1");
        push_length_prefixed_hash(&mut hasher, route.as_str().as_bytes());
        hasher.update(hex_digest(published_revision.as_ref()));
        hasher.update(hex_digest(self.revision.as_ref()));
        hasher.update(shuffle_seed);
        let digest = hasher.finalize();
        let mut remainder = 0_usize;
        for byte in digest {
            remainder = (remainder * 256 + usize::from(byte)) % self.entries.len();
        }
        &self.entries[remainder]
    }
}

fn push_length_prefixed(target: &mut Vec<u8>, value: &[u8]) {
    target.extend((value.len() as u64).to_be_bytes());
    target.extend(value);
}

fn push_length_prefixed_hash(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn hex_digest(value: &str) -> [u8; 32] {
    let mut bytes = [0; 32];
    let mut encoded = value.bytes();
    for byte in &mut bytes {
        let high = encoded.next().map(hex_nibble).unwrap_or_default();
        let low = encoded.next().map(hex_nibble).unwrap_or_default();
        *byte = (high << 4) | low;
    }
    bytes
}

fn hex_nibble(value: u8) -> u8 {
    if value.is_ascii_digit() {
        value - b'0'
    } else {
        value - b'a' + 10
    }
}

fn digest_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut value, "{byte:02x}");
    }
    value
}

impl Theme {
    /// Stable token used by the public `data-theme` attribute and wire formats.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Studio => "studio",
            Self::Reader => "reader",
        }
    }
}

/// Whether a client route renders viewer-independent public presentation.
///
/// The shell uses this boundary to prevent a previously visited public theme
/// from leaking onto private application routes.
#[must_use]
pub fn is_public_presentation_path(path: &str) -> bool {
    path == "/" || path.starts_with("/tags/") || path.starts_with("/~")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_closed_and_studio_is_the_default() {
        assert_eq!(Theme::default(), Theme::Studio);
        for (token, theme) in [
            ("terminal", Theme::Terminal),
            ("studio", Theme::Studio),
            ("reader", Theme::Reader),
        ] {
            assert_eq!(token.parse::<Theme>().unwrap(), theme);
            assert_eq!(theme.token(), token);
        }
        assert!("solarized".parse::<Theme>().is_err());
    }

    #[test]
    fn public_path_classifier_excludes_private_application_routes() {
        for path in ["/", "/tags/rust", "/~alice", "/~alice/2026/01/02/post"] {
            assert!(is_public_presentation_path(path), "{path}");
        }
        for path in ["/app", "/profile", "/admin/site", "/login"] {
            assert!(!is_public_presentation_path(path), "{path}");
        }
    }

    #[test]
    fn content_digest_and_role_tokens_are_closed() {
        let digest = "a".repeat(64).parse::<ThemeContentDigest>().unwrap();
        assert_eq!(digest.as_ref(), "a".repeat(64));
        assert!("A".repeat(64).parse::<ThemeContentDigest>().is_err());
        assert_eq!(ThemeImageRole::Logo.token(), "logo");
        assert_eq!(ThemeImageRole::Header.token(), "header");
        assert_eq!(
            ThemeImageBindingMode::PackagedDefault.token(),
            "packaged_default"
        );
        assert_eq!(
            ThemeImageBindingMode::ExplicitAbsent.token(),
            "explicit_absent"
        );
        assert_eq!(ThemeImageBindingMode::PackageAsset.token(), "package_asset");
        assert_eq!(ThemeImageBindingMode::Media.token(), "media");
        assert_eq!(ThemeImageBindingMode::HeaderPool.token(), "pool");
    }

    #[test]
    fn published_theme_presentation_serde_fixtures_are_complete() {
        let built_in = PublishedThemePresentation::built_in(Theme::Reader);
        let custom = PublishedThemePresentation {
            identity: PublishedThemeIdentity::Custom(crate::ids::ThemeId::from(42)),
            revision: Some("a".repeat(64).parse().unwrap()),
            stylesheet_url: format!("/themes/{}", "b".repeat(64)).parse().unwrap(),
            logo_url: Some(format!("/themes/{}", "c".repeat(64)).parse().unwrap()),
            header_url: Some(format!("/themes/{}", "d".repeat(64)).parse().unwrap()),
        };

        let built_in_fixture = r#"{"identity":{"kind":"built_in","value":"reader"},"revision":null,"stylesheet_url":"/style/jaunder-themes.css","logo_url":null,"header_url":null}"#;
        let custom_fixture = format!(
            r#"{{"identity":{{"kind":"custom","value":42}},"revision":"{}","stylesheet_url":"/themes/{}","logo_url":"/themes/{}","header_url":"/themes/{}"}}"#,
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64),
        );

        assert_eq!(serde_json::to_string(&built_in).unwrap(), built_in_fixture);
        assert_eq!(
            serde_json::from_str::<PublishedThemePresentation>(built_in_fixture).unwrap(),
            built_in
        );
        assert_eq!(serde_json::to_string(&custom).unwrap(), custom_fixture);
        assert_eq!(
            serde_json::from_str::<PublishedThemePresentation>(&custom_fixture).unwrap(),
            custom
        );
    }
    #[test]
    fn header_pool_v1_encoding_digest_and_assignment_match_vectors() {
        let package = ThemePoolEntry::Package("images/header.png".to_owned());
        let media = ThemePoolEntry::Media {
            source: "upload".to_owned(),
            digest: "b".repeat(64).parse().unwrap(),
            filename: "hero.jpg".to_owned(),
        };
        assert_eq!(
            digest_hex(&package.canonical_encoding()),
            "000000000000000011696d616765732f6865616465722e706e67"
        );
        assert_eq!(
            digest_hex(&media.canonical_encoding()),
            concat!(
                "01000000000000000675706c6f6164",
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "00000000000000086865726f2e6a7067"
            )
        );

        let pool = ThemeHeaderPool::new(vec![media.clone(), package.clone()]).unwrap();
        assert_eq!(
            pool.revision().as_ref(),
            "0cc494a6b66c4e3e70894c72c4b9dc98a10643c9b85c197d02ae1c8fb07ac0f3"
        );
        assert_eq!(pool.entries(), &[package, media.clone()]);

        let route = PublicThemeRoute::author(&"alice".parse().unwrap());
        let revision = "c".repeat(64).parse().unwrap();
        let mut seed = [0; 32];
        for (index, byte) in seed.iter_mut().enumerate() {
            *byte = u8::try_from(index).unwrap();
        }
        assert_eq!(pool.select(&route, &revision, &seed), &media);
    }

    #[test]
    fn header_pool_rejects_duplicates() {
        let entry = ThemePoolEntry::Package("images/header.png".to_owned());
        assert_eq!(
            ThemeHeaderPool::new(vec![entry.clone(), entry]).unwrap_err(),
            InvalidThemeHeaderPool::Duplicate
        );
        assert_eq!(
            ThemeHeaderPool::new(Vec::new()).unwrap_err(),
            InvalidThemeHeaderPool::Empty
        );
    }

    #[test]
    fn public_theme_routes_have_exact_canonical_spellings() {
        let alice = "alice".parse().unwrap();
        let rust = "rust".parse().unwrap();
        let permalink = PermalinkRoute::parse("alice", "2026", "01", "02", "hello").unwrap();

        assert_eq!(PublicThemeRoute::site().as_str(), "/");
        assert_eq!(PublicThemeRoute::site_tag(&rust).as_str(), "/tags/rust");
        assert_eq!(PublicThemeRoute::author(&alice).as_str(), "/~alice");
        assert_eq!(
            PublicThemeRoute::author_tag(&alice, &rust).as_str(),
            "/~alice/tags/rust"
        );
        assert_eq!(
            PublicThemeRoute::permalink(&permalink).as_str(),
            "/~alice/2026/01/02/hello"
        );
    }
}
