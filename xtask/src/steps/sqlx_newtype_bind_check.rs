//! The `sqlx-newtype-bind` gate closes every source-visible sqlx value-admission
//! door under `storage/src`.
//!
//! The typed storage seam is the only admission API: `bind_storage` for native
//! queries and `push_storage_bind` for native builders.  This gate is intentionally
//! syntax based and deny-by-default: it rejects every source-visible raw admission
//! shape rather than trying to identify primitive arguments.  Its only allowance is
//! the five direct `self.<raw>(value)` delegations in `storage/src/sql.rs`'s typed
//! extension implementations.  It does not claim rustc name resolution or insight
//! into arbitrary proc-macro expansion; query macros are therefore forbidden.
//!
//! `run_source_scan` supplies every governed Rust file and fails closed when the
//! root or a file cannot be read.  Parsing failures are reported as failures too.
use std::collections::HashSet;

use quote::ToTokens;
use syn::visit::{self, Visit};

use crate::result::CommandResult;
use crate::steps::scan::run_source_scan;

const POLICED_ROOT: &str = "storage/src";
const RAW_METHODS: &[&str] = &[
    "bind",
    "try_bind",
    "push_bind",
    "push_bind_unseparated",
    "with_arguments",
];
const RAW_CONSTRUCTORS: &[&str] = &[
    "query_with",
    "query_as_with",
    "query_scalar_with",
    "__query_with_result",
    "__query_scalar_with_result",
];

#[derive(Debug)]
struct Site {
    line: usize,
    detail: String,
}

#[derive(Default)]
struct AdmissionVisitor {
    sites: Vec<Site>,
    native_argument_aliases: HashSet<String>,
    native_argument_scopes: Vec<HashSet<String>>,
    query_macro_aliases: HashSet<String>,
    sqlx_receiver_aliases: HashSet<String>,
    seam_method: Option<String>,
    in_typed_seam_impl: bool,
    allow_seam_delegations: bool,
}
impl AdmissionVisitor {
    fn record<T: ToTokens>(&mut self, span: proc_macro2::Span, detail: T) {
        self.sites.push(Site {
            line: span.start().line,
            detail: detail.to_token_stream().to_string(),
        });
    }

    fn raw_method(method: &syn::Ident) -> bool {
        RAW_METHODS.contains(&method.to_string().as_str())
    }

    fn direct_seam_delegation(&self, call: &syn::ExprMethodCall) -> bool {
        let Some(seam_method) = &self.seam_method else {
            return false;
        };
        self.allow_seam_delegations
            && self.in_typed_seam_impl
            && matches!(seam_method.as_str(), "bind_storage" | "push_storage_bind")
            && call.receiver.to_token_stream().to_string() == "self"
            && call.args.len() == 1
            && call.args[0].to_token_stream().to_string() == "value"
            && matches!(
                (seam_method.as_str(), call.method.to_string().as_str()),
                ("bind_storage", "bind") | ("push_storage_bind", "push_bind")
            )
    }

    fn native_arguments(&self, path: &syn::Path) -> bool {
        path.segments.iter().any(|segment| {
            let name = segment.ident.to_string();
            matches!(name.as_str(), "PgArguments" | "SqliteArguments")
                || self.native_argument_aliases.contains(&name)
        })
    }

    fn ufcs_sqlx_method(&self, path: &syn::Path) -> bool {
        RAW_METHODS.contains(
            &path
                .segments
                .last()
                .expect("non-empty path")
                .ident
                .to_string()
                .as_str(),
        ) && path.segments.iter().rev().nth(1).is_some_and(|segment| {
            let name = segment.ident.to_string();
            matches!(
                name.as_str(),
                "Query" | "QueryAs" | "QueryScalar" | "QueryBuilder" | "Separated"
            ) || self.sqlx_receiver_aliases.contains(&name)
        })
    }

    fn forbidden_query_macro(&self, path: &syn::Path) -> bool {
        let name = path
            .segments
            .last()
            .map(|segment| segment.ident.to_string());
        name.is_some_and(|name| {
            self.query_macro_aliases.contains(&name)
                || matches!(
                    name.as_str(),
                    "query" | "query_as" | "query_scalar" | "query_file" | "query_file_as"
                )
                || name.starts_with("query_")
        })
    }
    fn inspect_macro<T: ToTokens>(&mut self, mac: &syn::Macro, detail: T) {
        if self.forbidden_query_macro(&mac.path) {
            self.record(
                mac.path
                    .segments
                    .last()
                    .expect("non-empty path")
                    .ident
                    .span(),
                detail,
            );
        }
    }

