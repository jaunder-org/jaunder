//! Jaunder-owned `AtomPub` foreign markers.
//!
//! `atom_syndication` owns namespace-aware extension I/O (ADR-0089). This leaf
//! recognizes only RFC 5023's `app:control/app:draft` and Jaunder's `j:slug`.

use atom_syndication::Entry;
use atom_syndication::extension::{ExpandedName, Extension, ExtensionContent};

use super::super::ns;

fn name(namespace_uri: &str, local_name: &str, preferred_prefix: &str) -> ExpandedName {
    ExpandedName {
        namespace_uri: Some(namespace_uri.to_string()),
        local_name: local_name.to_string(),
        preferred_prefix: Some(preferred_prefix.to_string()),
    }
}

fn has_name(extension: &Extension, namespace_uri: &str, local_name: &str) -> bool {
    extension.name.namespace_uri.as_deref() == Some(namespace_uri)
        && extension.name.local_name == local_name
}

fn direct_text(extension: &Extension) -> String {
    extension
        .content
        .iter()
        .filter_map(|content| match content {
            ExtensionContent::Text(text) => Some(text.as_str()),
            ExtensionContent::Element(_) => None,
        })
        .collect()
}

fn preferred_prefix(
    entry: &Entry,
    namespace_uri: &str,
    local_name: &str,
    fallback: &str,
) -> String {
    entry
        .extensions
        .iter()
        .filter(|extension| has_name(extension, namespace_uri, local_name))
        .find_map(|extension| extension.name.preferred_prefix.clone())
        .unwrap_or_else(|| fallback.to_string())
}

