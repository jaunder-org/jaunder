//! Deterministic typed-reference guard for flow documentation (#601).
//!
//! The checker reads only committed, reproducible inputs: mounted routes from the
//! application route catalog, server-function endpoints from the shared inventory,
//! and coverage status from the committed snapshot. Typed backticked `route:`, `endpoint:`, and
//! `matrix:` tokens are the only checked references.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use proc_macro2::{TokenStream, TokenTree};

use crate::files;
use crate::result::StepResult;
use crate::server_fn_coverage::io::{inventory, read_snapshot};

const STEP: &str = "flow-docs";
const FLOW_DIR: &str = "docs/flows";
const ROUTE_CATALOG_PATH: &str = "web/src/app/route_policy.rs";
const WEB_SRC: &str = "web/src";
const SNAPSHOT_PATH: &str = "docs/coverage/server-fns.json";
const FLOW_INDEX: &str = "docs/flows/README.md";

#[derive(Debug, Default, PartialEq, Eq)]
struct FlowRefs {
    routes: BTreeMap<String, Vec<String>>,
    endpoints: BTreeMap<String, Vec<String>>,
    matrix_refs: BTreeMap<MatrixRef, Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ParsedDoc {
    routes: Vec<String>,
    endpoints: Vec<String>,
    matrix_refs: Vec<MatrixRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
struct MatrixRef {
    path: String,
    fragment: String,
}

#[derive(Debug, Default)]
struct Report {
    errors: Vec<String>,
    endpoint_statuses: Vec<String>,
    unmapped_routes: Vec<String>,
    doc_count: usize,
    declared_route_count: usize,
    declared_endpoint_count: usize,
    matrix_ref_count: usize,
}

impl FlowRefs {
    fn add_route(&mut self, route: String, doc: &str) {
        self.routes.entry(route).or_default().push(doc.to_string());
    }

    fn add_endpoint(&mut self, endpoint: String, doc: &str) {
        self.endpoints
            .entry(endpoint)
            .or_default()
            .push(doc.to_string());
    }

    fn add_matrix_ref(&mut self, matrix_ref: MatrixRef, doc: &str) {
        self.matrix_refs
            .entry(matrix_ref)
            .or_default()
            .push(doc.to_string());
    }
}

impl Report {
    fn into_step(mut self) -> StepResult {
        self.errors.sort();
        self.endpoint_statuses.sort();
        self.unmapped_routes.sort();
        let detail = self.render();
        if self.errors.is_empty() {
            StepResult::ok(STEP).detail(detail)
        } else {
            StepResult::fail(STEP).detail(detail)
        }
    }

    fn render(&self) -> String {
        let mut out = vec![format!(
            "checked {} flow docs: {} route tokens, {} endpoint tokens, {} matrix tokens",
            self.doc_count,
            self.declared_route_count,
            self.declared_endpoint_count,
            self.matrix_ref_count
        )];
        if !self.errors.is_empty() {
            out.push("errors:".to_string());
            out.extend(self.errors.iter().map(|error| format!("- {error}")));
        }
        out.push("endpoint status:".to_string());
        if self.endpoint_statuses.is_empty() {
            out.push("- none".to_string());
        } else {
            out.extend(
                self.endpoint_statuses
                    .iter()
                    .map(|status| format!("- {status}")),
            );
        }
        out.push("unmapped routes:".to_string());
        if self.unmapped_routes.is_empty() {
            out.push("- none".to_string());
        } else {
            out.extend(
                self.unmapped_routes
                    .iter()
                    .map(|route| format!("- route:{route}")),
            );
        }
        out.join("\n")
    }
}

pub fn run() -> StepResult {
    match check(Path::new(".")) {
        Ok(report) => report.into_step(),
        Err(error) => StepResult::fail(STEP).detail(format!("{error:#}")),
    }
}

fn check(root: &Path) -> Result<Report> {
    let flow_dir = root.join(FLOW_DIR);
    let markdown = files::with_extension(&flow_dir, "md")
        .with_context(|| format!("scanning {}", flow_dir.display()))?;
    let routes = mounted_routes(root)?;
    let endpoints = endpoint_inventory(root)?;
    let snapshot = read_snapshot(&root.join(SNAPSHOT_PATH))
        .with_context(|| format!("reading {}", root.join(SNAPSHOT_PATH).display()))?;

    let mut refs = FlowRefs::default();
    let mut errors = Vec::new();
    let mut doc_count = 0;
    let mut heading_cache: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for file in markdown {
        let rel = rel(root, &file);
        let source = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        let (parsed, mut doc_errors) = parse_doc_refs(&rel, &source);
        if rel != FLOW_INDEX && parsed.matrix_refs.is_empty() {
            doc_errors.push(format!("{rel}: missing `matrix:` token"));
        }
        for route in parsed.routes {
            refs.add_route(route, &rel);
        }
        for endpoint in parsed.endpoints {
            refs.add_endpoint(endpoint, &rel);
        }
        for matrix_ref in parsed.matrix_refs {
            validate_matrix_ref(root, &rel, &matrix_ref, &mut heading_cache, &mut doc_errors)?;
            refs.add_matrix_ref(matrix_ref, &rel);
        }
        errors.append(&mut doc_errors);
        doc_count += 1;
    }

    for route in refs.routes.keys() {
        if !routes.contains(route) {
            errors.push(format!("{}: unknown mounted route", route_token(route)));
        }
    }

    for endpoint in refs.endpoints.keys() {
        if !endpoints.contains_key(endpoint) {
            errors.push(format!(
                "{}: unknown server endpoint",
                endpoint_token(endpoint)
            ));
        }
    }

    let mut endpoint_statuses = Vec::new();
    for (endpoint, locations) in &refs.endpoints {
        let Some(qualified) = endpoints.get(endpoint) else {
            continue;
        };
        if snapshot.covered.contains(qualified) {
            endpoint_statuses.push(format!("{}: covered", endpoint_token(endpoint)));
            continue;
        }

        errors.push(format!(
            "{}: declared in {} but missing from {}",
            endpoint_token(endpoint),
            locations.join(", "),
            SNAPSHOT_PATH
        ));
        endpoint_statuses.push(format!("{}: missing coverage", endpoint_token(endpoint)));
    }

    for (endpoint, qualified) in &endpoints {
        match refs.endpoints.get(endpoint) {
            None => errors.push(format!(
                "{}: unassigned source endpoint ({qualified})",
                endpoint_token(endpoint)
            )),
            Some(locations) if locations.len() > 1 => errors.push(format!(
                "{}: declared {} times ({})",
                endpoint_token(endpoint),
                locations.len(),
                locations.join(", ")
            )),
            Some(_) => {}
        }
    }

    let declared_routes: BTreeSet<String> = refs.routes.keys().cloned().collect();
    let unmapped_routes = routes.difference(&declared_routes).cloned().collect();
    let declared_endpoint_count = refs.endpoints.values().map(Vec::len).sum();
    let declared_route_count = refs.routes.values().map(Vec::len).sum();
    let matrix_ref_count = refs.matrix_refs.values().map(Vec::len).sum();

    Ok(Report {
        errors,
        endpoint_statuses,
        unmapped_routes,
        doc_count,
        declared_route_count,
        declared_endpoint_count,
        matrix_ref_count,
    })
}

fn mounted_routes(root: &Path) -> Result<BTreeSet<String>> {
    let path = root.join(ROUTE_CATALOG_PATH);
    let source =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    mounted_routes_in(&source)
        .with_context(|| format!("parsing application route catalog in {}", path.display()))
}

fn mounted_routes_in(source: &str) -> Result<BTreeSet<String>> {
    let file = syn::parse_file(source).context("cannot parse route catalog as Rust")?;
    let catalog = file
        .items
        .iter()
        .find_map(app_routes_macro_body)
        .context("cannot find `app_routes` route catalog macro")?;
    let mut routes = BTreeSet::from(["<shell>".to_string()]);
    collect_catalog_routes(catalog, &mut routes)?;
    if routes.len() == 1 {
        bail!("`app_routes` route catalog has no route entries");
    }
    Ok(routes)
}

fn app_routes_macro_body(item: &syn::Item) -> Option<TokenStream> {
    let syn::Item::Macro(item) = item else {
        return None;
    };
    (item.mac.path.is_ident("macro_rules")
        && item
            .ident
            .as_ref()
            .is_some_and(|ident| ident == "app_routes"))
    .then(|| item.mac.tokens.clone())
}

fn collect_catalog_routes(tokens: TokenStream, routes: &mut BTreeSet<String>) -> Result<()> {
    for token in tokens {
        let TokenTree::Group(group) = token else {
            continue;
        };
        let entry: Vec<TokenTree> = group.stream().into_iter().collect();
        if let Some(pattern) = catalog_entry_pattern(&entry)? {
            routes.insert(pattern);
        }
        collect_catalog_routes(group.stream(), routes)?;
    }
    Ok(())
}

fn catalog_entry_pattern(tokens: &[TokenTree]) -> Result<Option<String>> {
    if tokens.len() < 5
        || ident(tokens.first()).is_none()
        || !matches_punct(tokens.get(1), ',')
        || !matches!(ident(tokens.get(2)).as_deref(), Some("Public" | "Private"))
        || !matches_punct(tokens.get(3), ',')
    {
        return Ok(None);
    }
    let TokenTree::Literal(pattern) = &tokens[4] else {
        bail!("route catalog entry is missing its literal pattern");
    };
    let pattern = syn::parse_str::<syn::LitStr>(&pattern.to_string())?.value();
    if !pattern.starts_with('/') {
        bail!("route catalog pattern `{pattern}` is not root-relative");
    }
    Ok(Some(pattern))
}

fn endpoint_inventory(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut endpoints = BTreeMap::new();
    for server_fn in inventory(&root.join(WEB_SRC))? {
        let Some(endpoint) = &server_fn.endpoint else {
            bail!("{}: endpoint derivation failed", server_fn.qualified());
        };
        let path = format!("/api/{endpoint}");
        if endpoints
            .insert(path.clone(), server_fn.qualified())
            .is_some()
        {
            bail!("duplicate derived endpoint {path}");
        }
    }
    Ok(endpoints)
}

fn parse_doc_refs(path: &str, markdown: &str) -> (ParsedDoc, Vec<String>) {
    let mut refs = ParsedDoc::default();
    let mut errors = Vec::new();
    for code in backticked_tokens(markdown) {
        match typed_token(&code) {
            None => {}
            Some(Ok(TypedToken::Route(route))) => refs.routes.push(route),
            Some(Ok(TypedToken::Endpoint(endpoint))) => refs.endpoints.push(endpoint),
            Some(Ok(TypedToken::Matrix(matrix_ref))) => refs.matrix_refs.push(matrix_ref),
            Some(Err(error)) => errors.push(format!("{path}: {error}")),
        }
    }
    (refs, errors)
}

enum TypedToken {
    Route(String),
    Endpoint(String),
    Matrix(MatrixRef),
}

fn typed_token(code: &str) -> Option<Result<TypedToken, String>> {
    let (prefix, value) = code.split_once(':')?;
    if value.starts_with("//") {
        return None;
    }
    if prefix.is_empty()
        || !prefix
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch == '-')
    {
        return None;
    }
    Some(match prefix {
        "route" => parse_route_token(code, value).map(TypedToken::Route),
        "endpoint" => parse_endpoint_token(code, value).map(TypedToken::Endpoint),
        "matrix" => parse_matrix_token(code, value).map(TypedToken::Matrix),
        _ => Err(format!("unknown typed token `{code}`")),
    })
}

fn parse_route_token(token: &str, value: &str) -> Result<String, String> {
    if value == "<shell>" || (value.starts_with('/') && !value.contains(char::is_whitespace)) {
        Ok(value.to_string())
    } else {
        Err(format!("malformed route token `{token}`"))
    }
}

fn parse_endpoint_token(token: &str, value: &str) -> Result<String, String> {
    let parts: Vec<&str> = value.split('/').collect();
    if parts.len() == 4
        && parts[0].is_empty()
        && parts[1] == "api"
        && !parts[2].is_empty()
        && !parts[3].is_empty()
        && !value.contains(char::is_whitespace)
    {
        Ok(value.to_string())
    } else {
        Err(format!("malformed endpoint token `{token}`"))
    }
}

fn parse_matrix_token(token: &str, value: &str) -> Result<MatrixRef, String> {
    let Some((path, fragment)) = value.split_once('#') else {
        return Err(format!("malformed matrix token `{token}`"));
    };
    if path.is_empty()
        || fragment.is_empty()
        || fragment != heading_slug(fragment)
        || value.contains(char::is_whitespace)
    {
        return Err(format!("malformed matrix token `{token}`"));
    }
    Ok(MatrixRef {
        path: path.to_string(),
        fragment: fragment.to_string(),
    })
}

fn validate_matrix_ref(
    root: &Path,
    doc: &str,
    matrix_ref: &MatrixRef,
    cache: &mut BTreeMap<String, BTreeSet<String>>,
    errors: &mut Vec<String>,
) -> Result<()> {
    let headings = if let Some(headings) = cache.get(&matrix_ref.path) {
        headings.clone()
    } else {
        let path = root.join(&matrix_ref.path);
        let source = match std::fs::read_to_string(&path) {
            Ok(source) => source,
            Err(error) => {
                errors.push(format!(
                    "{doc}: {} points to unreadable file {}: {error}",
                    matrix_token(matrix_ref),
                    matrix_ref.path
                ));
                return Ok(());
            }
        };
        let headings = markdown_heading_slugs(&source);
        cache.insert(matrix_ref.path.clone(), headings.clone());
        headings
    };
    if !headings.contains(&matrix_ref.fragment) {
        errors.push(format!(
            "{doc}: {} does not match any heading in {}",
            matrix_token(matrix_ref),
            matrix_ref.path
        ));
    }
    Ok(())
}

fn markdown_heading_slugs(markdown: &str) -> BTreeSet<String> {
    markdown
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
            if hashes == 0 {
                return None;
            }
            let heading = trimmed[hashes..].trim_start();
            if heading.is_empty() {
                return None;
            }
            Some(heading_slug(heading))
        })
        .collect()
}

