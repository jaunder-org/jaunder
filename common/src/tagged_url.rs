//! [`TaggedUrl<T>`] — a validated, normalized absolute `http(s)` URL that also carries
//! its **role** in the type. One carrier, fifteen roles: the site origin ([`BaseUrl`]),
//! the `WebSub` hub target ([`HubUrl`]), the feed a ping announces ([`FeedUrl`]), and so
//! on. Two roles are distinct types, so transposing a pair of adjacent URL arguments is
//! a compile error rather than a silently wrong link (#875).
//!
//! The invariant is shared: [`FromStr`] is the one validator every role routes through
//! (scheme `http`/`https`, normalized through the `url` crate — host/scheme case,
//! default-port stripping, percent-encoding, canonical root slash), so two spellings of
//! one URL are not both representable (ADR-0063, invariant axis).
//!
//! The ergonomic trailer (`Display`, `AsRef`/`Borrow`/`Deref<str>`, `TryFrom<String>`,
//! `From<Self> for String`, `PartialEq<str>`, ordering, the validating serde + sqlx
//! bridges) is generated once by `#[derive(StrNewtype)]` and inherited by every role.
//!
//! **The alias rule:** every use site outside this module that names a *concrete* role
//! spells the alias (`HubUrl`), never `TaggedUrl<Hub>` inline. Two gates reduce a type
//! to a bare ident — the `site_config_keys!` macro's `$ty:ident` slot and
//! `server_fn_tracing_check` — and a generic spelling matches neither.
//!
//! Two things are outside the rule rather than exceptions to it, because neither names
//! a concrete role:
//!
//! - **A turbofish mint** (`compose::<Permalink>(…)`) spells the *tag*, for the case
//!   where the value is consumed inline and there is no binding to ascribe.
//! - **A signature generic over `UrlRole`** spells `TaggedUrl<T>` because it serves
//!   every role at once — `atompub::entry::rel_link` renders four, and
//!   `test_support::parse_url` mints any. Writing an alias there would be wrong, not
//!   merely unidiomatic.

use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::str::FromStr;

use macros::StrNewtype;
use thiserror::Error;

/// Marker trait for a URL role. Implemented by the fifteen zero-sized tags in this
/// module and by nothing else; it is a pure label, never stored in a value.
pub trait UrlRole {}

/// A validated, normalized absolute `http(s)` URL tagged with its role `T`.
///
/// Construct via [`FromStr`] (untrusted strings), [`compose`] (site root + path),
/// [`TaggedUrl::join`], [`TaggedUrl::with_query_pairs`], or [`TaggedUrl::retag`].
///
/// The marker is carried as `PhantomData<fn() -> T>` rather than `PhantomData<T>`, so
/// the type is `Send`/`Sync` whatever the tag is — the tag is a label, not a value.
/// `Clone`, `Debug`, `PartialEq`, `Eq`, and `Hash` are hand-written for the same reason:
/// `#[derive]` would add a `T: Clone`-style bound on a marker that is never stored.
///
/// Distinct roles are distinct types, so a value of one role does not satisfy a
/// parameter of another:
///
/// ```compile_fail
/// # use common::tagged_url::{BaseUrl, HubUrl};
/// # fn takes_hub(_: &HubUrl) {}
/// # let base: BaseUrl = "https://example.com".parse().unwrap();
/// takes_hub(&base);
/// ```
///
/// The matching role compiles — same fixture, so the negative above can only be failing
/// for the role mismatch:
///
/// ```
/// # use common::tagged_url::{BaseUrl, HubUrl};
/// # fn takes_hub(_: &HubUrl) {}
/// # let base: BaseUrl = "https://example.com".parse().unwrap();
/// let hub: HubUrl = "https://hub.example.com".parse().unwrap();
/// takes_hub(&hub);
/// ```
#[derive(StrNewtype)]
pub struct TaggedUrl<T: UrlRole>(String, PhantomData<fn() -> T>);

impl<T: UrlRole> Clone for TaggedUrl<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

impl<T: UrlRole> std::fmt::Debug for TaggedUrl<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TaggedUrl").field(&self.0).finish()
    }
}

impl<T: UrlRole> PartialEq for TaggedUrl<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T: UrlRole> Eq for TaggedUrl<T> {}

impl<T: UrlRole> Hash for TaggedUrl<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

/// Error returned when a string is not a valid absolute `http(s)` URL.
#[derive(Debug, Error)]
#[error("not a valid absolute http(s) URL")]
pub struct InvalidUrl;

