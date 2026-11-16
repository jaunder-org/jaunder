//! The `no-just` static check (#1965): `dev` replaced Just, so no hook, CI
//! workflow, Nix expression, script or tool source may name a Just command again.
//! A leftover `just X` in a help string sends a reader to a command that no longer
//! exists, and a leftover invocation fails only when that path finally runs.
//!
//! It flags, per line:
//!
//! 1. the file names `Justfile` and `<name>.just`;
//! 2. `just` followed by an option (`just --justfile`) or by a recipe-shaped word:
//!    lowercase with a hyphen, or starting with `_` (`just check-no-test`,
//!    `just _static`);
//! 3. `just` followed by any lowercase word where the text before it reads as a
//!    command position: the start of the line (optionally after `$ `), a backtick
//!    or quote, or one of `-c`, `exec`, `run:`, `&&`, `||`, `;`, `|`, `$(`;
//! 4. the quoted program name `"just"` or `'just'`.
//!
//! English stays legal: "just as", "just one" and "not just presence" have no
//! command position before them and no recipe-shaped word after them. `just` is
//! matched as a whole word, so `adjust`, `.just(` and `dev-runner-replaces-just`
//! are not invocations.
//!
//! Accepted limitation: a prose sentence that begins a line with "just", or a
//! hyphenated word after "just" ("just read-only"), is flagged. That is a false
//! alarm the author can see and reword, never a missed command.

use std::path::Path;

use crate::files;

/// Fixed roots; a missing one is a failure, so a move cannot disable the guard.
const POLICED_ROOTS: &[&str] = &[
    ".githooks",
    ".github",
    "nix",
    "scripts",
    "tools/dev/jaunder_dev",
];

/// Every `tools/<name>/src` is policed too, discovered so a new tool is covered.
const TOOLS: &str = "tools";

/// This file states the patterns it rejects in its own tests.
const SELF: &str = "tools/repository-policy/src/source_contracts/no_just_check.rs";

/// Text that puts the next word in a command position, matched against the
/// end of the line before `just` with trailing spaces removed.
const COMMAND_CONTEXTS: &[&str] = &["-c", "exec", "run:", "&&", "||", ";", "|", "$(", "$"];

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'
}

fn is_name(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Whether the text before `just` makes it a command. An opening backtick or
/// quote must touch `just`; a closing one ("`x` just works") is prose.
fn command_position(before: &str) -> bool {
    if before.ends_with(['`', '"', '\'']) {
        return true;
    }
    let before = before.trim_end();
    before.is_empty()
        || COMMAND_CONTEXTS.iter().any(|context| {
            before.strip_suffix(context).is_some_and(|rest| {
                // `-c` and `exec` are words: `rsync-c` and `reexec` are not contexts.
                !context.starts_with(|c: char| c.is_ascii_alphabetic() || c == '-')
                    || !rest.ends_with(is_name)
            })
        })
}

/// What one line holds that names a Just command, if anything.
fn line_problem(line: &str) -> Option<&'static str> {
    if line
        .split(|c: char| !is_name(c))
        .any(|word| word == "Justfile")
    {
        return Some("names the `Justfile`");
    }
    for (start, _) in line.match_indices(".just") {
        let named = line[..start].ends_with(is_name);
        let ends = !line[start + 5..].starts_with(is_name);
        if named && ends && !line[start + 5..].starts_with('(') {
            return Some("names a `.just` recipe file");
        }
    }
    for (start, _) in line.match_indices("just") {
        let before = &line[..start];
        let after = &line[start + 4..];
        if before.ends_with(is_word) || after.starts_with(is_word) {
            continue;
        }
        // A quoted program name, as in `Command::new("just")`.
        if ['"', '\'']
            .iter()
            .any(|&q| before.ends_with(q) && after.starts_with(q))
        {
            return Some("runs Just");
        }
        let rest = after.trim_start_matches([' ', '\t']);
        if rest.len() == after.len() {
            continue;
        }
        let follower: String = rest.chars().take_while(|&c| is_name(c)).collect();
        let recipe =
            follower.starts_with(|c: char| c.is_ascii_lowercase()) && follower.contains('-');
        if follower.starts_with(['-', '_']) || recipe {
            return Some("runs a Just recipe");
        }
        // In a command position any argument counts, including `"$@"`.
        let argument = rest.starts_with(|c: char| c.is_ascii_lowercase() || "\"'$".contains(c));
        if argument && command_position(before) {
            return Some("runs a Just recipe");
        }
    }
    None
}

