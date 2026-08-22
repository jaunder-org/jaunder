//! The `sqlx-newtype-bind` static check (#438, #686, #716): enumerates
//! primitive `sqlx` bind arguments under `storage/src` and denies them by
//! default.
//!
//! Domain values that are stored emit an `sqlx::Encode`/`Type`/`Decode` bridge
//! from one shared codegen (`macros::sqlx_bridge`) — `StrNewtype`,
//! `IdNewtype`, `NumNewtype`, `SqlxBridge`, and stored `#[text_enum(sqlx)]`
//! enums — so storage code should bind the typed value directly. Stripping to a
//! primitive at the bind boundary re-opens the transposition hazard the newtype
//! exists to close (ADR-0063 §2).
//!
//! This gate follows ADR-0085: it parses Rust, defines its population
//! structurally, and fails closed. The population is a `.bind(...)` whose
//! argument is visibly primitive without type inference: a literal, a cast to a
//! primitive, a `.as_str()` borrow, or an identifier whose current function
//! parameter / typed local binding is primitive (`bool`, `str`, `String`, or a
//! numeric primitive, including references to those). That is deliberately wider
//! than the old spelling search (`.as_ref()`, `&*`, `i64::from(...)`) and catches
//! the #716 shape where a primitive parameter is bound after the strip occurred
//! elsewhere.
//!
//! Exemptions are ADR-0094 markers, not a central allowlist: the line immediately
//! above the bind must carry `// sqlx-newtype-bind:allow <category> — <reason>`.
//! Categories are `permanent-primitive`, `test-fixture-corruption`, and
//! `deferred-newtype`; `deferred-newtype` must name a tracking issue. A marker is
//! an in-source assertion, not a proof: the gate checks that it is present,
//! categorized, non-orphaned, and points at exactly one primitive bind, but it
//! cannot prove the reason is true.
//!
//! ## Structural limits
//!
//! No call graph: the gate detects the primitive bind, not the caller that may
//! have stripped a domain value before passing it through a function or trait
//! seam. No SQL semantics: `COUNT(*)`, timestamps, booleans, and driver-required
//! primitives are not inferred from query text; they need markers. No type
//! inference: an unannotated field such as `record.size_bytes` is not classified
//! from the struct definition. The line-level marker is one level of indirection
//! off the strongest possible key, so the gate requires exactly one primitive
//! bind on the marked line and fails orphan markers.

use std::collections::HashMap;

use quote::ToTokens;
use syn::visit::{self, Visit};

use crate::markers;
use crate::result::CommandResult;
use crate::steps::scan::run_source_scan;

/// Source root scanned recursively for `.rs` files.
const POLICED_ROOT: &str = "storage/src";
const MARKER: &str = "sqlx-newtype-bind:allow";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    PermanentPrimitive,
    TestFixtureCorruption,
    DeferredNewtype,
}

impl Category {
    const ALL_LABELS: &[&str] = &[
        "permanent-primitive",
        "test-fixture-corruption",
        "deferred-newtype",
    ];

    fn parse(label: &str) -> Option<Self> {
        match label {
            "permanent-primitive" => Some(Self::PermanentPrimitive),
            "test-fixture-corruption" => Some(Self::TestFixtureCorruption),
            "deferred-newtype" => Some(Self::DeferredNewtype),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::PermanentPrimitive => "permanent-primitive",
            Self::TestFixtureCorruption => "test-fixture-corruption",
            Self::DeferredNewtype => "deferred-newtype",
        }
    }
}

#[derive(Debug, Clone)]
struct BindSite {
    line: usize,
    expr: String,
}

#[derive(Debug, Clone)]
struct MarkedSite {
    line: usize,
    category: Category,
    reason: String,
}

#[derive(Debug, Default)]
struct Scan {
    sites: Vec<BindSite>,
}

#[derive(Default)]
struct BindVisitor {
    scopes: Vec<HashMap<String, String>>,
    sites: Vec<BindSite>,
}

impl BindVisitor {
    fn enter_scope(&mut self, inputs: &syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma>) {
        let mut scope = HashMap::new();
        for input in inputs {
            if let syn::FnArg::Typed(pat) = input
                && let syn::Pat::Ident(ident) = pat.pat.as_ref()
                && primitive_type(&pat.ty)
            {
                scope.insert(
                    ident.ident.to_string(),
                    pat.ty.to_token_stream().to_string(),
                );
            }
        }
        self.scopes.push(scope);
    }

