//! The `e2e-server-fn-endpoints` static check (#712): Playwright's hardcoded
//! server-fn `/api/<vertical>/<op>` literals must name live server fns.
//!
//! ADR-0082 makes server-fn wire paths derived (`/api/<vertical>/<ident>`), and Rust
//! tests read generated `ServerFn::PATH` constants. Playwright cannot import that
//! Rust constant, so its string literals can drift silently and surface only as slow
//! e2e 404/timeouts. This gate turns that into one static failure naming the literal.
//!
//! **Population.** Every TypeScript string or template literal under
//! `end2end/tests/**/*.ts` that contains a concrete `/api/...` path, plus string
//! endpoint tails passed to `failServerFn(page, "vertical/op")` or
//! `stallServerFn(page, "vertical/op")`. The source is parsed as TypeScript via
//! Oxc; comments are not a population member. Dynamic helper templates such as
//! `` `**/api/${endpoint}` `` are ignored because they carry no concrete endpoint.
//!
//! **Source of truth.** The server-fn set is xtask's existing `#[macros::server]`
//! inventory and ADR-0082 derivation. `server/tests/web/server_fn_wire.rs` remains
//! the independent proof that real macro-generated `ServerFn::PATH` values match
//! that derivation; xtask still cannot link `web` and read those constants directly.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, bail};
use oxc_allocator::Allocator;
use oxc_ast::ast::{Argument, CallExpression, Expression, StringLiteral, TemplateLiteral};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::{Parser, ParserReturn};
use oxc_span::{GetSpan, SourceType};

use crate::files;
use crate::result::{CommandResult, StepResult};
use crate::server_fn_coverage::io::{WEB_SRC, inventory};

const STEP: &str = "e2e-server-fn-endpoints";
const E2E_ROOT: &str = "end2end/tests";
const HELPER_NAMES: &[&str] = &["failServerFn", "stallServerFn"];

struct AllowedNonServerEndpoint {
    endpoint: &'static str,
    reason: &'static str,
}

const ALLOWED_NON_SERVER_ENDPOINTS: &[AllowedNonServerEndpoint] = &[AllowedNonServerEndpoint {
    endpoint: "/api/client-telemetry",
    reason: "client telemetry endpoint is an axum route, not a #[macros::server] fn (#712)",
}];

#[derive(Debug, Clone, PartialEq, Eq)]
struct EndpointUse {
    file: String,
    line: usize,
    endpoint: String,
    source: EndpointSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EndpointSource {
    ApiLiteral,
    HelperTail,
}

impl EndpointSource {
    fn label(self) -> &'static str {
        match self {
            Self::ApiLiteral => "/api literal",
            Self::HelperTail => "server-fn helper argument",
        }
    }
}

pub fn run(result: &mut CommandResult) {
    let step = match problems(Path::new(".")) {
        Ok(None) => StepResult::ok(STEP),
        Ok(Some(detail)) => StepResult::fail(STEP).detail(detail),
        Err(error) => StepResult::fail(STEP).detail(error.to_string()),
    };
    result.push(step);
}

fn problems(root: &Path) -> Result<Option<String>> {
    let endpoints = endpoint_inventory(root)?;
    let allowed = allowed_non_server()?;
    allowlist_does_not_overlap_server_inventory(&endpoints, &allowed)?;
    let uses = endpoint_uses(root)?;

    let mut lines = Vec::new();
    for used in uses {
        if endpoints.contains_key(&used.endpoint) || allowed.contains(&used.endpoint) {
            continue;
        }
        lines.push(format!(
            "{}:{}: {} `{}` is not a derived server-fn endpoint and is not explicitly allowed. \
             Rename the Playwright literal or add a written non-server allowlist entry if this is \
             intentionally not a server fn (#712)",
            used.file,
            used.line,
            used.source.label(),
            used.endpoint,
        ));
    }

    Ok((!lines.is_empty()).then(|| lines.join("\n")))
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

fn allowed_non_server() -> Result<BTreeSet<String>> {
    let mut allowed = BTreeSet::new();
    for entry in ALLOWED_NON_SERVER_ENDPOINTS {
        if !entry.endpoint.starts_with("/api/") {
            bail!(
                "{}: non-server endpoint allowlist entries must be full /api paths",
                entry.endpoint
            );
        }
        if entry.reason.trim().is_empty() {
            bail!(
                "{}: non-server endpoint allowlist entry must carry a written reason (#712)",
                entry.endpoint
            );
        }
        if !allowed.insert(entry.endpoint.to_string()) {
            bail!(
                "duplicate non-server endpoint allowlist entry {}",
                entry.endpoint
            );
        }
    }
    Ok(allowed)
}

fn allowlist_does_not_overlap_server_inventory(
    endpoints: &BTreeMap<String, String>,
    allowed: &BTreeSet<String>,
) -> Result<()> {
    for endpoint in allowed {
        if let Some(qualified) = endpoints.get(endpoint) {
            bail!(
                "{endpoint}: non-server endpoint allowlist entry overlaps derived server fn {qualified}"
            );
        }
    }
    Ok(())
}

fn endpoint_uses(root: &Path) -> Result<Vec<EndpointUse>> {
    let files = files::with_extension(&root.join(E2E_ROOT), "ts")
        .with_context(|| format!("scanning {E2E_ROOT} for TypeScript tests"))?;
    let mut out = Vec::new();
    for path in files {
        let source = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        out.extend(endpoint_uses_in_file(&rel, &source)?);
    }
    Ok(out)
}

fn endpoint_uses_in_file(file: &str, source: &str) -> Result<Vec<EndpointUse>> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(file).unwrap_or_else(|_| SourceType::ts());
    let ParserReturn {
        program,
        diagnostics,
        panicked,
        ..
    } = Parser::new(&allocator, source, source_type).parse();
    if panicked || !diagnostics.is_empty() {
        let rendered = diagnostics
            .iter()
            .map(|error| format!("{error:?}"))
            .collect::<Vec<_>>()
            .join("; ");
        bail!("{file}: TypeScript parse failed: {rendered}");
    }

    let mut visitor = EndpointVisitor {
        file,
        source,
        uses: Vec::new(),
    };
    visitor.visit_program(&program);
    Ok(visitor.uses)
}