fn extension(namespace_uri: &str, local_name: &str, preferred_prefix: &str) -> Extension {
    Extension {
        name: name(namespace_uri, local_name, preferred_prefix),
        attributes: Vec::new(),
        content: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Draft flag (app:control/app:draft) helpers
// ---------------------------------------------------------------------------

/// Returns the explicit `app:control/app:draft` marker when present.
///
/// `Some(true)` is RFC 5023's `yes` value; any other explicit marker value is
/// `Some(false)`. Multiple valid markers retain [`is_draft`]'s established
/// meaning: `yes` wins.
#[must_use]
pub fn draft_marker(entry: &Entry) -> Option<bool> {
    let mut found = false;
    let mut draft = false;
    for control in entry
        .extensions
        .iter()
        .filter(|extension| has_name(extension, ns::APP_NS, "control"))
    {
        for child in control.content.iter().filter_map(|content| match content {
            ExtensionContent::Element(child) if has_name(child, ns::APP_NS, "draft") => Some(child),
            _ => None,
        }) {
            found = true;
            draft |= direct_text(child).trim().eq_ignore_ascii_case("yes");
        }
    }
    found.then_some(draft)
}

/// Returns true when the entry carries `app:control/app:draft = yes`.
#[must_use]
pub fn is_draft(entry: &Entry) -> bool {
    draft_marker(entry).unwrap_or(false)
}

/// Sets or clears the `app:control/app:draft` marker on an entry.
///
/// Only direct APP `draft` children are replaced. All attributes, text, foreign
/// children, and their ordering survive; an empty APP control is removed only
/// when clearing the marker.
pub fn set_draft(entry: &mut Entry, draft: bool) {
    for control in entry
        .extensions
        .iter_mut()
        .filter(|extension| has_name(extension, ns::APP_NS, "control"))
    {
        control.content.retain(|content| {
            !matches!(content, ExtensionContent::Element(child) if has_name(child, ns::APP_NS, "draft"))
        });
    }

    if draft {
        if let Some(control) = entry
            .extensions
            .iter_mut()
            .find(|extension| has_name(extension, ns::APP_NS, "control"))
        {
            let prefix = control
                .name
                .preferred_prefix
                .clone()
                .unwrap_or_else(|| "app".to_string());
            let mut marker = extension(ns::APP_NS, "draft", &prefix);
            marker
                .content
                .push(ExtensionContent::Text("yes".to_string()));
            control.content.push(ExtensionContent::Element(marker));
        } else {
            let prefix = preferred_prefix(entry, ns::APP_NS, "control", "app");
            let mut control = extension(ns::APP_NS, "control", &prefix);
            let mut marker = extension(ns::APP_NS, "draft", &prefix);
            marker
                .content
                .push(ExtensionContent::Text("yes".to_string()));
            control.content.push(ExtensionContent::Element(marker));
            entry.extensions.push(control);
        }
    } else {
        entry.extensions.retain(|extension| {
            !has_name(extension, ns::APP_NS, "control")
                || !extension.attributes.is_empty()
                || !extension.content.is_empty()
        });
    }
}

// ---------------------------------------------------------------------------
// Slug marker (j:slug) helpers
// ---------------------------------------------------------------------------

/// Read the read-only server slug from the first direct `j:slug` extension.
#[must_use]
pub fn j_slug(entry: &Entry) -> Option<String> {
    entry
        .extensions
        .iter()
        .find(|extension| has_name(extension, ns::J_NS, "slug"))
        .map(direct_text)
}

/// Set (idempotently replace) the direct `j:slug` extension. Emitted on every
/// outgoing entry; the server never reads an incoming one.
pub fn set_j_slug(entry: &mut Entry, slug: &str) {
    let prefix = preferred_prefix(entry, ns::J_NS, "slug", "j");
    entry
        .extensions
        .retain(|extension| !has_name(extension, ns::J_NS, "slug"));

    let mut marker = extension(ns::J_NS, "slug", &prefix);
    marker
        .content
        .push(ExtensionContent::Text(slug.to_string()));
    entry.extensions.push(marker);
}

#[cfg(test)]
mod tests {
    use super::*;
    use atom_syndication::extension::ExtensionAttribute;
    use atom_syndication::{Category, Content, Link, Text};

    use super::super::entry_document::entry_to_xml;

    fn sample_entry() -> Entry {
        Entry {
            id: "tag:example.com,2026:post/1".to_string(),
            title: Text::plain("Hello"),
            updated: chrono::DateTime::parse_from_rfc3339("2026-01-02T00:00:00Z").unwrap(),
            ..Default::default()
        }
    }

    fn extension_with_text(uri: &str, local: &str, prefix: &str, text: &str) -> Extension {
        let mut extension = extension(uri, local, prefix);
        extension
            .content
            .push(ExtensionContent::Text(text.to_string()));
        extension
    }

    #[test]
    fn set_and_read_j_slug_round_trips() {
        let mut entry = sample_entry();
        set_j_slug(&mut entry, "my-post");
        assert_eq!(j_slug(&entry), Some("my-post".to_string()));
        let parsed = entry_to_xml(&entry)
            .expect("serialize")
            .parse::<Entry>()
            .expect("reparse");
        assert_eq!(j_slug(&parsed), Some("my-post".to_string()));
    }

    #[test]
    fn re_setting_markers_replaces_rather_than_accumulates() {
        let mut entry = sample_entry();
        set_j_slug(&mut entry, "first");
        set_j_slug(&mut entry, "second");
        set_draft(&mut entry, true);
        set_draft(&mut entry, true);
        assert_eq!(j_slug(&entry), Some("second".to_string()));
        assert_eq!(
            entry
                .extensions
                .iter()
                .filter(|extension| has_name(extension, ns::J_NS, "slug"))
                .count(),
            1
        );
        assert_eq!(
            entry
                .extensions
                .iter()
                .filter(|extension| has_name(extension, ns::APP_NS, "control"))
                .count(),
            1
        );
        assert!(is_draft(&entry));
    }

    #[test]
    fn marker_serialization_reparses_with_expanded_names() {
        let mut entry = sample_entry();
        set_j_slug(&mut entry, "my-post");
        set_draft(&mut entry, true);
        let parsed = entry_to_xml(&entry)
            .expect("serialize")
            .parse::<Entry>()
            .expect("reparse");
        assert_eq!(j_slug(&parsed), Some("my-post".to_string()));
        assert!(is_draft(&parsed));
    }

    #[test]
    fn plain_entry_has_no_marker_extensions() {
        let entry = sample_entry();
        assert!(entry.extensions.is_empty());
        let parsed = entry_to_xml(&entry)
            .expect("serialize")
            .parse::<Entry>()
            .expect("reparse");
        assert!(parsed.extensions.is_empty());
    }

    #[test]
    fn draft_and_html_round_trip_through_serialize_then_parse() {
        let mut entry = sample_entry();
        entry.title = Text::plain("RT");
        entry.summary = Some(Text::plain("s"));
        entry.content = Some(Content {
            content_type: Some("html".to_string()),
            value: Some("<p>body & more</p>".to_string()),
            ..Default::default()
        });
        entry.categories = vec![
            Category {
                term: "a".to_string(),
                ..Default::default()
            },
            Category {
                term: "b".to_string(),
                ..Default::default()
            },
        ];
        entry.links = vec![Link {
            rel: "edit".to_string(),
            href: "https://h/atompub/alice/posts/1".to_string(),
            ..Default::default()
        }];
        entry.published =
            Some(chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap());
        set_draft(&mut entry, true);
        set_j_slug(&mut entry, "my-post");
        let parsed = entry_to_xml(&entry)
            .expect("serialize")
            .parse::<Entry>()
            .expect("reparse");
        assert!(is_draft(&parsed));
        assert_eq!(parsed.title().as_str(), "RT");
        assert_eq!(parsed.summary().map(Text::as_str), Some("s"));
        assert_eq!(parsed.links()[0].href(), "https://h/atompub/alice/posts/1");
        assert_eq!(j_slug(&parsed), Some("my-post".to_string()));
        assert_eq!(
            parsed.content().and_then(Content::value),
            Some("<p>body & more</p>")
        );
        assert_eq!(
            parsed
                .categories()
                .iter()
                .map(Category::term)
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    #[test]
    fn draft_marker_preserves_explicit_non_draft_presence_and_direct_text_only() {
        let absent = sample_entry();
        let mut explicit_no = sample_entry();
        let mut control = extension(ns::APP_NS, "control", "app");
        let mut no = extension_with_text(ns::APP_NS, "draft", "app", " no ");
        no.content
            .push(ExtensionContent::Element(extension_with_text(
                "urn:foreign",
                "yes",
                "x",
                "yes",
            )));
        control.content.push(ExtensionContent::Element(no));
        explicit_no.extensions.push(control);
        let mut explicit_yes = explicit_no.clone();
        explicit_yes.extensions[0]
            .content
            .push(ExtensionContent::Element(extension_with_text(
                ns::APP_NS,
                "draft",
                "app",
                "YES",
            )));
        assert_eq!(draft_marker(&absent), None);
        assert_eq!(draft_marker(&explicit_no), Some(false));
        assert_eq!(draft_marker(&explicit_yes), Some(true));
    }

    #[test]
    fn marker_matching_is_by_uri_not_prefix_or_local_name() {
        let mut entry = sample_entry();
        let mut foreign_control = extension("urn:other", "control", "app");
        foreign_control
            .content
            .push(ExtensionContent::Element(extension_with_text(
                "urn:other",
                "draft",
                "app",
                "yes",
            )));
        entry.extensions.push(foreign_control.clone());
        entry
            .extensions
            .push(extension_with_text(ns::APP_NS, "control", "other", ""));
        assert!(!is_draft(&entry));
        set_draft(&mut entry, false);
        assert_eq!(entry.extensions[0], foreign_control);
    }

    #[test]
    fn clearing_draft_preserves_control_attributes_mixed_content_and_foreign_children() {
        let mut entry = sample_entry();
        let mut control = extension(ns::APP_NS, "control", "pub");
        control.attributes.push(ExtensionAttribute {
            name: name("urn:foreign", "state", "f"),
            value: "kept".to_string(),
        });
        control.content = vec![
            ExtensionContent::Text("before".to_string()),
            ExtensionContent::Element(extension_with_text(ns::APP_NS, "draft", "pub", "yes")),
            ExtensionContent::Element(extension_with_text("urn:foreign", "draft", "f", "theirs")),
            ExtensionContent::Text("after".to_string()),
        ];
        entry.extensions.push(control);
        set_draft(&mut entry, false);
        let control = entry
            .extensions
            .iter()
            .find(|extension| has_name(extension, ns::APP_NS, "control"))
            .expect("control kept");
        assert_eq!(control.attributes.len(), 1);
        assert_eq!(direct_text(control), "beforeafter");
        assert!(control.content.iter().any(|content| matches!(content, ExtensionContent::Element(child) if has_name(child, "urn:foreign", "draft"))));
    }

    #[test]
    fn setting_draft_reuses_first_app_control_and_its_prefix_hint() {
        let mut entry = sample_entry();
        let mut control = extension(ns::APP_NS, "control", "pub");
        control
            .content
            .push(ExtensionContent::Element(extension_with_text(
                ns::APP_NS,
                "review",
                "pub",
                "pending",
            )));
        entry.extensions.push(control);
        set_draft(&mut entry, true);
        let controls = entry
            .extensions
            .iter()
            .filter(|extension| has_name(extension, ns::APP_NS, "control"))
            .collect::<Vec<_>>();
        assert_eq!(controls.len(), 1);
        let draft = controls[0]
            .content
            .iter()
            .find_map(|content| match content {
                ExtensionContent::Element(child) if has_name(child, ns::APP_NS, "draft") => {
                    Some(child)
                }
                _ => None,
            })
            .expect("draft");
        assert_eq!(draft.name.preferred_prefix.as_deref(), Some("pub"));
        assert_eq!(direct_text(draft), "yes");
    }

    #[test]
    fn setting_draft_preserves_foreign_extensions_and_reparses_safely() {
        let mut entry = sample_entry();
        entry
            .extensions
            .push(extension_with_text("urn:other", "thing", "app", "x"));
        set_draft(&mut entry, true);
        let parsed = entry_to_xml(&entry)
            .expect("serialize")
            .parse::<Entry>()
            .expect("reparse");
        assert!(is_draft(&parsed));
        assert!(parsed.extensions.iter().any(|extension| has_name(
            extension,
            "urn:other",
            "thing"
        )));
    }

    #[test]
    fn element_scoped_and_alternate_prefix_markers_are_recognized_and_reused() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom"><title>T</title><pub:control xmlns:pub="http://www.w3.org/2007/app"><pub:draft>no</pub:draft></pub:control><jaunder:slug xmlns:jaunder="https://jaunder.org/ns/atompub">old</jaunder:slug></entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, true);
        set_j_slug(&mut entry, "new");
        assert!(is_draft(&entry));
        assert_eq!(j_slug(&entry), Some("new".to_string()));
        let control = entry
            .extensions
            .iter()
            .find(|extension| has_name(extension, ns::APP_NS, "control"))
            .expect("control");
        assert_eq!(control.name.preferred_prefix.as_deref(), Some("pub"));
        assert_eq!(
            entry
                .extensions
                .last()
                .and_then(|extension| extension.name.preferred_prefix.as_deref()),
            Some("jaunder")
        );
    }

    #[test]
    fn slug_replacement_preserves_foreign_same_local_extension_and_order() {
        let mut entry = sample_entry();
        let foreign = extension_with_text("urn:other", "slug", "j", "theirs");
        entry.extensions.push(foreign.clone());
        entry
            .extensions
            .push(extension_with_text(ns::J_NS, "slug", "jaunder", "old"));
        set_j_slug(&mut entry, "ours");
        assert_eq!(entry.extensions[0], foreign);
        assert_eq!(j_slug(&entry), Some("ours".to_string()));
        assert_eq!(
            entry
                .extensions
                .last()
                .and_then(|extension| extension.name.preferred_prefix.as_deref()),
            Some("jaunder")
        );
    }

    #[test]
    fn clearing_draft_drops_only_truly_empty_controls() {
        let mut entry = sample_entry();
        let mut empty = extension(ns::APP_NS, "control", "app");
        empty
            .content
            .push(ExtensionContent::Element(extension_with_text(
                ns::APP_NS,
                "draft",
                "app",
                "yes",
            )));
        entry.extensions.push(empty);
        set_draft(&mut entry, false);
        assert!(entry.extensions.is_empty());
    }
}