    fn enter_closure_scope(
        &mut self,
        inputs: &syn::punctuated::Punctuated<syn::Pat, syn::token::Comma>,
    ) {
        let mut scope = HashMap::new();
        for input in inputs {
            if let syn::Pat::Type(pat_ty) = input
                && let syn::Pat::Ident(ident) = pat_ty.pat.as_ref()
                && primitive_type(&pat_ty.ty)
            {
                scope.insert(
                    ident.ident.to_string(),
                    pat_ty.ty.to_token_stream().to_string(),
                );
            }
        }
        self.scopes.push(scope);
    }

    fn leave_scope(&mut self) {
        self.scopes.pop();
    }

    fn record_local(&mut self, local: &syn::Local) {
        let syn::Pat::Type(pat_ty) = &local.pat else {
            return;
        };
        let syn::Pat::Ident(ident) = pat_ty.pat.as_ref() else {
            return;
        };
        if primitive_type(&pat_ty.ty)
            && let Some(scope) = self.scopes.last_mut()
        {
            scope.insert(
                ident.ident.to_string(),
                pat_ty.ty.to_token_stream().to_string(),
            );
        }
    }

    fn ident_is_primitive(&self, ident: &str) -> bool {
        self.scopes
            .iter()
            .rev()
            .any(|scope| scope.contains_key(ident))
    }

    fn expr_is_primitive_bind(&self, expr: &syn::Expr) -> bool {
        match expr {
            syn::Expr::Lit(lit) => primitive_lit(&lit.lit),
            syn::Expr::Path(path) => path
                .path
                .get_ident()
                .is_some_and(|ident| self.ident_is_primitive(&ident.to_string())),
            syn::Expr::Reference(reference) => self.expr_is_primitive_bind(&reference.expr),
            syn::Expr::Cast(cast) => primitive_type(&cast.ty),
            syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Deref(_)) => {
                self.expr_is_primitive_bind(&unary.expr)
            }
            syn::Expr::MethodCall(call) => call.method == "as_str",
            syn::Expr::Paren(paren) => self.expr_is_primitive_bind(&paren.expr),
            _ => false,
        }
    }
}

impl<'ast> Visit<'ast> for BindVisitor {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        self.enter_scope(&node.sig.inputs);
        visit::visit_block(self, &node.block);
        self.leave_scope();
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        self.enter_scope(&node.sig.inputs);
        visit::visit_block(self, &node.block);
        self.leave_scope();
    }

    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        self.enter_closure_scope(&node.inputs);
        visit::visit_expr(self, &node.body);
        self.leave_scope();
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        self.record_local(node);
        visit::visit_local(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if node.method == "bind"
            && let Some(arg) = node.args.first()
            && self.expr_is_primitive_bind(arg)
        {
            self.sites.push(BindSite {
                line: node.method.span().start().line,
                expr: arg.to_token_stream().to_string(),
            });
        }
        visit::visit_expr_method_call(self, node);
    }
}

fn primitive_lit(lit: &syn::Lit) -> bool {
    matches!(
        lit,
        syn::Lit::Bool(_) | syn::Lit::Int(_) | syn::Lit::Float(_) | syn::Lit::Str(_)
    )
}

fn primitive_type(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Reference(reference) => primitive_type(&reference.elem),
        syn::Type::Path(path) => path.path.segments.last().is_some_and(|segment| {
            matches!(
                segment.ident.to_string().as_str(),
                "bool"
                    | "str"
                    | "String"
                    | "i8"
                    | "i16"
                    | "i32"
                    | "i64"
                    | "i128"
                    | "isize"
                    | "u8"
                    | "u16"
                    | "u32"
                    | "u64"
                    | "u128"
                    | "usize"
                    | "f32"
                    | "f64"
            )
        }),
        syn::Type::Paren(paren) => primitive_type(&paren.elem),
        _ => false,
    }
}

fn primitive_binds(source: &str) -> Result<Scan, String> {
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut visitor = BindVisitor::default();
    visitor.visit_file(&file);
    Ok(Scan {
        sites: visitor.sites,
    })
}

fn parse_marker(reason: &str) -> Result<(Category, String), String> {
    if reason.trim().is_empty() {
        return Err("marker has no reason".to_string());
    }
    let mut parts = reason.trim().splitn(2, char::is_whitespace);
    let label = parts.next().unwrap_or_default();
    let rest = parts
        .next()
        .unwrap_or_default()
        .trim_start_matches([' ', '-', '—', ':'])
        .trim()
        .to_string();
    let Some(category) = Category::parse(label) else {
        return Err(format!(
            "marker category `{label}` is not one of {}",
            Category::ALL_LABELS.join(", ")
        ));
    };
    if rest.is_empty() {
        return Err("marker has no reason after its category".to_string());
    }
    if category == Category::DeferredNewtype && !names_issue(&rest) {
        return Err("deferred-newtype marker must name a tracking issue like #750".to_string());
    }
    Ok((category, rest))
}

