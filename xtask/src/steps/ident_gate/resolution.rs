use std::collections::BTreeSet;

/// Every ident that can denote `owner` anywhere in the scanned tree.
///
/// A renaming re-export in one module (`pub use crate::render::RenderedHtml as Doc;`)
/// makes `Doc::from_trusted` in *another* module a site on the owner's door, so a gate
/// that resolved qualifiers per-file alone would miss it (#790).
///
/// Deliberately **over-approximates**: an ident lands here on a name match alone, so a
/// `type ContentType = RenderedHtml;` anywhere in policed code would pull genuine
/// `ContentType` sites into the population. That is the fail-closed direction — an
/// over-large owner set costs a marker, an under-large one loses an XSS door.
///
/// The harvest is only as wide as the caller's roots: a rename living outside them is
/// invisible, which is why a gate's roots must cover every tree it claims to police.
///
/// A `syn` parse failure is skipped rather than fatal. This is a widening pass, and
/// [`super::traversal::scan`] already hard-errors on an unparseable file, so a
/// second error path here would only duplicate that one.
pub(super) fn owner_aliases(sources: &[(String, String)], owner: &str) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    set.insert(owner.to_string());
    for (_, source) in sources {
        let Ok(file) = syn::parse_file(source) else {
            continue;
        };
        collect_owner_aliases_in(&file.items, owner, &mut set);
    }
    set
}

/// Harvest owner aliases from a list of items, **recursing into inline modules**.
///
/// Recursing is not tidiness, it closes a fail-open. Miss a
/// `mod inner { pub use …Owner as Doc; }` here and `Doc` never enters the owner set —
/// while the file that then writes `use crate::a::inner::Doc;` *does* bind `Doc`, so
/// [`Resolver::membership`] reads `Doc::from_trusted` as another type and suppresses a
/// real door. Widening this pass can only move sites into the population, so recursing is
/// the safe direction; recursing [`Resolver::for_file`] without this would be the unsafe
/// one.
fn collect_owner_aliases_in(items: &[syn::Item], owner: &str, set: &mut BTreeSet<String>) {
    for item in items {
        match item {
            syn::Item::Use(u) => collect_owner_renames(&u.tree, owner, set),
            syn::Item::Type(t) if type_name(&t.ty).is_some_and(|id| id == owner) => {
                set.insert(t.ident.to_string());
            }
            syn::Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    collect_owner_aliases_in(inner, owner, set);
                }
            }
            _ => {}
        }
    }
}

/// Whose door a policed site belongs to.
///
/// Named for the question it answers rather than for `Gate::owner`, which is a type
/// *name* — this is a verdict about one site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Membership {
    /// The qualifier denotes the gate's owner type — the real door.
    Door,
    /// The qualifier denotes some other, named type. Not this door; no marker owed.
    OtherType,
    /// The qualifier could not be determined, so the site stays in the population.
    ///
    /// This is what keeps resolution from failing open: obscuring a qualifier buys a
    /// gate failure, not an exemption (#790).
    Unknown,
}

/// One file's answer to "what type does this bare ident denote?".
///
/// Only the two things a syntactic pass can know: what the file imports, and what it
/// defines. Everything else is [`Membership::Unknown`].
pub(super) struct Resolver {
    /// Idents bound by a non-glob `use`, mapped to the final segment of their path.
    imported: BTreeSet<String>,
    /// Type names defined in this file — `struct`, `enum`, `union`, `type`.
    defined: BTreeSet<String>,
}

impl Resolver {
    /// Collect one file's `use` bindings and type definitions.
    pub(super) fn for_file(file: &syn::File) -> Self {
        let mut imported = BTreeSet::new();
        let mut defined = BTreeSet::new();
        for item in &file.items {
            match item {
                syn::Item::Use(u) => collect_bound_names(&u.tree, &mut imported),
                syn::Item::Struct(s) => {
                    defined.insert(s.ident.to_string());
                }
                syn::Item::Enum(e) => {
                    defined.insert(e.ident.to_string());
                }
                syn::Item::Union(u) => {
                    defined.insert(u.ident.to_string());
                }
                syn::Item::Type(t) => {
                    defined.insert(t.ident.to_string());
                }
                _ => {}
            }
        }
        Self { imported, defined }
    }

