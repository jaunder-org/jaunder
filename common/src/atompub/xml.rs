//! Shared `quick-xml` writer helpers for the `AtomPub` serializers.
//!
//! These exist to keep element and escaping behavior consistent across the
//! service and categories serializers, which had each hand-rolled their own
//! element writes.
//!
//! They once served the Atom documents too — entry, feed, and media-link — which
//! is why the set is wider than those two callers strictly need. Those documents
//! are now written by `atom_syndication` and no longer pass through here.
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
pub(super) fn write_empty_element<V: AsRef<str>>(
    writer: &mut Writer<Vec<u8>>,
    name: &str,
    attrs: &[(&str, V)],
) {
    let mut start = BytesStart::new(name);
    for (key, value) in attrs {
        start.push_attribute((*key, value.as_ref()));
    }
    let _ = writer.write_event(Event::Empty(start));
}
