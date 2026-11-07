//! Literal argv admission; storage and lifecycle selectors remain supervisor-owned.
use crate::sandbox_name::SandboxName;
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    #[default]
    Empty,
    Standard,
    Demo,
}
impl Profile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::Standard => "standard",
            Self::Demo => "demo",
        }
    }
}

#[derive(Parser)]
#[command(about = "Run a disposable or named Jaunder UX sandbox")]
pub struct Cli {
    pub name: Option<SandboxName>,
    #[arg(long, value_enum)]
    pub profile: Option<Profile>,
    #[arg(long)]
    pub reset: bool,
    #[arg(last = true)]
    pub command: Vec<OsString>,
}
impl Cli {
    pub fn validate(&self) -> Result<(), String> {
        if self.reset && self.name.is_none() {
            return Err("--reset requires an existing NAME".into());
        }
        if !self.command.is_empty() {
            if self.name.is_none() || self.profile.is_some() || self.reset {
                return Err("command mode requires NAME and forbids --profile/--reset".into());
            }
            let allowed = [
                "site-config",
                "user-create",
                "app-password-create",
                "user-invite",
                "smtp-test",
                "backup",
                "websub",
            ];
            if !self.command[0]
                .to_str()
                .is_some_and(|name| allowed.contains(&name))
            {
                return Err("command is not permitted in sandbox command mode".into());
            }
            if self.command.iter().any(|arg| {
                // ASCII selector prefixes remain selectors even when a path suffix is non-UTF-8.
                let arg = arg.as_encoded_bytes();
                arg == b"--storage-path"
                    || arg == b"--db"
                    || arg.starts_with(b"--storage-path=")
                    || arg.starts_with(b"--db=")
            }) {
                return Err("sandbox command mode owns --storage-path and --db".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_retains_the_validated_name_inward() {
        let cli =
            Cli::try_parse_from(["ux-sandbox", "writing-flow_2", "--profile", "demo"]).unwrap();
        let name: &SandboxName = cli.name.as_ref().unwrap();
        assert_eq!(name.as_ref(), "writing-flow_2");
        assert_eq!(cli.profile, Some(Profile::Demo));
        cli.validate().unwrap();
    }
}