    /// Classify a path whose leaf is a policed ident.
    ///
    /// `impl_self` is the enclosing `impl`'s self-type name, so `Self::` resolves.
    ///
    /// The owner set is consulted **first**, so a renamed owner is recognised before any
    /// other reading of the same ident. Getting that order wrong is what would let a
    /// cross-file rename (`use …Owner as Doc;` elsewhere, `use crate::a::Doc;` here)
    /// resolve as another type and be suppressed.
    pub(super) fn membership(
        &self,
        path: &syn::Path,
        owners: &BTreeSet<String>,
        impl_self: Option<&str>,
    ) -> Membership {
        let segments: Vec<&syn::Ident> = path.segments.iter().map(|s| &s.ident).collect();
        // The leaf is the policed ident; the segment before it names the type.
        let Some(qualifier) = segments.len().checked_sub(2).map(|i| segments[i]) else {
            // A single-segment path is an unqualified call — nothing to resolve.
            return Membership::Unknown;
        };
        let name = qualifier.to_string();
        if owners.contains(&name) {
            return Membership::Door;
        }
        if name == "Self" {
            return match impl_self {
                Some(ty) if owners.contains(ty) => Membership::Door,
                Some(_) => Membership::OtherType,
                None => Membership::Unknown,
            };
        }
        // A multi-segment path spells the type out, so it resolves by construction.
        if segments.len() > 2 || self.imported.contains(&name) || self.defined.contains(&name) {
            return Membership::OtherType;
        }
        Membership::Unknown
    }
}

/// Every ident a non-glob `use` tree brings into scope, by the name it is bound to.
fn collect_bound_names(tree: &syn::UseTree, out: &mut BTreeSet<String>) {
    match tree {
        syn::UseTree::Path(p) => collect_bound_names(&p.tree, out),
        syn::UseTree::Group(g) => {
            for t in &g.items {
                collect_bound_names(t, out);
            }
        }
        syn::UseTree::Name(n) => {
            out.insert(n.ident.to_string());
        }
        syn::UseTree::Rename(r) => {
            out.insert(r.rename.to_string());
        }
        syn::UseTree::Glob(_) => {}
    }
}

/// Walk a `use` tree, recording the new name of any `… as X` that renames `owner`.
fn collect_owner_renames(tree: &syn::UseTree, owner: &str, set: &mut BTreeSet<String>) {
    match tree {
        syn::UseTree::Path(p) => collect_owner_renames(&p.tree, owner, set),
        syn::UseTree::Group(g) => {
            for t in &g.items {
                collect_owner_renames(t, owner, set);
            }
        }
        syn::UseTree::Rename(r) if r.ident == owner => {
            set.insert(r.rename.to_string());
        }
        syn::UseTree::Rename(_) | syn::UseTree::Name(_) | syn::UseTree::Glob(_) => {}
    }
}