    fn native_argument_local(&self, name: &str) -> bool {
        self.native_argument_scopes
            .iter()
            .rev()
            .any(|scope| scope.contains(name))
    }

    fn collect_argument(&mut self, pat: &syn::Pat) {
        let syn::Pat::Type(pat) = pat else { return };
        let syn::Pat::Ident(ident) = pat.pat.as_ref() else {
            return;
        };
        let tokens = pat.ty.to_token_stream().to_string();
        if self.native_arguments_type(&tokens) {
            self.native_argument_scopes
                .last_mut()
                .expect("function scope exists")
                .insert(ident.ident.to_string());
        }
    }
    fn inspect_import<T: ToTokens>(
        &mut self,
        original: &str,
        local: &str,
        span: proc_macro2::Span,
        detail: T,
    ) {
        if matches!(original, "query" | "query_as" | "query_scalar") {
            self.query_macro_aliases.insert(local.to_owned());
        }
        if matches!(
            original,
            "Query" | "QueryAs" | "QueryScalar" | "QueryBuilder" | "Separated"
        ) {
            self.sqlx_receiver_aliases.insert(local.to_owned());
        }
        if matches!(original, "PgArguments" | "SqliteArguments") {
            self.native_argument_aliases.insert(local.to_owned());
        }
        if RAW_CONSTRUCTORS.contains(&original)
            || RAW_METHODS.contains(&original)
            || matches!(
                original,
                "add" | "Arguments" | "IntoArguments" | "PgArguments" | "SqliteArguments"
            )
        {
            self.record(span, detail);
        }
    }

    fn native_arguments_type(&self, tokens: &str) -> bool {
        tokens.contains("PgArguments")
            || tokens.contains("SqliteArguments")
            || self
                .native_argument_aliases
                .iter()
                .any(|alias| tokens.contains(alias))
    }
}

