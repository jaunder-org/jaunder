# ADR-DRAFT: Caddy Logs Reach VictoriaLogs Through the Native Journal Upload Receiver

- Status: proposed
- Date: 2026-11-02
- Issue: [#1850](https://github.com/jaunder-org/jaunder/issues/1850)

## Context

[ADR-0196](../0196-single-host-nixos-deployment-stack.md) established the
managed single-host stack, Caddy ingress, and the loopback Victoria services.
Jaunder's application logs now use the native OTLP/Collector route and shared
`transform/jaunder_privacy` policy
([native OTLP process logs](native-otlp-process-logs.md)). Caddy's own access
and service logs are a separate source: they are not Jaunder application events
and must not be admitted to that Collector policy.

The retained human route approval for `3ea422e6-2db3-4b8b-89e0-994148751fca` /
`1850-native-journal-route` selects the `jaunder` journal namespace and
supported `systemd-journal-upload` sender, initially covering Caddy plus
namespace-internal bookkeeping. It does not select a strict unit filter,
whole-host journal, or Caddy Collector hop. The later human clarification for
`afa90716-9923-413f-aa2d-fdf293fde4d6` / `1850-recovery-replay`, “Accept
possible recovery replay,” accepts that an interrupted native upload may resend
records already stored. Neither approval promises exactly-once delivery or
universal losslessness.

## Decision

The managed stack assigns Caddy to `LogNamespace=jaunder`. Each enabled managed
virtual host—the application host and optional observability host—emits exactly
one native JSON access-log stream to stdout. Set the NixOS vhost `logFormat` to
`null` to disable its default per-vhost file logger; add Caddy's stdout JSON
logger in the managed Caddyfile. Caddy's native redaction for `Cookie`,
`Set-Cookie`, `Authorization`, and `Proxy-Authorization` remains enabled; do not
enable `log_credentials`. Retain the full emitted record and service/error
message without body capture or further projection.

Use NixOS `services.journald.upload` to manage `systemd-journal-upload`, its URL
configuration, persistent state, restart policy, and upstream hardening.
Configure the native receiver base URL as
`http://127.0.0.1:9428/logs/insert/journald`; the pinned uploader appends
`/upload`, reaching VictoriaLogs' `/logs/insert/journald/upload` receiver.
Override `ExecStart` only to add `--namespace=jaunder`, retaining `--save-state`
and the shipped service's state directory, identity, journal-group access,
restart, and hardening. When Caddy runtime credentials are enabled, merge
`LoadCredential` and `RefreshOnReload` into the service configuration without
replacing `LogNamespace`.

The uploader source is the `jaunder` namespace, including any namespace-internal
entries; it is not an exact `caddy.service` filter. Namespace selection does not
narrow the uploader's OS journal permissions. Do not upload the default
namespace or route Caddy through the Collector. Caddy diagnostics are
namespace-aware: use `journalctl --namespace=jaunder -u caddy.service`;
default-namespace `journalctl -u caddy.service` does not show these records.

The receiver and VictoriaLogs remain loopback-only. Preserve native VictoriaLogs
retention and operator overrides, other store listeners, Caddy credential
handling, and Jaunder OTLP/Collector/privacy behavior unchanged.

## Consequences

Caddy's emitted access records—including URI, IP, headers, TLS details when
present, `bytes_read`, `user_id`, timing, status, size, and response headers—and
emitted service/error messages become available in VictoriaLogs, subject to
Caddy's existing four-header redaction. `bytes_read` is a count, not
request-body capture. Operator-only sensitive values that Caddy itself emits may
persist for the native VictoriaLogs retention period.

The supported upload path uses a saved journal cursor and retries after receiver
interruption. It can replay a previously uploaded record after a failed request;
a focused VM observed such a replay while recovering its test records. The stack
therefore promises no duplicate logger/duplicate ingestion in steady state, but
does not promise exactly-once delivery, no replay during recovery, or universal
losslessness. No deduplication layer is introduced.

The focused VM and existing credential-reload VM prove their tested
configurations and paths, not every host event or every failure schedule. See
[the current architecture](../../ARCHITECTURE.md#what-the-flake-ships) for the
implemented projection; the delivery and privacy boundaries above remain
explicit.
