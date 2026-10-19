//! The `e2e-collector-config` static check: the shared collector configuration may
//! expose only the two OTLP receivers whose host endpoints `e2e-local` supplies per
//! invocation, with separate trace/log file pipelines under the capture contract.
//! Any broader component surface needs an explicit ownership decision.

use std::collections::BTreeSet;

use serde_yaml::Value;
use syn::{Expr, ImplItem, Item, Lit, Pat, Stmt, Type};

use crate::result::{CommandResult, StepResult};

const STEP: &str = "e2e-collector-config";
const CONFIG: &str = "end2end/otel-collector.yaml";
const POLICY: &str = "nix/otel-processors.yaml";
// Digest of the reviewed JSON value (sorted object keys, compact encoding).
// Policy changes need new semantic wire proof, not just a matching processor name.
const POLICY_DIGEST: &str = "70f370a4dbf469027245f6ebc74dd3c664235bd19094d6c919f0b06d7c973f53";

fn policy_problem(source: &str) -> Option<String> {
    use sha2::{Digest as _, Sha256};
    let value: serde_json::Value = match serde_yaml::from_str(source) {
        Ok(value) => value,
        Err(error) => return Some(format!("{POLICY}: invalid YAML: {error}")),
    };
    let bytes = match serde_json::to_vec(&value) {
        Ok(bytes) => bytes,
        Err(error) => return Some(format!("{POLICY}: cannot encode JSON: {error}")),
    };
    (Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        != POLICY_DIGEST)
        .then(|| format!("{POLICY}: policy differs from the reviewed privacy transform"))
}
const CAPTURE_SOURCE: &str = "host/src/capture.rs";
const GRPC_ENDPOINT: &str = "${env:OTELCOL_GRPC_ENDPOINT}";
const HTTP_ENDPOINT: &str = "${env:OTELCOL_HTTP_ENDPOINT}";

