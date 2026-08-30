use quote::ToTokens;
use syn::spanned::Spanned;

use super::approve_set::{ApproveSet, is_from_row, unapproved_leaves};

/// One unapproved decode target: where it is, and why it failed approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DecodeSite {
    /// Rendered decode target, whitespace-stripped.
    pub(super) target: String,
    /// Rendered first argument / field name for the diagnostic.
    pub(super) what: String,
    /// The leaf types that are not approved — the reason this site is in the report.
    pub(super) unapproved: Vec<String>,
    pub(super) line: usize,
}

/// Renders `t` to source text with whitespace removed, making diagnostics stable against
/// rustfmt reflow and `syn`'s token spacing (`Option < i64 >` → `Option<i64>`).
pub(super) fn render<T: ToTokens>(t: &T) -> String {
    t.to_token_stream()
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// The decode methods this gate reads, and where their target type sits in a
/// turbofish. `query_scalar::<DB, T>` puts it second; `get::<T, I>` puts it first.
fn target_index(name: &str) -> Option<usize> {
    match name {
        "query_scalar" | "query_as" => Some(1),
        "get" | "try_get" => Some(0),
        _ => None,
    }
}

/// The `n`th *type* argument in a generic-argument list, skipping lifetimes and
/// consts. Shared by the free-call path (`query_scalar::<_, T>`) and the method path
/// (`row.get::<T, _>`), which spell their turbofish differently but index it the same.
fn nth_type_of<'a>(
    args: impl Iterator<Item = &'a syn::GenericArgument>,
    n: usize,
) -> Option<syn::Type> {
    args.filter_map(|a| match a {
        syn::GenericArgument::Type(t) => Some(t.clone()),
        _ => None,
    })
    .nth(n)
}

/// The `n`th type argument of a path's angle-bracketed turbofish, if present.
fn nth_type_arg(args: &syn::PathArguments, n: usize) -> Option<syn::Type> {
    let syn::PathArguments::AngleBracketed(ab) = args else {
        return None;
    };
    nth_type_of(ab.args.iter(), n)
}

/// Walks a file collecting unapproved decode targets, carrying the enclosing function or
/// trait-default-method return type and the enclosing `let` ascription so each call can take
/// its **nearest** declared type.
struct Scanner<'a> {
    /// The types a decode may legally land in.
    approve: &'a ApproveSet,
    out: Vec<DecodeSite>,
    /// `(line, column-argument)` of each turbofish-less `.get`/`try_get` in struct-literal
    /// field position — a hard failure, not a decode record.
    unreadable_fields: Vec<(usize, String)>,
    /// Spans of those calls, so [`Scanner::record`] can decline them rather than pinning
    /// them to whatever `let` or `fn` return happens to enclose the struct literal.
    field_positions: std::collections::HashSet<(usize, usize)>,
    fn_ret: Option<syn::Type>,
    let_ty: Option<syn::Type>,
}

impl Scanner<'_> {
    fn new(
        field_positions: std::collections::HashSet<(usize, usize)>,
        approve: &ApproveSet,
    ) -> Scanner<'_> {
        Scanner {
            out: Vec::new(),
            unreadable_fields: Vec::new(),
            field_positions,
            approve,
            fn_ret: None,
            let_ty: None,
        }
    }

    /// Records one decode with the nearest declared target, if that target has an
    /// unapproved leaf. `turbofish` wins, then the enclosing `let`, then the enclosing
    /// function or trait-default-method return.
    fn record(&mut self, turbofish: Option<syn::Type>, what: String, span: proc_macro2::Span) {
        let target = turbofish
            .or_else(|| self.let_ty.clone())
            .or_else(|| self.fn_ret.clone());
        let Some(target) = target else {
            // Unreadable: no turbofish, no ascription, no enclosing function or
            // trait-default-method return. Out of population by construction — see the module doc.
            return;
        };
        let unapproved = unapproved_leaves(&target, self.approve);
        if unapproved.is_empty() {
            return;
        }
        self.out.push(DecodeSite {
            target: render(&target),
            what,
            unapproved,
            line: span.start().line,
        });
    }

    fn visit_block_with(&mut self, ret: Option<syn::Type>, block: &syn::Block) {
        let prev_ret = std::mem::replace(&mut self.fn_ret, ret);
        let prev_let = self.let_ty.take();
        syn::visit::Visit::visit_block(self, block);
        self.fn_ret = prev_ret;
        self.let_ty = prev_let;
    }
}

