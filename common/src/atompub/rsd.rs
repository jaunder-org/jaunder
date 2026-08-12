//! `RSD` (Really Simple Discovery) document serializer for `AtomPub`.
//!
//! An `RSD` document advertises publishing capabilities to client applications
//! like `MarsEdit` and external blog editors. This module provides [`render_rsd_document`]
//! to generate an `RSD` document pointing to the `AtomPub` service and home page.

use crate::tagged_url::{HomepageUrl, ServiceDocUrl};

/// Serializes a Really Simple Discovery (`RSD`) document.
///
/// Returns an `RSD` document (as specified in the `RSD` specification) with the given
/// service URL and homepage URL embedded. The service URL is the `AtomPub` Service
/// Document endpoint; the homepage URL is the site's public-facing home.
///
/// Both URLs are XML-escaped to prevent injection. Typing them as
/// [`TaggedUrl`](crate::tagged_url::TaggedUrl)s does not make that escaping redundant:
/// `&` is legal in a query string and survives URL normalization, so an unescaped hub or
/// homepage URL carrying one would emit malformed XML.
///
/// The two URLs carry distinct roles, so transposing them is a compile error rather
/// than an `RSD` document that advertises the homepage as the publishing endpoint
/// (#875):
///
/// ```compile_fail
/// # use common::atompub::rsd::render_rsd_document;
/// # use common::tagged_url::{HomepageUrl, ServiceDocUrl};
/// # fn f(service: &ServiceDocUrl, homepage: &HomepageUrl) {
/// let _ = render_rsd_document(homepage, service);
/// # }
/// ```
///
/// The correct order compiles — same fixture, so the negative above can only be
/// failing for the transposition:
///
/// ```
/// # use common::atompub::rsd::render_rsd_document;
/// # use common::tagged_url::{HomepageUrl, ServiceDocUrl};
/// # fn f(service: &ServiceDocUrl, homepage: &HomepageUrl) {
/// let _ = render_rsd_document(service, homepage);
/// # }
/// ```
///
/// # Infallible
///
/// This function is infallible — it always returns a `String`.
#[must_use]
pub fn render_rsd_document(service_url: &ServiceDocUrl, homepage_url: &HomepageUrl) -> String {
    format!(
        r#"<?xml version="1.0"?>
<rsd version="1.0" xmlns="http://archipelago.phrasewise.com/rsd">
  <service>
    <engineName>Jaunder</engineName>
    <homePageLink>{homepage}</homePageLink>
    <apis>
      <api name="Atom" preferred="true" apiLink="{service}" blogID=""/>
    </apis>
  </service>
</rsd>"#,
        // `escape` takes `impl Into<Cow<str>>`, and deref coercion does not apply
        // through a generic parameter — hence the explicit `as_ref`. Its `Cow` is
        // formatted directly: `Display` is all `format!` needs, so a URL with
        // nothing to escape stays borrowed.
        homepage = quick_xml::escape::escape(homepage_url.as_ref()),
        service = quick_xml::escape::escape(service_url.as_ref()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::parse_url;

    #[test]
    fn rsd_document_contains_engine_name_and_urls() {
        let out = render_rsd_document(
            &parse_url("https://example.com/atompub/service"),
            &parse_url("https://example.com/home"),
        );
        assert!(out.contains("<engineName>Jaunder</engineName>"));
        assert!(out.contains("https://example.com/atompub/service"));
        assert!(out.contains("https://example.com/home"));
        assert!(out.contains("apiLink="));
    }

    // `&` is a legal query separator and survives `TaggedUrl` normalization, so
    // escaping it is what keeps the document well-formed XML — not defence in
    // depth. (`<` and `"` are percent-encoded by normalization and cannot reach
    // this function.)
    #[test]
    fn rsd_document_escapes_query_ampersand() {
        let out = render_rsd_document(
            &parse_url("https://example.com/atompub?foo=1&bar=2"),
            &parse_url("https://example.com/home"),
        );
        assert!(out.contains("foo=1&amp;bar=2"));
        assert!(!out.contains("foo=1&bar=2"));
    }
}
