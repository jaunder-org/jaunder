//! The backend and browser identities shared by e2e lanes and their analyses.

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum E2eBackend {
    Sqlite,
    Postgres,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum E2eBrowser {
    Chromium,
    Firefox,
}

impl E2eBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            E2eBackend::Sqlite => "sqlite",
            E2eBackend::Postgres => "postgres",
        }
    }
}

impl E2eBrowser {
    pub fn as_str(self) -> &'static str {
        match self {
            E2eBrowser::Chromium => "chromium",
            E2eBrowser::Firefox => "firefox",
        }
    }
}

/// Already-lifted lane evidence: `diagnostics_root` holds one `e2e-<lane>`
/// directory per lane, checked against the caller-selected catalog.
#[derive(Debug, clap::Args)]
pub struct LiftedEvidence {
    #[arg(long)]
    pub diagnostics_root: std::path::PathBuf,
    #[arg(long)]
    pub catalog: std::path::PathBuf,
}