fn names_issue(reason: &str) -> bool {
    let bytes = reason.as_bytes();
    bytes.windows(2).enumerate().any(|(idx, pair)| {
        pair[0] == b'#'
            && pair[1].is_ascii_digit()
            && reason[idx + 1..].bytes().any(|b| b.is_ascii_digit())
    })
}

/// The failure detail for offending primitive binds and malformed markers, or `None`.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut lines = Vec::new();
    let mut marked = Vec::new();

    for (path, source) in scanned {
        let scan = match primitive_binds(source) {
            Ok(scan) => scan,
            Err(e) => {
                lines.push(format!(
                    "{path}: {e} — an unparsed file is invisible to this gate, which is exactly \
                     the blind spot it exists to close. Fix the file or the parser; do not skip it."
                ));
                continue;
            }
        };
        let comments = markers::line_comments(source);
        let mut sites_by_line: HashMap<usize, Vec<&BindSite>> = HashMap::new();
        for site in &scan.sites {
            sites_by_line.entry(site.line).or_default().push(site);
        }

        for (idx, comment) in comments.iter().enumerate() {
            let marker_line = idx + 1;
            let Some(reason) = comment.and_then(|c| markers::marker_in_comment(c, MARKER)) else {
                continue;
            };
            let pointed_line = marker_line + 1;
            match sites_by_line.get(&pointed_line).map(Vec::as_slice).unwrap_or(&[]) {
                [] => lines.push(format!(
                    "{path}:{marker_line}: `{MARKER}` marker is orphaned — the next line has no \
                     primitive sqlx bind. Delete the stale marker or move it directly above the bind."
                )),
                [_site] => match parse_marker(reason) {
                    Ok((category, parsed_reason)) => marked.push((
                        path.clone(),
                        MarkedSite {
                            line: pointed_line,
                            category,
                            reason: parsed_reason,
                        },
                    )),
                    Err(error) => lines.push(format!(
                        "{path}:{marker_line}: malformed `{MARKER}` marker — {error}. Use \
                         `// {MARKER} permanent-primitive — <reason>`, \
                         `test-fixture-corruption`, or `deferred-newtype #NNNN`."
                    )),
                },
                many => lines.push(format!(
                    "{path}:{marker_line}: `{MARKER}` marker points at line {pointed_line}, which \
                     has {} primitive sqlx binds. Split the bind chain so one marker covers one site.",
                    many.len()
                )),
            }
        }

        for site in &scan.sites {
            let marked_line = site
                .line
                .checked_sub(2)
                .and_then(|idx| comments.get(idx))
                .and_then(|comment| comment.and_then(|c| markers::marker_in_comment(c, MARKER)))
                .is_some();
            if !marked_line {
                lines.push(format!(
                    "{path}:{}: `.bind({})` binds a primitive value. Type the seam so a domain \
                     value reaches sqlx, or add a one-line marker immediately above this bind with \
                     category and reason.",
                    site.line, site.expr
                ));
            }
        }
    }

    if lines.is_empty() {
        return None;
    }

    lines.push(
        "  recovery: this gate enumerates primitive binds rather than searching for known strip \
         spellings. Typing the seam is the default fix. Markers are for scalar facts, intentional \
         corrupt test rows, or tracked `deferred-newtype` debt; a marker is trusted prose, so keep \
         the census small enough to re-read."
            .to_string(),
    );
    if !marked.is_empty() {
        lines.push("  currently marked primitive-bind exemptions:".to_string());
        marked.sort_by(|a, b| (&a.0, a.1.line).cmp(&(&b.0, b.1.line)));
        for (path, site) in marked {
            lines.push(format!(
                "    - {path}:{} — {} — {}",
                site.line,
                site.category.label(),
                site.reason
            ));
        }
    }
    Some(lines.join("\n"))
}

