//! Fail-closed structural census for the application write capability (#363).
//!
//! The approved census deliberately excludes administrative database lifecycle
//! work (migrations, backup restore, and PostgreSQL bootstrap), backend dialect
//! traits, and the internal media reclaim-lock query. They have different
//! ownership semantics; only the named application mutation API is composable
//! through `WriteScope`.

use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};

use syn::spanned::Spanned;

use crate::result::CommandResult;
use crate::steps::scan::run_source_scan;

const POLICED_ROOTS: &[&str] = &["storage/src", "server/src", "web/src"];

/// The authoritative, closed application-mutation census. Counts add to 61.
const AUDITED_TRAITS: &[(&str, &[&str])] = &[
    (
        "AudienceStorage",
        &[
            "create_audience",
            "rename_audience",
            "delete_audience",
            "add_member",
            "remove_member",
        ],
    ),
    (
        "EmailVerificationStorage",
        &["create_email_verification", "use_email_verification"],
    ),
    ("FeedCacheStorage", &["upsert", "delete"]),
    (
        "FeedEventStorage",
        &[
            "enqueue",
            "enqueue_many",
            "claim_pending_batch",
            "mark_regenerated",
            "mark_pinged",
            "retry_regeneration",
            "dead_letter_regeneration",
            "retry_publication",
            "dead_letter_publication",
            "redrive_dead_letters",
            "restart_regeneration",
            "reset_regeneration",
        ],
    ),
    ("InviteStorage", &["create_invite", "claim_invite"]),
    ("MediaStorage", &["create_media", "try_delete_media"]),
    (
        "PasswordResetStorage",
        &["create_password_reset", "use_password_reset"],
    ),
    (
        "PostStorage",
        &[
            "create_post",
            "create_posts",
            "update_post",
            "publish_post",
            "soft_delete_post",
            "unpublish_post",
            "set_post_tags",
        ],
    ),
    (
        "PublisherStorage",
        &[
            "mutate_hub",
            "mutate_feed_window",
            "repair_malformed_hub",
            "commit_cache",
        ],
    ),
    (
        "SessionStorage",
        &[
            "create_session",
            "authenticate",
            "revoke_session",
            "revoke_all_for_user",
        ],
    ),
    (
        "SiteConfigStorage",
        &[
            "set",
            "delete",
            "set_identity",
            "set_registration_policy",
            "set_base_url",
            "set_media_limits",
            "set_media_uploads_enabled",
            "set_backup_config",
            "set_default_audience",
            "set_theme",
        ],
    ),
    ("SubscriptionStorage", &["subscribe", "unsubscribe"]),
    ("UserConfigStorage", &["set", "delete"]),
    (
        "UserStorage",
        &[
            "create_user",
            "authenticate",
            "update_profile",
            "set_email",
            "set_password",
        ],
    ),
];

/// Capability-taking helpers which are intentionally not application mutations.
/// Keep each exclusion explicit: an unlisted helper is a compatibility path and
/// must be reviewed rather than silently disappearing from the census.
const INTERNAL_CAPABILITY_EXCLUSIONS: &[(&str, &str)] = &[
    ("MediaStorage", "media_entry_is_reclaimable"), // read under the caller's reclaim lock
    ("MediaStorage", "lock_media_reference"), // internal serialization for filesystem reconciliation
];

/// Non-application capability bridges. Dialects execute the audited trait's
/// work; `Backend` provides its sealed connection bridge. None is a callable
/// compatibility mutation API.
const INTERNAL_CAPABILITY_TRAITS: &[&str] = &[
    "Backend",
    "FeedEventDialect",
    "PostDialect",
    "SessionDialect",
];

const BYPASS_EXCLUDED_PATHS: &[&str] = &[
    // The sole application transaction factory. Test-only raw-SQL lock fixtures
    // live in `storage/src/test_support/backend.rs` and are excluded by `cfg(test)`.
    "storage/src/write_scope.rs",
    "storage/src/test_support/backend.rs",
];

fn path_ends_in(path: &syn::Path, name: &str) -> bool {
    path.segments
        .last()
        .is_some_and(|segment| segment.ident == name)
}

