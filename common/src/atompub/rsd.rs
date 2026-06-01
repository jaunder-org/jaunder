//! `RSD` (Really Simple Discovery) document serializer for `AtomPub`.
//!
//! An `RSD` document advertises publishing capabilities to client applications
//! like `MarsEdit` and external blog editors. This module provides [`render_rsd_document`]
//! to generate an `RSD` document pointing to the `AtomPub` service and home page.

/// Serializes a Really Simple Discovery (`RSD`) document.
///
/// Returns an `RSD` document (as specified in the `RSD` specification) with the given
/// service URL and homepage URL embedded. The service URL is the `AtomPub` Service
/// Document endpoint; the homepage URL is the site's public-facing home.
///
/// Both URLs are XML-escaped to prevent injection.
///
/// # Infallible
///
/// This function is infallible — it always returns a `String`.
#[must_use]
pub fn render_rsd_document(service_url: &str, homepage_url: &str) -> String {
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
        homepage = quick_xml::escape::escape(homepage_url).into_owned(),
        service = quick_xml::escape::escape(service_url).into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rsd_document_contains_engine_name_and_urls() {
        let out = render_rsd_document(
            "https://example.com/atompub/service",
            "https://example.com/home",
        );
        assert!(out.contains("<engineName>Jaunder</engineName>"));
        assert!(out.contains("https://example.com/atompub/service"));
        assert!(out.contains("https://example.com/home"));
        assert!(out.contains("apiLink="));
    }

    #[test]
    fn rsd_document_escapes_special_characters() {
        let out = render_rsd_document(
            "https://example.com/atompub?foo=1&bar=2",
            "https://example.com/?x<y",
        );
        assert!(out.contains("foo=1&amp;bar=2"));
        assert!(out.contains("&lt;"));
    }
}
