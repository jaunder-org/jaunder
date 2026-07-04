//! On-demand drift guard for the Nix `coverage` derivation's source filter (#241).
//!
//! #231 bounded the `coverage` derivation's `src` to cargo sources (+ an explicit
//! `csr/index.html`), closing the #37 impurity. This module guards that filter
//! against *silent* drift: [`probe_verdict`] asserts the two contract invariants over
//! three measured `coverage.drvPath` values —
//!
//! - adding a filter-**excluded** file must NOT change the drvPath (else the filter
//!   re-admits junk → the #37 impurity returns), and
//! - adding an **instrumented** `.rs` MUST change the drvPath (else the filter drops
//!   source → a coverage hole the stateless gate can never see).
//!
//! The pure verdict lives here; the impure orchestration (an ephemeral worktree that
//! stages each probe file and evaluates its drvPath) is [`probe_source`]. See the
//! spec for the load-bearing subtlety: nix ignores *untracked* new files even on a
//! dirty tree, so probe files must be `git add`-ed to be measured.

use std::fmt;

/// The two ways the coverage `src` filter can drift — each a distinct contract break.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftError {
    /// A filter-excluded file changed `coverage.drvPath`: the filter now admits junk
    /// (the #37 impurity regressed).
    AdmitsJunk { base: String, junk: String },
    /// An instrumented `.rs` did NOT change `coverage.drvPath`: the filter drops
    /// source, so those lines are never measured (a coverage hole).
    DropsSource { base: String },
}

impl fmt::Display for DriftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DriftError::AdmitsJunk { base, junk } => write!(
                f,
                "coverage src filter admits junk: staging an excluded file changed \
                 coverage.drvPath ({base} -> {junk}) — the #37 impurity regressed"
            ),
            DriftError::DropsSource { base } => write!(
                f,
                "coverage src filter drops source: staging an instrumented .rs left \
                 coverage.drvPath unchanged ({base}) — those lines would never be measured"
            ),
        }
    }
}

impl std::error::Error for DriftError {}

/// Assert the coverage `src` filter's two invariants over three measured drvPaths:
/// `base` (clean HEAD), `junk` (base + a staged filter-excluded file), and `rs`
/// (base + a staged instrumented `.rs`). Impurity (admits-junk) is checked before the
/// coverage hole (drops-source) so the more severe regression is reported first.
pub fn probe_verdict(base: &str, junk: &str, rs: &str) -> Result<(), DriftError> {
    if junk != base {
        return Err(DriftError::AdmitsJunk {
            base: base.to_owned(),
            junk: junk.to_owned(),
        });
    }
    if rs == base {
        return Err(DriftError::DropsSource {
            base: base.to_owned(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_when_junk_excluded_and_source_measured() {
        assert_eq!(probe_verdict("d-base", "d-base", "d-rs"), Ok(()));
    }

    #[test]
    fn admits_junk_when_junk_moves_drvpath() {
        assert_eq!(
            probe_verdict("d-base", "d-JUNKMOVED", "d-rs"),
            Err(DriftError::AdmitsJunk {
                base: "d-base".into(),
                junk: "d-JUNKMOVED".into()
            })
        );
    }

    #[test]
    fn drops_source_when_rs_does_not_move_drvpath() {
        assert_eq!(
            probe_verdict("d-base", "d-base", "d-base"),
            Err(DriftError::DropsSource {
                base: "d-base".into()
            })
        );
    }

    #[test]
    fn admits_junk_takes_precedence_over_drops_source() {
        // Both broken: junk moved AND rs == base. Junk (impurity) is checked first.
        assert_eq!(
            probe_verdict("d-base", "d-JUNKMOVED", "d-base"),
            Err(DriftError::AdmitsJunk {
                base: "d-base".into(),
                junk: "d-JUNKMOVED".into()
            })
        );
    }

    #[test]
    fn display_names_the_broken_invariant() {
        let j = DriftError::AdmitsJunk {
            base: "b".into(),
            junk: "j".into(),
        };
        assert!(j.to_string().contains("admits") && j.to_string().contains("junk"));
        let s = DriftError::DropsSource { base: "b".into() };
        assert!(s.to_string().contains("drops") && s.to_string().contains("source"));
    }
}
