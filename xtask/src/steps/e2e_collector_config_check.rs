//! The `e2e-collector-config` static check: the shared collector configuration may
//! expose only the two OTLP receivers whose host endpoints `e2e-local` supplies per
//! invocation. Any broader component surface needs an explicit ownership decision.

use std::collections::BTreeSet;

use serde_yaml::Value;
use syn::{Expr, ImplItem, Item, Lit, Pat, Stmt, Type};

use crate::result::{CommandResult, StepResult};

const STEP: &str = "e2e-collector-config";
const CONFIG: &str = "end2end/otel-collector.yaml";
const CAPTURE_SOURCE: &str = "host/src/capture.rs";
const GRPC_ENDPOINT: &str = "${env:OTELCOL_GRPC_ENDPOINT}";
const HTTP_ENDPOINT: &str = "${env:OTELCOL_HTTP_ENDPOINT}";

fn capture_exporter_path(source: &str) -> Result<String, String> {
    let file = syn::parse_file(source).map_err(|error| format!("invalid Rust: {error}"))?;
    let mut directory_env = None;
    let mut otel_filename = None;

    for item in file.items {
        match item {
            Item::Const(item) if item.ident == "DIR_ENV" => {
                if let Expr::Lit(expression) = item.expr.as_ref()
                    && let Lit::Str(value) = &expression.lit
                {
                    directory_env = Some(value.value());
                }
            }
            Item::Impl(item) => {
                let Type::Path(self_type) = item.self_ty.as_ref() else {
                    continue;
                };
                if !self_type.path.is_ident("Stream") {
                    continue;
                }
                for member in item.items {
                    let ImplItem::Fn(method) = member else {
                        continue;
                    };
                    if method.sig.ident != "filename" {
                        continue;
                    }
                    let Some(Stmt::Expr(Expr::Match(expression), _)) = method.block.stmts.last()
                    else {
                        continue;
                    };
                    for arm in &expression.arms {
                        let Pat::Path(pattern) = &arm.pat else {
                            continue;
                        };
                        let segments = pattern
                            .path
                            .segments
                            .iter()
                            .map(|segment| segment.ident.to_string())
                            .collect::<Vec<_>>();
                        if segments.as_slice() != ["Stream", "Otel"] {
                            continue;
                        }
                        if let Expr::Lit(expression) = arm.body.as_ref()
                            && let Lit::Str(value) = &expression.lit
                        {
                            otel_filename = Some(value.value());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let directory_env =
        directory_env.ok_or_else(|| "public `DIR_ENV` string constant not found".to_owned())?;
    let otel_filename =
        otel_filename.ok_or_else(|| "`Stream::Otel` filename mapping not found".to_owned())?;
    Ok(format!("${{env:{directory_env}}}/{otel_filename}"))
}

fn expect_keys(value: &Value, path: &str, expected: &[&str], found: &mut Vec<String>) {
    let Some(mapping) = value.as_mapping() else {
        found.push(format!("{CONFIG}: `{path}` must be a mapping"));
        return;
    };
    let mut actual = BTreeSet::new();
    for key in mapping.keys() {
        let Some(key) = key.as_str() else {
            found.push(format!("{CONFIG}: `{path}` contains a non-string key"));
            return;
        };
        actual.insert(key);
    }
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        found.push(format!(
            "{CONFIG}: `{path}` keys must be {expected:?}; found {actual:?}"
        ));
    }
}

fn expect_string(value: &Value, path: &str, expected: &str, found: &mut Vec<String>) {
    let Some(actual) = value.as_str() else {
        found.push(format!("{CONFIG}: `{path}` must be a string"));
        return;
    };
    if actual != expected {
        found.push(format!(
            "{CONFIG}: `{path}` must be `{expected}`; found `{actual}`"
        ));
    }
}

fn expect_sequence(value: &Value, path: &str, expected: &[&str], found: &mut Vec<String>) {
    let Some(sequence) = value.as_sequence() else {
        found.push(format!("{CONFIG}: `{path}` must be a sequence"));
        return;
    };
    let actual = sequence
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if actual.len() != sequence.len() {
        found.push(format!("{CONFIG}: `{path}` must contain only strings"));
    } else if actual != expected {
        found.push(format!(
            "{CONFIG}: `{path}` must be {expected:?}; found {actual:?}"
        ));
    }
}

/// Return every collector-surface violation. Parsing and shape checks fail closed so
/// malformed or newly broadened configuration cannot silently escape the host gate.
pub fn problems(config_source: &str, capture_source: &str) -> Option<String> {
    let config: Value = match serde_yaml::from_str(config_source) {
        Ok(config) => config,
        Err(error) => return Some(format!("{CONFIG}: invalid YAML: {error}")),
    };
    let capture_path = match capture_exporter_path(capture_source) {
        Ok(path) => path,
        Err(error) => return Some(format!("{CAPTURE_SOURCE}: {error}")),
    };
    let mut found = Vec::new();

    expect_keys(
        &config,
        "root",
        &["exporters", "processors", "receivers", "service"],
        &mut found,
    );
    expect_keys(&config["receivers"], "receivers", &["otlp"], &mut found);
    expect_keys(
        &config["receivers"]["otlp"],
        "receivers.otlp",
        &["protocols"],
        &mut found,
    );
    let protocols = &config["receivers"]["otlp"]["protocols"];
    expect_keys(
        protocols,
        "receivers.otlp.protocols",
        &["grpc", "http"],
        &mut found,
    );
    for (protocol, endpoint) in [("grpc", GRPC_ENDPOINT), ("http", HTTP_ENDPOINT)] {
        let path = format!("receivers.otlp.protocols.{protocol}");
        expect_keys(&protocols[protocol], &path, &["endpoint"], &mut found);
        expect_string(
            &protocols[protocol]["endpoint"],
            &format!("{path}.endpoint"),
            endpoint,
            &mut found,
        );
    }

    expect_keys(&config["processors"], "processors", &["batch"], &mut found);
    expect_keys(
        &config["processors"]["batch"],
        "processors.batch",
        &[],
        &mut found,
    );
    expect_keys(&config["exporters"], "exporters", &["file"], &mut found);
    expect_keys(
        &config["exporters"]["file"],
        "exporters.file",
        &["path"],
        &mut found,
    );
    expect_string(
        &config["exporters"]["file"]["path"],
        "exporters.file.path",
        &capture_path,
        &mut found,
    );

    expect_keys(&config["service"], "service", &["pipelines"], &mut found);
    expect_keys(
        &config["service"]["pipelines"],
        "service.pipelines",
        &["traces"],
        &mut found,
    );
    let traces = &config["service"]["pipelines"]["traces"];
    expect_keys(
        traces,
        "service.pipelines.traces",
        &["exporters", "processors", "receivers"],
        &mut found,
    );
    expect_sequence(
        &traces["receivers"],
        "service.pipelines.traces.receivers",
        &["otlp"],
        &mut found,
    );
    expect_sequence(
        &traces["processors"],
        "service.pipelines.traces.processors",
        &["batch"],
        &mut found,
    );
    expect_sequence(
        &traces["exporters"],
        "service.pipelines.traces.exporters",
        &["file"],
        &mut found,
    );

    (!found.is_empty()).then(|| found.join("\n"))
}

pub fn run(result: &mut CommandResult) {
    let config_source = match std::fs::read_to_string(CONFIG) {
        Ok(source) => source,
        Err(error) => {
            result.push(StepResult::fail(STEP).detail(format!("cannot read {CONFIG}: {error}")));
            return;
        }
    };
    let capture_source = match std::fs::read_to_string(CAPTURE_SOURCE) {
        Ok(source) => source,
        Err(error) => {
            result.push(
                StepResult::fail(STEP).detail(format!("cannot read {CAPTURE_SOURCE}: {error}")),
            );
            return;
        }
    };
    match problems(&config_source, &capture_source) {
        Some(detail) => result.push(StepResult::fail(STEP).detail(detail)),
        None => result.push(StepResult::ok(STEP)),
    }
}

#[cfg(test)]
mod tests {
    use super::{GRPC_ENDPOINT, HTTP_ENDPOINT, problems};

    const CAPTURE_SOURCE: &str = r#"
pub const DIR_ENV: &str = "CAPTURE_ROOT";
enum Stream { Mail, Otel }
impl Stream {
    pub fn filename(self) -> &'static str {
        match self {
            Stream::Mail => "mail.jsonl",
            Stream::Otel => "traces.jsonl",
        }
    }
}
"#;

    fn valid_config() -> String {
        format!(
            r#"receivers:
  otlp:
    protocols:
      grpc:
        endpoint: {GRPC_ENDPOINT}
      http:
        endpoint: {HTTP_ENDPOINT}
processors:
  batch: {{}}
exporters:
  file:
    path: ${{env:CAPTURE_ROOT}}/traces.jsonl
service:
  pipelines:
    traces:
      receivers: [otlp]
      processors: [batch]
      exporters: [file]
"#
        )
    }

    #[test]
    fn accepts_the_owned_collector_surface() {
        assert_eq!(problems(&valid_config(), CAPTURE_SOURCE), None);
    }

    #[test]
    fn rejects_a_fixed_receiver_endpoint() {
        let source = valid_config().replace(GRPC_ENDPOINT, "127.0.0.1:4317");
        let detail = problems(&source, CAPTURE_SOURCE).expect("fixed endpoint must fail");
        assert!(detail.contains(GRPC_ENDPOINT), "{detail}");
    }

    #[test]
    fn rejects_a_different_capture_path() {
        let source = valid_config().replace("${env:CAPTURE_ROOT}/traces.jsonl", "other.jsonl");
        let detail = problems(&source, CAPTURE_SOURCE).expect("capture path must fail");
        assert!(
            detail.contains("${env:CAPTURE_ROOT}/traces.jsonl"),
            "{detail}"
        );
    }

    #[test]
    fn rejects_an_added_component_surface() {
        let source = valid_config().replace(
            "processors:\n",
            "extensions:\n  health_check: {}\nprocessors:\n",
        );
        let detail = problems(&source, CAPTURE_SOURCE).expect("extension must fail");
        assert!(detail.contains("`root` keys"), "{detail}");
    }

    #[test]
    fn rejects_malformed_yaml() {
        let detail = problems("receivers: [", CAPTURE_SOURCE).expect("malformed YAML must fail");
        assert!(detail.contains("invalid YAML"), "{detail}");
    }

    #[test]
    fn rejects_an_unreadable_capture_contract() {
        let detail = problems(&valid_config(), "not Rust").expect("capture contract must fail");
        assert!(detail.contains("invalid Rust"), "{detail}");
    }
}