/// The declared return type of `sig`, or `None` for `-> ()`.
fn return_type(sig: &syn::Signature) -> Option<syn::Type> {
    match &sig.output {
        syn::ReturnType::Type(_, t) => Some((**t).clone()),
        syn::ReturnType::Default => None,
    }
}

/// Strips the wrappers that can sit between a struct-literal field and the call that
/// produces its value, so `name: row.try_get("c")?` is recognised as field position.
///
/// **The peel set is deliberately short and closed**: `?`, `.await`, and parens/groups.
/// Anything else — `.unwrap()`, a cast, a nested call — is *not* field position and this
/// rule does not reach it. A longer peel would be guesswork about which expressions
/// "really" mean the field, and the whole design rests on the gate never guessing.
fn peel_to_call(expr: &syn::Expr) -> &syn::Expr {
    match expr {
        syn::Expr::Try(e) => peel_to_call(&e.expr),
        syn::Expr::Await(e) => peel_to_call(&e.base),
        syn::Expr::Paren(e) => peel_to_call(&e.expr),
        syn::Expr::Group(e) => peel_to_call(&e.expr),
        other => other,
    }
}

/// `(line, column)` of every turbofish-less `.get`/`try_get` sitting in struct-literal
/// field position.
///
/// This is the class #715's module doc wrongly called safe: it claimed the destination
/// field's declared type pins such a decode "and that declaration is itself policed as a
/// declared target". True for a `#[derive(FromRow)]` struct; false for a plain one, whose
/// fields are policed by nothing. `syn` cannot follow the struct name to its definition —
/// it may be in another file or another crate — so the gate does not try. It refuses to be
/// blind instead: write the type at the call, and rule 1 reads it.
fn unreadable_field_positions(file: &syn::File) -> std::collections::HashSet<(usize, usize)> {
    struct Finder(std::collections::HashSet<(usize, usize)>);
    impl<'ast> syn::visit::Visit<'ast> for Finder {
        fn visit_expr_struct(&mut self, i: &'ast syn::ExprStruct) {
            for f in &i.fields {
                if let syn::Expr::MethodCall(m) = peel_to_call(&f.expr)
                    && target_index(&m.method.to_string()).is_some()
                    && m.turbofish.is_none()
                {
                    let s = m.method.span().start();
                    self.0.insert((s.line, s.column));
                }
            }
            syn::visit::visit_expr_struct(self, i);
        }
    }
    let mut finder = Finder(std::collections::HashSet::new());
    syn::visit::Visit::visit_file(&mut finder, file);
    finder.0
}

