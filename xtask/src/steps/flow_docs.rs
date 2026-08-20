//! Deterministic typed-reference guard for flow documentation (#601).
//!
//! The checker reads only committed, reproducible inputs: mounted routes from the
//! router source, server-function endpoints from the shared inventory, and
//! covered/allowlisted status from the committed snapshot and allowlist. Typed
//! backticked `route:`, `endpoint:`, and `matrix:` tokens are the only checked
//! references.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use proc_macro2::{TokenStream, TokenTree};
use syn::visit::Visit;

use crate::files;
use crate::result::StepResult;
use crate::server_fn_coverage::io::{inventory, read_allowlist, read_artifact};
use crate::server_fn_coverage::{AllowlistEntry, Snapshot};

const STEP: &str = "flow-docs";
const FLOW_DIR: &str = "docs/flows";
const ROUTER_PATH: &str = "web/src/app/component.rs";
const WEB_SRC: &str = "web/src";
const SNAPSHOT_PATH: &str = "docs/coverage/server-fns.json";
const ALLOWLIST_PATH: &str = "docs/coverage/server-fns-allowlist.json";
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

#[cfg(not(test))]
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
    let snapshot: Snapshot = read_artifact(&root.join(SNAPSHOT_PATH))
        .with_context(|| format!("reading {}", root.join(SNAPSHOT_PATH).display()))?;
    let allowlist = read_allowlist(&root.join(ALLOWLIST_PATH))
        .with_context(|| format!("reading {}", root.join(ALLOWLIST_PATH).display()))?;
    let allowlisted: BTreeMap<String, AllowlistEntry> = allowlist
        .into_iter()
        .map(|entry| (entry.server_fn.clone(), entry))
        .collect();

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
        if let Some(entry) = allowlisted.get(qualified) {
            endpoint_statuses.push(format!(
                "{}: allowlisted — {} ({})",
                endpoint_token(endpoint),
                entry.reason,
                entry.issue
            ));
            continue;
        }
        errors.push(format!(
            "{}: declared in {} but missing from {} and {}",
            endpoint_token(endpoint),
            locations.join(", "),
            SNAPSHOT_PATH,
            ALLOWLIST_PATH
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
    let path = root.join(ROUTER_PATH);
    let source =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    mounted_routes_in(&source)
        .with_context(|| format!("parsing mounted routes in {}", path.display()))
}

fn mounted_routes_in(source: &str) -> Result<BTreeSet<String>> {
    let file = syn::parse_file(source).context("cannot parse router source as Rust")?;
    let mut visitor = ViewVisitor::default();
    visitor.visit_file(&file);
    if !visitor.errors.is_empty() {
        bail!(visitor.errors.join("\n"));
    }
    Ok(visitor.routes)
}

#[derive(Default)]
struct ViewVisitor {
    routes: BTreeSet<String>,
    errors: Vec<String>,
}

impl<'ast> Visit<'ast> for ViewVisitor {
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if mac.path.is_ident("view")
            && let Err(error) = collect_routes_from_view(&mac.tokens, &mut self.routes)
        {
            self.errors.push(error.to_string());
        }
        syn::visit::visit_macro(self, mac);
    }
}

#[derive(Clone)]
struct OpenRoute {
    name: String,
    prefix: Vec<String>,
}

