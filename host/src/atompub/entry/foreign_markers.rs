//! Jaunder-owned `AtomPub` foreign markers.
//!
//! `atom_syndication` owns namespace-aware extension I/O (ADR-0089). This leaf
//! recognizes RFC 5023's `app:control/app:draft` and Jaunder's `j:slug`,
//! `j:etag`, and repeated `j:audience` target set.

use atom_syndication::Entry;
use atom_syndication::extension::{ExpandedName, Extension, ExtensionContent};
use common::etag::ETag;
use common::ids::AudienceId;
use common::visibility::AudienceTarget;
use thiserror::Error;

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
// Audience markers (j:audience) helpers
// ---------------------------------------------------------------------------

/// An invalid repeated `j:audience` representation.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error("invalid Jaunder Atom audience")]
pub struct InvalidAtomAudience;

fn parse_audience(value: &str) -> Result<AudienceTarget, InvalidAtomAudience> {
    match value {
        "public" => Ok(AudienceTarget::Public),
        "subscribers" => Ok(AudienceTarget::Subscribers),
        "private" => Ok(AudienceTarget::Private),
        _ => {
            let id = value
                .strip_prefix("named:")
                .filter(|id| {
                    let mut bytes = id.bytes();
                    matches!(bytes.next(), Some(b'1'..=b'9'))
                        && bytes.all(|byte| byte.is_ascii_digit())
                })
                .and_then(|id| id.parse::<i64>().ok())
                .filter(|id| *id > 0)
                .ok_or(InvalidAtomAudience)?;
            Ok(AudienceTarget::Named(AudienceId::from(id)))
        }
    }
}

fn audience_value(audience: &AudienceTarget) -> String {
    match audience {
        AudienceTarget::Public => "public".to_string(),
        AudienceTarget::Subscribers => "subscribers".to_string(),
        AudienceTarget::Private => "private".to_string(),
        AudienceTarget::Named(id) => format!("named:{id}"),
    }
}

fn projected_audiences(audiences: &[AudienceTarget]) -> Vec<AudienceTarget> {
    let mut canonical = if audiences.is_empty() {
        vec![AudienceTarget::Private]
    } else {
        audiences.to_vec()
    };
    canonical.sort_by_key(|audience| match audience {
        AudienceTarget::Public => (0, 0),
        AudienceTarget::Subscribers => (1, 0),
        AudienceTarget::Named(id) => (2, i64::from(*id)),
        AudienceTarget::Private => (3, 0),
    });
    canonical
}

pub(crate) fn canonical_audience_values(audiences: &[AudienceTarget]) -> Vec<String> {
    projected_audiences(audiences)
        .iter()
        .map(audience_value)
        .collect()
}

fn canonical_audiences(
    audiences: &[AudienceTarget],
) -> Result<Vec<AudienceTarget>, InvalidAtomAudience> {
    let canonical = projected_audiences(audiences);
    let mut unique = Vec::with_capacity(canonical.len());
    for audience in &canonical {
        if unique.contains(audience) {
            return Err(InvalidAtomAudience);
        }
        unique.push(audience.clone());
    }
    if canonical
        .iter()
        .any(|audience| matches!(audience, AudienceTarget::Private))
        && canonical.len() != 1
    {
        return Err(InvalidAtomAudience);
    }
    Ok(canonical)
}