impl<'ast> syn::visit::Visit<'ast> for Scanner<'_> {
    fn visit_item_fn(&mut self, i: &'ast syn::ItemFn) {
        self.visit_block_with(return_type(&i.sig), &i.block);
    }

    fn visit_impl_item_fn(&mut self, i: &'ast syn::ImplItemFn) {
        self.visit_block_with(return_type(&i.sig), &i.block);
    }

    fn visit_trait_item_fn(&mut self, i: &'ast syn::TraitItemFn) {
        if let Some(block) = &i.default {
            self.visit_block_with(return_type(&i.sig), block);
        }
    }

    fn visit_local(&mut self, i: &'ast syn::Local) {
        let ascribed = match &i.pat {
            syn::Pat::Type(pt) => Some((*pt.ty).clone()),
            _ => None,
        };
        let prev = std::mem::replace(&mut self.let_ty, ascribed);
        if let Some(init) = &i.init {
            syn::visit::Visit::visit_local_init(self, init);
        }
        self.let_ty = prev;
    }

    fn visit_expr_call(&mut self, i: &'ast syn::ExprCall) {
        if let syn::Expr::Path(p) = &*i.func
            && let Some(last) = p.path.segments.last()
        {
            let name = last.ident.to_string();
            if let Some(idx) = target_index(&name) {
                let turbofish = nth_type_arg(&last.arguments, idx);
                let what = i.args.first().map(render).unwrap_or_default();
                self.record(turbofish, what, last.ident.span());
            }
        }
        syn::visit::visit_expr_call(self, i);
    }

    fn visit_expr_method_call(&mut self, i: &'ast syn::ExprMethodCall) {
        let name = i.method.to_string();
        if let Some(idx) = target_index(&name) {
            let span = i.method.span().start();
            let what = i.args.first().map(render).unwrap_or_default();
            if self.field_positions.contains(&(span.line, span.column)) {
                // Field position with no turbofish: a hard failure, and NOT a decode
                // record. Falling through to `record` would pin it to the enclosing `fn`
                // return — the struct literal's own type, or worse an unrelated one — and
                // report a target the author never wrote.
                self.unreadable_fields.push((span.line, what));
            } else {
                let turbofish = i
                    .turbofish
                    .as_ref()
                    .and_then(|t| nth_type_of(t.args.iter(), idx));
                self.record(turbofish, what, i.method.span());
            }
        }
        syn::visit::visit_expr_method_call(self, i);
    }

    /// A `#[derive(FromRow)]` struct is a declared decode target: each field is a
    /// column decode, and the field's type is where the newtype belongs.
    fn visit_item_struct(&mut self, i: &'ast syn::ItemStruct) {
        // Same predicate `collect_declarations` uses to decide delegation — they must not
        // drift, or a struct could be approved as a composite while its fields go
        // unpoliced.
        if is_from_row(&i.attrs) {
            for f in &i.fields {
                let unapproved = unapproved_leaves(&f.ty, self.approve);
                if !unapproved.is_empty() {
                    self.out.push(DecodeSite {
                        target: render(&f.ty),
                        what: f
                            .ident
                            .as_ref()
                            .map(std::string::ToString::to_string)
                            .unwrap_or_default(),
                        unapproved,
                        line: f.ty.span().start().line,
                    });
                }
            }
        }
        syn::visit::visit_item_struct(self, i);
    }

    /// A tuple `type` alias is a declared `query_as` target. `syn` cannot tell one
    /// from any other tuple alias, so every tuple alias under the root is policed —
    /// see the module doc.
    fn visit_item_type(&mut self, i: &'ast syn::ItemType) {
        if let syn::Type::Tuple(t) = &*i.ty {
            for (n, elem) in t.elems.iter().enumerate() {
                let unapproved = unapproved_leaves(elem, self.approve);
                if !unapproved.is_empty() {
                    self.out.push(DecodeSite {
                        target: render(elem),
                        what: format!("{}.{n}", i.ident),
                        unapproved,
                        line: elem.span().start().line,
                    });
                }
            }
        }
        syn::visit::visit_item_type(self, i);
    }
}

/// Every structurally readable decode with an unapproved target in `source`, or the parse error.
///
/// A file that will not parse is **not** silently skipped: an unparsed file is a file
/// the gate cannot see, and a gate that quietly shrinks its own population is the
/// failure this whole design exists to prevent. Pure, so it is unit-tested directly.
pub(super) fn decodes(source: &str, approve: &ApproveSet) -> Result<FileScan, String> {
    let file = syn::parse_file(source).map_err(|e| format!("cannot parse as Rust: {e}"))?;
    let mut scanner = Scanner::new(unreadable_field_positions(&file), approve);
    syn::visit::Visit::visit_file(&mut scanner, &file);
    Ok(FileScan {
        sites: scanner.out,
        unreadable_fields: scanner.unreadable_fields,
    })
}

/// One file's scan: the decode records, and the field-position calls whose target is not
/// written anywhere the gate can read.
pub(super) struct FileScan {
    pub(super) sites: Vec<DecodeSite>,
    /// `(line, column-argument)` per failure.
    pub(super) unreadable_fields: Vec<(usize, String)>,
}

