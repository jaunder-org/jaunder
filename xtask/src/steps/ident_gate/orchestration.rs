use crate::result::CommandResult;
use crate::steps::scan::run_source_scan;

use super::marker_policy::{Why, classify};
use super::resolution::owner_aliases;
use super::traversal::{MentionContext, scan};

fn mention_where(context: &MentionContext) -> String {
    match context {
        MentionContext::Module => "at module scope".to_owned(),
        MentionContext::Function(name) => format!("in fn `{name}`"),
    }
}

/// The prose a [`Gate`] fails in. A gate's diagnosis is most of its value — the
/// reader has to learn what they tripped and what to do instead — so the wording
/// stays with the gate rather than being generalised into something that fits every
/// gate and helps at none.
pub struct Report {
    /// What was found, as the sentence's subject: `` "`PreEscaped`" ``, "an
    /// unescaped-HTML sink". Rendered as `{subject} {where} {verdict}`.
    pub subject: &'static str,
    /// Why it fails, following the `in fn \`x\`` / `at module scope` phrase.
    pub verdict: &'static str,
    /// The recovery paragraph, ending in the phrase that introduces the derived
    /// census (conventionally "Currently marked:").
    pub recovery: &'static str,
}

/// A complete enumerating gate: the population it reads structurally, the roots it
/// scans, and the words it fails in. The only way out is an in-source marker on the
/// line above the site (#778) — there is no list here to edit.
pub struct Gate {
    /// Step name in the xtask result (`"html-sink"`). Also the marker's token
    /// stem, so the two can never drift apart.
    pub step: &'static str,
    /// Source roots scanned recursively for `.rs` files. A missing root is a hard
    /// failure, so a moved or renamed tree can never quietly disable the guard.
    pub roots: &'static [&'static str],
    /// The names this gate polices — its population, read structurally from what the
    /// AST says, never from a pattern believed to characterise violations (ADR-0085
    /// principle 1). An occurrence of any of these idents is a member, in ordinary
    /// code or inside macro tokens, wherever it appears.
    ///
    /// Matching the ident rather than a call shape is what keeps such a gate an
    /// enumeration instead of a search for the spelling someone anticipated — a
    /// builder call, a struct field and a bare reference are all inside the
    /// population rather than silently outside it (ADR-0085 principle 3).
    pub population: &'static [&'static str],
    /// The type whose door this gate guards, when the population is an **associated fn
    /// name** another type may legitimately share (#790).
    ///
    /// With `Some(ty)`, a site whose qualifier resolves to a type other than `ty` is not
    /// this gate's door and owes no marker; a qualifier that cannot be resolved stays in
    /// the population, so the narrowing never fails open. Deciding membership this way is
    /// **structural** — it identifies the door rather than exempting a site from it, so
    /// ADR-0085 principle 3 is not in play.
    ///
    /// `None` polices the bare ident wherever it appears. That is right for a population
    /// that is a type (`PreEscaped`) or a method reached through `.` (`set_inner_html`),
    /// where there is no qualifier to read.
    pub owner: Option<&'static str>,
    pub report: Report,
}

impl Gate {
    /// 1-based `(line, enclosing-fn)` of every mention in one source that this
    /// gate's markers do not cover, plus every orphan marker (empty fn name).
    ///
    /// Test-only: [`Gate::problems`] parses once and classifies itself, so this is
    /// the single-source convenience the gates' unit tests assert through — and
    /// pairing the parse with the marker rule here means the two halves cannot
    /// drift apart per gate. Orphan markers come back with an empty function name.
    #[cfg(test)]
    pub fn violations(&self, source: &str) -> Result<Vec<(usize, String)>, String> {
        // Single-file owner set: a fixture is the whole tree as far as this helper is
        // concerned, so a rename it declares is honored and one it does not is not.
        let aliases = self
            .owner
            .map(|ty| owner_aliases(&[(String::new(), source.to_string())], ty));
        let owner = self.owner.zip(aliases.as_ref());
        let c = classify(
            source,
            &scan(source, self.population, owner)?,
            &self.marker_token(),
        );
        let mut out: Vec<(usize, String)> = c
            .unexempt
            .into_iter()
            .map(|u| (u.line, u.context.legacy_label()))
            .collect();
        out.extend(c.orphans.into_iter().map(|line| (line, String::new())));
        out.sort();
        Ok(out)
    }

    /// The marker token this gate honors — its step name plus `:allow`. Derived
    /// rather than declared so a gate cannot be renamed out of sync with the
    /// markers that exempt its sites.
    fn marker_token(&self) -> String {
        format!("{}:allow", self.step)
    }

    /// The failure detail for every offending mention across the scanned files, or
    /// `None` when every site is marked. A per-file parse failure is surfaced
    /// (never swallowed). Pure given the `(path, source)` pairs, so gates unit-test
    /// it directly.
    ///
    /// On failure the detail ends with the **derived** census — every marked site
    /// the scan found. Unlike the declared allowlist it replaces, that census is
    /// computed from the tree, so it cannot go stale and there is no reconciliation
    /// pass to keep it honest.
    pub fn problems(&self, scanned: &[(String, String)]) -> Option<String> {
        let token = self.marker_token();
        // Harvested once, across every scanned file, before any classification: a
        // renaming re-export in one module decides membership in another (#790, D2).
        let aliases = self.owner.map(|ty| owner_aliases(scanned, ty));
        let owner = self.owner.zip(aliases.as_ref());
        let mut lines = Vec::new();
        let mut census = Vec::new();
        for (path, source) in scanned {
            match scan(source, self.population, owner) {
                Err(msg) => lines.push(format!(
                    "{path}: {msg} — an unparsed file is invisible to this gate, which is exactly \
                     the blind spot it exists to close. Fix the file or the parser; do not skip it."
                )),
                Ok(found) => {
                    let c = classify(source, &found, &token);
                    for u in c.unexempt {
                        let where_ = mention_where(&u.context);
                        lines.push(match u.why {
                            Why::Unmarked => format!(
                                "{path}:{}: {} {where_} {}",
                                u.line, self.report.subject, self.report.verdict
                            ),
                            Why::NoReason => format!(
                                "{path}:{}: {} {where_} carries a bare `{token}` marker — an \
                                 exemption with no reason is not an exemption; say why this site \
                                 is safe",
                                u.line, self.report.subject
                            ),
                            Why::Shared(n) => format!(
                                "{path}:{}: {n} `{}` sites share this line, so one marker cannot \
                                 justify them — split the line so each carries its own",
                                u.line, self.step
                            ),
                        });
                    }
                    for line in c.orphans {
                        lines.push(format!(
                            "{path}:{line}: `{token}` marker on a line with no `{}` site — a \
                             stale exemption; delete it",
                            self.step
                        ));
                    }
                    census.extend(c.marked.into_iter().map(|m| (path.clone(), m)));
                }
            }
        }

        if lines.is_empty() {
            return None;
        }
        lines.sort();
        lines.push(self.report.recovery.to_string());
        census.sort_by(|a, b| (&a.0, a.1.line).cmp(&(&b.0, b.1.line)));
        for (path, m) in census {
            lines.push(format!("    - {path}:{} — {}", m.line, m.reason));
        }
        Some(lines.join("\n"))
    }
}

/// Read every `.rs` file under each of `roots`, hand the `(path, source)` pairs to
/// `problems`, and push the resulting step.
pub fn run_scan(
    result: &mut CommandResult,
    step: &'static str,
    roots: &'static [&'static str],
    problems: impl FnOnce(&[(String, String)]) -> Option<String>,
) {
    run_source_scan(result, step, roots, problems);
}