fn capture_exporter_path(source: &str, stream: &str) -> Result<String, String> {
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
                        if segments.as_slice() != ["Stream", stream] {
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
        otel_filename.ok_or_else(|| format!("`Stream::{stream}` filename mapping not found"))?;
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
pub fn problems(config_source: &str, capture_source: &str, policy_source: &str) -> Option<String> {
    if let Some(problem) = policy_problem(policy_source) {
        return Some(problem);
    }
    let config: Value = match serde_yaml::from_str(config_source) {
        Ok(config) => config,
        Err(error) => return Some(format!("{CONFIG}: invalid YAML: {error}")),
    };
    let capture_path = match capture_exporter_path(capture_source, "Otel") {
        Ok(path) => path,
        Err(error) => return Some(format!("{CAPTURE_SOURCE}: {error}")),
    };
    let log_path = match capture_exporter_path(capture_source, "OtelLogs") {
        Ok(path) => path,
        Err(error) => return Some(format!("{CAPTURE_SOURCE}: {error}")),
    };
    for stream in ["Otel", "Diag", "Mail", "WebSub"] {
        let other_path = match capture_exporter_path(capture_source, stream) {
            Ok(path) => path,
            Err(error) => return Some(format!("{CAPTURE_SOURCE}: {error}")),
        };
        if other_path == log_path {
            return Some(format!(
                "{CAPTURE_SOURCE}: log capture must not share `Stream::{stream}` output"
            ));
        }
    }
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
    expect_keys(
        &config["exporters"],
        "exporters",
        &["file", "file/logs"],
        &mut found,
    );
    for (exporter, path) in [("file", capture_path), ("file/logs", log_path)] {
        let key = format!("exporters.{exporter}");
        expect_keys(&config["exporters"][exporter], &key, &["path"], &mut found);
        expect_string(
            &config["exporters"][exporter]["path"],
            &format!("{key}.path"),
            &path,
            &mut found,
        );
    }

    expect_keys(&config["service"], "service", &["pipelines"], &mut found);
    expect_keys(
        &config["service"]["pipelines"],
        "service.pipelines",
        &["traces", "logs"],
        &mut found,
    );
    for (signal, exporter) in [("traces", "file"), ("logs", "file/logs")] {
        let pipeline = &config["service"]["pipelines"][signal];
        expect_keys(
            pipeline,
            &format!("service.pipelines.{signal}"),
            &["exporters", "processors", "receivers"],
            &mut found,
        );
        expect_sequence(
            &pipeline["receivers"],
            &format!("service.pipelines.{signal}.receivers"),
            &["otlp"],
            &mut found,
        );
        expect_sequence(
            &pipeline["processors"],
            &format!("service.pipelines.{signal}.processors"),
            &["transform/jaunder_privacy", "batch"],
            &mut found,
        );
        expect_sequence(
            &pipeline["exporters"],
            &format!("service.pipelines.{signal}.exporters"),
            &[exporter],
            &mut found,
        );
    }

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
    let policy_source = match std::fs::read_to_string(POLICY) {
        Ok(source) => source,
        Err(error) => {
            result.push(StepResult::fail(STEP).detail(format!("cannot read {POLICY}: {error}")));
            return;
        }
    };
    match problems(&config_source, &capture_source, &policy_source) {
        Some(detail) => result.push(StepResult::fail(STEP).detail(detail)),
        None => result.push(StepResult::ok(STEP)),
    }
}

#[cfg(test)]
mod tests {
    use super::{GRPC_ENDPOINT, HTTP_ENDPOINT};
    const POLICY: &str = include_str!("../../../nix/otel-processors.yaml");
    fn problems(config: &str, capture: &str) -> Option<String> {
        super::problems(config, capture, POLICY)
    }

    const CAPTURE_SOURCE: &str = r#"
pub const DIR_ENV: &str = "CAPTURE_ROOT";
enum Stream { Mail, WebSub, Diag, Otel, OtelLogs }
impl Stream {
    pub fn filename(self) -> &'static str {
        match self {
            Stream::Mail => "mail.jsonl",
            Stream::WebSub => "websub.jsonl",
            Stream::Diag => "diag.log",
            Stream::Otel => "traces.jsonl",
            Stream::OtelLogs => "logs.jsonl",
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
  file/logs:
    path: ${{env:CAPTURE_ROOT}}/logs.jsonl
service:
  pipelines:
    traces:
      receivers: [otlp]
      processors: [transform/jaunder_privacy, batch]
      exporters: [file]
    logs:
      receivers: [otlp]
      processors: [transform/jaunder_privacy, batch]
      exporters: [file/logs]
"#
        )
    }

    #[test]
    fn accepts_the_owned_collector_surface() {
        assert_eq!(problems(&valid_config(), CAPTURE_SOURCE), None);
    }

    #[test]
    fn rejects_missing_malformed_or_changed_policy_and_raw_bypasses() {
        for policy in [
            "",
            "{",
            "{}",
            &POLICY.replace("propagate", "ignore"),
            &POLICY.replace("host::error", "other::error"),
        ] {
            assert!(super::problems(&valid_config(), CAPTURE_SOURCE, policy).is_some());
        }
        for processors in [
            "batch",
            "batch, transform/jaunder_privacy",
            "transform/jaunder_privacy",
            "transform/jaunder_privacy, batch, other",
        ] {
            assert!(
                problems(
                    &valid_config().replace("transform/jaunder_privacy, batch", processors),
                    CAPTURE_SOURCE
                )
                .is_some()
            );
        }
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
    fn accepts_the_repository_collector_and_capture_contract() {
        assert_eq!(
            problems(
                include_str!("../../../end2end/otel-collector.yaml"),
                include_str!("../../../host/src/capture.rs"),
            ),
            None,
        );
    }

    #[test]
    fn rejects_log_mixing_and_broadened_pipelines() {
        for source in [
            valid_config().replace("exporters: [file/logs]", "exporters: [file]"),
            valid_config().replace(
                "${env:CAPTURE_ROOT}/logs.jsonl",
                "${env:CAPTURE_ROOT}/traces.jsonl",
            ),
            valid_config().replace("receivers: [otlp]", "receivers: [journald]"),
            valid_config().replace(
                "processors: [transform/jaunder_privacy, batch]",
                "processors: [batch, transform]",
            ),
            valid_config().replace("    logs:\n", "    metrics:\n"),
            valid_config().replace("  file/logs:\n", "  debug: {}\n  file/logs:\n"),
        ] {
            assert!(problems(&source, CAPTURE_SOURCE).is_some(), "{source}");
        }
    }

    #[test]
    fn rejects_metrics_backends_in_the_capture_only_profile() {
        for exporter in [
            "prometheusremotewrite: {endpoint: 'http://127.0.0.1:8428/metrics/api/v1/write'}",
            "otlphttp/victoriametrics: {metrics_endpoint: 'http://127.0.0.1:8428/metrics/opentelemetry/v1/metrics'}",
        ] {
            let source =
                valid_config().replace("exporters:\n", &format!("exporters:\n  {exporter}\n"));
            assert!(problems(&source, CAPTURE_SOURCE).is_some(), "{source}");
        }
        let source = valid_config().replace(
            "  pipelines:\n",
            "  pipelines:\n    metrics:\n      receivers: [otlp]\n      processors: [batch]\n      exporters: [otlphttp/victoriametrics]\n",
        );
        assert!(problems(&source, CAPTURE_SOURCE).is_some(), "{source}");
    }

    #[test]
    fn rejects_missing_or_shared_log_filename_contract() {
        for source in [
            CAPTURE_SOURCE.replace("Stream::OtelLogs => \"logs.jsonl\",", ""),
            CAPTURE_SOURCE.replace("\"logs.jsonl\"", "\"traces.jsonl\""),
            CAPTURE_SOURCE.replace("\"logs.jsonl\"", "\"diag.log\""),
            CAPTURE_SOURCE.replace("\"logs.jsonl\"", "\"mail.jsonl\""),
            CAPTURE_SOURCE.replace("\"logs.jsonl\"", "\"websub.jsonl\""),
        ] {
            assert!(problems(&valid_config(), &source).is_some(), "{source}");
        }
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
