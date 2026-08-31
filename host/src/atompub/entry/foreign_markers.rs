//! Jaunder-owned `AtomPub` foreign markers and their namespace bookkeeping.
//!
//! `atom_syndication` owns Atom document I/O (ADR-0089). This leaf owns only
//! the extension-map representation upstream does not model: RFC 5023's
//! `app:control/app:draft` and Jaunder's `j:slug` (ADR-0023). Every read
//! resolves namespace bindings rather than treating `app` or `j` as reserved
//! spellings; every write keeps an entry-level `xmlns:*` binding exactly while
//! its extension needs it.

use std::collections::BTreeMap;

use atom_syndication::Entry;
use atom_syndication::extension::Extension;

use super::super::ns;

/// Resolves `prefix` through an innermost-first chain of namespace scopes — an
/// element's own declarations shadow its parent's, which shadow the entry's.
fn resolve<'a>(scopes: &[&'a BTreeMap<String, String>], prefix: &str) -> Option<&'a str> {
    scopes
        .iter()
        .find_map(|scope| scope.get(prefix))
        .map(String::as_str)
}

/// Whether a child extension's qualified name carries a prefix resolving to `uri`,
/// given the declarations its own element, its parent and the entry supply.
///
/// An unprefixed name is in the default namespace — Atom, in these documents —
/// and so is never one of ours.
fn child_in_namespace(
    namespaces: &BTreeMap<String, String>,
    parent_attrs: &BTreeMap<String, String>,
    child: &Extension,
    uri: &str,
) -> bool {
    child.name.split_once(':').is_some_and(|(prefix, _)| {
        resolve(&[&child.attrs, parent_attrs, namespaces], prefix) == Some(uri)
    })
}

/// The entry's `uri`-namespaced extensions with local name `local`, paired with the
/// prefix each was spelled under — under whatever prefix (or prefixes) the document
/// used, and whichever scope declared it.
fn markers_in<'a>(
    entry: &'a Entry,
    uri: &'a str,
    local: &'a str,
) -> impl Iterator<Item = (&'a str, &'a Extension)> {
    entry
        .extensions
        .iter()
        .flat_map(move |(prefix, by_local)| {
            by_local
                .get(local)
                .into_iter()
                .flatten()
                .map(move |ext| (prefix.as_str(), ext))
        })
        .filter(move |(prefix, ext)| resolve(&[&ext.attrs, &entry.namespaces], prefix) == Some(uri))
}

/// The prefix to write a `uri`-namespaced `local` marker under: the one already
/// labelling the entry's own such marker, so a re-set lands in the element the
/// entry already has rather than beside it; else [`writable_prefix`].
fn marker_prefix(entry: &Entry, uri: &str, local: &str, preferred: &str) -> String {
    markers_in(entry, uri, local)
        .next()
        .map_or_else(|| writable_prefix(entry, uri, preferred), |(p, _)| p.into())
}

