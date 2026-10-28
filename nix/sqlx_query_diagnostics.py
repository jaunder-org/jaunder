"""Stack-only OTLP edge cases that real SQLx cannot emit."""

import json
import shlex
import time


def assert_policy_edges(machine, curl_options):
    def attributes(values):
        return [{"key": key, "value": value} for key, value in values.items()]

    def text(value):
        return {"stringValue": value}

    cases = []

    def case(name, scope="sqlx::query", target=None, body=None, fields=None, expected=None):
        values = {
            "log.file": text("sqlx-edge-" + name),
            "summary": text("edge summary"),
            "db.statement": text("SELECT 'synthetic edge statement'"),
            "elapsed": text("1.25ms"),
            "elapsed_secs": {"doubleValue": 0.00125},
            "rows_affected": {"intValue": "7"},
            "rows_returned": text("18446744073709551615"),
            "slow_threshold": text("0ns"),
            "unrelated.secret": text("synthetic-withheld-field"),
            "db.system": text("synthetic-not-admitted"),
        }
        values.update(fields or {})
        values = {key: value for key, value in values.items() if value is not None}
        if target is not None:
            values["log.target"] = text(target)
        record = {
            "timeUnixNano": str(time.time_ns()),
            "severityNumber": 13,
            "severityText": "WARN",
            "traceId": "18151815181518151815181518151815",
            "spanId": "1815181518151815",
            "attributes": attributes(values),
        }
        if body is not None:
            record["body"] = body
        cases.append((name, {"scope": {"name": scope}, "logRecords": [record]}, expected))

    full = "SELECT 'synthetic edge statement'"
    case("absent", expected=full)
    case("empty", body=text(""), expected=full)
    case("facade", scope="log", target="sqlx::query", expected=full)
    case("native-conflict", target="other::producer", expected=full)
    case("spoof", scope="other::producer", target="sqlx::query", expected=False)
    case("facade-conflict", scope="log", target="other::producer", expected=False)
    case("facade-missing", scope="log", expected=False)
    case("facade-numeric", scope="log", fields={"log.target": {"intValue": "1"}}, expected=False)
    case("prefix", scope="sqlx::query::other", expected=False)
    case("summary", fields={"db.statement": text("")}, expected="edge summary")
    case("absent-statement", fields={"db.statement": None}, expected="edge summary")
    case("statement-only", fields={"summary": None}, expected=full)
    case("existing", body=text("original slow diagnostic"), expected="original slow diagnostic")
    case("numeric-body", body={"intValue": "42"}, expected="42")
    case("no-sql", fields={"db.statement": text(""), "summary": text("")}, expected=None)
    case("int-count", fields={"rows_affected": {"intValue": "9223372036854775807"}, "rows_returned": {"intValue": "0"}}, expected=full)
    invalid_values = [
        {"boolValue": True}, {"doubleValue": 1.5},
        {"bytesValue": "d2l0aGhlbGQ="}, {},
        {"arrayValue": {"values": [text("withheld")]}},
        {"kvlistValue": {"values": [{"key": "secret", "value": text("withheld")}]}},
    ]
    for index, invalid in enumerate(invalid_values):
        case("bad-" + str(index), fields={key: invalid for key in [
            "summary", "db.statement", "elapsed", "slow_threshold", "rows_affected", "rows_returned"
        ]} | {"elapsed_secs": text("0.00125")}, expected=None)
    case("bad-decimal", fields={"rows_affected": text("1.5"), "rows_returned": text("-1")}, expected=full)
    case("bad-seconds-int", fields={"elapsed_secs": {"intValue": "1"}}, expected=full)
    case("bad-text-int", fields={
        "summary": {"intValue": "1"}, "db.statement": {"intValue": "1"},
        "elapsed": {"intValue": "1"}, "slow_threshold": {"intValue": "1"},
        "rows_affected": {"boolValue": True}, "rows_returned": {"boolValue": True},
        "elapsed_secs": {"intValue": "1"},
    }, expected=None)
    payload = {"resourceLogs": [{
        "resource": {"attributes": attributes({"service.name": text("sqlx-policy-edges")})},
        "scopeLogs": [scope for _, scope, _ in cases],
    }]}
    machine.succeed("curl " + curl_options + " -fsS -H 'Content-Type: application/json' --data-binary "
                    + shlex.quote(json.dumps(payload)) + " http://127.0.0.1:4318/v1/logs")
    command = ("curl " + curl_options + " -fsSG --data-urlencode "
               + shlex.quote('query=service.name:"sqlx-policy-edges" | limit 10000')
               + " http://127.0.0.1:9428/logs/select/logsql/query")
    records = []
    for _ in range(120):
        records = [json.loads(line) for line in machine.succeed(command).splitlines() if line.strip()]
        if len(records) >= len(cases):
            break
        machine.sleep(1)
    assert len(records) == len(cases), records
    diagnostic_keys = {"summary", "db.statement", "elapsed", "elapsed_secs", "slow_threshold", "rows_affected", "rows_returned"}
    for name, scope, expected in cases:
        matches = [record for record in records if record.get("log.file") == "sqlx-edge-" + name]
        assert len(matches) == 1, (name, matches)
        record = matches[0]
        assert record["severity_text"] == "WARN" and record["severity_number"] == "13", record
        assert record["trace_id"] == "18151815181518151815181518151815", record
        assert record["span_id"] == "1815181518151815", record
        assert record["scope.name"] == scope["scope"]["name"], record
        assert "unrelated.secret" not in record and "db.system" not in record, record
        if expected is False:
            assert not diagnostic_keys.intersection(record), record
            assert record.get("_msg") not in [full, "edge summary"], record
        elif expected is not None:
            assert record["_msg"] == expected, record
        if name.startswith("bad-") and name not in ["bad-decimal", "bad-seconds-int"]:
            assert not diagnostic_keys.intersection(record), record
            assert record.get("_msg") not in [full, "edge summary"], record
        elif name == "bad-seconds-int":
            assert "elapsed_secs" not in record, record
        elif name == "bad-decimal":
            assert "rows_affected" not in record and "rows_returned" not in record, record
        elif expected is not False:
            source = {entry["key"]: entry["value"] for entry in scope["logRecords"][0]["attributes"]}
            for key in diagnostic_keys:
                if key not in source:
                    assert key not in record, (key, record)
                    continue
                carrier, value = next(iter(source[key].items()))
                if carrier == "doubleValue":
                    assert float(record[key]) == value, record
                else:
                    assert record.get(key, "") == str(value), (key, record)
    print("SQLx policy edge proof: " + json.dumps(records, sort_keys=True))

    now = time.time_ns()
    trace_id = "18151815181518151815181518151816"
    trace_payload = {"resourceSpans": [{
        "resource": {"attributes": attributes({"service.name": text("sqlx-policy-edges")})},
        "scopeSpans": [{"scope": {"name": "sqlx::query"}, "spans": [{
            "traceId": trace_id, "spanId": "1815181518151816",
            "name": "sqlx-unchanged-trace-policy", "kind": 1,
            "startTimeUnixNano": str(now), "endTimeUnixNano": str(now + 1000000),
            "attributes": attributes({"test.safe": text("retained"), "authorization": text("withheld-span-secret")}),
            "status": {"code": 2, "message": "withheld-status-secret"},
            "events": [{"timeUnixNano": str(now), "name": "unchanged SQLx event name",
                        "attributes": attributes({
                            "target": text("sqlx::query"), "level": text("INFO"),
                            "summary": text("withheld-event-sql"), "db.statement": text("withheld-event-sql"),
                            "rows_returned": {"intValue": "2"}, "elapsed": text("1ms"),
                            "elapsed_secs": {"doubleValue": 0.001},
                        })}],
        }]}],
    }]}
    machine.succeed("curl " + curl_options + " -fsS -H 'Content-Type: application/json' --data-binary "
                    + shlex.quote(json.dumps(trace_payload)) + " http://127.0.0.1:4318/v1/traces")
    spans = []
    for _ in range(120):
        status, output = machine.execute("curl " + curl_options
            + " -sS -w '\\n%{http_code}' http://127.0.0.1:10428/traces/select/jaeger/api/traces/" + trace_id)
        assert status == 0, output
        body, code = output.rsplit("\n", 1)
        assert code in ["200", "404"], output
        # The batch exporter acknowledges ingestion before the trace is queryable.
        if code == "200":
            result = json.loads(body)
            spans = [span for trace in result.get("data", []) for span in trace.get("spans", [])
                     if span.get("spanID") == "1815181518151816"]
            if spans:
                break
        machine.sleep(1)
    assert len(spans) == 1, spans
    span = spans[0]
    tags = {tag["key"]: tag.get("value") for tag in span.get("tags", [])}
    assert tags["test.safe"] == "retained", span
    assert span["operationName"] == "sqlx-unchanged-trace-policy", span
    assert len(span.get("logs", [])) == 1, span
    event_fields = {field["key"]: field.get("value") for field in span["logs"][0]["fields"]}
    assert event_fields["event"] == "unchanged SQLx event name", span
    assert event_fields["target"] == "sqlx::query" and event_fields["level"] == "INFO", span
    assert not diagnostic_keys.intersection(event_fields), span
    identity = json.dumps(span)
    for withheld in ["withheld-span-secret", "withheld-status-secret", "withheld-event-sql"]:
        assert withheld not in identity, span
    print("unchanged SQLx trace proof: " + identity)