fn collect_routes_from_view(tokens: &TokenStream, routes: &mut BTreeSet<String>) -> Result<()> {
    let tokens: Vec<TokenTree> = tokens.clone().into_iter().collect();
    let mut i = 0;
    let mut stack: Vec<OpenRoute> = Vec::new();
    while i < tokens.len() {
        if !matches_punct(tokens.get(i), '<') {
            i += 1;
            continue;
        }
        if matches_punct(tokens.get(i + 1), '/') {
            let Some(name) = ident(tokens.get(i + 2)) else {
                i += 1;
                continue;
            };
            i += 3;
            while i < tokens.len() && !matches_punct(tokens.get(i), '>') {
                i += 1;
            }
            if i < tokens.len() {
                i += 1;
            }
            if matches!(name.as_str(), "ParentRoute" | "Route") {
                match stack.pop() {
                    Some(open) if open.name == name => {}
                    Some(open) => bail!(
                        "route tag nesting desynced: closed {name} while {} was open",
                        open.name
                    ),
                    None => {
                        bail!("route tag nesting desynced: closed {name} with no matching open tag")
                    }
                }
            }
            continue;
        }

        let Some(name) = ident(tokens.get(i + 1)) else {
            i += 1;
            continue;
        };
        let (tag, next) = parse_open_tag(&tokens, i + 2)?;
        i = next;
        if !matches!(name.as_str(), "ParentRoute" | "Route") {
            continue;
        }
        let path = tag
            .path
            .with_context(|| format!("<{name}> is missing a `path=` attribute"))?;
        let segments = normalize_path_expr(path)?;
        let mut mounted = current_prefix(&stack);
        mounted.extend(segments.clone());
        if name == "ParentRoute" && mounted.is_empty() {
            routes.insert("<shell>".to_string());
        } else {
            routes.insert(render_route(&mounted));
        }
        if !tag.self_closing {
            stack.push(OpenRoute {
                name: name.to_string(),
                prefix: mounted,
            });
        }
    }
    if let Some(open) = stack.last() {
        bail!("route tag nesting desynced: <{}> was not closed", open.name);
    }
    Ok(())
}

struct ParsedTag {
    path: Option<TokenStream>,
    self_closing: bool,
}

fn parse_open_tag(tokens: &[TokenTree], mut i: usize) -> Result<(ParsedTag, usize)> {
    let mut path = None;
    loop {
        if i >= tokens.len() {
            bail!("unterminated route tag");
        }
        if matches_punct(tokens.get(i), '>') {
            return Ok((
                ParsedTag {
                    path,
                    self_closing: false,
                },
                i + 1,
            ));
        }
        if matches_punct(tokens.get(i), '/') && matches_punct(tokens.get(i + 1), '>') {
            return Ok((
                ParsedTag {
                    path,
                    self_closing: true,
                },
                i + 2,
            ));
        }
        let Some(attr) = ident(tokens.get(i)) else {
            i += 1;
            continue;
        };
        if !matches_punct(tokens.get(i + 1), '=') {
            i += 1;
            continue;
        }
        let (value, next) = read_attr_value(tokens, i + 2)?;
        if attr == "path" {
            path = Some(value);
        }
        i = next;
    }
}

fn read_attr_value(tokens: &[TokenTree], mut i: usize) -> Result<(TokenStream, usize)> {
    let mut value = Vec::new();
    loop {
        if i >= tokens.len() {
            bail!("unterminated route attribute value");
        }
        if matches_punct(tokens.get(i), '>')
            || (matches_punct(tokens.get(i), '/') && matches_punct(tokens.get(i + 1), '>'))
        {
            break;
        }
        if ident(tokens.get(i)).is_some() && matches_punct(tokens.get(i + 1), '=') {
            break;
        }
        value.push(tokens[i].clone());
        i += 1;
    }
    Ok((value.into_iter().collect(), i))
}

fn normalize_path_expr(tokens: TokenStream) -> Result<Vec<String>> {
    normalize_expr(syn::parse2(tokens).context("cannot parse route `path=` expression")?)
}

fn normalize_expr(expr: syn::Expr) -> Result<Vec<String>> {
    match expr {
        syn::Expr::Call(call) => {
            let name = match call.func.as_ref() {
                syn::Expr::Path(path) => path
                    .path
                    .segments
                    .last()
                    .map(|segment| segment.ident.to_string())
                    .unwrap_or_default(),
                _ => String::new(),
            };
            let Some(first) = call.args.first() else {
                bail!("route segment call is missing its string literal");
            };
            let syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(value),
                ..
            }) = first
            else {
                bail!("route segment call must take a string literal");
            };
            match name.as_str() {
                "StaticSegment" => {
                    if value.value().is_empty() {
                        Ok(Vec::new())
                    } else {
                        Ok(vec![value.value()])
                    }
                }
                "ParamSegment" => Ok(vec![format!(":{}", value.value())]),
                "TildeUsername" => Ok(vec![format!("~:{}", value.value())]),
                _ => bail!("unsupported route segment `{name}`"),
            }
        }
        syn::Expr::Tuple(tuple) => {
            let mut out = Vec::new();
            for elem in tuple.elems {
                out.extend(normalize_expr(elem)?);
            }
            Ok(out)
        }
        syn::Expr::Paren(paren) => normalize_expr(*paren.expr),
        other => bail!("unsupported route expression `{}`", quote::quote!(#other)),
    }
}