fn heading_slug(heading: &str) -> String {
    let mut out = String::new();
    let mut pending_sep = false;
    for ch in heading.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            if pending_sep && !out.is_empty() {
                out.push('-');
            }
            out.push(ch);
            pending_sep = false;
        } else if ch.is_ascii_whitespace() || matches!(ch, '-' | '_' | '/') {
            pending_sep = !out.is_empty();
        }
    }
    out
}

fn backticked_tokens(markdown: &str) -> Vec<String> {
    let bytes = markdown.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'`' {
            i += 1;
            continue;
        }
        let mut run = 1;
        while i + run < bytes.len() && bytes[i + run] == b'`' {
            run += 1;
        }
        if run != 1 {
            i += run;
            continue;
        }
        let start = i + 1;
        let mut j = start;
        while j < bytes.len() {
            if bytes[j] != b'`' {
                j += 1;
                continue;
            }
            let mut close = 1;
            while j + close < bytes.len() && bytes[j + close] == b'`' {
                close += 1;
            }
            if close == 1 {
                out.push(markdown[start..j].to_string());
                i = j + 1;
                break;
            }
            j += close;
        }
        if j >= bytes.len() {
            break;
        }
    }
    out
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

fn route_token(route: &str) -> String {
    format!("route:{route}")
}