impl<'ast> Visit<'ast> for AdmissionVisitor {
    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.native_argument_scopes.push(HashSet::new());
        for input in &node.sig.inputs {
            if let syn::FnArg::Typed(argument) = input {
                self.collect_argument(&argument.pat);
            }
        }
        let previous = self.seam_method.replace(node.sig.ident.to_string());
        visit::visit_impl_item_fn(self, node);
        self.seam_method = previous;
        self.native_argument_scopes.pop();
    }

    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.native_argument_scopes.push(HashSet::new());
        for input in &node.sig.inputs {
            if let syn::FnArg::Typed(argument) = input {
                self.collect_argument(&argument.pat);
            }
        }
        visit::visit_item_fn(self, node);
        self.native_argument_scopes.pop();
    }
    fn visit_block(&mut self, node: &'ast syn::Block) {
        self.native_argument_scopes.push(HashSet::new());
        visit::visit_block(self, node);
        self.native_argument_scopes.pop();
    }

    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        self.native_argument_scopes.push(HashSet::new());
        for input in &node.inputs {
            self.collect_argument(input);
        }
        visit::visit_expr_closure(self, node);
        self.native_argument_scopes.pop();
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if Self::raw_method(&node.method) && !self.direct_seam_delegation(node) {
            self.record(node.method.span(), node);
        }
        if node.method == "add"
            && matches!(&*node.receiver, syn::Expr::Path(path) if self.native_argument_local(&path.path.to_token_stream().to_string()))
        {
            self.record(node.method.span(), node);
        }
        visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = node.func.as_ref()
            && (self.native_arguments(&path.path)
                || RAW_CONSTRUCTORS.contains(
                    &path
                        .path
                        .segments
                        .last()
                        .expect("non-empty path")
                        .ident
                        .to_string()
                        .as_str(),
                )
                || self.ufcs_sqlx_method(&path.path)
                || (path
                    .path
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "add")
                    && path
                        .path
                        .to_token_stream()
                        .to_string()
                        .contains("Arguments")))
        {
            self.record(
                path.path
                    .segments
                    .last()
                    .expect("non-empty path")
                    .ident
                    .span(),
                node,
            );
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_expr_macro(&mut self, node: &'ast syn::ExprMacro) {
        self.inspect_macro(&node.mac, node);
        visit::visit_expr_macro(self, node);
    }

    fn visit_item_macro(&mut self, node: &'ast syn::ItemMacro) {
        self.inspect_macro(&node.mac, node);
        visit::visit_item_macro(self, node);
    }

    fn visit_stmt_macro(&mut self, node: &'ast syn::StmtMacro) {
        self.inspect_macro(&node.mac, node);
        visit::visit_stmt_macro(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        let trait_name = node
            .trait_
            .as_ref()
            .and_then(|(_, path, _)| path.segments.last())
            .map(|segment| segment.ident.to_string());
        if matches!(trait_name.as_deref(), Some("Arguments" | "IntoArguments")) {
            self.record(node.impl_token.span, node);
        }
        let previous = self.in_typed_seam_impl;
        self.in_typed_seam_impl = matches!(
            trait_name.as_deref(),
            Some("QueryStorageExt" | "QueryBuilderStorageExt")
        ) && matches!(
            node.self_ty.to_token_stream().to_string().as_str(),
            self_type if self_type.contains("Query") || self_type.contains("Separated")
        );
        visit::visit_item_impl(self, node);
        self.in_typed_seam_impl = previous;
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        let (ident, type_tokens) = match &node.pat {
            syn::Pat::Ident(ident) => (&ident.ident, String::new()),
            syn::Pat::Type(pat) if matches!(pat.pat.as_ref(), syn::Pat::Ident(_)) => {
                let syn::Pat::Ident(ident) = pat.pat.as_ref() else {
                    unreachable!()
                };
                (&ident.ident, pat.ty.to_token_stream().to_string())
            }
            _ => {
                visit::visit_local(self, node);
                return;
            }
        };
        if self.native_arguments_type(&type_tokens) {
            self.native_argument_scopes
                .last_mut()
                .expect("local has enclosing scope")
                .insert(ident.to_string());
        }
        visit::visit_local(self, node);
    }
    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        if self.native_arguments_type(&node.ty.to_token_stream().to_string()) {
            self.native_argument_aliases.insert(node.ident.to_string());
        }
        visit::visit_item_type(self, node);
    }

    fn visit_use_name(&mut self, node: &'ast syn::UseName) {
        let name = node.ident.to_string();
        self.inspect_import(&name, &name, node.ident.span(), node);
        visit::visit_use_name(self, node);
    }

    fn visit_use_rename(&mut self, node: &'ast syn::UseRename) {
        let original = node.ident.to_string();
        let local = node.rename.to_string();
        self.inspect_import(&original, &local, node.rename.span(), node);
        visit::visit_use_rename(self, node);
    }
}

fn admissions(source: &str, allow_seam_delegations: bool) -> Result<Vec<Site>, String> {
    let file = syn::parse_file(source).map_err(|error| format!("cannot parse as Rust: {error}"))?;
    let mut visitor = AdmissionVisitor {
        allow_seam_delegations,
        ..AdmissionVisitor::default()
    };
    visitor.visit_file(&file);
    Ok(visitor.sites)
}