/// The prefix to declare a `uri` marker under when the entry has none: its existing
/// binding for the namespace, else `preferred` — or a numbered variant of it when
/// some *other* namespace already owns that spelling, since redeclaring it would
/// silently relabel that namespace's markers.
fn writable_prefix(entry: &Entry, uri: &str, preferred: &str) -> String {
    if let Some((prefix, _)) = entry.namespaces.iter().find(|(_, bound)| *bound == uri) {
        return prefix.clone();
    }
    // A prefix some other extension is spelled with is spoken for even when the
    // entry element doesn't declare it — the declaration may be on that element.
    let taken = |p: &str| entry.namespaces.contains_key(p) || entry.extensions.contains_key(p);
    if !taken(preferred) {
        return preferred.to_string();
    }
    let mut n = 1;
    loop {
        let candidate = format!("{preferred}{n}");
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Drops the `xmlns:` declaration of every `uri`-bound prefix that is left
/// labelling nothing.
///
/// The writer emits `Entry::namespaces` verbatim, so a declaration has to track
/// the extensions that need it: a stale one is noise on the wire, and dropping
/// one too early would emit unbound-prefix XML.
fn prune_bindings(entry: &mut Entry, uri: &str) {
    let extensions = &entry.extensions;
    entry
        .namespaces
        .retain(|prefix, bound| bound != uri || extensions.contains_key(prefix));
}

/// Drops every extension map left empty by a removal, so `prune_bindings` and
/// `extensions_in` see a prefix exactly while it still labels something.
fn prune_empty_extensions(entry: &mut Entry) {
    entry.extensions.retain(|_, by_local| !by_local.is_empty());
}

// ---------------------------------------------------------------------------
// Draft flag (app:control/app:draft) helpers
// ---------------------------------------------------------------------------

/// Returns the explicit `app:control/app:draft` marker when present.
///
/// `Some(true)` is RFC 5023's `yes` value; any other explicit marker value is
/// `Some(false)`. This preserves the distinction between an explicit
/// non-draft marker and no marker, which callers that merge lifecycle sources
/// need. Multiple valid markers retain [`is_draft`]'s established meaning:
/// `yes` wins.
#[must_use]
pub fn draft_marker(entry: &Entry) -> Option<bool> {
    markers_in(entry, ns::APP_NS, "control")
        .filter_map(|(_, control)| control_draft_marker(&entry.namespaces, control))
        .reduce(|draft, marker| draft || marker)
}

/// Returns true when the entry carries `app:control/app:draft = yes`.
#[must_use]
pub fn is_draft(entry: &Entry) -> bool {
    draft_marker(entry).unwrap_or(false)
}

fn control_draft_marker(
    namespaces: &BTreeMap<String, String>,
    control: &Extension,
) -> Option<bool> {
    let drafts = control.children.get("draft")?;
    let mut found = false;
    let mut is_draft = false;
    for draft in drafts {
        if child_in_namespace(namespaces, &control.attrs, draft, ns::APP_NS) {
            found = true;
            is_draft |= draft
                .value
                .as_deref()
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("yes"));
        }
    }
    found.then_some(is_draft)
}

/// Sets or clears the `app:control/app:draft` marker on an entry.
///
/// `app:draft` is the only child RFC 5023 defines for `app:control`, but it is
/// explicitly not the only one that may appear there, so this clears the draft
/// child alone: any other control child the document carried survives, and the
/// `app:control` wrapper is dropped only once nothing is left inside it.
pub fn set_draft(entry: &mut Entry, draft: bool) {
    // Reuse the prefix the entry's own control element is spelled with, so a document
    // that spells APP another way keeps its spelling and the flag lands *in* that
    // control — RFC 5023 §B gives an entry one `app:control`, not one per prefix.
    // Chosen *before* the removal, which prunes a binding once nothing is left under
    // it — including the one naming that spelling.
    let prefix = draft.then(|| marker_prefix(entry, ns::APP_NS, "control", "app"));
    remove_draft_marker(entry);

    if let Some(prefix) = prefix {
        let draft_ext = Extension {
            name: format!("{prefix}:draft"),
            value: Some("yes".to_string()),
            ..Extension::default()
        };
        let by_local = entry.extensions.entry(prefix.clone()).or_default();
        let mut controls = by_local.remove("control").unwrap_or_default();
        match controls.first_mut() {
            Some(control) => {
                control
                    .children
                    .insert("draft".to_string(), vec![draft_ext]);
                // The prefix is declared on the entry element below, so a declaration
                // this element carried is redundant — and upstream would re-emit it as
                // a plain attribute rather than an `xmlns:`, which is worse than none.
                control
                    .attrs
                    .retain(|key, value| key != &prefix || value != ns::APP_NS);
            }
            None => controls.push(Extension {
                name: format!("{prefix}:control"),
                children: BTreeMap::from([("draft".to_string(), vec![draft_ext])]),
                ..Extension::default()
            }),
        }
        by_local.insert("control".to_string(), controls);
        entry.namespaces.insert(prefix, ns::APP_NS.to_string());
    }
}

/// Strips the draft child from every `app:control` the entry carries, whatever
/// prefix that namespace was spelled with, so setting the flag replaces rather
/// than accumulates and clearing it leaves nothing stale behind.
fn remove_draft_marker(entry: &mut Entry) {
    let namespaces = &entry.namespaces;
    for (declared, by_local) in &mut entry.extensions {
        let Some(controls) = by_local.get_mut("control") else {
            continue;
        };
        controls.retain_mut(|control| {
            if resolve(&[&control.attrs, namespaces], declared) != Some(ns::APP_NS) {
                return true;
            }
            let parent_attrs = &control.attrs;
            if let Some(drafts) = control.children.get_mut("draft") {
                drafts.retain(|d| !child_in_namespace(namespaces, parent_attrs, d, ns::APP_NS));
            }
            control.children.retain(|_, children| !children.is_empty());
            // A control element that carried nothing but the draft flag has said all
            // it has to say; one with other children, a value, or an attribute beyond
            // its own namespace declaration has not.
            !control.children.is_empty()
                || control.value.is_some()
                || control
                    .attrs
                    .iter()
                    .any(|(key, value)| key != declared || value != ns::APP_NS)
        });
        by_local.retain(|_, exts| !exts.is_empty());
    }
    prune_empty_extensions(entry);
    prune_bindings(entry, ns::APP_NS);
}

// ---------------------------------------------------------------------------
// Slug marker (j:slug) helpers
// ---------------------------------------------------------------------------

/// Read the read-only server slug from a `j:slug` extension, if present.
#[must_use]
pub fn j_slug(entry: &Entry) -> Option<String> {
    markers_in(entry, ns::J_NS, "slug").find_map(|(_, ext)| ext.value.clone())
}

/// Set (idempotently replace) the `j:slug` extension. Emitted on every outgoing
/// entry; the server never reads an incoming one.
pub fn set_j_slug(entry: &mut Entry, slug: &str) {
    // As in `set_draft`: the prefix is chosen before the removal that may prune the
    // binding naming the document's own spelling of the namespace.
    let prefix = marker_prefix(entry, ns::J_NS, "slug", "j");

    // Idempotent replace: drop any slug already in our namespace, then re-add one —
    // so re-setting never leaves a stale or duplicate marker behind. A `slug` in
    // someone else's namespace is not ours to remove.
    let namespaces = &entry.namespaces;
    for (declared, by_local) in &mut entry.extensions {
        if let Some(slugs) = by_local.get_mut("slug") {
            slugs.retain(|ext| resolve(&[&ext.attrs, namespaces], declared) != Some(ns::J_NS));
        }
        by_local.retain(|_, exts| !exts.is_empty());
    }
    prune_empty_extensions(entry);
    prune_bindings(entry, ns::J_NS);

    let ext = Extension {
        name: format!("{prefix}:slug"),
        value: Some(slug.to_string()),
        ..Extension::default()
    };
    // Pushed, not inserted: the prefix may still label a `slug` in someone else's
    // namespace, and replacing the whole vec would be the trampling this avoids.
    entry
        .extensions
        .entry(prefix.clone())
        .or_default()
        .entry("slug".to_string())
        .or_default()
        .push(ext);
    // As with `set_draft`, this helper owns its prefix's `xmlns:j` declaration.
    // There is no clearing counterpart: a slug is set on every outgoing entry.
    entry.namespaces.insert(prefix, ns::J_NS.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
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

    /// The `(type, value)` of an entry's `<content>`. This marker round-trip
    /// always supplies content, so absence is a broken test.
    fn content_parts(entry: &Entry) -> (Option<&str>, Option<&str>) {
        let content = entry.content().expect("entry carries <content>");
        (content.content_type(), content.value())
    }

    #[test]
    fn set_and_read_j_slug_round_trips() {
        let mut entry = sample_entry();
        set_j_slug(&mut entry, "my-post");
        assert_eq!(j_slug(&entry), Some("my-post".to_string()));
    }

    #[test]
    fn re_setting_a_marker_replaces_rather_than_accumulates() {
        // Both the extension map and `namespaces` are keyed maps, so a repeated set
        // overwrites its key rather than appending a second marker or a duplicate
        // xmlns declaration.
        let mut entry = sample_entry();
        set_j_slug(&mut entry, "first");
        set_j_slug(&mut entry, "second");
        set_draft(&mut entry, true);
        set_draft(&mut entry, true);

        assert_eq!(j_slug(&entry), Some("second".to_string()));
        let out = entry_to_xml(&entry).expect("serialize");
        assert_eq!(out.matches("<j:slug>").count(), 1, "out: {out}");
        assert_eq!(out.matches("app:control").count(), 2, "out: {out}"); // open + close
        assert_eq!(out.matches("xmlns:j=").count(), 1, "out: {out}");
        assert_eq!(out.matches("xmlns:app=").count(), 1, "out: {out}");
    }

    #[test]
    fn j_slug_is_serialized_with_namespace() {
        let mut entry = sample_entry();
        set_j_slug(&mut entry, "my-post");
        let out = entry_to_xml(&entry).expect("serialize");
        assert!(
            out.contains(r#"xmlns:j="https://jaunder.org/ns/atompub""#),
            "out: {out}"
        );
        assert!(out.contains("<j:slug>my-post</j:slug>"), "out: {out}");
    }

    #[test]
    fn an_entry_with_no_markers_declares_neither_prefix() {
        // A prefix is declared exactly while the marker it labels is present, so a
        // plain entry carries neither declaration.
        let entry = sample_entry();
        let out = entry_to_xml(&entry).expect("serialize");
        assert!(!out.contains("xmlns:j"), "out: {out}");
        assert!(!out.contains("xmlns:app"), "out: {out}");
    }

    #[test]
    fn serializes_draft_entry_with_app_control_and_escapes_html() {
        let mut entry = sample_entry();
        set_draft(&mut entry, true);
        entry.content = Some(Content {
            content_type: Some("html".to_string()),
            value: Some("<p>x</p>".to_string()),
            ..Default::default()
        });
        let out = entry_to_xml(&entry).expect("serialize");
        assert!(out.contains("app:draft"), "out: {out}");
        assert!(out.contains("yes"), "out: {out}");
        assert!(out.contains("type=\"html\""), "out: {out}");
        assert!(out.contains("&lt;p&gt;x&lt;/p&gt;"), "out: {out}");
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

        let out = entry_to_xml(&entry).expect("serialize");
        let parsed = out.parse::<Entry>().expect("re-parse");
        assert!(is_draft(&parsed), "draft flag lost; xml: {out}");
        assert_eq!(parsed.title().as_str(), "RT");
        assert_eq!(parsed.summary().map(Text::as_str), Some("s"));
        assert_eq!(parsed.links().len(), 1);
        assert_eq!(parsed.links()[0].rel(), "edit");
        assert_eq!(parsed.links()[0].href(), "https://h/atompub/alice/posts/1");
        assert_eq!(
            parsed
                .published()
                .map(chrono::DateTime::to_rfc3339)
                .as_deref(),
            Some("2026-01-01T00:00:00+00:00")
        );
        assert_eq!(
            parsed.updated().to_rfc3339(),
            entry.updated().to_rfc3339(),
            "updated lost; xml: {out}"
        );
        assert_eq!(j_slug(&parsed), Some("my-post".to_string()));
        assert_eq!(
            content_parts(&parsed),
            (Some("html"), Some("<p>body & more</p>"))
        );
        let terms: Vec<&str> = parsed.categories().iter().map(Category::term).collect();
        assert_eq!(terms, vec!["a", "b"]);
    }

    #[test]
    fn set_draft_false_clears_existing_marker() {
        let mut entry = sample_entry();
        set_draft(&mut entry, true);
        assert!(is_draft(&entry));
        set_draft(&mut entry, false);
        assert!(!is_draft(&entry));
        assert!(
            !entry_to_xml(&entry)
                .expect("serialize")
                .contains("app:draft")
        );
    }

    #[test]
    fn draft_marker_preserves_explicit_non_draft_presence() {
        let absent = r#"<entry xmlns="http://www.w3.org/2005/Atom"><title>T</title></entry>"#
            .parse::<Entry>()
            .expect("parse");
        let explicit_no =
            r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>T</title>
  <app:control><app:draft>no</app:draft></app:control>
</entry>"#
                .parse::<Entry>()
                .expect("parse");
        let explicit_yes =
            r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>T</title>
  <app:control><app:draft>yes</app:draft></app:control>
</entry>"#
                .parse::<Entry>()
                .expect("parse");

        assert_eq!(draft_marker(&absent), None);
        assert_eq!(draft_marker(&explicit_no), Some(false));
        assert_eq!(draft_marker(&explicit_yes), Some(true));
        assert!(!is_draft(&explicit_no));
        assert!(is_draft(&explicit_yes));
    }

    #[test]
    fn a_control_element_outside_the_app_namespace_is_not_a_draft_flag() {
        // `app` is a conventional prefix, not a reserved one: the same spelling bound
        // to someone else's namespace is someone else's element.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="https://example.com/ns/other">
  <title>T</title>
  <app:control><app:draft>yes</app:draft></app:control>
</entry>"#;
        let entry = xml.parse::<Entry>().expect("parse");
        assert!(!is_draft(&entry));
    }

    #[test]
    fn an_unprefixed_draft_child_is_not_the_app_draft_flag() {
        // An unprefixed child is in the default namespace — Atom here, not APP.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>T</title>
  <app:control><draft>yes</draft></app:control>
</entry>"#;
        let entry = xml.parse::<Entry>().expect("parse");
        assert!(!is_draft(&entry));
    }

    #[test]
    fn clearing_the_draft_flag_leaves_a_foreign_control_element_alone() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="https://example.com/ns/other">
  <title>T</title>
  <app:control><app:draft>yes</app:draft></app:control>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, false);

        let out = entry_to_xml(&entry).expect("serialize");
        assert!(out.contains("app:control"), "out: {out}");
        assert!(out.contains("app:draft"), "out: {out}");
        assert!(
            out.contains(r#"xmlns:app="https://example.com/ns/other""#),
            "out: {out}"
        );
    }

    #[test]
    fn clearing_the_draft_flag_keeps_the_rest_of_the_control_element() {
        // RFC 5023 defines `app:draft` as the only *current* child of `app:control`,
        // not as the only possible one — so clearing the flag clears the flag.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>T</title>
  <app:control><app:draft>yes</app:draft><app:review>pending</app:review></app:control>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        assert!(is_draft(&entry));
        set_draft(&mut entry, false);
        assert!(!is_draft(&entry));

        let out = entry_to_xml(&entry).expect("serialize");
        assert!(
            out.contains("<app:review>pending</app:review>"),
            "out: {out}"
        );
        assert!(out.contains("app:control"), "out: {out}");
        assert!(!out.contains("app:draft"), "out: {out}");
        // The surviving child still needs its prefix declared.
        assert!(
            out.contains(r#"xmlns:app="http://www.w3.org/2007/app""#),
            "out: {out}"
        );
    }

    #[test]
    fn setting_the_draft_flag_reuses_the_entrys_own_control_element() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>T</title>
  <app:control><app:review>pending</app:review></app:control>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, true);
        assert!(is_draft(&entry));

        let out = entry_to_xml(&entry).expect("serialize");
        assert_eq!(out.matches("app:control").count(), 2, "out: {out}"); // open + close
        assert!(
            out.contains("<app:review>pending</app:review>"),
            "out: {out}"
        );
    }

    #[test]
    fn a_marker_is_written_under_the_prefix_the_entry_already_binds() {
        // The namespace is what matters; the prefix spelling is the document's.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom"
       xmlns:pub="http://www.w3.org/2007/app"
       xmlns:jaunder="https://jaunder.org/ns/atompub">
  <title>T</title>
  <pub:control><pub:draft>no</pub:draft></pub:control>
  <jaunder:slug>old</jaunder:slug>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, true);
        set_j_slug(&mut entry, "new");

        assert!(is_draft(&entry));
        assert_eq!(j_slug(&entry), Some("new".to_string()));

        let out = entry_to_xml(&entry).expect("serialize");
        assert!(out.contains("<pub:draft>yes</pub:draft>"), "out: {out}");
        assert!(
            out.contains("<jaunder:slug>new</jaunder:slug>"),
            "out: {out}"
        );
        assert_eq!(out.matches("pub:control").count(), 2, "out: {out}"); // open + close
        assert_eq!(out.matches("jaunder:slug").count(), 2, "out: {out}"); // open + close
        assert!(!out.contains("xmlns:app="), "out: {out}");
        assert!(!out.contains("xmlns:j="), "out: {out}");
    }

    #[test]
    fn a_marker_never_takes_a_prefix_another_namespace_owns() {
        // Redeclaring `app` here would silently relabel `<app:thing>` as APP.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="https://example.com/ns/other">
  <title>T</title>
  <app:thing>x</app:thing>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, true);

        let out = entry_to_xml(&entry).expect("serialize");
        assert!(
            out.contains(r#"xmlns:app="https://example.com/ns/other""#),
            "out: {out}"
        );
        assert!(out.contains("<app:thing>x</app:thing>"), "out: {out}");
        // …and the draft flag still lands, under a prefix of its own.
        let parsed = out.parse::<Entry>().expect("re-parse");
        assert!(is_draft(&parsed), "out: {out}");
    }

    #[test]
    fn a_marker_skips_past_every_prefix_already_taken() {
        // The numbered fallback has to keep counting: `app1` is spoken for too.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom"
       xmlns:app="https://example.com/ns/other"
       xmlns:app1="https://example.com/ns/another">
  <title>T</title>
  <app:thing>x</app:thing>
  <app1:thing>y</app1:thing>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, true);

        let out = entry_to_xml(&entry).expect("serialize");
        assert!(
            out.contains(r#"xmlns:app2="http://www.w3.org/2007/app""#),
            "out: {out}"
        );
        assert!(out.contains("<app:thing>x</app:thing>"), "out: {out}");
        assert!(out.contains("<app1:thing>y</app1:thing>"), "out: {out}");
        let parsed = out.parse::<Entry>().expect("re-parse");
        assert!(is_draft(&parsed), "out: {out}");
    }

    #[test]
    fn setting_the_draft_flag_leaves_the_namespaces_other_extensions_alone() {
        // An APP-namespaced extension that simply isn't a control element.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:app="http://www.w3.org/2007/app">
  <title>T</title>
  <app:edited>2026-01-02T00:00:00Z</app:edited>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, true);
        assert!(is_draft(&entry));

        let out = entry_to_xml(&entry).expect("serialize");
        assert!(
            out.contains("<app:edited>2026-01-02T00:00:00Z</app:edited>"),
            "out: {out}"
        );
        assert_eq!(out.matches("app:control").count(), 2, "out: {out}"); // open + close
    }

    #[test]
    fn a_marker_declaring_its_namespace_on_its_own_element_is_ours() {
        // A client may scope the declaration to the marker rather than the entry.
        // Upstream keeps it as an attribute, which is the only record of what the
        // prefix meant — matching on `Entry::namespaces` alone would drop both of
        // these on the floor.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <app:control xmlns:app="http://www.w3.org/2007/app"><app:draft>yes</app:draft></app:control>
  <j:slug xmlns:j="https://jaunder.org/ns/atompub">theirs</j:slug>
</entry>"#;
        let entry = xml.parse::<Entry>().expect("parse");
        assert!(is_draft(&entry));
        assert_eq!(j_slug(&entry), Some("theirs".to_string()));
    }

    #[test]
    fn re_setting_an_element_scoped_marker_replaces_it_and_hoists_the_declaration() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom">
  <title>T</title>
  <app:control xmlns:app="http://www.w3.org/2007/app"><app:draft>yes</app:draft></app:control>
  <j:slug xmlns:j="https://jaunder.org/ns/atompub">old</j:slug>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, true);
        set_j_slug(&mut entry, "new");

        let out = entry_to_xml(&entry).expect("serialize");
        // One of each marker, and the prefixes now declared where the writer can emit
        // them — an element-scoped declaration comes back out as a plain attribute.
        assert_eq!(out.matches("app:control").count(), 2, "out: {out}"); // open + close
        assert_eq!(out.matches("j:slug").count(), 2, "out: {out}"); // open + close
        assert!(
            out.contains(r#"xmlns:app="http://www.w3.org/2007/app""#),
            "out: {out}"
        );
        assert!(
            out.contains(r#"xmlns:j="https://jaunder.org/ns/atompub""#),
            "out: {out}"
        );
        let parsed = out.parse::<Entry>().expect("re-parse");
        assert!(is_draft(&parsed), "out: {out}");
        assert_eq!(j_slug(&parsed), Some("new".to_string()));
    }

    #[test]
    fn the_draft_flag_lands_in_the_entrys_control_whichever_prefix_spells_it() {
        // Both prefixes are APP, but only one labels the entry's control element —
        // RFC 5023 §B gives an entry one `app:control`, not one per prefix.
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom"
       xmlns:a="http://www.w3.org/2007/app"
       xmlns:app="http://www.w3.org/2007/app">
  <title>T</title>
  <app:control><app:review>pending</app:review></app:control>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        set_draft(&mut entry, true);
        assert!(is_draft(&entry));

        let out = entry_to_xml(&entry).expect("serialize");
        assert_eq!(out.matches(":control").count(), 2, "out: {out}"); // open + close
        assert!(out.contains("<app:draft>yes</app:draft>"), "out: {out}");
    }

    #[test]
    fn a_slug_outside_the_jaunder_namespace_is_neither_read_nor_replaced() {
        let xml = r#"<entry xmlns="http://www.w3.org/2005/Atom" xmlns:j="https://example.com/ns/other">
  <title>T</title>
  <j:slug>theirs</j:slug>
</entry>"#;
        let mut entry = xml.parse::<Entry>().expect("parse");
        assert_eq!(j_slug(&entry), None);

        set_j_slug(&mut entry, "ours");
        assert_eq!(j_slug(&entry), Some("ours".to_string()));

        let out = entry_to_xml(&entry).expect("serialize");
        assert!(out.contains("<j:slug>theirs</j:slug>"), "out: {out}");
        assert!(
            out.contains(r#"xmlns:j="https://example.com/ns/other""#),
            "out: {out}"
        );
        let parsed = out.parse::<Entry>().expect("re-parse");
        assert_eq!(j_slug(&parsed), Some("ours".to_string()));
    }

    #[test]
    fn draft_entry_declares_the_app_namespace_and_clearing_removes_it() {
        let mut entry = sample_entry();
        set_draft(&mut entry, true);
        let out = entry_to_xml(&entry).expect("serialize");
        assert!(
            out.contains(r#"xmlns:app="http://www.w3.org/2007/app""#),
            "out: {out}"
        );

        set_draft(&mut entry, false);
        let out = entry_to_xml(&entry).expect("serialize");
        assert!(!out.contains("xmlns:app"), "out: {out}");
        assert!(!out.contains("app:draft"), "out: {out}");
    }
}