impl<T: UrlRole> FromStr for TaggedUrl<T> {
    type Err = InvalidUrl;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url = url::Url::parse(s.trim()).map_err(|_| InvalidUrl)?;
        // The invariant: `http`/`https` only. `url` lowercases the scheme and, for
        // these special schemes, guarantees a non-empty host on a successful parse
        // (an empty authority is a parse error), so the scheme check is the whole
        // invariant. `url.to_string()` is the canonical form: host lowercased,
        // default port stripped, root path present, percent-encoding canonicalized.
        if !matches!(url.scheme(), "http" | "https") {
            return Err(InvalidUrl);
        }
        Ok(Self(url.to_string(), PhantomData))
    }
}

impl<T: UrlRole> TaggedUrl<T> {
    /// Compose this URL with a (typically site-absolute) `path`, via `url`'s join, and
    /// tag the result with the ascribed output role `U`. The joined result is
    /// re-validated through [`FromStr`], so a `path` that introduces a non-`http(s)`
    /// scheme is rejected. It does **not** enforce same-origin: an absolute `http`
    /// `path` resolves to its own host and is accepted — every in-tree call site passes
    /// a server-built `/…` literal, never untrusted input.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidUrl`] if `path` cannot be joined onto the base or the joined
    /// result is not a valid absolute `http(s)` URL.
    pub fn join<U: UrlRole>(&self, path: &str) -> Result<TaggedUrl<U>, InvalidUrl> {
        let base = url::Url::parse(&self.0).map_err(|_| InvalidUrl)?;
        let joined = base.join(path).map_err(|_| InvalidUrl)?;
        joined.as_str().parse()
    }

    /// Append query `pairs` to this URL, percent-encoding keys and values via `url`'s
    /// [`query_pairs_mut`](url::Url::query_pairs_mut), and return the new URL under the
    /// ascribed output role `U`. Use this to build cursor URLs (e.g. a feed `next` link)
    /// instead of `format!`-ing a query string, so reserved characters in values are
    /// encoded correctly.
    #[must_use]
    pub fn with_query_pairs<U: UrlRole>(&self, pairs: &[(&str, &str)]) -> TaggedUrl<U> {
        let Ok(mut url) = url::Url::parse(&self.0) else {
            // `self.0` is a valid URL by construction, so this never fires.
            unreachable!("TaggedUrl holds a valid url");
        };
        url.query_pairs_mut().extend_pairs(pairs.iter().copied());
        TaggedUrl(url.to_string(), PhantomData)
    }

    /// Assert that this URL plays a different role.
    ///
    /// **Every call site MUST carry a comment stating the domain identity it
    /// asserts** — e.g. "the collection URL *is* the feed id". A retag with no
    /// such justification is a review failure (#875).
    ///
    /// The bytes are unchanged; only the role label moves. This is the third and last
    /// minting door, and the only one that does not re-validate — it does not need to,
    /// because the value already passed [`FromStr`].
    #[must_use]
    pub fn retag<U: UrlRole>(self) -> TaggedUrl<U> {
        TaggedUrl(self.0, PhantomData)
    }
}

/// Compose a **required** site root `base` with a site-absolute `path` into a
/// [`TaggedUrl`] under the ascribed output role `U`, via [`TaggedUrl::join`] (correct
/// slash boundary + encoding). A configured base is a *type* precondition: callers
/// narrow `SiteIdentity.base_url` once at the request/regeneration boundary before
/// composing (the server maps the missing-base case to its own error —
/// `HandlerError::BaseUrlRequired` / `RegenerateError::BaseUrlRequired`), so this
/// function neither takes an `Option` nor fails at runtime — feeds/atompub cannot emit a
/// relative `atom:id` (#560).
///
/// Infallible: every call site passes a server-built canonical `/…` path onto a valid
/// base, so [`TaggedUrl::join`] cannot fail.
///
/// Composition starts at the site root and nowhere else, so only a [`BaseUrl`] is
/// accepted:
///
/// ```compile_fail
/// # use common::tagged_url::{compose, BaseUrl, FeedUrl, HubUrl};
/// # let hub: HubUrl = "https://hub.example.com".parse().unwrap();
/// let feed: FeedUrl = compose(&hub, "/feed.xml");
/// ```
///
/// From a `BaseUrl` it compiles — same fixture, so the negative above can only be
/// failing for the role of the base:
///
/// ```
/// # use common::tagged_url::{compose, BaseUrl, FeedUrl, HubUrl};
/// # let hub: HubUrl = "https://hub.example.com".parse().unwrap();
/// let base: BaseUrl = "https://example.com".parse().unwrap();
/// let feed: FeedUrl = compose(&base, "/feed.xml");
/// ```
#[must_use]
pub fn compose<U: UrlRole>(base: &BaseUrl, path: &str) -> TaggedUrl<U> {
    let Ok(url) = base.join(path) else {
        // Callers pass server-built canonical `/…` paths onto a valid base.
        unreachable!("compose: valid base joined with a server-built path");
    };
    url
}

