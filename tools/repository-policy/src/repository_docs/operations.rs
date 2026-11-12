//! Fixed documentation operations; composition and population ordering belong to Just.
use std::path::Path;

/// One independent documentation policy, never a workflow selection.
#[derive(Clone, Copy, Debug)]
pub enum Operation {
    AdrFormat,
    AdrReadmeParity,
    AdrViewParity,
    Links,
    Flows,
    Sequence,
}
impl Operation {
    /// Stable independent verdict identity, including the sequence policy's name.
    pub fn name(self) -> &'static str {
        match self {
            Self::AdrFormat => "adr-format",
            Self::AdrReadmeParity => "adr-readme-parity",
            Self::AdrViewParity => "adr-view-parity",
            Self::Links => "doc-links",
            Self::Flows => "flow-docs",
            Self::Sequence => "identifier-collisions",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "adr-format" => Some(Self::AdrFormat),
            "adr-readme-parity" => Some(Self::AdrReadmeParity),
            "adr-view-parity" => Some(Self::AdrViewParity),
            "doc-links" => Some(Self::Links),
            "flow-docs" => Some(Self::Flows),
            "sequence-check" => Some(Self::Sequence),
            _ => None,
        }
    }
    /// Run exactly one policy without checkout configuration or mutation.
    pub fn check(self, root: &Path) -> Result<Option<String>, String> {
        let problems = match self {
            Self::Links => {
                let problems =
                    super::links::problems(root).map_err(|error| format!("{error:#}"))?;
                if problems.is_empty() {
                    return Ok(None);
                }
                return Err(problems.join("\n"));
            }
            Self::Flows => return super::flows::run_at(root),
            Self::Sequence => return super::sequence::run_at(root),
            Self::AdrFormat => crate::adr::format_problems(root),
            Self::AdrReadmeParity => {
                const RECOVERY: &str = "  recovery: nix run --accept-flake-config --no-update-lock-file --no-write-lock-file .#adr-promoter -- sync-readme";
                return match super::readme::parity_report(root) {
                    Ok(problems) if problems.is_empty() => Ok(None),
                    Ok(problems) => Err(format!("{}\n{RECOVERY}", problems.join("\n"))),
                    Err(error) => Err(format!("{error:#}\n{RECOVERY}")),
                };
            }
            Self::AdrViewParity => {
                const RECOVERY: &str = "  recovery: none mechanical — describe each ADR above in the relevant section of docs/ARCHITECTURE.md and cite it there, as a link to `adr/NNNN-<slug>.md` or a bare `ADR-NNNN` token. There is no exemption list.";
                return match super::view::view_parity_problems(root) {
                    Ok(problems) if problems.is_empty() => Ok(None),
                    Ok(problems) => Err(format!("{}\n{RECOVERY}", problems.join("\n"))),
                    Err(error) => Err(format!("{error:#}\n{RECOVERY}")),
                };
            }
        };
        if problems.is_empty() {
            Ok(None)
        } else {
            Err(problems.join("\n"))
        }
    }
}
