use super::macros_audit::{BRIDGE_ATTRIBUTES, BRIDGE_DERIVES};
use super::scan::render;

/// Roots scanned for *declarations*, to build the approve-set. Wider than
/// [`POLICED_ROOT`]: the domain types a `storage` decode targets are declared in `common`.
///
/// A missing or unparseable file here fails the gate, exactly as under the policed root —
/// the approve-set is what makes a decode legal, so a shrunken one is a gate that has
/// quietly changed its own rule.
/// `host/src` is here because the *validated* halves of a few split newtypes live there,
/// server-only and serde-free — `host::invite::InviteCode` is the one storage decodes
/// today, and it is exactly the type `common`'s wasm-facing `ProfferedInviteCode` is not.
/// Missing it would have cost a spurious allowlist entry for a properly typed decode.
pub(super) const DECLARATION_ROOTS: &[&str] = &["common/src", "host/src", "storage/src"];

/// Generic containers walked *through* to reach leaves. Anything else is a leaf that must
/// itself be approved, so an unrecognised wrapper fails closed.
const CONTAINERS: &[&str] = &["Vec", "Option", "Box", "Cow", "Arc", "Rc"];

/// Foreign types that are legitimate column targets but are declared outside this repo, so
/// no declaration scan can find them.
///
/// The only hand-maintained part of the approve-set, and small precisely because the ~35
/// domain types derive automatically. Each entry is a deliberate statement that decoding a
pub(super) const APPROVED_FOREIGN: &[(&str, &str)] = &[(
    "DateTime",
    "chrono timestamps — the correct target for temporal columns whose surrounding row \
     shape already carries any needed role identity",
)];

/// The types a decode may land in: domain types with a bridge, plus derived composites whose
/// fields or tuple elements this gate polices separately.
#[derive(Default)]
pub(super) struct ApproveSet {
    pub(super) approved: std::collections::HashSet<String>,
    /// `#[derive(FromRow)]` structs, tuple aliases, and narrowly-proven handwritten
    /// `sqlx::FromRow` composites declared under a scanned root.
    ///
    /// Derived fields and the direct typed gets in a proven handwritten decoder are
    /// policed independently. Other handwritten implementations require exact
    /// allowlist entries.
    composites: std::collections::HashSet<String>,
    /// Type aliases, mapping the alias ident to the last path segment of its target
    /// (`HubUrl` → `TaggedUrl`).
    ///
    /// Not an approval of its own: a leaf is resolved through this map once and then has to
    /// meet the same bar. It exists because a generic newtype carries the bridge on the
    /// generic type, while every decode names a role alias (#875) — and because the aliases
    /// are declared in `common/src` while the decodes they name sit in `storage/src`, this
    /// is collected under **every** declaration root, not the policed one alone.
    aliases: std::collections::HashMap<String, String>,
}

/// Whether `attrs` carry a bridge-emitting macro.
///
/// Two shapes, and the rule differs between them (#746). A **derive** from
/// [`BRIDGE_DERIVES`] approves on its presence alone: whether it actually emits the bridge
/// depends on the type's own options (`no_sqlx`, `secret`), which is not a static property
/// — so this over-approves, harmlessly, since the compiler rejects a decode into a type
/// with no `Decode` impl. A **`#[text_enum]` attribute** carries an explicit `sqlx` flag,
/// so the gate reads it and is exact: `TargetKind` has it, `Channel` does not.
fn has_bridge(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        let last = a.path().segments.last().map(|s| s.ident.to_string());
        match last.as_deref() {
            Some("derive") => a.meta.require_list().is_ok_and(|l| {
                l.tokens.clone().into_iter().any(|t| match t {
                    proc_macro2::TokenTree::Ident(i) => {
                        BRIDGE_DERIVES.contains(&i.to_string().as_str())
                    }
                    _ => false,
                })
            }),
            Some(name) if BRIDGE_ATTRIBUTES.contains(&name) => {
                a.meta.require_list().is_ok_and(|l| {
                    l.tokens
                        .clone()
                        .into_iter()
                        .any(|t| matches!(&t, proc_macro2::TokenTree::Ident(i) if i == "sqlx"))
                })
            }
            _ => false,
        }
    })
}