/// Targets of every decode the scanner found, for terse assertions.
#[cfg(test)]
pub(super) fn targets(src: &str) -> Vec<String> {
    decodes(src, &super::approve_set::approve())
        .expect("parses")
        .sites
        .into_iter()
        .map(|d| d.target)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::approve_set::approve;
    use super::*;

    /// Line numbers of the turbofish-less struct-literal field decodes in `src`.
    fn field_failures(src: &str) -> Vec<usize> {
        decodes(src, &approve())
            .expect("parses")
            .unreadable_fields
            .into_iter()
            .map(|(line, _)| line)
            .collect()
    }

    #[test]
    fn turbofish_target_is_collected() {
        let src = r#"fn f() { sqlx::query_scalar::<_, i64>("SELECT a").fetch_one(p); }"#;
        assert_eq!(targets(src), vec!["i64"]);
    }

    #[test]
    fn ascribed_let_target_is_collected() {
        let src = r#"fn f() { let id: i64 = sqlx::query_scalar("SELECT a").fetch_one(p); }"#;
        assert_eq!(targets(src), vec!["i64"]);
    }

    #[test]
    fn vec_of_tuple_with_option_i64_is_collected() {
        // Site #9's shape (`posts.rs`'s `pa.audience_id`): the type is on the `let`,
        // wrapped in a `Vec` and a tuple. #686's audit required a turbofish and so
        // could not see it. A gate that misses this misses the site it was built for.
        let src = r#"
            fn f() {
                let rows: Vec<(String, Option<i64>)> = sqlx::query_as("SELECT a, b").fetch_all(p);
            }
        "#;
        assert_eq!(targets(src), vec!["Vec<(String,Option<i64>)>"]);
    }

    #[test]
    fn ascribed_row_get_is_collected() {
        // No live instance of this shape remains (#715), so only a synthetic
        // source can prove the gate bites here.
        let src = r#"fn f() { let id: i64 = r.get("id"); }"#;
        assert_eq!(targets(src), vec!["i64"]);
    }

    #[test]
    fn fn_return_type_covers_every_arm() {
        // One fn return type supplies the declared target for a decode in each arm.
        let src = r#"
            async fn scalar_i64(&self, sql: &str) -> Result<i64, sqlx::Error> {
                match self {
                    A(pool) => sqlx::query_scalar(sql).fetch_one(pool).await,
                    B(pool) => sqlx::query_scalar(sql).fetch_one(pool).await,
                }
            }
        "#;
        assert_eq!(targets(src).len(), 2);
    }

    #[test]
    fn trait_default_return_type_is_collected() {
        let src = r#"
            trait Store {
                fn load(&self) -> Result<i64, E> {
                    self.get("id")
                }
            }
        "#;
        assert_eq!(targets(src), vec!["Result<i64,E>"]);
    }

    #[test]
    fn turbofish_wins_over_trait_default_return() {
        let src = r#"
            trait Store {
                fn load(&self) -> Result<i64, E> {
                    self.get::<Option<i64>, _>("id")
                }
            }
        "#;
        assert_eq!(targets(src), vec!["Option<i64>"]);
    }

    #[test]
    fn ascription_wins_over_trait_default_return() {
        let src = r#"
            trait Store {
                fn load(&self) -> Result<i64, E> {
                    let id: Option<i64> = self.get("id");
                    Ok(id.unwrap_or_default())
                }
            }
        "#;
        assert_eq!(targets(src), vec!["Option<i64>"]);
    }

    #[test]
    fn required_trait_method_contributes_no_site() {
        let src = r#"
            trait Store {
                fn load(&self) -> Result<i64, E>;
            }
        "#;
        assert_eq!(targets(src), Vec::<String>::new());
    }

    #[test]
    fn one_let_over_two_calls_yields_two_records() {
        // One `let` over two dialect calls still yields two records.
        let src = r#"
            fn f() {
                let live_count: i64 = match pool {
                    A(p) => sqlx::query_scalar("SELECT COUNT(*) FROM a").fetch_one(p).await?,
                    B(p) => sqlx::query_scalar("SELECT COUNT(*) FROM b").fetch_one(p).await?,
                };
            }
        "#;
        assert_eq!(targets(src).len(), 2);
    }

    #[test]
    fn turbofish_wins_over_the_enclosing_fn_return() {
        // Both positions fire; precedence must choose the turbofish and record one site.
        let src = r#"
            async fn schema_version(c: &mut C) -> Result<i64, BackupError> {
                Ok(sqlx::query_scalar::<_, Option<i64>>("SELECT MAX(version) FROM m")
                    .fetch_one(c).await?.unwrap_or_default())
            }
        "#;
        assert_eq!(targets(src), vec!["Option<i64>"]);
    }

    #[test]
    fn ascription_wins_over_the_enclosing_fn_return() {
        let src = r#"
            fn f() -> Result<i64, E> {
                let n: Option<i64> = sqlx::query_scalar("SELECT MAX(v) FROM m").fetch_one(c)?;
                Ok(n.unwrap_or_default())
            }
        "#;
        assert_eq!(targets(src), vec!["Option<i64>"]);
    }

    // ---- narrowly-proven handwritten FromRow composites ----

    #[test]
    fn from_row_struct_field_is_collected() {
        let src = r#"
            #[derive(sqlx::FromRow)]
            struct R { revision_id: i64, slug: Slug }
        "#;
        assert_eq!(targets(src), vec!["i64"]);
    }

    #[test]
    fn tuple_alias_element_is_collected() {
        let src = "type CacheTuple = (FeedPath, i64, DateTime<Utc>);";
        assert_eq!(targets(src), vec!["i64"]);
    }

    #[test]
    fn a_plain_struct_without_from_row_is_not_collected() {
        let src = "struct NotARow { count: i64 }";
        assert!(targets(src).is_empty());
    }

    // ---- the gate must not over-bite ----

    #[test]
    fn an_unturbofished_struct_literal_field_is_a_failure() {
        // "The destination field's declaration polices the decode" only holds for
        // `#[derive(FromRow)]` structs. `Rec` here is a plain struct, so nothing
        // polices it — the exact shape that hid `FeedEventRecord.attempts` and
        // `ColumnInfo.name` (#728).
        let src = r#"fn f() { Rec { id: r.get("id"), attempts: r.get("attempts") }; }"#;
        assert_eq!(field_failures(src).len(), 2);
        // …and it is NOT also recorded as a decode against some enclosing type.
        assert!(targets(src).is_empty());
    }

    #[test]
    fn a_turbofished_struct_literal_field_is_read_normally() {
        let src = r#"fn f() { Rec { id: r.get::<i64, _>("id") }; }"#;
        assert!(field_failures(src).is_empty());
        assert_eq!(targets(src), vec!["i64"]);
    }

    #[test]
    fn the_peel_set_is_exactly_try_await_and_parens() {
        // The live sites are `name: row.try_get("c")?`, so `?` must peel. `.await` and
        // parens travel with it. Everything else is deliberately out of reach — a longer
        // peel would be guesswork about which expressions "really" mean the field.
        for src in [
            r#"fn f() { Rec { a: r.try_get("c")? }; }"#,
            r#"fn f() { Rec { a: r.get("c").await }; }"#,
            r#"fn f() { Rec { a: (r.get("c")) }; }"#,
        ] {
            assert_eq!(field_failures(src).len(), 1, "must bite: {src}");
        }
        // `.unwrap()` puts the call outside field position: the field's value is the
        // `unwrap` call, not the `get`.
        let src = r#"fn f() { Rec { a: r.get("c").unwrap() }; }"#;
        assert!(field_failures(src).is_empty(), "must not bite: {src}");
    }

    #[test]
    fn field_position_does_not_reach_a_nested_argument() {
        // A `get` buried in an argument is not the field's value.
        let src = r#"fn f() { Rec { a: helper(r.get("c")) }; }"#;
        assert!(field_failures(src).is_empty());
    }

    #[test]
    fn an_unascribed_get_is_not_collected() {
        // `syn` cannot tell `sqlx::Row::get` from `serde_json::Map::get`, and both live
        // under the policed root. Guessing by receiver name would be the pattern search
        // this gate forbids — so this class is out of population and documented.
        let src = r#"fn f() { let value = row.get(&column.name).ok_or_else(|| e)?; }"#;
        assert!(targets(src).is_empty());
    }

    #[test]
    fn a_hoist_does_not_leak_into_the_next_function() {
        // Scope discipline: an ascription in one fn must not type a decode in the next.
        let src = r#"
            fn a() { let n: i64 = sqlx::query_scalar("SELECT COUNT(*)").fetch_one(p); }
            fn b() { sqlx::query_scalar("SELECT slug").fetch_one(p); }
        "#;
        assert_eq!(targets(src).len(), 1);
    }
}