/// The failure detail for every offending line, or `None` when clean.
fn problems(scanned: &[(String, String)]) -> Option<String> {
    let mut found = Vec::new();
    for (path, source) in scanned {
        if path == SELF {
            continue;
        }
        for (index, line) in source.lines().enumerate() {
            if let Some(what) = line_problem(line) {
                found.push(format!("{path}:{}: {what}: {}", index + 1, line.trim()));
            }
        }
    }
    if found.is_empty() {
        return None;
    }
    found.push(
        "  recovery: Just is gone; call the `dev` command of the same name (`dev check`, \
         `dev static <name>`)."
            .into(),
    );
    Some(found.join("\n"))
}

fn roots() -> Result<Vec<String>, String> {
    let mut roots: Vec<String> = POLICED_ROOTS.iter().map(|root| root.to_string()).collect();
    let tools =
        std::fs::read_dir(TOOLS).map_err(|error| format!("cannot scan {TOOLS}: {error}"))?;
    for entry in tools {
        let source = entry
            .map_err(|error| format!("cannot scan {TOOLS}: {error}"))?
            .path()
            .join("src");
        if source.is_dir() {
            roots.push(source.display().to_string());
        }
    }
    roots.sort();
    Ok(roots)
}

/// Scan every file under the policed roots and return the verdict step.
pub fn run() -> Result<Option<String>, String> {
    let mut scanned = Vec::new();
    for root in roots()? {
        let paths = files::every(Path::new(&root))
            .map_err(|error| format!("cannot scan {root}: {error}"))?;
        for path in paths {
            let bytes = std::fs::read(&path)
                .map_err(|error| format!("{}: cannot read — {error}", path.display()))?;
            // Bytecode and other binary files cannot hold a command a reader follows.
            scanned.push((
                path.display().to_string(),
                String::from_utf8_lossy(&bytes).into_owned(),
            ));
        }
    }
    match problems(&scanned) {
        Some(detail) => Err(detail),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::{SELF, line_problem, problems};

    #[test]
    fn invocations_and_file_names_are_flagged() {
        for line in [
            "just check",
            "  just prepush",
            "$ just validate",
            "run `just check` first",
            "        run: nix develop .#ci -c just ci-validate-host",
            "exec just \"$@\"",
            "handle=$(just tsc verify)",
            "true && just precommit",
            "Command::new(\"just\")",
            "\"just test-local\"",
            "rerun with just e2e-local",
            "just --justfile recipes/static.just docs verify",
            "just _static all",
            "cp Justfile out/",
            "    || relative == \"Justfile\"",
            "source recipes/prettier.just",
        ] {
            assert!(line_problem(line).is_some(), "{line:?} should be flagged");
        }
    }

    #[test]
    fn english_and_lookalikes_are_not_flagged() {
        for line in [
            "// Report drift from the size #1549 achieved, not just pass/fail.",
            "/// The browsers to build: just `--browser` if given, else both.",
            "# not just an option. When this defaulted to `common`, both crons",
            "// Exclusive boundary: exactly 30 and just under pass; strictly over fails.",
            "// head just because a later read might happen to return the expected one.",
            "/// attribute *ordering*, not just presence.",
            "    // just as before",
            "    # you just get fewer skips next time.",
            "// `comment_marker_is` just proved the first token matches",
            "let adjust = justify(x);",
            "fixture.just(recipe)",
            "\"\"\"Jaunder's `dev` workflow runner (ADR draft: dev-runner-replaces-just).\"\"\"",
            "mod no_just_check;",
            "a just-in-time build",
            "it is just one step",
            "# trusted application CSS, not just Cargo-recognized host sources.",
            "Justfiles are gone",
            "dev check",
        ] {
            assert_eq!(line_problem(line), None, "{line:?} should pass");
        }
    }

    #[test]
    fn a_problem_names_the_file_line_and_recovery() {
        let detail = problems(&[(
            ".githooks/pre-push".into(),
            "#!/bin/sh\nexec just prepush\n".into(),
        )])
        .expect("a problem");
        assert!(detail.contains(".githooks/pre-push:2: runs a Just recipe"));
        assert!(detail.contains("recovery:"));
    }

    #[test]
    fn clean_sources_and_this_file_pass() {
        assert_eq!(
            problems(&[
                ("scripts/x".into(), "exec dev prepush\n".into()),
                (SELF.into(), "just check\n".into()),
            ]),
            None
        );
    }
}
