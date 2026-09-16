//! The `e2e-collector-config` static check: the shared collector configuration may
//! expose only the two OTLP receivers whose host endpoints `e2e-local` supplies per
//! invocation. Any broader component surface needs an explicit ownership decision.

use std::collections::BTreeSet;

use serde_yaml::Value;

use crate::result::{CommandResult, StepResult};

const STEP: &str = "e2e-collector-config";
const CONFIG: &str = "end2end/otel-collector.yaml";
const GRPC_ENDPOINT: &str = "${env:OTELCOL_GRPC_ENDPOINT}";
const HTTP_ENDPOINT: &str = "${env:OTELCOL_HTTP_ENDPOINT}";

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

fn expect_string(value: &Value, path: &str, expected: Option<&str>, found: &mut Vec<String>) {
    let Some(actual) = value.as_str() else {
        found.push(format!("{CONFIG}: `{path}` must be a string"));
        return;
    };
    if let Some(expected) = expected
        && actual != expected
    {
        found.push(format!(
            "{CONFIG}: `{path}` must be `{expected}` so e2e-local owns the listener; found `{actual}`"
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
pub fn problems(source: &str) -> Option<String> {
    let config: Value = match serde_yaml::from_str(source) {
        Ok(config) => config,
        Err(error) => return Some(format!("{CONFIG}: invalid YAML: {error}")),
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
            Some(endpoint),
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
        None,
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
    let step = match std::fs::read_to_string(CONFIG) {
        Err(error) => StepResult::fail(STEP).detail(format!("cannot read {CONFIG}: {error}")),
        Ok(source) => match problems(&source) {
            Some(detail) => StepResult::fail(STEP).detail(detail),
            None => StepResult::ok(STEP),
        },
    };
    result.push(step);
}

#[cfg(test)]
mod tests {
    use super::{GRPC_ENDPOINT, HTTP_ENDPOINT, problems};

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
    path: capture-output
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
        assert_eq!(problems(&valid_config()), None);
    }

    #[test]
    fn rejects_a_fixed_receiver_endpoint() {
        let source = valid_config().replace(GRPC_ENDPOINT, "127.0.0.1:4317");
        let detail = problems(&source).expect("fixed endpoint must fail");
        assert!(detail.contains("e2e-local owns the listener"), "{detail}");
    }

    #[test]
    fn rejects_an_added_component_surface() {
        let source = valid_config().replace(
            "processors:\n",
            "extensions:\n  health_check: {}\nprocessors:\n",
        );
        let detail = problems(&source).expect("extension must fail");
        assert!(detail.contains("`root` keys"), "{detail}");
    }

    #[test]
    fn rejects_malformed_yaml() {
        let detail = problems("receivers: [").expect("malformed YAML must fail");
        assert!(detail.contains("invalid YAML"), "{detail}");
    }
}