/// The operator-configured site origin — the root every other role composes from.
pub struct Base;
impl UrlRole for Base {}
/// A `WebSub` hub, the target of a publish ping.
pub struct Hub;
impl UrlRole for Hub {}
/// A syndication feed document (Atom/RSS/JSON, or an `AtomPub` posts collection).
pub struct Feed;
impl UrlRole for Feed {}
/// The `href` of an `AtomPub` service-document collection (posts *or* media).
pub struct CollectionHref;
impl UrlRole for CollectionHref {}
/// The canonical location of the resource a feed describes.
pub struct Canonical;
impl UrlRole for Canonical {}
/// A post's public permalink.
pub struct Permalink;
impl UrlRole for Permalink {}
/// Where a media object was fetched from. Named `MediaOrigin`, not `MediaSource`:
/// `common::media` already declares a `MediaSource` enum, and the sqlx decode gate and
/// `server_fn_tracing_check` both reduce a type to a bare ident.
pub struct MediaOrigin;
impl UrlRole for MediaOrigin {}
/// A paging link (`first`/`next`/`previous`) within a paginated collection.
pub struct Pagination;
impl UrlRole for Pagination {}
/// An `atom:id` — the stable identifier of a feed or entry.
pub struct EntryId;
impl UrlRole for EntryId {}
/// An `AtomPub` `edit` link: the member resource itself.
pub struct EditUri;
impl UrlRole for EditUri {}
/// An `AtomPub` `edit-media` link: the member's binary.
pub struct EditMediaUri;
impl UrlRole for EditMediaUri {}
/// The `src` of an entry's out-of-line content.
pub struct ContentSrc;
impl UrlRole for ContentSrc {}
/// The `AtomPub` service document.
pub struct ServiceDoc;
impl UrlRole for ServiceDoc {}
/// A user's public homepage.
pub struct Homepage;
impl UrlRole for Homepage {}
/// A one-shot confirmation link mailed to a user (verify, invite, password reset).
pub struct MailConfirm;
impl UrlRole for MailConfirm {}