/// Which kind of root a scanned declaration file sits under.
///
/// Named rather than a `bool` because the two call sites read very differently: at the
/// scan it is derived from the root, but in a test `collect_declarations(src, true, …)`
/// says nothing about *what* is true.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Root {
    /// Under [`POLICED_ROOT`] — decode sites here are checked, so composites declared here
    /// can be approved by delegation.
    Policed,
    /// Scanned for declarations only. Bridge-carrying types still count; composites do
    /// not, because nothing here polices their fields.
    DeclarationsOnly,
}

/// Whether `attrs` derive `sqlx::FromRow`.
pub(super) fn is_from_row(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|a| a.path().is_ident("derive") && render(&a.meta).contains("FromRow"))
}

/// Whether attributes contain only non-transforming documentation.
fn has_only_doc_attrs(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().all(|attr| attr.path().is_ident("doc"))
}

/// Whether `path` is written as the unaliased `sqlx::FromRow` trait.
fn is_sqlx_from_row(path: &syn::Path) -> bool {
    path.leading_colon.is_none()
        && path.segments.len() == 2
        && path.segments.first().is_some_and(|s| s.ident == "sqlx")
        && path.segments.last().is_some_and(|s| s.ident == "FromRow")
}

/// Whether `expr` is exactly the `row` parameter, rather than an alias or a qualified path.
fn is_row(expr: &syn::Expr) -> bool {
    matches!(
        expr,
        syn::Expr::Path(p) if p.qself.is_none() && p.path.is_ident("row")
    )
}

/// Whether `ty` contains an inferred type anywhere within its syntax.
fn has_inferred_type(ty: &syn::Type) -> bool {
    struct Finder(bool);

    impl<'ast> syn::visit::Visit<'ast> for Finder {
        fn visit_type_infer(&mut self, _: &'ast syn::TypeInfer) {
            self.0 = true;
        }
    }

    let mut finder = Finder(false);
    syn::visit::Visit::visit_type(&mut finder, ty);
    finder.0
}

/// Whether `expr` is the sole direct row access that a proven decoder may make.
fn is_direct_typed_row_get(expr: &syn::Expr) -> bool {
    let syn::Expr::Try(try_expr) = expr else {
        return false;
    };
    let syn::Expr::MethodCall(call) = &*try_expr.expr else {
        return false;
    };
    if call.method != "try_get" || !is_row(&call.receiver) || call.args.len() != 1 {
        return false;
    }
    let Some(turbofish) = &call.turbofish else {
        return false;
    };
    if turbofish.args.len() != 2 {
        return false;
    }
    let mut args = turbofish.args.iter();
    matches!(
        (args.next(), args.next()),
        (
            Some(syn::GenericArgument::Type(ty)),
            Some(syn::GenericArgument::Type(syn::Type::Infer(_)))
        ) if !has_inferred_type(ty)
    )
}

struct FlatFromRowProof {
    valid: bool,
    row_uses: usize,
}