struct EndpointVisitor<'s> {
    file: &'s str,
    source: &'s str,
    uses: Vec<EndpointUse>,
}

impl<'a> Visit<'a> for EndpointVisitor<'_> {
    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        self.record_api_literal(&it.value, it.span.start as usize);
    }

    fn visit_template_literal(&mut self, it: &TemplateLiteral<'a>) {
        if it.expressions.is_empty() {
            if let Some(quasi) = it.quasis.first() {
                let value = quasi.value.cooked.as_ref().unwrap_or(&quasi.value.raw);
                self.record_api_literal(value, it.span.start as usize);
            }
            return;
        }

        for quasi in &it.quasis {
            self.record_api_literal(&quasi.value.raw, quasi.span.start as usize);
        }
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if helper_callee(&it.callee).is_some_and(|name| HELPER_NAMES.contains(&name))
            && let Some(endpoint) = it.arguments.get(1).and_then(argument_string_value)
        {
            let endpoint = normalize_helper_tail(endpoint.as_ref());
            self.push(
                endpoint,
                it.arguments[1].span().start as usize,
                EndpointSource::HelperTail,
            );
        }
        walk::walk_call_expression(self, it);
    }
}

impl EndpointVisitor<'_> {
    fn record_api_literal(&mut self, value: &str, span_start: usize) {
        for endpoint in concrete_api_endpoints(value) {
            self.push(endpoint, span_start, EndpointSource::ApiLiteral);
        }
    }

    fn push(&mut self, endpoint: String, span_start: usize, source: EndpointSource) {
        self.uses.push(EndpointUse {
            file: self.file.to_string(),
            line: line_number(self.source, span_start),
            endpoint,
            source,
        });
    }
}

fn helper_callee<'a>(callee: &'a Expression<'a>) -> Option<&'a str> {
    match callee {
        Expression::Identifier(ident) => Some(ident.name.as_str()),
        _ => None,
    }
}

fn argument_string_value<'a>(arg: &'a Argument<'a>) -> Option<std::borrow::Cow<'a, str>> {
    match arg {
        Argument::StringLiteral(lit) => Some(std::borrow::Cow::Borrowed(lit.value.as_ref())),
        Argument::TemplateLiteral(tpl) if tpl.expressions.is_empty() => {
            tpl.quasis.first().map(|q| {
                std::borrow::Cow::Borrowed(q.value.cooked.as_ref().unwrap_or(&q.value.raw).as_ref())
            })
        }
        _ => None,
    }
}

fn normalize_helper_tail(value: &str) -> String {
    format!("/api/{}", value.trim_start_matches('/'))
}

fn concrete_api_endpoints(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = value;
    while let Some(idx) = rest.find("/api/") {
        let after = &rest[idx + "/api/".len()..];
        let tail_len = after
            .char_indices()
            .take_while(|(_, ch)| is_endpoint_char(*ch))
            .map(|(i, ch)| i + ch.len_utf8())
            .last()
            .unwrap_or(0);
        if tail_len > 0 {
            let tail = &after[..tail_len];
            if valid_api_tail(tail) {
                out.push(format!("/api/{tail}"));
            }
        }
        rest = &after[tail_len..];
    }
    out
}

fn valid_api_tail(tail: &str) -> bool {
    !tail.is_empty() && tail.chars().all(is_endpoint_char)
}

fn is_endpoint_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '/')
}