/// Scan every Rust file under [`POLICED_ROOT`] and push the result step.
pub fn run(result: &mut CommandResult) {
    run_source_scan(result, "sqlx-newtype-bind", &[POLICED_ROOT], problems);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site_lines(src: &str) -> Vec<usize> {
        primitive_binds(src)
            .expect("parse")
            .sites
            .into_iter()
            .map(|s| s.line)
            .collect()
    }

    #[test]
    fn typed_newtype_bind_is_clean() {
        let src = "fn f(slug: Slug) { q.bind(slug); }";
        assert!(site_lines(src).is_empty());
    }

    #[test]
    fn primitive_parameter_bind_is_flagged_without_strip_spelling() {
        let src = "fn f(min_items: i64) { q.bind(min_items); }";
        assert_eq!(site_lines(src), vec![1]);
    }

    #[test]
    fn primitive_closure_parameter_bind_is_flagged() {
        let src = "fn f() { let bind_limit = |limit: i64| q.bind(limit); }";
        assert_eq!(site_lines(src), vec![1]);
    }

    #[test]
    fn dereferenced_primitive_reference_bind_is_flagged() {
        let src = "fn f(value: &bool) { q.bind(*value); }";
        assert_eq!(site_lines(src), vec![1]);
    }

    #[test]
    fn primitive_local_and_as_str_bind_are_flagged() {
        let src = r#"
fn f() {
    let limit: i64 = 10;
    q.bind(limit);
    q.bind("bad");
    q.bind(name.as_str());
}
"#;
        assert_eq!(site_lines(src), vec![4, 5, 6]);
    }

    #[test]
    fn categorized_marker_exempts_one_site() {
        let src = r#"
fn f(is_operator: bool) {
    // sqlx-newtype-bind:allow permanent-primitive — boolean operator flag has no domain identity.
    q.bind(is_operator);
}
"#;
        assert_eq!(
            problems(&[("storage/src/users.rs".into(), src.into())]),
            None
        );
    }

    #[test]
    fn unmarked_site_reports_recovery() {
        let src = "fn f(min_items: i64) { q.bind(min_items); }";
        let detail = problems(&[("storage/src/posts.rs".into(), src.into())]).expect("problem");
        assert!(detail.contains("storage/src/posts.rs:1"));
        assert!(detail.contains("binds a primitive value"));
        assert!(detail.contains("Typing the seam is the default fix"));
    }

    #[test]
    fn marker_with_no_reason_fails() {
        let src = r#"
fn f(is_operator: bool) {
    // sqlx-newtype-bind:allow
    q.bind(is_operator);
}
"#;
        let detail = problems(&[("storage/src/users.rs".into(), src.into())]).expect("problem");
        assert!(detail.contains("marker has no reason"));
    }

    #[test]
    fn marker_with_unknown_category_fails() {
        let src = r#"
fn f(is_operator: bool) {
    // sqlx-newtype-bind:allow because I said so
    q.bind(is_operator);
}
"#;
        let detail = problems(&[("storage/src/users.rs".into(), src.into())]).expect("problem");
        assert!(detail.contains("marker category `because`"));
    }

    #[test]
    fn deferred_marker_without_issue_fails() {
        let src = r#"
fn f(limit_i: i64) {
    // sqlx-newtype-bind:allow deferred-newtype — type this later.
    q.bind(limit_i);
}
"#;
        let detail =
            problems(&[("storage/src/feed_events.rs".into(), src.into())]).expect("problem");
        assert!(detail.contains("deferred-newtype marker must name"));
    }

    #[test]
    fn orphan_marker_fails() {
        let src = r#"
fn f(slug: Slug) {
    // sqlx-newtype-bind:allow permanent-primitive — stale.
    q.bind(slug);
}
"#;
        let detail = problems(&[("storage/src/posts.rs".into(), src.into())]).expect("problem");
        assert!(detail.contains("marker is orphaned"));
    }

    #[test]
    fn shared_line_marker_fails() {
        let src = r#"
fn f(a: bool, b: bool) {
    // sqlx-newtype-bind:allow permanent-primitive — two binds are ambiguous.
    q.bind(a).bind(b);
}
"#;
        let detail = problems(&[("storage/src/users.rs".into(), src.into())]).expect("problem");
        assert!(detail.contains("has 2 primitive sqlx binds"));
    }

    #[test]
    fn failure_includes_marker_census() {
        let src = r##"
fn f(ok: bool, bad: i64) {
    // sqlx-newtype-bind:allow permanent-primitive — boolean operator flag has no domain identity.
    q.bind(ok);
    q.bind(bad);
}
"##;
        let detail = problems(&[("storage/src/users.rs".into(), src.into())]).expect("problem");
        assert!(detail.contains("currently marked primitive-bind exemptions"));
        assert!(detail.contains("storage/src/users.rs:4 — permanent-primitive"));
    }

    #[test]
    fn unparseable_file_fails_closed() {
        let detail = problems(&[("storage/src/bad.rs".into(), "fn {".into())]).expect("problem");
        assert!(detail.contains("cannot parse as Rust"));
    }
}