impl FlatFromRowProof {
    fn new() -> Self {
        Self {
            valid: true,
            row_uses: 0,
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for FlatFromRowProof {
    fn visit_expr_path(&mut self, i: &'ast syn::ExprPath) {
        if i.qself.is_none() && i.path.is_ident("row") {
            self.row_uses += 1;
        }
        syn::visit::visit_expr_path(self, i);
    }

    fn visit_pat_ident(&mut self, i: &'ast syn::PatIdent) {
        if i.ident == "row" {
            self.valid = false;
        }
        syn::visit::visit_pat_ident(self, i);
    }

    fn visit_macro(&mut self, _: &'ast syn::Macro) {
        self.valid = false;
    }

    fn visit_item(&mut self, _: &'ast syn::Item) {
        self.valid = false;
    }

    fn visit_expr_block(&mut self, _: &'ast syn::ExprBlock) {
        self.valid = false;
    }

    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {
        self.valid = false;
    }

    fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {
        self.valid = false;
    }

    fn visit_expr_const(&mut self, _: &'ast syn::ExprConst) {
        self.valid = false;
    }

    fn visit_expr_unsafe(&mut self, _: &'ast syn::ExprUnsafe) {
        self.valid = false;
    }

    fn visit_expr_if(&mut self, _: &'ast syn::ExprIf) {
        self.valid = false;
    }

    fn visit_expr_match(&mut self, _: &'ast syn::ExprMatch) {
        self.valid = false;
    }

    fn visit_expr_loop(&mut self, _: &'ast syn::ExprLoop) {
        self.valid = false;
    }

    fn visit_expr_for_loop(&mut self, _: &'ast syn::ExprForLoop) {
        self.valid = false;
    }

    fn visit_expr_while(&mut self, _: &'ast syn::ExprWhile) {
        self.valid = false;
    }

    fn visit_expr_try_block(&mut self, _: &'ast syn::ExprTryBlock) {
        self.valid = false;
    }

    fn visit_expr_method_call(&mut self, i: &'ast syn::ExprMethodCall) {
        if i.method == "from_row" {
            self.valid = false;
        }
        syn::visit::visit_expr_method_call(self, i);
    }

    fn visit_expr_call(&mut self, i: &'ast syn::ExprCall) {
        if is_from_row_call_path(&i.func) {
            self.valid = false;
        }
        syn::visit::visit_expr_call(self, i);
    }
}

/// Whether an invoked path ends in `from_row`, including a qualified UFCS path.
fn is_from_row_call_path(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Path(path) => path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "from_row"),
        syn::Expr::Paren(paren) => is_from_row_call_path(&paren.expr),
        syn::Expr::Group(group) => is_from_row_call_path(&group.expr),
        _ => false,
    }
}

/// Whether `expr` is the only terminating expression allowed in a proven decoder.
fn is_ok_self(expr: &syn::Expr) -> bool {
    matches!(
        expr,
        syn::Expr::Call(call)
            if matches!(&*call.func, syn::Expr::Path(p) if p.qself.is_none() && p.path.is_ident("Ok"))
                && call.args.len() == 1
                && matches!(
                    call.args.first(),
                    Some(syn::Expr::Struct(record))
                        if record.qself.is_none()
                            && record.path.is_ident("Self")
                            && record.rest.is_none()
                )
    )
}

/// Proves the narrow handwritten `FromRow` grammar used to delegate a composite target.
///
/// The body is a flat sequence of `let` statements followed by `Ok(Self { ... })`. A local
/// that reads the `row` parameter must be exactly one typed `try_get` with one column index;
/// other locals may transform prior values, such as PostRecord's tags JSON parser.
fn from_row_has_flat_direct_gets(method: &syn::ImplItemFn) -> bool {
    if !has_only_doc_attrs(&method.attrs) {
        return false;
    }

    if method.sig.inputs.len() != 1
        || !matches!(
            method.sig.inputs.first(),
            Some(syn::FnArg::Typed(argument))
                if matches!(&*argument.pat, syn::Pat::Ident(pat) if pat.ident == "row")
        )
    {
        return false;
    }

    let Some((last, locals)) = method.block.stmts.split_last() else {
        return false;
    };
    if !matches!(last, syn::Stmt::Expr(expr, None) if is_ok_self(expr)) {
        return false;
    }

    for stmt in locals {
        let syn::Stmt::Local(local) = stmt else {
            return false;
        };
        if !has_only_doc_attrs(&local.attrs) {
            return false;
        }

        let Some(init) = &local.init else {
            return false;
        };
        if init.diverge.is_some() {
            return false;
        }
        let mut proof = FlatFromRowProof::new();
        syn::visit::Visit::visit_expr(&mut proof, &init.expr);
        if !proof.valid
            || (proof.row_uses == 1 && !is_direct_typed_row_get(&init.expr))
            || proof.row_uses > 1
        {
            return false;
        }
        syn::visit::Visit::visit_pat(&mut proof, &local.pat);
        if !proof.valid {
            return false;
        }
    }

    let syn::Stmt::Expr(result, None) = last else {
        return false;
    };
    let mut proof = FlatFromRowProof::new();
    syn::visit::Visit::visit_expr(&mut proof, result);
    proof.valid && proof.row_uses == 0
}

/// The simple self-type name for a handwritten `sqlx::FromRow` implementation.
///
/// The declaration scan deliberately groups every matching implementation under this name:
/// mutually exclusive `cfg` branches still share a target, so one proven branch must not approve
/// a sibling the proof cannot read.
fn handwritten_from_row_name(item: &syn::ItemImpl) -> Option<&syn::Ident> {
    let Some((_, trait_path, _)) = &item.trait_ else {
        return None;
    };
    if !is_sqlx_from_row(trait_path) {
        return None;
    }
    let syn::Type::Path(self_type) = &*item.self_ty else {
        return None;
    };
    if self_type.qself.is_some() || self_type.path.segments.len() != 1 {
        return None;
    }
    self_type.path.get_ident()
}

/// Whether `item` carries a narrowly-proven handwritten `sqlx::FromRow`.
fn handwritten_from_row_proves(item: &syn::ItemImpl) -> bool {
    if !has_only_doc_attrs(&item.attrs) {
        return false;
    }
    let mut methods = item.items.iter().filter_map(|item| match item {
        syn::ImplItem::Fn(method) if method.sig.ident == "from_row" => Some(method),
        _ => None,
    });
    let Some(method) = methods.next() else {
        return false;
    };
    methods.next().is_none() && from_row_has_flat_direct_gets(method)
}

/// `root` gates composites: bridge-carrying types are approved wherever declared because the
/// bridge is the whole claim. Derived `FromRow` structs and tuple aliases are approved only
/// under the policed root, where their fields or elements are checked. A hand-written
/// `sqlx::FromRow` implementation is approved only when every matching implementation for its
/// simple self type proves the same direct column boundary syntactically. Every other
/// hand-written implementation needs an exact allowlist entry.
///
/// Only top-level `file.items` are read: a declaration inside an inline `mod` is not seen.
/// That direction is safe — an unseen declaration is an unapproved type, so the gate bites
/// rather than waving something through — but it is a boundary, not an oversight.
pub(super) fn collect_declarations(
    source: &str,
    root: Root,
    set: &mut ApproveSet,
) -> Result<(), String> {
    let policed = root == Root::Policed;
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut handwritten = std::collections::HashMap::<String, bool>::new();
    for item in &file.items {
        match item {
            syn::Item::Struct(s) if has_bridge(&s.attrs) => {
                set.approved.insert(s.ident.to_string());
            }
            syn::Item::Enum(e) if has_bridge(&e.attrs) => {
                set.approved.insert(e.ident.to_string());
            }
            syn::Item::Struct(s) if policed && is_from_row(&s.attrs) => {
                set.composites.insert(s.ident.to_string());
            }
            syn::Item::Impl(i) if policed => {
                if let Some(name) = handwritten_from_row_name(i) {
                    let proves = handwritten_from_row_proves(i);
                    handwritten
                        .entry(name.to_string())
                        .and_modify(|all_prove| *all_prove &= proves)
                        .or_insert(proves);
                }
            }
            syn::Item::Type(t) if policed && matches!(&*t.ty, syn::Type::Tuple(_)) => {
                set.composites.insert(t.ident.to_string());
            }
            syn::Item::Type(t) => {
                if let syn::Type::Path(p) = &*t.ty
                    && let Some(last) = p.path.segments.last()
                {
                    set.aliases
                        .insert(t.ident.to_string(), last.ident.to_string());
                }
            }
            _ => {}
        }
    }
    for (name, all_prove) in handwritten {
        if all_prove {
            set.composites.insert(name);
        } else {
            set.composites.remove(&name);
        }
    }
    Ok(())
}

/// Every leaf of `ty` that is not an approved column type — empty when the decode is fine.
///
/// `Result<T, E>` recurses into `T` **only**: the error arm is never decoded from a column,
/// so asking `BackupError` to be an approved column type would be nonsense.
pub(super) fn unapproved_leaves(ty: &syn::Type, set: &ApproveSet) -> Vec<String> {
    match ty {
        syn::Type::Path(p) => {
            let Some(last) = p.path.segments.last() else {
                return Vec::new();
            };
            let name = last.ident.to_string();
            let args: Vec<&syn::Type> = match &last.arguments {
                syn::PathArguments::AngleBracketed(ab) => ab
                    .args
                    .iter()
                    .filter_map(|a| match a {
                        syn::GenericArgument::Type(t) => Some(t),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            if name == "Result" {
                return args
                    .first()
                    .map(|t| unapproved_leaves(t, set))
                    .unwrap_or_default();
            }
            if CONTAINERS.contains(&name.as_str()) {
                return args
                    .iter()
                    .flat_map(|t| unapproved_leaves(t, set))
                    .collect();
            }
            if set.approved.contains(&name) || set.composites.contains(&name) {
                return Vec::new();
            }
            // One hop through the alias map, then the same bar again. Reported under the
            // resolved name, because that is the type a fix has to put a bridge on.
            let resolved = set.aliases.get(&name).cloned().unwrap_or(name);
            if set.approved.contains(&resolved) || set.composites.contains(&resolved) {
                Vec::new()
            } else {
                vec![resolved]
            }
        }
        syn::Type::Tuple(t) => t
            .elems
            .iter()
            .flat_map(|e| unapproved_leaves(e, set))
            .collect(),
        syn::Type::Reference(r) => unapproved_leaves(&r.elem, set),
        syn::Type::Paren(p) => unapproved_leaves(&p.elem, set),
        syn::Type::Group(g) => unapproved_leaves(&g.elem, set),
        syn::Type::Slice(s) => unapproved_leaves(&s.elem, set),
        syn::Type::Array(a) => unapproved_leaves(&a.elem, set),
        _ => Vec::new(),
    }
}

/// A synthetic approve-set, so the pure tests never touch the filesystem.
///
/// The names here stand in for what the real declaration scan finds: `Slug`/`PostId`
/// for bridge-carrying domain types, `DateTime` for [`APPROVED_FOREIGN`], and
/// `PostRow`/`FeedCacheRowRecord` for composites approved by delegation.
#[cfg(test)]
pub(super) fn approve() -> ApproveSet {
    let names = |ns: &[&str]| ns.iter().map(|s| (*s).to_string()).collect();
    ApproveSet {
        approved: names(&[
            "Slug",
            "PostId",
            "UserId",
            "AudienceId",
            "Email",
            "FeedPath",
            "TagId",
            "Tag",
            "DateTime",
        ]),
        composites: names(&["PostRow", "FeedCacheRowRecord"]),
        aliases: std::collections::HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::scan::{decodes, targets};
    use super::*;

    /// Parses a synthetic handwritten decoder and returns composites approved by delegation.
    fn handwritten_composites(src: &str) -> std::collections::HashSet<String> {
        let mut set = ApproveSet::default();
        collect_declarations(src, Root::Policed, &mut set).expect("parses");
        set.composites
    }

    /// A `PostRecord` handwritten decoder with caller-provided locals and final statement.
    fn post_record_decoder(locals: &str, final_statement: &str) -> String {
        format!(
            r#"
                impl<'r, R> sqlx::FromRow<'r, R> for PostRecord {{
                    fn from_row(row: &'r R) -> sqlx::Result<Self> {{
                        {locals}
                        {final_statement}
                    }}
                }}
            "#
        )
    }

    #[test]
    fn handwritten_from_row_with_tags_parser_is_approved_and_its_gets_stay_policed() {
        let src = post_record_decoder(
            r#"
                let post_id = row.try_get::<PostId, _>("post_id")?;
                let tags_json = row.try_get::<String, _>("tags")?;
                let tags = parse_post_tags_json(&tags_json, post_id)?;
            "#,
            "Ok(Self { post_id, tags })",
        );
        let mut approved = approve();
        collect_declarations(&src, Root::Policed, &mut approved).expect("parses");
        assert!(approved.composites.contains("PostRecord"));

        let query = format!(
            r#"{src} fn fetch() {{ sqlx::query_as::<_, PostRecord>("SELECT *").fetch_one(p); }}"#
        );
        let targets: Vec<String> = decodes(&query, &approved)
            .expect("parses")
            .sites
            .into_iter()
            .map(|site| site.target)
            .collect();
        assert_eq!(
            targets,
            vec!["String"],
            "the PostRecord query target is delegated, but the aggregate payload remains policed"
        );
    }

    #[test]
    fn handwritten_from_row_rejects_a_row_alias() {
        let src = post_record_decoder(
            r#"
                let row_alias = row;
                let post_id = row_alias.try_get::<PostId, _>("post_id")?;
            "#,
            "Ok(Self { post_id })",
        );
        assert!(!handwritten_composites(&src).contains("PostRecord"));
    }

    #[test]
    fn handwritten_from_row_rejects_shadowing_and_nested_scopes() {
        let shadowed = post_record_decoder(
            r#"
                let row = another_row;
                let post_id = row.try_get::<PostId, _>("post_id")?;
            "#,
            "Ok(Self { post_id })",
        );
        assert!(!handwritten_composites(&shadowed).contains("PostRecord"));

        let nested = post_record_decoder(
            r#"let post_id = { row.try_get::<PostId, _>("post_id")? };"#,
            "Ok(Self { post_id })",
        );
        assert!(!handwritten_composites(&nested).contains("PostRecord"));
    }

    #[test]
    fn handwritten_from_row_rejects_ufcs_and_helper_row_flow() {
        let ufcs = post_record_decoder(
            r#"let post_id = Row::try_get::<PostId, _>(row, "post_id")?;"#,
            "Ok(Self { post_id })",
        );
        assert!(!handwritten_composites(&ufcs).contains("PostRecord"));

        let helper = post_record_decoder(
            r#"let post_id = decode_post_id("post_id", row)?;"#,
            "Ok(Self { post_id })",
        );
        assert!(!handwritten_composites(&helper).contains("PostRecord"));
    }

    #[test]
    fn handwritten_from_row_rejects_untyped_inferred_and_raw_access() {
        for locals in [
            r#"let post_id = row.try_get("post_id")?;"#,
            r#"let post_id = row.try_get::<_, _>("post_id")?;"#,
            r#"let post_id = row.get::<PostId, _>("post_id")?;"#,
        ] {
            let src = post_record_decoder(locals, "Ok(Self { post_id })");
            assert!(
                !handwritten_composites(&src).contains("PostRecord"),
                "must reject {locals}"
            );
        }
    }

    #[test]
    fn handwritten_from_row_rejects_from_row_delegation_and_nonflat_bodies() {
        let delegated = post_record_decoder(
            r#"let post_id = Other::from_row(&other_row)?;"#,
            "Ok(Self { post_id })",
        );
        assert!(!handwritten_composites(&delegated).contains("PostRecord"));

        let qualified = post_record_decoder(
            r#"let post_id = <Other as sqlx::FromRow>::from_row(&other_row)?;"#,
            "Ok(Self { post_id })",
        );
        assert!(!handwritten_composites(&qualified).contains("PostRecord"));

        let nonflat = post_record_decoder(
            r#"
                let post_id = row.try_get::<PostId, _>("post_id")?;
                if true {}
            "#,
            "Ok(Self { post_id })",
        );
        assert!(!handwritten_composites(&nonflat).contains("PostRecord"));
    }

    #[test]
    fn handwritten_from_row_requires_every_cfg_style_implementation_to_prove() {
        // These duplicate impls model mutually exclusive cfg branches. The parser sees both,
        // but this synthetic source is not compiled, so the test can show aggregation itself
        // instead of letting cfg attributes independently reject the proof.
        let proven = post_record_decoder(
            r#"let post_id = row.try_get::<PostId, _>("post_id")?;"#,
            "Ok(Self { post_id })",
        );
        let all_proven = format!("{proven}\n{proven}");
        assert!(handwritten_composites(&all_proven).contains("PostRecord"));

        let unproven = post_record_decoder(
            r#"let post_id = row.try_get("post_id")?;"#,
            "Ok(Self { post_id })",
        );
        let unproven_sibling = format!("{proven}\n{unproven}");
        assert!(!handwritten_composites(&unproven_sibling).contains("PostRecord"));
    }

    #[test]
    fn handwritten_from_row_rejects_attributes_on_proven_decoder_nodes() {
        let attributed_impl = r#"
            #[decoder_transform]
            impl<'r, R> sqlx::FromRow<'r, R> for PostRecord {
                fn from_row(row: &'r R) -> sqlx::Result<Self> {
                    let post_id = row.try_get::<PostId, _>("post_id")?;
                    Ok(Self { post_id })
                }
            }
        "#;
        let attributed_method = r#"
            impl<'r, R> sqlx::FromRow<'r, R> for PostRecord {
                #[decoder_transform]
                fn from_row(row: &'r R) -> sqlx::Result<Self> {
                    let post_id = row.try_get::<PostId, _>("post_id")?;
                    Ok(Self { post_id })
                }
            }
        "#;
        let attributed_local = post_record_decoder(
            r#"
                #[decoder_transform]
                let post_id = row.try_get::<PostId, _>("post_id")?;
            "#,
            "Ok(Self { post_id })",
        );

        for source in [
            attributed_impl,
            attributed_method,
            attributed_local.as_str(),
        ] {
            assert!(!handwritten_composites(source).contains("PostRecord"));
        }
    }

    // ---- declared decode targets ----

    #[test]
    fn a_typed_decode_is_not_collected() {
        let src = r#"fn f() { sqlx::query_scalar::<_, PostId>("SELECT post_id").fetch_one(p); }"#;
        assert!(targets(src).is_empty());
    }

    #[test]
    fn every_unapproved_target_is_collected_with_no_special_casing() {
        // `bool`/`String` targets are in population and need a written reason like
        // anything else — leaving them invisible and recorded nowhere is the defect
        // #728 exists to close.
        //
        // Note there is no primitive list anywhere in the rule: `bool`, `String`, `i64`,
        // `u32`, `char` and `Uuid` fail for one reason — nothing approved them.
        for target in [
            "bool",
            "String",
            "i64",
            "u32",
            "char",
            "f64",
            "Uuid",
            "NaiveDate",
        ] {
            let src = format!(
                r#"fn f() {{ sqlx::query_scalar::<_, {target}>("SELECT c").fetch_one(p); }}"#
            );
            assert_eq!(targets(&src), vec![target.to_string()], "for {target}");
        }
    }

    #[test]
    fn slice_and_array_leaves_are_reached() {
        // `Type::Slice`/`Type::Array` were absent from #715's recursion, so a `&[u8]`
        // target slipped through even though `u8` is no more approved than `i64`.
        for target in ["&[u8]", "[u8; 32]"] {
            let src = format!(r#"fn f() {{ let v: {target} = r.get("c"); }}"#);
            assert_eq!(targets(&src).len(), 1, "for {target}");
        }
    }

    #[test]
    fn a_composite_is_approved_by_delegation_but_its_parts_are_not() {
        // The property that keeps delegation honest. `PostRow` passes as a target — its
        // fields are policed at the declaration, which is where the newtype belongs — but
        // that approval does NOT extend to a primitive field of it.
        let approved_target =
            r#"fn f() { sqlx::query_as::<_, PostRow>("SELECT *").fetch_one(p); }"#;
        assert!(targets(approved_target).is_empty());

        let leaky_field = r#"
            #[derive(sqlx::FromRow)]
            struct PostRow { slug: Slug, tags: String }
        "#;
        assert_eq!(targets(leaky_field), vec!["String"]);
    }

    #[test]
    fn a_composite_the_gate_never_read_is_not_approved() {
        // Delegation is a second policed population, not a promise. A composite the gate
        // never read has had no field checked, so approving it would be an unbacked claim.
        let src = r#"fn f() { sqlx::query_as::<_, SomeForeignRow>("SELECT *").fetch_one(p); }"#;
        assert_eq!(targets(src), vec!["SomeForeignRow"]);
    }

    #[test]
    fn composites_are_collected_only_from_the_policed_root() {
        // The scoping that keeps delegation backed. A `FromRow` struct or tuple alias
        // declared in `common/src` is NOT approved: its fields are never examined, because
        // field policing runs under `storage/src` alone. Approving it would be the unbacked
        // promise delegation exists to avoid — and this is the direction that fails open,
        // so nothing else would catch it.
        let src = "
            #[derive(sqlx::FromRow)]
            struct ForeignRow { a: Slug }
            type ForeignTuple = (Slug, PostId);
        ";
        let mut policed = ApproveSet::default();
        collect_declarations(src, Root::Policed, &mut policed).expect("parses");
        assert!(policed.composites.contains("ForeignRow"));
        assert!(policed.composites.contains("ForeignTuple"));

        let mut declaration_only = ApproveSet::default();
        collect_declarations(src, Root::DeclarationsOnly, &mut declaration_only).expect("parses");
        assert!(
            declaration_only.composites.is_empty(),
            "a composite outside the policed root has had no field checked"
        );
    }

    #[test]
    fn bridge_types_are_collected_from_every_declaration_root() {
        // The other half of the asymmetry: a bridge-carrying type is approved wherever it
        // is declared, because the bridge itself is the claim — no second check needed.
        // `host::invite::InviteCode` is decoded by storage and lives outside `common`.
        let src = "
            #[derive(Clone, StrNewtype)]
            pub struct InviteCode(String);
        ";
        let mut outside = ApproveSet::default();
        collect_declarations(src, Root::DeclarationsOnly, &mut outside).expect("parses");
        assert!(outside.approved.contains("InviteCode"));
    }

    #[test]
    fn resolves_an_alias_to_its_approved_underlying_newtype() {
        // `pub type HubUrl = TaggedUrl<Hub>;` — the decode names the alias, the bridge is on
        // the generic type. Without resolution every role alias would false-fail (#875).
        let mut set = ApproveSet::default();
        set.approved.insert("TaggedUrl".to_owned());
        set.aliases
            .insert("HubUrl".to_owned(), "TaggedUrl".to_owned());

        let ty: syn::Type = syn::parse_quote!(Option<HubUrl>);
        assert!(unapproved_leaves(&ty, &set).is_empty());
    }

    #[test]
    fn rejects_an_alias_to_an_unapproved_type() {
        // Resolution is not approval: an alias whose target carries no bridge still fails,
        // and the message names the *underlying* type, which is what must be fixed.
        let mut set = ApproveSet::default();
        set.aliases
            .insert("Mystery".to_owned(), "NotDerived".to_owned());

        let ty: syn::Type = syn::parse_quote!(Option<Mystery>);
        assert_eq!(unapproved_leaves(&ty, &set), vec!["NotDerived".to_owned()]);
    }

    #[test]
    fn still_rejects_a_bare_unapproved_type() {
        let set = ApproveSet::default();
        let ty: syn::Type = syn::parse_quote!(Option<Undeclared>);
        assert_eq!(unapproved_leaves(&ty, &set), vec!["Undeclared".to_owned()]);
    }

    #[test]
    fn collects_generic_aliases_from_a_declarations_only_root() {
        // The cross-crate half: the aliases live in `common/src`, the decodes they name live
        // in `storage/src`. Collecting them only under the policed root would miss them all.
        let mut set = ApproveSet::default();
        collect_declarations(
            "pub type HubUrl = TaggedUrl<Hub>;",
            Root::DeclarationsOnly,
            &mut set,
        )
        .expect("parses");
        assert_eq!(set.aliases.get("HubUrl"), Some(&"TaggedUrl".to_owned()));
    }

    #[test]
    fn tuple_alias_collection_is_unchanged() {
        let mut set = ApproveSet::default();
        collect_declarations(
            "pub type MediaRow = (i64, String);",
            Root::Policed,
            &mut set,
        )
        .expect("parses");
        assert!(
            set.aliases.is_empty(),
            "tuple aliases keep their existing handling"
        );
        assert!(set.composites.contains("MediaRow"));
    }

    #[test]
    fn a_result_error_arm_is_never_a_decode_target() {
        // `Result<T, E>`: the error arm is not decoded from a column, so an unapproved
        // `E` must not fail the decode. Every fn-return site in the tree hits this.
        let src = r#"
            fn f() -> Result<Slug, BackupError> {
                sqlx::query_scalar("SELECT slug").fetch_one(p)
            }
        "#;
        assert!(targets(src).is_empty());
    }

    #[test]
    fn a_typed_tuple_decode_is_not_collected() {
        // #686's `query_as` tuples stay green for free — covering them here means they
        // can never silently regress.
        let src = r#"fn f() { sqlx::query_as::<_, (UserId, Email)>("SELECT a, b").fetch_one(p); }"#;
        assert!(targets(src).is_empty());
    }

    // ---- struct-literal field position: the gate refuses to be blind ----

    #[test]
    fn leaf_recursion_reaches_through_wrappers_without_over_matching() {
        // The question is "is any leaf unapproved", not "is any leaf i64".
        let set = approve();
        let ty: syn::Type = syn::parse_quote!(Vec<(String, Option<PostId>)>);
        assert_eq!(unapproved_leaves(&ty, &set), vec!["String".to_string()]);
        let ty: syn::Type = syn::parse_quote!(Vec<(Slug, DateTime<Utc>)>);
        assert!(unapproved_leaves(&ty, &set).is_empty());
        let ty: syn::Type = syn::parse_quote!(Option<AudienceId>);
        assert!(unapproved_leaves(&ty, &set).is_empty());
    }
}