fn endpoint_token(endpoint: &str) -> String {
    format!("endpoint:{endpoint}")
}

fn matrix_token(matrix_ref: &MatrixRef) -> String {
    format!("matrix:{}#{}", matrix_ref.path, matrix_ref.fragment)
}

fn ident(token: Option<&TokenTree>) -> Option<String> {
    match token {
        Some(TokenTree::Ident(ident)) => Some(ident.to_string()),
        _ => None,
    }
}

fn matches_punct(token: Option<&TokenTree>, ch: char) -> bool {
    matches!(token, Some(TokenTree::Punct(punct)) if punct.as_char() == ch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdirs");
        }
        std::fs::write(path, content).expect("write file");
    }

    fn route_catalog_source() -> &'static str {
        r#"
macro_rules! app_routes {
    ($consumer:ident) => {
        $consumer! {
            (Local, Public, "/", leptos_router::StaticSegment(""), $crate::local::LocalPage)
            (App, Private, "/app", leptos_router::StaticSegment("app"), $crate::cockpit::CockpitPage)
            (Login, Public, "/login", leptos_router::StaticSegment("login"), $crate::auth::LoginPage)
            (UserTimeline, Public, "/:username", leptos_router::ParamSegment("username"), $crate::posts::UserTimelinePage)
            (Post, Public, "/~:username/:year/:month/:day/:slug", ($crate::route_segments::TildeUsername("username"), leptos_router::ParamSegment("year"), leptos_router::ParamSegment("month"), leptos_router::ParamSegment("day"), leptos_router::ParamSegment("slug")), $crate::posts::PostPage)
            (PostHistory, Private, "/posts/:post_id/history", (leptos_router::StaticSegment("posts"), leptos_router::ParamSegment("post_id"), leptos_router::StaticSegment("history")), $crate::posts::PostHistoryPage)
        }
    };
}
"#
    }

    fn write_route_catalog(root: &Path) {
        write(&root.join(ROUTE_CATALOG_PATH), route_catalog_source());
    }

    fn write_server_fns(root: &Path, defs: &[(&str, &[&str])]) {
        for (vertical, idents) in defs {
            let source: String = idents
                .iter()
                .map(|ident| format!("#[macros::server]\npub async fn {ident}() {{}}\n"))
                .collect();
            write(&root.join(format!("web/src/{vertical}/api.rs")), &source);
        }
    }

    fn write_snapshot(root: &Path, covered: &[&str]) {
        let covered = covered
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(",");
        write(
            &root.join(SNAPSHOT_PATH),
            &format!("{{\"covered\":[{covered}],\"orphans\":{{}}}}"),
        );
    }

    fn write_matrix(root: &Path) {
        write(
            &root.join("docs/coverage/csr-e2e-matrix.md"),
            "# CSR matrix\n\n## Audiences, subscriptions, and visibility\n\n## Authentication\n",
        );
    }

    fn write_readme(root: &Path, body: &str) {
        write(&root.join(FLOW_INDEX), body);
    }

    fn write_flow(root: &Path, name: &str, body: &str) {
        write(&root.join(format!("docs/flows/{name}.md")), body);
    }

    fn run(root: &Path) -> StepResult {
        check(root).expect("check succeeds").into_step()
    }

    fn base_fixture() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_route_catalog(tmp.path());
        write_matrix(tmp.path());
        write_snapshot(tmp.path(), &[]);
        write_readme(tmp.path(), "# Flow index\n\n`route:<shell>`\n");
        tmp
    }

    #[test]
    fn extracts_typed_tokens_from_prose_tables_and_mermaid_and_ignores_arbitrary_paths() {
        let markdown = r#"
# Flow

Prose `route:/login` and `/login` and `/api/posts/create` and `/tmp/x`.

| Endpoint |
| --- |
| `endpoint:/api/posts/create` |
| `/api/posts/create` |

```mermaid
graph TD
    A[`matrix:docs/coverage/csr-e2e-matrix.md#audiences-subscriptions-and-visibility`]
    B[`/ignored`]
```
"#;
        let (parsed, errors) = parse_doc_refs("docs/flows/flow.md", markdown);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(parsed.routes, vec!["/login"]);
        assert_eq!(parsed.endpoints, vec!["/api/posts/create"]);
        assert_eq!(
            parsed.matrix_refs,
            vec![MatrixRef {
                path: "docs/coverage/csr-e2e-matrix.md".to_string(),
                fragment: "audiences-subscriptions-and-visibility".to_string(),
            }]
        );
    }

    #[test]
    fn mounted_routes_come_from_the_catalog_including_shell_public_and_private_routes() {
        let routes = mounted_routes_in(route_catalog_source()).expect("routes parse");
        assert_eq!(
            routes,
            BTreeSet::from([
                "<shell>".to_string(),
                "/".to_string(),
                "/app".to_string(),
                "/login".to_string(),
                "/:username".to_string(),
                "/~:username/:year/:month/:day/:slug".to_string(),
                "/posts/:post_id/history".to_string(),
            ])
        );
    }

    #[test]
    fn mounted_routes_reads_the_catalog_not_the_router_component() {
        let tmp = tempfile::tempdir().expect("tempdir");
        write_route_catalog(tmp.path());
        write(
            &tmp.path().join("web/src/app/component.rs"),
            "fn app() { view! { <Route path=StaticSegment(\"ghost\") /> } }",
        );

        let routes = mounted_routes(tmp.path()).expect("routes parse");
        assert!(!routes.contains("/ghost"));
        assert!(routes.contains("/app"));
    }

    #[test]
    fn malformed_and_unknown_typed_tokens_fail() {
        let (_, errors) = parse_doc_refs(
            "docs/flows/flow.md",
            "`widget:thing` `route:login` `endpoint:/api/posts` `matrix:docs/coverage/csr-e2e-matrix.md`",
        );
        assert_eq!(errors.len(), 4, "{errors:?}");
        assert!(
            errors
                .iter()
                .any(|error| error.contains("unknown typed token `widget:thing`"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("malformed route token `route:login`"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("malformed endpoint token `endpoint:/api/posts`"))
        );
        assert!(errors.iter().any(|error| {
            error.contains("malformed matrix token `matrix:docs/coverage/csr-e2e-matrix.md`")
        }));
    }

    #[test]
    fn unknown_route_and_endpoint_fail() {
        let tmp = base_fixture();
        write_flow(
            tmp.path(),
            "flow",
            "# Flow\n\n`matrix:docs/coverage/csr-e2e-matrix.md#authentication`\n`route:/ghost`\n`endpoint:/api/posts/missing`\n",
        );

        let step = run(tmp.path());
        assert!(!step.ok);
        let detail = step.detail.unwrap_or_default();
        assert!(
            detail.contains("route:/ghost: unknown mounted route"),
            "{detail}"
        );
        assert!(
            detail.contains("endpoint:/api/posts/missing: unknown server endpoint"),
            "{detail}"
        );
    }

    #[test]
    fn duplicate_endpoints_fail() {
        let tmp = base_fixture();
        write_server_fns(tmp.path(), &[("posts", &["create"])]);
        write_snapshot(tmp.path(), &["posts::create"]);
        write_flow(
            tmp.path(),
            "a",
            "# A\n\n`matrix:docs/coverage/csr-e2e-matrix.md#authentication`\n`endpoint:/api/posts/create`\n",
        );
        write_flow(
            tmp.path(),
            "b",
            "# B\n\n`matrix:docs/coverage/csr-e2e-matrix.md#authentication`\n`endpoint:/api/posts/create`\n",
        );

        let step = run(tmp.path());
        assert!(!step.ok);
        let detail = step.detail.unwrap_or_default();
        assert!(
            detail.contains("endpoint:/api/posts/create: declared 2 times"),
            "{detail}"
        );
    }

    #[test]
    fn unassigned_endpoints_fail() {
        let tmp = base_fixture();
        write_server_fns(
            tmp.path(),
            &[("posts", &["create"]), ("sessions", &["revoke"])],
        );
        write_snapshot(tmp.path(), &["posts::create", "sessions::revoke"]);
        write_flow(
            tmp.path(),
            "flow",
            "# Flow\n\n`matrix:docs/coverage/csr-e2e-matrix.md#authentication`\n`endpoint:/api/posts/create`\n",
        );

        let step = run(tmp.path());
        assert!(!step.ok);
        let detail = step.detail.unwrap_or_default();
        assert!(
            detail.contains(
                "endpoint:/api/sessions/revoke: unassigned source endpoint (sessions::revoke)"
            ),
            "{detail}"
        );
    }

    #[test]
    fn non_index_docs_require_matrix_tokens() {
        let tmp = base_fixture();
        write_flow(tmp.path(), "flow", "# Flow\n\nNo matrix token here.\n");

        let step = run(tmp.path());
        assert!(!step.ok);
        let detail = step.detail.unwrap_or_default();
        assert!(
            detail.contains("docs/flows/flow.md: missing `matrix:` token"),
            "{detail}"
        );
    }

    #[test]
    fn matrix_files_and_headings_must_exist() {
        let tmp = base_fixture();
        write_flow(
            tmp.path(),
            "missing-file",
            "# Missing\n\n`matrix:docs/coverage/missing.md#authentication`\n",
        );
        write_flow(
            tmp.path(),
            "missing-heading",
            "# Missing heading\n\n`matrix:docs/coverage/csr-e2e-matrix.md#ghost-heading`\n",
        );

        let step = run(tmp.path());
        assert!(!step.ok);
        let detail = step.detail.unwrap_or_default();
        assert!(
            detail.contains(
                "docs/flows/missing-file.md: matrix:docs/coverage/missing.md#authentication points to unreadable file docs/coverage/missing.md"
            ),
            "{detail}"
        );
        assert!(
            detail.contains(
                "docs/flows/missing-heading.md: matrix:docs/coverage/csr-e2e-matrix.md#ghost-heading does not match any heading in docs/coverage/csr-e2e-matrix.md"
            ),
            "{detail}"
        );
    }

    #[test]
    fn declared_endpoints_must_be_covered() {
        let tmp = base_fixture();
        write_server_fns(tmp.path(), &[("posts", &["create"])]);
        write_flow(
            tmp.path(),
            "flow",
            "# Flow\n\n`matrix:docs/coverage/csr-e2e-matrix.md#authentication`\n`endpoint:/api/posts/create`\n",
        );

        let step = run(tmp.path());
        assert!(!step.ok);
        let detail = step.detail.unwrap_or_default();
        assert!(
            detail.contains(
                "endpoint:/api/posts/create: declared in docs/flows/flow.md but missing from docs/coverage/server-fns.json"
            ),
            "{detail}"
        );
        assert!(
            detail.contains("endpoint:/api/posts/create: missing coverage"),
            "{detail}"
        );
    }

    #[test]
    fn covered_endpoints_are_reported_and_unmapped_routes_are_informational() {
        let tmp = base_fixture();
        write_server_fns(
            tmp.path(),
            &[("posts", &["create"]), ("sessions", &["revoke"])],
        );
        write_snapshot(tmp.path(), &["posts::create", "sessions::revoke"]);
        write_readme(
            tmp.path(),
            "# Flow index\n\n`route:<shell>`\n`route:/~:username/:year/:month/:day/:slug`\n",
        );
        write_flow(
            tmp.path(),
            "posts",
            "# Posts\n\n`matrix:docs/coverage/csr-e2e-matrix.md#audiences-subscriptions-and-visibility`\n`route:/:username`\n`endpoint:/api/posts/create`\n",
        );
        write_flow(
            tmp.path(),
            "sessions",
            "# Sessions\n\n`matrix:docs/coverage/csr-e2e-matrix.md#authentication`\n`endpoint:/api/sessions/revoke`\n",
        );

        let step = run(tmp.path());
        assert!(step.ok, "{:?}", step.detail);
        let detail = step.detail.unwrap_or_default();
        assert!(
            detail.contains("endpoint:/api/posts/create: covered"),
            "{detail}"
        );
        assert!(
            detail.contains("endpoint:/api/sessions/revoke: covered"),
            "{detail}"
        );
        assert!(detail.contains("route:/login"), "{detail}");
        assert!(detail.contains("route:/"), "{detail}");
    }

    #[test]
    fn retired_evidence_file_does_not_affect_the_report() {
        // Issue #757 made the snapshot this step's only generated input. Keep a
        // malformed retired file inert so reintroducing an evidence read bites.
        let tmp = base_fixture();
        write_server_fns(tmp.path(), &[("posts", &["create"])]);
        write_snapshot(tmp.path(), &["posts::create"]);
        write_flow(
            tmp.path(),
            "flow",
            "# Flow\n\n`matrix:docs/coverage/csr-e2e-matrix.md#authentication`\n`endpoint:/api/posts/create`\n",
        );

        let without = run(tmp.path());
        write(
            &tmp.path().join("docs/coverage/server-fns-evidence.json"),
            "{ definitely: not json }",
        );
        let with = run(tmp.path());
        assert_eq!(without.name, with.name);
        assert_eq!(without.ok, with.ok);
        assert_eq!(without.skipped, with.skipped);
        assert_eq!(without.detail, with.detail);
    }

    #[test]
    #[ignore = "Task 3 populates docs/flows before this repository-wide assertion can run"]
    fn repository_flow_corpus_is_valid() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let step = run(root);
        assert!(step.ok, "{}", step.detail.unwrap_or_default());
    }
}