/// Reads the complete canonical target set from direct `j:audience` extensions.
///
/// Absence is distinct from explicit Private. Empty stored targeting is emitted
/// as `private` by [`set_j_audiences`], so a present element set is always
/// nonempty.
///
/// # Errors
///
/// Returns [`InvalidAtomAudience`] when any value is noncanonical, duplicated,
/// or combines Private with another target.
pub fn j_audiences(entry: &Entry) -> Result<Option<Vec<AudienceTarget>>, InvalidAtomAudience> {
    let values = entry
        .extensions
        .iter()
        .filter(|extension| has_name(extension, ns::J_NS, "audience"))
        .map(|extension| match extension.content.as_slice() {
            [ExtensionContent::Text(value)] => parse_audience(value),
            _ => Err(InvalidAtomAudience),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.is_empty() {
        Ok(None)
    } else {
        canonical_audiences(&values).map(Some)
    }
}

/// Replaces every direct `j:audience` extension with the canonical target set.
///
/// An empty stored target set is Private under ADR-0020 and is serialized as an
/// explicit `private` marker rather than omission.
///
/// # Errors
///
/// Returns [`InvalidAtomAudience`] when the typed set contains a duplicate or
/// combines Private with another target.
pub fn set_j_audiences(
    entry: &mut Entry,
    audiences: &[AudienceTarget],
) -> Result<(), InvalidAtomAudience> {
    let audiences = canonical_audiences(audiences)?;
    let prefix = preferred_prefix(entry, ns::J_NS, "audience", "j");
    entry
        .extensions
        .retain(|extension| !has_name(extension, ns::J_NS, "audience"));
    for audience in audiences {
        let value = audience_value(&audience);
        let mut marker = extension(ns::J_NS, "audience", &prefix);
        marker.content.push(ExtensionContent::Text(value));
        entry.extensions.push(marker);
    }
    Ok(())
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

/// Read one direct, text-only Jaunder Member validator from a Collection Entry.
///
/// This uses the expanded XML name; a textual prefix is not part of the wire
/// identity. Ambiguous or malformed foreign markup is not a validator.
#[must_use]
pub fn j_member_etag(entry: &Entry) -> Option<ETag> {
    let mut markers = entry
        .extensions
        .iter()
        .filter(|extension| has_name(extension, ns::J_NS, "etag"));
    let marker = markers.next()?;
    if markers.next().is_some() || !marker.attributes.is_empty() {
        return None;
    }
    match marker.content.as_slice() {
        [ExtensionContent::Text(text)] => text.parse().ok(),
        _ => None,
    }
}

/// Attach the read-only Member validator to a Collection Entry.
///
/// This is deliberately not part of standalone Member responses: their validator
/// travels in the HTTP `ETag` header, while a feed needs per-Entry metadata.
pub fn set_j_member_etag(entry: &mut Entry, etag: &ETag) {
    let prefix = preferred_prefix(entry, ns::J_NS, "etag", "j");
    entry
        .extensions
        .retain(|extension| !has_name(extension, ns::J_NS, "etag"));

    let mut marker = extension(ns::J_NS, "etag", &prefix);
    marker
        .content
        .push(ExtensionContent::Text(etag.to_string()));
    entry.extensions.push(marker);
}

#[cfg(test)]
mod tests {
    use super::*;
    use atom_syndication::extension::ExtensionAttribute;
    use atom_syndication::{Category, Content, Link, Text};
    use common::ids::AudienceId;
    use common::visibility::AudienceTarget;

    use super::super::entry_document::entry_to_xml;

    fn sample_entry() -> Entry {
        r#"<entry xmlns="http://www.w3.org/2005/Atom"><id>tag:example.com,2026:post/1</id><title>Hello</title><updated>2026-01-02T00:00:00Z</updated></entry>"#
            .parse()
            .expect("valid Atom entry")
    }

    fn extension_with_text(uri: &str, local: &str, prefix: &str, text: &str) -> Extension {
        let mut extension = extension(uri, local, prefix);
        extension
            .content
            .push(ExtensionContent::Text(text.to_string()));
        extension
    }

    #[test]
    fn collection_member_etag_is_a_single_expanded_name() {
        let mut entry = sample_entry();
        let etag: ETag = "\"sha256-example\"".parse().unwrap();
        set_j_member_etag(&mut entry, &etag);
        let parsed = entry_to_xml(&entry)
            .unwrap()
            .parse::<Entry>()
            .expect("Atom entry round trips");
        assert_eq!(j_member_etag(&parsed), Some(etag.clone()));
        let mut renamed = parsed;
        renamed
            .extensions
            .iter_mut()
            .find(|ext| has_name(ext, ns::J_NS, "etag"))
            .unwrap()
            .name
            .preferred_prefix = Some("other".to_string());
        assert_eq!(j_member_etag(&renamed), Some(etag));
        renamed
            .extensions
            .push(extension_with_text(ns::J_NS, "etag", "j", "\"other\""));
        assert_eq!(j_member_etag(&renamed), None);
        renamed
            .extensions
            .retain(|ext| !has_name(ext, ns::J_NS, "etag"));
        renamed.extensions.push(extension(ns::J_NS, "etag", "j"));
        assert_eq!(j_member_etag(&renamed), None);
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
    fn j_audiences_round_trip_in_canonical_union_order() {
        let mut entry = sample_entry();
        set_j_audiences(
            &mut entry,
            &[
                AudienceTarget::Named(AudienceId::from(7)),
                AudienceTarget::Subscribers,
                AudienceTarget::Public,
                AudienceTarget::Named(AudienceId::from(2)),
            ],
        )
        .expect("valid audience union");

        let parsed = entry_to_xml(&entry)
            .expect("serialize")
            .parse::<Entry>()
            .expect("reparse");
        assert_eq!(
            j_audiences(&parsed).expect("valid audience extensions"),
            Some(vec![
                AudienceTarget::Public,
                AudienceTarget::Subscribers,
                AudienceTarget::Named(AudienceId::from(2)),
                AudienceTarget::Named(AudienceId::from(7)),
            ])
        );
    }

    #[test]
    fn j_audiences_distinguishes_absence_from_explicit_private() {
        let absent = sample_entry();
        assert_eq!(j_audiences(&absent).expect("absence is valid"), None);

        let mut private = sample_entry();
        set_j_audiences(&mut private, &[]).expect("empty storage targets mean Private");
        assert_eq!(
            j_audiences(&private).expect("private marker is valid"),
            Some(vec![AudienceTarget::Private])
        );
    }

    #[test]
    fn j_audiences_rejects_noncanonical_values_and_invalid_sets() {
        for invalid in [
            "",
            " public",
            "PUBLIC",
            "named:",
            "named:0",
            "named:01",
            "named:+1",
            "named:-1",
            "named:9223372036854775808",
        ] {
            let mut entry = sample_entry();
            entry
                .extensions
                .push(extension_with_text(ns::J_NS, "audience", "j", invalid));
            assert_eq!(
                j_audiences(&entry),
                Err(InvalidAtomAudience),
                "value {invalid:?}"
            );
        }

        for invalid in [
            vec![AudienceTarget::Public, AudienceTarget::Public],
            vec![AudienceTarget::Private, AudienceTarget::Subscribers],
        ] {
            let mut entry = sample_entry();
            assert_eq!(
                set_j_audiences(&mut entry, &invalid),
                Err(InvalidAtomAudience)
            );
        }
    }

    #[test]
    fn j_audiences_rejects_nested_or_mixed_content() {
        let entry: Entry = format!(
            r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:j="{}" xmlns:x="urn:foreign"><id>tag:example.com,2026:post/1</id><title>Hello</title><updated>2026-01-02T00:00:00Z</updated><j:audience>pub<x:ignored/>lic</j:audience></entry>"#,
            ns::J_NS,
        )
        .parse()
        .expect("well-formed mixed audience extension");

        assert_eq!(j_audiences(&entry), Err(InvalidAtomAudience));
    }

    #[test]
    fn j_audience_matching_uses_the_namespace_uri() {
        let mut entry = sample_entry();
        entry.extensions.push(extension_with_text(
            "urn:foreign",
            "audience",
            "j",
            "public",
        ));
        entry.extensions.push(extension_with_text(
            ns::J_NS,
            "audience",
            "other",
            "subscribers",
        ));
        assert_eq!(
            j_audiences(&entry).expect("Jaunder marker is valid"),
            Some(vec![AudienceTarget::Subscribers])
        );
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
        let mut entry: Entry = r#"<entry xmlns="http://www.w3.org/2005/Atom"><id>tag:example.com,2026:post/1</id><title>Hello</title><updated>2026-01-02T00:00:00Z</updated><published>2026-01-01T00:00:00Z</published></entry>"#
            .parse()
            .expect("valid Atom entry");
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
