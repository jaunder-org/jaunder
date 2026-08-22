//! `RSD` (Really Simple Discovery) document serializer for `AtomPub`.
//!
//! An `RSD` document advertises publishing capabilities to client applications
//! like `MarsEdit` and external blog editors. This module provides [`render_rsd_document`]
//! to generate an `RSD` document pointing to the `AtomPub` service and home page.

use crate::tagged_url::{HomepageUrl, ServiceDocUrl};
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, Event};

use super::xml::{write_empty_element, write_text_element};

/// Serializes a Really Simple Discovery (`RSD`) document.
///
/// Returns an `RSD` document (as specified in the `RSD` specification) with the given
/// service URL and homepage URL embedded. The service URL is the `AtomPub` Service
/// Document endpoint; the homepage URL is the site's public-facing home.
///
/// URL text and attributes are XML-escaped by `quick-xml`. Typing them as
/// [`TaggedUrl`](crate::tagged_url::TaggedUrl)s does not make that escaping
/// redundant: `&` is legal in a query string and survives URL normalization, so
/// an unescaped hub or homepage URL carrying one would emit malformed XML.
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
    let mut writer = Writer::new(Vec::new());
    let _ = writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)));

    let mut root = BytesStart::new("rsd");
    root.push_attribute(("version", "1.0"));
    root.push_attribute(("xmlns", "http://archipelago.phrasewise.com/rsd"));
    let _ = writer.write_event(Event::Start(root));

    let _ = writer.write_event(Event::Start(BytesStart::new("service")));
    write_text_element(&mut writer, "engineName", "Jaunder");
    write_text_element(&mut writer, "homePageLink", homepage_url.as_ref());

    let _ = writer.write_event(Event::Start(BytesStart::new("apis")));
    write_empty_element(
        &mut writer,
        "api",
        &[
            ("name", "Atom"),
            ("preferred", "true"),
            ("apiLink", service_url.as_ref()),
            ("blogID", ""),
        ],
    );
    let _ = writer.write_event(Event::End(BytesEnd::new("apis")));

    let _ = writer.write_event(Event::End(BytesEnd::new("service")));
    let _ = writer.write_event(Event::End(BytesEnd::new("rsd")));

    String::from_utf8_lossy(&writer.into_inner()).into_owned()
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
        assert!(
            out.contains(r#"<rsd version="1.0" xmlns="http://archipelago.phrasewise.com/rsd">"#)
        );
        assert!(out.contains("<engineName>Jaunder</engineName>"));
        assert!(out.contains("<homePageLink>https://example.com/home</homePageLink>"));
        assert!(out.contains(r#"<api name="Atom" preferred="true" apiLink="https://example.com/atompub/service" blogID=""/>"#));
    }

    // `&` is a legal query separator and survives `TaggedUrl` normalization, so
    // escaping it is what keeps the document well-formed XML — not defence in
    // depth. (`<` and `"` are percent-encoded by normalization and cannot reach
    // this function.)
    #[test]
    fn rsd_document_escapes_url_query_ampersands() {
        let out = render_rsd_document(
            &parse_url("https://example.com/atompub?foo=1&bar=2"),
            &parse_url("https://example.com/home?x=1&y=2"),
        );
        assert!(out.contains(r#"apiLink="https://example.com/atompub?foo=1&amp;bar=2""#));
        assert!(out.contains("<homePageLink>https://example.com/home?x=1&amp;y=2</homePageLink>"));
        assert!(!out.contains("foo=1&bar=2"));
        assert!(!out.contains("x=1&y=2"));
    }
}