fn is_write_transaction(ty: &syn::Type) -> bool {
    let syn::Type::Reference(reference) = ty else {
        return false;
    };
    reference.mutability.is_some()
        && matches!(reference.elem.as_ref(), syn::Type::Path(path) if path_ends_in(&path.path, "WriteTransaction"))
}

fn has_exact_capability(method: &syn::TraitItemFn) -> bool {
    method.sig.inputs.iter().any(|argument| {
        matches!(argument,
            syn::FnArg::Typed(pat_type)
                if matches!(pat_type.pat.as_ref(), syn::Pat::Ident(ident) if ident.ident == "transaction")
                    && is_write_transaction(pat_type.ty.as_ref())
        )
    })
}

fn capability_methods(item: &syn::ItemTrait) -> impl Iterator<Item = &syn::TraitItemFn> {
    item.items.iter().filter_map(|member| match member {
        syn::TraitItem::Fn(method) if method.sig.inputs.iter().any(|argument| {
            matches!(argument, syn::FnArg::Typed(pat_type) if is_write_transaction(pat_type.ty.as_ref()))
        }) => Some(method),
        _ => None,
    })
}

fn line(span: proc_macro2::Span) -> usize {
    span.start().line
}

fn expected_methods(trait_name: &str) -> Option<&'static [&'static str]> {
    AUDITED_TRAITS
        .iter()
        .find_map(|(name, methods)| (*name == trait_name).then_some(*methods))
}

fn is_internal_exclusion(trait_name: &str, method_name: &str) -> bool {
    INTERNAL_CAPABILITY_EXCLUSIONS
        .iter()
        .any(|(trait_, method)| *trait_ == trait_name && *method == method_name)
}

fn pool_like(expression: &syn::Expr) -> bool {
    match expression {
        syn::Expr::Path(path) => path.path.segments.last().is_some_and(|segment| {
            segment
                .ident
                .to_string()
                .to_ascii_lowercase()
                .contains("pool")
        }),
        syn::Expr::Field(field) => field
            .member
            .to_token_stream()
            .to_string()
            .to_ascii_lowercase()
            .contains("pool"),
        _ => false,
    }
}

fn pool_connection_acquisition(expression: &syn::Expr) -> bool {
    match expression {
        syn::Expr::Try(try_) => pool_connection_acquisition(try_.expr.as_ref()),
        syn::Expr::Await(await_) => pool_connection_acquisition(await_.base.as_ref()),
        syn::Expr::Paren(paren) => pool_connection_acquisition(paren.expr.as_ref()),
        syn::Expr::Group(group) => pool_connection_acquisition(group.expr.as_ref()),
        syn::Expr::MethodCall(call) => call.method == "acquire",
        _ => false,
    }
}

fn local_binding_name(pattern: &syn::Pat) -> Option<String> {
    match pattern {
        syn::Pat::Ident(ident) => Some(ident.ident.to_string()),
        syn::Pat::Type(type_) => local_binding_name(type_.pat.as_ref()),
        syn::Pat::Paren(paren) => local_binding_name(paren.pat.as_ref()),
        _ => None,
    }
}

fn pool_connection(expression: &syn::Expr, pool_connections: &BTreeSet<String>) -> bool {
    pool_like(expression)
        || matches!(expression,
            syn::Expr::Path(path)
                if path.path.segments.len() == 1
                    && pool_connections.contains(&path.path.segments[0].ident.to_string())
        )
}

fn direct_transaction_start(
    expression: &syn::Expr,
    pool_connections: &BTreeSet<String>,
    database_root: bool,
) -> bool {
    match expression {
        syn::Expr::MethodCall(call) => {
            matches!(call.method.to_string().as_str(), "begin" | "begin_with")
                && (database_root || pool_connection(call.receiver.as_ref(), pool_connections))
        }
        syn::Expr::Call(call) => {
            matches!(call.func.as_ref(), syn::Expr::Path(path) if path_ends_in(&path.path, "begin") || path_ends_in(&path.path, "begin_with"))
        }
        _ => false,
    }
}

fn is_internal_capability_trait(trait_name: &str) -> bool {
    INTERNAL_CAPABILITY_TRAITS.contains(&trait_name)
}