/// The site origin. See [`Base`].
pub type BaseUrl = TaggedUrl<Base>;
/// A `WebSub` hub. See [`Hub`].
pub type HubUrl = TaggedUrl<Hub>;
/// A syndication feed. See [`Feed`].
pub type FeedUrl = TaggedUrl<Feed>;
/// A service-document collection `href`. See [`CollectionHref`].
pub type CollectionHrefUrl = TaggedUrl<CollectionHref>;
/// A canonical location. See [`Canonical`].
pub type CanonicalUrl = TaggedUrl<Canonical>;
/// A post permalink. See [`Permalink`].
pub type PermalinkUrl = TaggedUrl<Permalink>;
/// Where a media object came from. See [`MediaOrigin`].
pub type MediaSourceUrl = TaggedUrl<MediaOrigin>;
/// A paging link. See [`Pagination`].
pub type PaginationUrl = TaggedUrl<Pagination>;
/// An `atom:id`. See [`EntryId`].
pub type EntryIdUrl = TaggedUrl<EntryId>;
/// An `AtomPub` `edit` link. See [`EditUri`].
pub type EditUriUrl = TaggedUrl<EditUri>;
/// An `AtomPub` `edit-media` link. See [`EditMediaUri`].
pub type EditMediaUriUrl = TaggedUrl<EditMediaUri>;
/// An out-of-line content `src`. See [`ContentSrc`].
pub type ContentSrcUrl = TaggedUrl<ContentSrc>;
/// The `AtomPub` service document. See [`ServiceDoc`].
pub type ServiceDocUrl = TaggedUrl<ServiceDoc>;
/// A user homepage. See [`Homepage`].
pub type HomepageUrl = TaggedUrl<Homepage>;
/// A mailed confirmation link. See [`MailConfirm`].
pub type MailConfirmUrl = TaggedUrl<MailConfirm>;

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    // -- FromStr: invariant + normalization, shared by every role (D4, D6) --

    #[test]
    fn rejects_non_http_schemes() {
        for bad in [
            "file:///etc/passwd",
            "ftp://h/x",
            "javascript:alert(1)",
            "mailto:a@b.c",
        ] {
            assert!(bad.parse::<BaseUrl>().is_err(), "should reject {bad}");
        }
        assert!("nonsense://x".parse::<HubUrl>().is_err());
    }

    #[test]
    fn rejects_unparseable_or_relative() {
        for bad in ["https://", "/feed.rss", "not a url", ""] {
            assert!(bad.parse::<BaseUrl>().is_err(), "should reject {bad}");
            assert!(bad.parse::<HubUrl>().is_err(), "should reject {bad}");
        }
    }

    #[test]
    fn parses_and_normalizes() {
        let b: BaseUrl = "HTTPS://Example.COM:443".parse().expect("parses");
        assert_eq!(b, *"https://example.com/");
    }

    #[test]
    fn normalizes_host_case_and_default_port() {
        assert_eq!(
            "https://Example.COM:443/".parse::<BaseUrl>().unwrap(),
            *"https://example.com/"
        );
        assert_eq!("http://H:80/".parse::<BaseUrl>().unwrap(), *"http://h/");
    }

    #[test]
    fn adds_canonical_root_slash() {
        assert_eq!(
            "https://example.com".parse::<BaseUrl>().unwrap(),
            *"https://example.com/"
        );
    }

    #[test]
    fn from_str_is_idempotent() {
        let once = "https://Example.com/Path".parse::<BaseUrl>().unwrap();
        assert_eq!(once.as_ref().parse::<BaseUrl>().unwrap(), once);
    }

    #[test]
    fn trailer_derefs_and_displays() {
        let u = "https://example.com/".parse::<BaseUrl>().unwrap();
        let s: &str = &u; // Deref<str>
        assert_eq!(s, "https://example.com/"); // Display
        assert_eq!(u.to_string(), "https://example.com/");
        assert_eq!(u, *"https://example.com/"); // PartialEq<str>
    }

    #[test]
    fn trailer_orders_and_debug_prints_the_inner_string() {
        let a: FeedUrl = "https://example.com/a".parse().unwrap();
        let b: FeedUrl = "https://example.com/b".parse().unwrap();
        assert!(a < b);
        assert_eq!(format!("{a:?}"), "TaggedUrl(\"https://example.com/a\")");
    }

    #[test]
    fn trailer_round_trips_through_string() {
        let a: HubUrl = "https://hub.example.com/".parse().unwrap();
        let s = String::from(a.clone());
        assert_eq!(HubUrl::try_from(s).unwrap(), a);
    }

    // -- join (D3) --

    #[test]
    fn join_composes_without_double_slash() {
        let base: BaseUrl = "https://example.com/".parse().unwrap();
        assert_eq!(
            base.join::<Feed>("/feed.rss").unwrap(),
            *"https://example.com/feed.rss"
        );
        assert_eq!(
            base.join::<Feed>("/tags/rust/").unwrap(),
            *"https://example.com/tags/rust/"
        );
    }

    #[test]
    fn join_mints_a_new_role() {
        let base: BaseUrl = "https://example.com".parse().expect("parses");
        let edit: EditUriUrl = base.join("/edit/1").expect("joins");
        assert_eq!(edit, *"https://example.com/edit/1");
    }

    #[test]
    fn join_preserves_query() {
        let base: BaseUrl = "https://example.com/".parse().unwrap();
        let joined: FeedUrl = base
            .join("/atompub/alice/posts?updated_before=x&id_before=1")
            .unwrap();
        assert_eq!(
            joined,
            *"https://example.com/atompub/alice/posts?updated_before=x&id_before=1"
        );
    }

    #[test]
    fn join_of_canonical_feed_path_is_unchanged_path() {
        // percent_encode_path removal regression (AC#2): canonical feed paths need no
        // escaping.
        let base: BaseUrl = "https://example.com/".parse().unwrap();
        let joined: FeedUrl = base.join("/~alice/tags/rust/feed.atom").unwrap();
        assert_eq!(joined, *"https://example.com/~alice/tags/rust/feed.atom");
    }

    #[test]
    fn join_rejects_non_http_result() {
        // An absolute path with a non-http(s) scheme replaces the base entirely; the
        // re-validation through FromStr then rejects it. Pins join's Err branch.
        let base: BaseUrl = "https://example.com/".parse().unwrap();
        assert!(base.join::<Feed>("mailto:foo@bar.example").is_err());
        assert!(base.join::<Feed>("ftp://other.example/x").is_err());
    }

    #[test]
    fn join_does_not_guarantee_same_origin() {
        // Documented limitation: an absolute *http* path resolves to the other host and
        // is accepted. All real call sites pass server-built "/…" literals, never user
        // input, so this cannot fire in-tree.
        let base: BaseUrl = "https://example.com/".parse().unwrap();
        let joined: FeedUrl = base.join("http://other.example/evil").unwrap();
        assert_eq!(joined, *"http://other.example/evil");
    }

    // -- with_query_pairs (#560, D5) --

    #[test]
    fn with_query_pairs_encodes_and_appends() {
        let base: FeedUrl = "https://ex.com/atompub/alice/posts".parse().unwrap();
        let out: PaginationUrl = base.with_query_pairs(&[
            ("updated_before", "2026-01-02T03:04:05Z"),
            ("id_before", "5"),
        ]);
        let parsed = url::Url::parse(out.as_ref()).unwrap();
        let got: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(
            got,
            vec![
                (
                    "updated_before".to_string(),
                    "2026-01-02T03:04:05Z".to_string()
                ),
                ("id_before".to_string(), "5".to_string()),
            ]
        );
        // A value with reserved chars round-trips *decoded*.
        let out2: PaginationUrl = base.with_query_pairs(&[("q", "a b&c=d")]);
        let p2 = url::Url::parse(out2.as_ref()).unwrap();
        assert_eq!(p2.query_pairs().next().unwrap().1.into_owned(), "a b&c=d");
    }

    #[test]
    fn with_query_pairs_mints_a_new_role() {
        let feed: FeedUrl = "https://example.com/posts".parse().expect("parses");
        let next: PaginationUrl = feed.with_query_pairs(&[("page", "2")]);
        assert_eq!(next, *"https://example.com/posts?page=2");
    }

    // -- compose: required base, ascribed role (#560, #875, D1) --

    #[test]
    fn compose_joins_against_required_base() {
        let base: BaseUrl = "https://example.com/".parse().unwrap();
        assert_eq!(
            compose::<Feed>(&base, "/feed.rss"),
            *"https://example.com/feed.rss"
        );
        assert_eq!(
            compose::<Permalink>(&base, "/~a/2026/01/02/x"),
            *"https://example.com/~a/2026/01/02/x"
        );
    }

    #[test]
    fn compose_mints_the_ascribed_role() {
        let base: BaseUrl = "https://example.com".parse().expect("parses");
        let feed: FeedUrl = compose(&base, "/feed.xml");
        assert_eq!(feed, *"https://example.com/feed.xml");
    }

    // -- retag (#875) --

    #[test]
    fn retag_preserves_the_bytes() {
        let feed: FeedUrl = "https://example.com/feed.xml".parse().expect("parses");
        // The collection URL *is* the feed's atom:id — the identity this whole test
        // exists to document.
        let id: EntryIdUrl = feed.clone().retag();
        assert_eq!(id.as_ref(), feed.as_ref());
    }

    // -- the tag is a pure label: no bounds leak onto it --

    #[test]
    fn roles_are_clonable_and_hashable_without_bounds_on_the_tag() {
        let a: HubUrl = "https://hub.example.com".parse().expect("parses");
        let mut set = HashSet::new();
        set.insert(a.clone());
        assert!(set.contains(&a));
    }

    #[test]
    fn roles_are_send_and_sync_whatever_the_tag_is() {
        // `PhantomData<fn() -> T>` (not `PhantomData<T>`) is what makes this
        // unconditional: the marker is never stored, so the tag's own auto traits
        // must not matter.
        fn assert_send_sync<X: Send + Sync>() {}
        assert_send_sync::<HubUrl>();
        assert_send_sync::<TaggedUrl<Homepage>>();
    }

    #[test]
    fn two_roles_carry_the_same_bytes_independently() {
        let feed: FeedUrl = "https://example.com/x".parse().unwrap();
        let hub: HubUrl = "https://example.com/x".parse().unwrap();
        assert_eq!(feed.as_ref(), hub.as_ref());
    }

    // -- serde bridge --

    #[test]
    fn serde_round_trips_transparently() {
        let a: HubUrl = "https://hub.example.com/".parse().expect("parses");
        let json = serde_json::to_string(&a).expect("serializes");
        assert_eq!(json, "\"https://hub.example.com/\"");
        assert_eq!(
            serde_json::from_str::<HubUrl>(&json).expect("deserializes"),
            a
        );
    }

    #[test]
    fn serde_rejects_an_invalid_url() {
        assert!(serde_json::from_str::<HubUrl>("\"nonsense://x\"").is_err());
    }
}
