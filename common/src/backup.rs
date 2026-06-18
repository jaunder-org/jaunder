//! The backup **configuration** model, shared between the server's scheduled
//! backup worker (`server::backup`) and the web admin surface that reads and
//! writes these values through `site_config`. This module holds only the value
//! types plus their validation and defaults; the actual export, archiving, and
//! retention-pruning logic lives in the `storage` and `server` crates.

use croner::Cron;
use serde::{Deserialize, Serialize};

/// How a backup is written to its destination.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupMode {
    /// An expanded directory (`backup-<timestamp>/` holding `manifest.json`, the
    /// dumped tables, and the media tree). The default.
    #[default]
    Directory,
    /// A single `backup-<timestamp>.tar.gz` archive of the same contents.
    Archive,
}

/// A validated six-field cron schedule expression.
///
/// Can only be constructed via [`BackupSchedule::parse`] or
/// [`BackupSchedule::default`], guaranteeing the inner string is always valid.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackupSchedule(String);

impl BackupSchedule {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Cron::new(s.trim())
            .with_seconds_required()
            .parse()
            .ok()
            .map(|_| Self(s.trim().to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for BackupSchedule {
    fn default() -> Self {
        Self("0 0 0 * * *".to_owned())
    }
}

/// Default for [`BackupConfig::retention_count`]: keep the seven most recent backups.
pub const DEFAULT_BACKUP_RETENTION_COUNT: usize = 7;

/// The persisted backup settings (stored in `site_config`, surfaced in the
/// admin UI, and consumed by the scheduled backup worker).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackupConfig {
    /// Filesystem path backups are written under. `None` means no destination is
    /// configured and scheduled backups are disabled.
    pub destination_path: Option<String>,
    /// When scheduled backups run, as a validated six-field cron expression.
    pub schedule: BackupSchedule,
    /// How many of the most recent backups to keep; older ones are pruned.
    pub retention_count: usize,
    /// Whether each backup is written as a directory or a single archive.
    pub mode: BackupMode,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            destination_path: None,
            schedule: BackupSchedule::default(),
            retention_count: DEFAULT_BACKUP_RETENTION_COUNT,
            mode: BackupMode::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_schedule_parse_accepts_valid_six_field_cron() {
        assert!(BackupSchedule::parse("0 0 0 * * *").is_some());
        assert!(BackupSchedule::parse("0 30 2 * * MON-FRI").is_some());
    }

    #[test]
    fn backup_schedule_parse_rejects_invalid_expressions() {
        assert!(BackupSchedule::parse("").is_none());
        assert!(BackupSchedule::parse("not a cron").is_none());
        assert!(BackupSchedule::parse("* * * * *").is_none()); // five-field, not six
        assert!(BackupSchedule::parse("99 0 0 * * *").is_none());
    }

    #[test]
    fn backup_schedule_default_is_valid() {
        let s = BackupSchedule::default();
        assert_eq!(s.as_str(), "0 0 0 * * *");
        assert!(BackupSchedule::parse(s.as_str()).is_some());
    }

    #[test]
    fn backup_schedule_parse_trims_whitespace() {
        let s = BackupSchedule::parse("  0 0 0 * * *  ").unwrap();
        assert_eq!(s.as_str(), "0 0 0 * * *");
    }

    #[test]
    fn backup_config_default_has_no_destination() {
        assert_eq!(BackupConfig::default().destination_path, None);
    }
}