fn line_number(source: &str, byte_offset: usize) -> usize {
    source[..byte_offset.min(source.len())]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoints(values: &[EndpointUse]) -> Vec<(&str, EndpointSource)> {
        values
            .iter()
            .map(|v| (v.endpoint.as_str(), v.source))
            .collect()
    }

    #[test]
    fn extracts_valid_full_endpoint_literals() {
        let uses = endpoint_uses_in_file(
            "end2end/tests/posts.spec.ts",
            r#"await page.request.post(`${BASE_URL}/api/posts/create`);
               await page.route("**/api/posts/update", () => {});"#,
        )
        .unwrap();
        assert_eq!(
            endpoints(&uses),
            vec![
                ("/api/posts/create", EndpointSource::ApiLiteral),
                ("/api/posts/update", EndpointSource::ApiLiteral),
            ]
        );
    }

    #[test]
    fn ignores_dynamic_helper_template() {
        let uses = endpoint_uses_in_file(
            "end2end/tests/helpers.ts",
            r#"await page.route(`**/api/${endpoint}`, () => {});"#,
        )
        .unwrap();
        assert!(uses.is_empty());
    }

    #[test]
    fn extracts_helper_endpoint_tails() {
        let uses = endpoint_uses_in_file(
            "end2end/tests/auth.spec.ts",
            r#"await failServerFn(page, "auth/login");
               await stallServerFn(page, `registration/register`);"#,
        )
        .unwrap();
        assert_eq!(
            endpoints(&uses),
            vec![
                ("/api/auth/login", EndpointSource::HelperTail),
                ("/api/registration/register", EndpointSource::HelperTail),
            ]
        );
    }

    #[test]
    fn ignores_comments() {
        let uses = endpoint_uses_in_file(
            "end2end/tests/posts.spec.ts",
            "// POST /api/posts/create is documented here\n",
        )
        .unwrap();
        assert!(uses.is_empty());
    }

    #[test]
    fn stale_full_endpoint_fails() {
        let tmp = fixture_root(&["/api/posts/create"], &["/api/posts/cretae"], &[]);
        let detail = problems(tmp.path()).unwrap().expect("problem");
        assert!(detail.contains("/api/posts/cretae"), "{detail}");
        assert!(detail.contains("/api literal"), "{detail}");
    }

    #[test]
    fn stale_helper_endpoint_tail_fails() {
        let tmp = fixture_root(&["/api/auth/login"], &[], &["auth/logni"]);
        let detail = problems(tmp.path()).unwrap().expect("problem");
        assert!(detail.contains("/api/auth/logni"), "{detail}");
        assert!(detail.contains("server-fn helper argument"), "{detail}");
    }

    #[test]
    fn malformed_helper_endpoint_tail_fails() {
        let tmp = fixture_root(&["/api/auth/login"], &[], &["auth-login"]);
        let detail = problems(tmp.path()).unwrap().expect("problem");
        assert!(detail.contains("/api/auth-login"), "{detail}");
        assert!(detail.contains("server-fn helper argument"), "{detail}");
    }

    #[test]
    fn valid_literals_and_allowed_non_server_endpoint_pass() {
        let tmp = fixture_root(
            &["/api/posts/create"],
            &["/api/posts/create", "/api/client-telemetry"],
            &["posts/create"],
        );
        assert_eq!(problems(tmp.path()).unwrap(), None);
    }

    #[test]
    fn unallowed_non_server_endpoint_fails() {
        let tmp = fixture_root(&[], &["/api/not-a-server-fn"], &[]);
        let detail = problems(tmp.path()).unwrap().expect("problem");
        assert!(detail.contains("/api/not-a-server-fn"), "{detail}");
        assert!(detail.contains("not explicitly allowed"), "{detail}");
    }

    #[test]
    fn allowed_non_server_entries_are_unique() {
        let allowed = allowed_non_server().unwrap();
        assert!(allowed.contains("/api/client-telemetry"));
    }

    #[test]
    fn allowlist_cannot_overlap_server_inventory() {
        let endpoints = BTreeMap::from([(
            "/api/posts/create".to_string(),
            "web.posts.create".to_string(),
        )]);
        let allowed = BTreeSet::from(["/api/posts/create".to_string()]);
        let err = allowlist_does_not_overlap_server_inventory(&endpoints, &allowed)
            .expect_err("overlap must fail");
        assert!(
            err.to_string().contains("overlaps derived server fn"),
            "{err:?}"
        );
    }

    fn fixture_root(
        server_endpoints: &[&str],
        full_literals: &[&str],
        helper_tails: &[&str],
    ) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("web/src")).expect("mkdir web src");
        for endpoint in server_endpoints {
            let tail = endpoint.strip_prefix("/api/").expect("full endpoint");
            let (vertical, ident) = tail.split_once('/').expect("vertical/op");
            let dir = tmp.path().join("web/src").join(vertical);
            std::fs::create_dir_all(&dir).expect("mkdir web vertical");
            std::fs::write(
                dir.join("api.rs"),
                format!("#[macros::server]\npub async fn {ident}() {{}}\n"),
            )
            .expect("write web api");
        }
        let e2e = tmp.path().join(E2E_ROOT);
        std::fs::create_dir_all(&e2e).expect("mkdir e2e");
        let mut ts = String::new();
        for literal in full_literals {
            ts.push_str(&format!("await page.route(\"**{literal}\", () => {{}});\n"));
        }
        for tail in helper_tails {
            ts.push_str(&format!("await failServerFn(page, \"{tail}\");\n"));
        }
        std::fs::write(e2e.join("fixture.spec.ts"), ts).expect("write e2e");
        tmp
    }
}