const ADMIN_TRANSACTION_FUNCTIONS: &[(&str, &str)] = &[(
    "storage/src/postgres/posts.rs",
    "apply_post_media_reference_backfill",
)];

fn is_admin_transaction_function(path: &str, name: &syn::Ident) -> bool {
    ADMIN_TRANSACTION_FUNCTIONS
        .iter()
        .any(|(allowed_path, allowed_name)| *allowed_path == path && name == allowed_name)
}

struct BypassVisitor<'a> {
    path: &'a str,
    lines: &'a mut Vec<String>,
    pool_connections: BTreeSet<String>,
}

impl<'ast> syn::visit::Visit<'ast> for BypassVisitor<'_> {
    fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
        if is_admin_transaction_function(self.path, &function.sig.ident) {
            return;
        }
        let pool_connections = std::mem::take(&mut self.pool_connections);
        syn::visit::visit_item_fn(self, function);
        self.pool_connections = pool_connections;
    }

    fn visit_impl_item_fn(&mut self, function: &'ast syn::ImplItemFn) {
        if is_admin_transaction_function(self.path, &function.sig.ident) {
            return;
        }
        let pool_connections = std::mem::take(&mut self.pool_connections);
        syn::visit::visit_impl_item_fn(self, function);
        self.pool_connections = pool_connections;
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        let pool_connections = self.pool_connections.clone();
        syn::visit::visit_block(self, block);
        self.pool_connections = pool_connections;
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        if let Some(name) = local_binding_name(&local.pat) {
            self.pool_connections.remove(&name);
            if local
                .init
                .as_ref()
                .is_some_and(|init| pool_connection_acquisition(init.expr.as_ref()))
            {
                self.pool_connections.insert(name);
            }
        }
        syn::visit::visit_local(self, local);
    }

    fn visit_expr(&mut self, expression: &'ast syn::Expr) {
        let database_root =
            self.path.starts_with("storage/src/") || self.path.starts_with("server/src/");
        if direct_transaction_start(expression, &self.pool_connections, database_root) {
            self.lines.push(format!(
                "{}:{}: direct transaction start bypasses WriteScope::run",
                self.path,
                line(expression.span())
            ));
        }
        syn::visit::visit_expr(self, expression);
    }
}