/// Reports every raw sqlx value-admission syntax under the governed tree.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    for (path, source) in scanned {
        match admissions(source, path == "storage/src/sql.rs") {
            Ok(sites) => lines.extend(sites.into_iter().map(|site| {
                format!(
                    "{path}:{}: forbidden raw sqlx value admission `{}`; use the typed storage seam",
                    site.line, site.detail
                )
            })),
            Err(error) => lines.push(format!(
                "{path}: {error} — an unparsable governed file is invisible to this gate, so it fails closed"
            )),
        }
        if source.contains("sqlx-newtype-bind:allow") {
            lines.push(format!(
                "{path}: obsolete sqlx-newtype-bind exemption marker; the typed seam has no exemptions"
            ));
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Scan every Rust file under [`POLICED_ROOT`] and push the result step.
pub fn run(result: &mut CommandResult) {
    run_source_scan(result, "sqlx-newtype-bind", &[POLICED_ROOT], problems);
}

#[cfg(test)]
mod tests {
    use std::io;

    use crate::steps::scan::run_source_scan_with;

    use super::*;

    fn rejected(source: &str) {
        assert!(
            problems(&[("storage/src/example.rs".to_string(), source.to_string())]).is_some(),
            "expected rejection: {source}"
        );
    }

    #[test]
    fn admits_only_exact_typed_seam_delegations() {
        assert!(problems(&[(
            "storage/src/sql.rs".to_string(),
            "impl QueryStorageExt for Query { fn bind_storage(self, value: T) { self.bind(value); } } impl QueryBuilderStorageExt for QueryBuilder { fn push_storage_bind(&mut self, value: T) { self.push_bind(value); } }".to_string(),
        )]).is_none());
        assert!(
            problems(&[(
                "storage/src/example.rs".to_string(),
                "impl X { fn bind_storage(self, value: T) { self.bind(value); } }".to_string(),
            )])
            .is_some()
        );
        assert!(
            problems(&[(
                "storage/src/sql.rs".to_string(),
                "impl X { fn bind_storage(self, value: T) { self.bind(value); } }".to_string(),
            )])
            .is_some()
        );
    }

    #[test]
    fn allows_non_sqlx_associated_bind_and_generic_arguments_type() {
        assert!(problems(&[(
            "storage/src/example.rs".to_string(),
            "fn f<'q, DB>() { let _: Query<'q, DB, DB::Arguments<'q>>; UnixListener::bind(\"path\"); }".to_string(),
        )]).is_none());
    }

    #[test]
    fn rejects_each_raw_method_family() {
        rejected("fn f(q: Query, value: T) { q.bind(value); }");
        rejected("fn f(q: Query, value: T) { q.try_bind(value); }");
        rejected("fn f(q: Query, value: T) { q.push_bind(value); }");
        rejected("fn f(q: Query, value: T) { q.push_bind_unseparated(value); }");
        rejected("fn f(q: Query, value: T) { q.with_arguments(value); }");
        rejected("fn f(value: T) { let q = hidden(); q.bind(value); }");
        rejected("fn f(q: Unknown, value: T) { q.bind(value); }");
    }

    #[test]
    fn rejects_prebuilt_arguments_and_aliases() {
        rejected("fn f(value: T) { Query::bind(value); }");
        rejected("fn f(value: T) { sqlx::query_with(\"\", value); }");
        rejected("fn f(value: T) { sqlx::query_as_with(\"\", value); }");
        rejected("fn f(value: T) { sqlx::query_scalar_with(\"\", value); }");
        rejected("fn f(value: T) { sqlx::__query_with_result(\"\", value); }");
        rejected("fn f(value: T) { sqlx::__query_scalar_with_result(\"\", value); }");
        rejected("fn f(value: T) { Arguments::add(value); }");
        rejected("use sqlx::query_with as admitted;");
        rejected("impl sqlx::Arguments for Args {}");
        rejected("impl sqlx::IntoArguments<'_> for Args {}");
    }

    #[test]
    fn rejects_imported_query_constructor_binds() {
        rejected("use sqlx::query; fn f(value: T) { let q = query(\"SELECT ?\"); q.bind(value); }");
        rejected(
            "use sqlx::query as make_query; fn f(value: T) { let q = make_query(\"SELECT ?\"); q.bind(value); }",
        );
    }

    #[test]
    fn rejects_native_argument_construction_and_aliases() {
        rejected("fn f() { PgArguments::default(); }");
        rejected("fn f() { SqliteArguments::default(); }");
        rejected("use sqlx::postgres::PgArguments as Args;");
        rejected("use sqlx::sqlite::SqliteArguments as Args;");
    }

    #[test]
    fn rejects_query_macros_markers_and_parse_failures() {
        rejected("fn f() { sqlx::query!(\"SELECT 1\"); }");
        rejected("fn f() { sqlx::query_as!(Row, \"SELECT 1\"); }");
        rejected("fn f() { sqlx::query_scalar!(\"SELECT 1\"); }");
        rejected("// sqlx-newtype-bind:allow nope\nfn f() {}");
        rejected("fn f( {");
    }

    #[test]
    fn unreadable_source_fails_closed() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(directory.path().join("audited.rs"), "fn f() {}").expect("fixture source");
        let mut result = CommandResult::new("test");
        run_source_scan_with(
            &mut result,
            "sqlx-newtype-bind",
            &[directory.path().to_str().expect("utf-8 path")],
            |_| Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied")),
            problems,
        );
        assert!(!result.steps[0].ok);
        assert!(
            result.steps[0]
                .detail
                .as_deref()
                .unwrap()
                .contains("denied")
        );
    }
}