/// The final path segment of a type, when it is a plain path — the type's own name.
pub(super) fn type_name(ty: &syn::Type) -> Option<&syn::Ident> {
    match ty {
        syn::Type::Path(p) => p.path.segments.last().map(|s| &s.ident),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::owner_aliases;

    fn src(text: &str) -> (String, String) {
        ("a.rs".to_string(), text.to_string())
    }

    #[test]
    fn the_owner_is_always_in_its_own_alias_set() {
        let set = owner_aliases(&[], "Owner");
        assert_eq!(
            set.len(),
            1,
            "an empty tree yields the owner alone: {set:?}"
        );
        assert!(set.contains("Owner"));
    }

    #[test]
    fn a_renaming_use_of_the_owner_contributes_its_new_name() {
        let set = owner_aliases(&[src("use crate::render::Owner as Doc;\n")], "Owner");
        assert!(set.contains("Doc"), "a renamed import can denote the owner");
    }

    #[test]
    fn a_type_alias_to_the_owner_contributes_its_name() {
        assert!(owner_aliases(&[src("type Html = Owner;\n")], "Owner").contains("Html"));
    }

    #[test]
    fn a_nested_use_group_still_yields_the_rename() {
        let set = owner_aliases(
            &[src("use crate::render::{Sanitizer, Owner as Doc};\n")],
            "Owner",
        );
        assert!(set.contains("Doc"));
    }

    #[test]
    fn unrelated_renames_and_aliases_are_ignored() {
        let set = owner_aliases(
            &[src(
                "use crate::media::ContentType as Ct;\ntype Bytes = Vec<u8>;\n",
            )],
            "Owner",
        );
        assert_eq!(set.len(), 1, "only the owner itself: {set:?}");
    }

    #[test]
    fn a_plain_non_renaming_import_of_the_owner_adds_nothing() {
        let set = owner_aliases(&[src("use crate::render::Owner;\n")], "Owner");
        assert_eq!(set.len(), 1, "already the owner's own name: {set:?}");
    }

    #[test]
    fn the_harvest_spans_files_and_is_order_independent() {
        let a = (
            "a.rs".to_string(),
            "use crate::render::Owner as Doc;\n".to_string(),
        );
        let b = ("b.rs".to_string(), "type Html = Owner;\n".to_string());
        let forward = owner_aliases(&[a.clone(), b.clone()], "Owner");
        let backward = owner_aliases(&[b, a], "Owner");
        assert_eq!(forward, backward);
        assert!(forward.contains("Doc") && forward.contains("Html"));
    }

    #[test]
    fn an_unparseable_file_is_skipped_rather_than_panicking() {
        assert_eq!(owner_aliases(&[src("fn (((")], "Owner").len(), 1);
    }

    /// A rename inside an inline module must be harvested. Missing it is fail-**open**:
    /// the importing file binds the alias, so the resolver would read the door as another
    /// type and suppress it. Found by the whole-branch standards review on #790.
    #[test]
    fn a_rename_inside_an_inline_module_is_harvested() {
        let set = owner_aliases(
            &[src("mod inner { pub use crate::render::Owner as Doc; }\n")],
            "Owner",
        );
        assert!(
            set.contains("Doc"),
            "nested renames must widen the set: {set:?}"
        );
    }

    #[test]
    fn a_type_alias_inside_an_inline_module_is_harvested() {
        let set = owner_aliases(&[src("mod inner { pub type Html = Owner; }\n")], "Owner");
        assert!(set.contains("Html"), "{set:?}");
    }

    #[test]
    fn nesting_is_harvested_to_any_depth() {
        let set = owner_aliases(
            &[src(
                "mod a { mod b { pub use crate::render::Owner as Deep; } }\n",
            )],
            "Owner",
        );
        assert!(set.contains("Deep"), "{set:?}");
    }
}

/// Qualifier resolution (#790): the rule is "prove this is not the owner's door, or
/// leave it in the population", so every branch that returns [`Membership::Unknown`]
/// matters as much as the ones that resolve.
#[cfg(test)]
mod resolver_tests {
    use std::collections::BTreeSet;

    use super::{Membership, Resolver};

    /// The first path in `file` whose **last** segment is `leaf`, in visit order.
    ///
    /// Returns a single-segment path for an unqualified call — "unqualified" is a verdict
    /// the resolver produces, not an absence. `use` items are skipped, or a fixture's own
    /// import would be found before its call site.
    fn first_policed_path(file: &syn::File, leaf: &str) -> Option<syn::Path> {
        struct Find<'a> {
            leaf: &'a str,
            found: Option<syn::Path>,
        }
        impl<'ast> syn::visit::Visit<'ast> for Find<'_> {
            fn visit_item_use(&mut self, _: &'ast syn::ItemUse) {}
            fn visit_path(&mut self, p: &'ast syn::Path) {
                if self.found.is_none() && p.segments.last().is_some_and(|s| s.ident == self.leaf) {
                    self.found = Some(p.clone());
                }
                syn::visit::visit_path(self, p);
            }
        }
        let mut find = Find { leaf, found: None };
        syn::visit::visit_file(&mut find, file);
        find.found
    }

    fn resolve(src: &str, owners: &[&str], impl_self: Option<&str>) -> Membership {
        let file: syn::File = syn::parse_str(src).expect("fixture parses");
        let set: BTreeSet<String> = owners.iter().map(|s| (*s).to_string()).collect();
        let path = first_policed_path(&file, "from_trusted").expect("fixture has a site");
        Resolver::for_file(&file).membership(&path, &set, impl_self)
    }

    #[test]
    fn a_bare_owner_qualifier_is_the_door() {
        let src = "fn f() { Owner::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::Door);
    }

    #[test]
    fn a_renamed_owner_qualifier_is_the_door() {
        // The #778 hole, closed by resolution rather than by over-approximation.
        let src = "use crate::render::Owner as Doc;\nfn f() { Doc::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner", "Doc"], None), Membership::Door);
    }

    #[test]
    fn a_fully_qualified_owner_path_is_still_the_door() {
        // Fails OPEN if ">2 segments" is read as "not the door".
        let src = "fn f() { crate::render::Owner::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::Door);
    }

    #[test]
    fn a_multi_segment_path_names_its_type_and_needs_no_import() {
        let src = "fn f() { crate::media::ContentType::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::OtherType);
    }

    #[test]
    fn self_inside_the_owners_impl_is_the_door() {
        let src = "fn f() { Self::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], Some("Owner")), Membership::Door);
    }

    #[test]
    fn self_inside_another_impl_is_not_the_door() {
        let src = "fn f() { Self::from_trusted(x); }\n";
        assert_eq!(
            resolve(src, &["Owner"], Some("ContentType")),
            Membership::OtherType
        );
    }

    #[test]
    fn self_with_no_enclosing_impl_is_unknown() {
        let src = "fn f() { Self::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::Unknown);
    }

    #[test]
    fn a_qualifier_defined_in_this_file_resolves_to_itself() {
        let src = "struct ContentType(String);\nfn f() { ContentType::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::OtherType);
    }

    #[test]
    fn a_qualifier_imported_by_a_flat_use_resolves() {
        let src = "use crate::media::ContentType;\nfn f() { ContentType::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::OtherType);
    }

    #[test]
    fn a_qualifier_imported_by_a_nested_use_group_resolves() {
        // The form `common/src/feed/feed_path.rs:7` actually uses.
        let src = "use crate::{media::ContentType, tag::Tag};\nfn f() { ContentType::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::OtherType);
    }

    #[test]
    fn an_in_file_type_alias_resolves_without_the_owner_set() {
        // The alias is NOT seeded into `owners`, so this exercises the in-file branch
        // rather than short-circuiting on the owner set.
        let src = "type Ct = ContentType;\nfn f() { Ct::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::OtherType);
    }

    #[test]
    fn an_unbound_bare_qualifier_is_unknown() {
        let src = "use foo::*;\nfn f() { Mystery::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::Unknown);
    }

    #[test]
    fn a_generic_parameter_qualifier_is_unknown() {
        let src = "fn f<T>() { T::from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::Unknown);
    }

    #[test]
    fn an_unqualified_call_is_unknown() {
        let src = "fn f() { from_trusted(x); }\n";
        assert_eq!(resolve(src, &["Owner"], None), Membership::Unknown);
    }
}