/// Return deterministic diagnostics for complete, lexically sorted source input.
pub fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut traits = BTreeMap::<String, Vec<(String, syn::ItemTrait)>>::new();
    let mut problems = Vec::new();

    for (path, source) in scanned {
        let file = match syn::parse_file(source) {
            Ok(file) => file,
            Err(error) => {
                problems.push(format!("{path}: cannot parse — write-transaction contract cannot verify this file: {error}"));
                continue;
            }
        };
        for item in &file.items {
            let syn::Item::Trait(item_trait) = item else {
                continue;
            };
            let trait_name = item_trait.ident.to_string();
            if expected_methods(&trait_name).is_some() {
                traits
                    .entry(trait_name)
                    .or_default()
                    .push((path.clone(), item_trait.clone()));
            } else if capability_methods(item_trait).next().is_some()
                && !is_internal_capability_trait(&trait_name)
            {
                problems.push(format!(
                    "{path}:{}: {trait_name} is an unapproved WriteTransaction trait",
                    line(item_trait.ident.span())
                ));
            }
        }
        if !BYPASS_EXCLUDED_PATHS.contains(&path.as_str()) {
            syn::visit::visit_file(
                &mut BypassVisitor {
                    path,
                    lines: &mut problems,
                    pool_connections: BTreeSet::new(),
                },
                &file,
            );
        }
    }

    for (trait_name, expected) in AUDITED_TRAITS {
        let Some(declarations) = traits.get(*trait_name) else {
            problems.push(format!(
                "storage census: missing audited trait {trait_name}"
            ));
            continue;
        };
        if declarations.len() != 1 {
            for (path, declaration) in declarations {
                problems.push(format!(
                    "{path}:{}: duplicate audited trait {trait_name}",
                    line(declaration.ident.span())
                ));
            }
            continue;
        }
        let (path, declaration) = &declarations[0];
        let mut found = BTreeSet::new();
        for method in capability_methods(declaration) {
            let method_name = method.sig.ident.to_string();
            if is_internal_exclusion(trait_name, &method_name) {
                continue;
            }
            if !expected.contains(&method_name.as_str()) {
                problems.push(format!("{path}:{}: {trait_name}::{method_name} is an unapproved WriteTransaction compatibility path", line(method.sig.ident.span())));
                continue;
            }
            found.insert(method_name);
        }
        for method_name in *expected {
            let Some(method) = declaration.items.iter().find_map(|member| match member {
                syn::TraitItem::Fn(method) if method.sig.ident == method_name => Some(method),
                _ => None,
            }) else {
                problems.push(format!(
                    "{path}:{}: missing audited declaration {trait_name}::{method_name}",
                    line(declaration.ident.span())
                ));
                continue;
            };
            if !has_exact_capability(method) {
                problems.push(format!("{path}:{}: {trait_name}::{method_name} must take `transaction: &mut WriteTransaction`", line(method.sig.ident.span())));
            }
        }
        if found.len() != expected.len() {
            problems.push(format!(
                "{path}:{}: {trait_name} has {} audited WriteTransaction methods; expected {}",
                line(declaration.ident.span()),
                found.len(),
                expected.len()
            ));
        }
    }

    match scanned
        .iter()
        .find(|(path, _)| path == "storage/src/write_scope.rs")
    {
        Some((path, source)) => {
            if let Ok(file) = syn::parse_file(source) {
                let sealed = file.items.iter().find_map(|item| match item {
                    syn::Item::Struct(item) if item.ident == "WriteTransaction" => Some(item),
                    _ => None,
                });
                if !sealed.is_some_and(|item| {
                    item.fields
                        .iter()
                        .all(|field| matches!(field.vis, syn::Visibility::Inherited))
                }) {
                    problems.push(format!(
                        "{path}: WriteTransaction must remain sealed with private fields"
                    ));
                }
            }
        }
        None => problems
            .push("storage/src/write_scope.rs: missing sealed WriteTransaction definition".into()),
    }

    problems.sort();
    problems.dedup();
    (!problems.is_empty()).then(|| problems.join("\n"))
}

/// Run the census over all production source roots. Any traversal, read, UTF-8,
/// or parse failure is fatal; a partial census must never pass.
pub fn run(result: &mut CommandResult) {
    run_source_scan(
        result,
        "write-transaction-contract",
        POLICED_ROOTS,
        problems,
    );
}

#[cfg(test)]
mod tests {
    use std::io;

    use crate::CommandResult;
    use crate::steps::scan::run_source_scan_with;

    use super::problems;

    fn fixture(source: &str) -> Vec<(String, String)> {
        vec![
            ("storage/src/audited.rs".into(), source.into()),
            ("storage/src/write_scope.rs".into(), "pub struct WriteTransaction { transaction: HeldTransaction }\nenum HeldTransaction {}".into()),
        ]
    }

    fn census(methods: &[(&str, &[&str])]) -> String {
        methods
            .iter()
            .map(|(trait_name, names)| {
                let methods = names
                    .iter()
                    .map(|name| {
                        format!("async fn {name}(&self, transaction: &mut WriteTransaction);")
                    })
                    .collect::<String>();
                format!("trait {trait_name} {{ {methods} }}")
            })
            .collect()
    }

    fn complete_census() -> String {
        census(super::AUDITED_TRAITS)
    }

    #[test]
    fn exact_forty_eight_method_census_passes() {
        let source = complete_census().replacen(
            "trait MediaStorage {",
            "trait MediaStorage { async fn media_entry_is_reclaimable(&self, transaction: &mut WriteTransaction);",
            1,
        );
        assert!(problems(&fixture(&source)).is_none());
    }

