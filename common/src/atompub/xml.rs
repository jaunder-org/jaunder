//! Shared `quick-xml` writer helpers for the `AtomPub` serializers.
//!
//! `write_text_element` was previously defined identically in `entry.rs` and
//! `service.rs`, and several serializers hand-rolled their own empty-element
//! writes. Centralizing them here removes that duplication and keeps element
//! and escaping behavior consistent across the entry, feed, service,
//! categories, and media-link serializers.
//!
//! Every serializer writes into an in-memory `Writer<Vec<u8>>`, whose only
//! failure mode is real I/O — which a `Vec<u8>` never produces. These helpers
//! therefore discard the (impossible) writer error and are infallible, so the
//! serializers can return a plain `String` rather than a `Result` with a dead
//! error path.
//!
//! The RSD serializer (`rsd.rs`) is intentionally not a client of these: it
//! formats a fixed template and escapes its two URLs directly, rather than
//! driving a `quick-xml` writer.

use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;

/// Writes a `<name>text</name>` element. The text is XML-escaped by `quick-xml`.
pub(super) fn write_text_element(writer: &mut Writer<Vec<u8>>, name: &str, text: &str) {
    let _ = writer.write_event(Event::Start(BytesStart::new(name)));
    let _ = writer.write_event(Event::Text(BytesText::new(text)));
    let _ = writer.write_event(Event::End(BytesEnd::new(name)));
}

/// Writes a self-closing `<name k="v" .../>` element. Attribute values are
/// XML-escaped by `quick-xml`.
pub(super) fn write_empty_element(
    writer: &mut Writer<Vec<u8>>,
    name: &str,
    attrs: &[(&str, &str)],
) {
    let mut start = BytesStart::new(name);
    for &(key, value) in attrs {
        start.push_attribute((key, value));
    }
    let _ = writer.write_event(Event::Empty(start));
}

/// Writes a self-closing `<link rel="..." href="..."/>` element.
pub(super) fn write_link(writer: &mut Writer<Vec<u8>>, rel: &str, href: &str) {
    write_empty_element(writer, "link", &[("rel", rel), ("href", href)]);
}
