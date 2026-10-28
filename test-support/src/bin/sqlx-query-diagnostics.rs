//! Stack-VM-only real SQLx producer and independent pre-Collector event capture.
//!
//! Built as a feature-gated binary, not an example: examples activate the
//! package's dev-dependencies, including test-only cheap password hashing that
//! correctly cannot be compiled into this release-profile VM artifact.
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use host::telemetry::{TelemetryConfig, TelemetryRawConfig};
use opentelemetry::trace::TraceContextExt as _;
use serde_json::{Map, Value, json};
use sqlx::{ConnectOptions as _, Connection as _, Row as _};
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;

// Four words fit SQLx's summary; the longer query retains exact framing bytes.
const QUERIES: [&str; 2] = [
    "SELECT 1",
    "SELECT 1815 AS synthetic_sqlx_query_diagnostics UNION ALL SELECT 1816 AS synthetic_sqlx_query_diagnostics",
];

#[derive(Clone)]
struct OriginalEvents(Arc<Mutex<Vec<Value>>>);

struct Fields(Map<String, Value>);

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().into(), json!(value));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().into(), json!(value));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().into(), json!(value));
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.insert(field.name().into(), json!(value));
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0
            .insert(field.name().into(), json!(format!("{value:?}")));
    }
}

impl<S: tracing::Subscriber> Layer<S> for OriginalEvents {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if event.metadata().target() != "sqlx::query" {
            return;
        }
        let mut fields = Fields(Map::new());
        event.record(&mut fields);
        let context = opentelemetry::Context::current();
        let span = context.span();
        let ids = span.span_context();
        let Ok(mut originals) = self.0.lock() else {
            eprintln!("SQLx diagnostic event capture mutex poisoned");
            std::process::exit(1);
        };
        originals.push(json!({
            "fields": fields.0,
            "severity_text": event.metadata().level().as_str(),
            "severity_number": if *event.metadata().level() == tracing::Level::WARN { "13" } else { "9" },
            "trace_id": ids.is_valid().then(|| ids.trace_id().to_string()),
            "span_id": ids.is_valid().then(|| ids.span_id().to_string()),
        }));
    }
}

fn telemetry_config() -> anyhow::Result<TelemetryConfig> {
    let config = TelemetryConfig::from_raw(
        false,
        TelemetryRawConfig {
            log_filter: Ok(Some("off".into())),
            rust_log: Ok(None),
            log_format: Ok(None),
            jaunder_otlp_endpoint: std::env::var("JAUNDER_OTEL_EXPORTER_OTLP_ENDPOINT")
                .map(Some)
                .or_else(|error| match error {
                    std::env::VarError::NotPresent => Ok(None),
                    other => Err(other),
                }),
            otlp_endpoint: Ok(None),
            slow_op_ms: Ok(None),
            e2e_seed_process: Ok(None),
        },
    );
    anyhow::ensure!(
        config.otlp_endpoint_configured(),
        "native OTLP endpoint required"
    );
    Ok(config)
}

async fn execute_postgres_queries(
    options: &sqlx::postgres::PgConnectOptions,
    slow: bool,
    executions: &mut Vec<Value>,
) -> anyhow::Result<()> {
    let mut connection = sqlx::PgConnection::connect_with(options).await?;
    for &query in &QUERIES {
        let rows = sqlx::query(query).fetch_all(&mut connection).await?;
        let values = rows
            .iter()
            .map(|row| row.get::<i32, _>(0))
            .collect::<Vec<_>>();
        executions.push(json!({"query": query, "slow": slow, "values": values}));
    }
    connection.close().await?;
    Ok(())
}

async fn execute_sqlite_queries(
    options: &sqlx::sqlite::SqliteConnectOptions,
    slow: bool,
    executions: &mut Vec<Value>,
) -> anyhow::Result<()> {
    let mut connection = sqlx::SqliteConnection::connect_with(options).await?;
    for &query in &QUERIES {
        let rows = sqlx::query(query).fetch_all(&mut connection).await?;
        let values = rows
            .iter()
            .map(|row| row.get::<i32, _>(0))
            .collect::<Vec<_>>();
        executions.push(json!({"query": query, "slow": slow, "values": values}));
    }
    connection.close().await?;
    Ok(())
}

async fn execute_backend_queries(
    database: &str,
    slow: bool,
    threshold: Duration,
    executions: &mut Vec<Value>,
) -> anyhow::Result<()> {
    if database.starts_with("postgres") {
        let options = sqlx::postgres::PgConnectOptions::from_str(database)?
            .log_statements(log::LevelFilter::Info)
            .log_slow_statements(log::LevelFilter::Warn, threshold);
        execute_postgres_queries(&options, slow, executions).await
    } else {
        let options = sqlx::sqlite::SqliteConnectOptions::from_str(database)?
            .log_statements(log::LevelFilter::Info)
            .log_slow_statements(log::LevelFilter::Warn, threshold);
        execute_sqlite_queries(&options, slow, executions).await
    }
}

async fn execute_queries(database: &str) -> anyhow::Result<Vec<Value>> {
    let mut executions = Vec::new();
    for slow in [false, true] {
        let threshold = if slow {
            Duration::ZERO
        } else {
            storage::StorageRuntimeConfig::default().sql_slow_query_threshold()
        };
        execute_backend_queries(database, slow, threshold, &mut executions).await?;
    }
    Ok(executions)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let database = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("database URL required"))?;
    let config = telemetry_config()?;
    let originals = OriginalEvents(Arc::default());
    let guard = host::telemetry::init_tracing_with_layer(&config, Some(originals.clone()));
    let executions = execute_queries(&database).await?;
    drop(guard);
    let originals = originals
        .0
        .lock()
        .map_err(|_| anyhow::anyhow!("SQLx diagnostic event capture mutex poisoned"))?;
    println!(
        "{}",
        json!({"executions": executions, "originals": *originals})
    );
    Ok(())
}