    #[test]
    fn missing_extra_duplicate_and_no_capability_fail() {
        let missing = complete_census().replacen(
            "async fn create_invite(&self, transaction: &mut WriteTransaction);",
            "",
            1,
        );
        assert!(
            problems(&fixture(&missing))
                .unwrap()
                .contains("missing audited declaration InviteStorage::create_invite")
        );

        let extra = complete_census().replacen(
            "trait UserStorage {",
            "trait UserStorage { async fn compatibility_write(&self, transaction: &mut WriteTransaction);",
            1,
        );
        assert!(
            problems(&fixture(&extra))
                .unwrap()
                .contains("unapproved WriteTransaction compatibility path")
        );

        let duplicate = format!("{} trait UserStorage {{}}", complete_census());
        assert!(
            problems(&fixture(&duplicate))
                .unwrap()
                .contains("duplicate audited trait UserStorage")
        );

        let no_capability = complete_census().replacen(
            "async fn create_invite(&self, transaction: &mut WriteTransaction);",
            "async fn create_invite(&self);",
            1,
        );

        assert!(
            problems(&fixture(&no_capability))
                .unwrap()
                .contains("InviteStorage::create_invite must take")
        );
    }
    #[test]
    fn only_named_admin_function_is_excluded_from_bypass_scan() {
        let admin = format!(
            "{} struct Adapter; impl Adapter {{ async fn apply_post_media_reference_backfill(pool: sqlx::PgPool) {{ pool.begin().await.unwrap(); }} }}",
            complete_census()
        );
        let mut sources = fixture(&admin);
        sources[0].0 = "storage/src/postgres/posts.rs".into();
        assert!(problems(&sources).is_none());

        let non_admin = format!(
            "{} struct Adapter; impl Adapter {{ async fn bypass(pool: sqlx::PgPool) {{ pool.begin().await.unwrap(); }} }}",
            complete_census()
        );
        sources[0].1 = non_admin;
        assert!(
            problems(&sources)
                .unwrap()
                .contains("direct transaction start bypasses WriteScope::run")
        );
    }

    #[test]
    fn web_state_begin_is_not_a_pool_transaction_start() {
        let source = format!(
            "{} fn begin_upload(state: UploadState) {{ state.begin(); }}",
            complete_census()
        );
        let mut sources = fixture(&source);
        sources[0].0 = "web/src/media/component.rs".into();
        assert!(problems(&sources).is_none());
    }

    #[test]
    fn parse_failure_and_direct_pool_begin_fail_closed() {
        assert!(
            problems(&[("storage/src/bad.rs".into(), "fn {".into())])
                .unwrap()
                .contains("cannot parse")
        );
        let direct = format!(
            "{} async fn bypass(pool: sqlx::PgPool) {{ pool.begin().await.unwrap(); }}",
            complete_census()
        );
        assert!(
            problems(&fixture(&direct))
                .unwrap()
                .contains("direct transaction start bypasses WriteScope::run")
        );
    }

    #[test]
    fn acquired_pool_connection_transaction_start_fails_closed() {
        let acquired = format!(
            "{} async fn bypass(db: sqlx::PgPool) {{ let mut connection: sqlx::pool::PoolConnection<sqlx::Postgres> = (db.acquire().await?); connection.begin().await?; }}",
            complete_census()
        );
        let mut sources = fixture(&acquired);
        sources[0].0 = "web/src/acquired.rs".into();
        assert!(
            problems(&sources)
                .unwrap()
                .contains("direct transaction start bypasses WriteScope::run")
        );

        let begin_with = format!(
            "{} async fn bypass(db: sqlx::PgPool) {{ let mut connection = db.acquire().await?; connection.begin_with(\"BEGIN\").await?; }}",
            complete_census()
        );
        sources[0].1 = begin_with;
        assert!(
            problems(&sources)
                .unwrap()
                .contains("direct transaction start bypasses WriteScope::run")
        );
    }

    #[test]
    fn unreadable_source_fails_before_the_census_runs() {
        let directory = tempfile::tempdir().expect("tempdir");
        let source = directory.path().join("audited.rs");
        std::fs::write(&source, complete_census()).expect("fixture source");
        let mut result = CommandResult::new("test");

        run_source_scan_with(
            &mut result,
            "write-transaction-contract",
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

    #[test]
    fn write_scope_and_capability_helper_flow_pass() {
        let valid = format!(
            "{} async fn helper(transaction: &mut WriteTransaction) {{ let _ = transaction; }} async fn caller(scope: &WriteScope) {{ scope.run(|transaction| Box::pin(helper(transaction))).await.unwrap(); }}",
            complete_census()
        );
        assert!(problems(&fixture(&valid)).is_none());
    }
}