fn current_prefix(stack: &[OpenRoute]) -> Vec<String> {
    stack
        .last()
        .map(|open| open.prefix.clone())
        .unwrap_or_default()
}

fn render_route(segments: &[String]) -> String {
    if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    }
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

    fn component_source() -> &'static str {
        r#"
use crate::route_segments::TildeUsername;
use leptos_router::{
    ParamSegment, StaticSegment,
    components::{ParentRoute, Route, Router, Routes},
};

fn app() {
    view! {
        <Router>
            <Routes fallback=|| "Page not found.".into_view()>
                <ParentRoute path=StaticSegment("") view=AppShell>
                    <Route path=StaticSegment("") view=HomePage />
                    <Route path=StaticSegment("login") view=LoginPage />
                    <Route path=ParamSegment("username") view=UserTimelinePage />
                    <Route
                        path=(
                            TildeUsername("username"),
                            ParamSegment("year"),
                            ParamSegment("month"),
                            ParamSegment("day"),
                            ParamSegment("slug"),
                        )
                        view=PostPage
                    />
                </ParentRoute>
            </Routes>
        </Router>
    }
}
"#
    }

    fn write_component(root: &Path) {
        write(&root.join(ROUTER_PATH), component_source());
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

    fn write_allowlist(root: &Path, entries: &[(&str, &str, &str)]) {
        let json = entries
            .iter()
            .map(|(server_fn, reason, issue)| {
                format!(
                    "{{\"server_fn\":\"{server_fn}\",\"reason\":\"{reason}\",\"issue\":\"{issue}\"}}"
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        write(&root.join(ALLOWLIST_PATH), &format!("[{json}]"));
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
        write_component(tmp.path());
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
    fn mounted_routes_normalize_shell_param_and_tilde_username_and_skip_fallback() {
        let routes = mounted_routes_in(component_source()).expect("routes parse");
        assert_eq!(
            routes,
            BTreeSet::from([
                "<shell>".to_string(),
                "/".to_string(),
                "/login".to_string(),
                "/:username".to_string(),
                "/~:username/:year/:month/:day/:slug".to_string(),
            ])
        );
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
    fn declared_endpoints_must_be_covered_or_allowlisted() {
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
                "endpoint:/api/posts/create: declared in docs/flows/flow.md but missing from docs/coverage/server-fns.json and docs/coverage/server-fns-allowlist.json"
            ),
            "{detail}"
        );
        assert!(
            detail.contains("endpoint:/api/posts/create: missing coverage"),
            "{detail}"
        );
    }

    #[test]
    fn covered_and_allowlisted_endpoints_are_reported_and_unmapped_routes_are_informational() {
        let tmp = base_fixture();
        write_server_fns(
            tmp.path(),
            &[("posts", &["create"]), ("sessions", &["revoke"])],
        );
        write_snapshot(tmp.path(), &["posts::create"]);
        write_allowlist(
            tmp.path(),
            &[(
                "sessions::revoke",
                "no second browser session in this fixture",
                "#707",
            )],
        );
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
            detail.contains("endpoint:/api/sessions/revoke: allowlisted"),
            "{detail}"
        );
        assert!(detail.contains("route:/login"), "{detail}");
        assert!(detail.contains("route:/"), "{detail}");
    }

    #[test]
    fn evidence_file_does_not_affect_the_report() {
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
        let step = run(Path::new("."));
        assert!(step.ok, "{}", step.detail.unwrap_or_default());
    }
}
