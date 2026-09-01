use std::{fs, io};

use storage::StorageRuntimeConfig;

/// Context added when a command requires an initialized application database.
pub(super) const INIT_FIRST_CONTEXT: &str =
    "database could not be opened; run `jaunder init` first";

fn inherited(name: &str) -> Result<Option<String>, std::env::VarError> {
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Resolves the application connection snapshot at the command boundary.
///
/// Bootstrap commands intentionally do not call this: their credentials are
/// explicit administrative inputs, never application password overrides.
/// `SQLite` has no `PostgreSQL` credential path, so it must not observe a broken
/// `PostgreSQL` password file or variable.
fn storage_runtime_config_from_raw(
    database: &storage::DbConnectOptions,
    sql_slow_ms: Result<Option<String>, std::env::VarError>,
    password_file: Result<Option<io::Result<String>>, std::env::VarError>,
    password: Result<Option<String>, std::env::VarError>,
) -> Result<StorageRuntimeConfig, storage::PostgresPasswordError> {
    match database {
        storage::DbConnectOptions::Sqlite(_) => {
            StorageRuntimeConfig::from_raw(sql_slow_ms, Ok(None), Ok(None))
        }
        storage::DbConnectOptions::Postgres { .. } => {
            StorageRuntimeConfig::from_raw(sql_slow_ms, password_file, password)
        }
    }
}

pub(super) fn storage_runtime_config(
    database: &storage::DbConnectOptions,
) -> Result<StorageRuntimeConfig, storage::PostgresPasswordError> {
    let sql_slow_ms = inherited("JAUNDER_SQL_SLOW_MS");
    match database {
        storage::DbConnectOptions::Sqlite(_) => {
            storage_runtime_config_from_raw(database, sql_slow_ms, Ok(None), Ok(None))
        }
        storage::DbConnectOptions::Postgres { .. } => {
            let password_file = match std::env::var("JAUNDER_DB_PASSWORD_FILE") {
                Ok(path) => Ok(Some(fs::read_to_string(path))),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(error) => Err(error),
            };
            storage_runtime_config_from_raw(
                database,
                sql_slow_ms,
                password_file,
                inherited("JAUNDER_DB_PASSWORD"),
            )
        }
    }
}

/// Converts a mutation outcome into its confirmed value for a CLI operation.
///
/// A lost commit acknowledgement leaves the operator unable to safely retry.
pub(super) fn require_confirmed_mutation<T>(
    outcome: common::mutation::MutationOutcome<T>,
    operation: &str,
) -> anyhow::Result<T> {
    match outcome {
        common::mutation::MutationOutcome::Confirmed(value) => Ok(value),
        common::mutation::MutationOutcome::CommitIndeterminate(_) => Err(anyhow::anyhow!(
            "{operation} commit acknowledgement was indeterminate"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use storage::DbConnectOptions;
    use tempfile::TempDir;

    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt as _;

    #[test]
    fn subprocess_classifies_command_configuration_inputs() {
        const SCENARIO: &str = "JAUNDER_TEST_COMMAND_CONFIG_SCENARIO";
        if let Some(scenario) = std::env::var_os(SCENARIO) {
            let database: DbConnectOptions = "postgres://app@localhost/jaunder"
                .parse()
                .expect("PostgreSQL URL");
            let result = storage_runtime_config(&database);
            match scenario.to_string_lossy().as_ref() {
                "file" | "password" | "invalid-threshold" => {
                    result.expect("valid command configuration");
                }
                "invalid-file-variable" => {
                    assert!(matches!(
                        result,
                        Err(storage::PostgresPasswordError::FileVariable(_))
                    ));
                }
                _ => unreachable!("parent supplies a closed configuration scenario set"),
            }
            return;
        }

        let dir = TempDir::new().expect("password directory");
        let password_file = dir.path().join("password");
        std::fs::write(&password_file, "from-file\n").expect("password fixture");
        for scenario in [
            "file",
            "password",
            "invalid-threshold",
            "invalid-file-variable",
        ] {
            let mut command =
                std::process::Command::new(std::env::current_exe().expect("test executable"));
            command.args([
                "--exact",
                "commands::support::tests::subprocess_classifies_command_configuration_inputs",
                "--nocapture",
            ]);
            command.env(SCENARIO, scenario);
            for name in [
                "JAUNDER_SQL_SLOW_MS",
                "JAUNDER_DB_PASSWORD_FILE",
                "JAUNDER_DB_PASSWORD",
            ] {
                command.env_remove(name);
            }
            match scenario {
                "file" => {
                    command.env("JAUNDER_DB_PASSWORD_FILE", &password_file);
                }
                "password" => {
                    command.env("JAUNDER_DB_PASSWORD", "from-variable");
                }
                "invalid-threshold" => {
                    command.env(
                        "JAUNDER_SQL_SLOW_MS",
                        std::ffi::OsString::from_vec(vec![0xff]),
                    );
                }
                "invalid-file-variable" => {
                    command.env(
                        "JAUNDER_DB_PASSWORD_FILE",
                        std::ffi::OsString::from_vec(vec![0xff]),
                    );
                }
                _ => unreachable!("closed parent scenario set"),
            }
            assert!(
                command
                    .status()
                    .expect("spawn configuration child")
                    .success(),
                "configuration child scenario {scenario} must succeed"
            );
        }
    }

    #[test]
    fn sqlite_runtime_config_ignores_broken_postgres_credential_inputs() {
        let database: DbConnectOptions = "sqlite:/tmp/jaunder.db".parse().expect("SQLite URL");
        let runtime = storage_runtime_config_from_raw(
            &database,
            Ok(None),
            Err(std::env::VarError::NotUnicode(std::ffi::OsString::from(
                "invalid-password-file-variable",
            ))),
            Err(std::env::VarError::NotUnicode(std::ffi::OsString::from(
                "invalid-password-variable",
            ))),
        )
        .expect("SQLite does not resolve PostgreSQL credentials");

        assert_eq!(
            runtime.sql_slow_query_threshold(),
            Duration::from_secs(5),
            "SQLite retains the shared threshold default"
        );
    }

    #[test]
    fn confirmed_mutation_returns_its_value() {
        let value = require_confirmed_mutation(
            common::mutation::MutationOutcome::Confirmed(42_u8),
            "unused operation",
        )
        .expect("confirmed mutation");

        assert_eq!(value, 42);
    }

    #[test]
    fn indeterminate_mutation_reports_the_operation() {
        let error = require_confirmed_mutation(
            common::mutation::MutationOutcome::CommitIndeterminate(()),
            "user creation",
        )
        .expect_err("indeterminate mutation");

        assert_eq!(
            error.to_string(),
            "user creation commit acknowledgement was indeterminate"
        );
    }
}
