//! Pure helpers for sequence-numbered identifier files (ADRs, migrations):
//! parsing the leading number, detecting duplicate prefixes, checking backend
//! parity, and choosing the next free number. No I/O — unit-tested in isolation.

use std::collections::{BTreeMap, BTreeSet};

/// The leading integer of a filename like `0034-foo.md` or `0023_create_x.sql`.
/// `None` when the name does not start with a digit.
pub fn leading_number(filename: &str) -> Option<u32> {
    let digits: String = filename.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Filename without its final extension: `0023_create_x.sql` -> `0023_create_x`.
fn stem(filename: &str) -> &str {
    match filename.rfind('.') {
        Some(i) => &filename[..i],
        None => filename,
    }
}

/// Numbers used by more than one file, each with its sorted filenames. Files
/// without a leading number are ignored. Sorted by number for stable output.
pub fn duplicate_prefixes(filenames: &[String]) -> Vec<(u32, Vec<String>)> {
    let mut by_number: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for name in filenames {
        if let Some(n) = leading_number(name) {
            by_number.entry(n).or_default().push(name.clone());
        }
    }
    by_number
        .into_iter()
        .filter(|(_, names)| names.len() > 1)
        .map(|(n, mut names)| {
            names.sort();
            (n, names)
        })
        .collect()
}

/// Migration stems present in one backend directory but not the other — a
/// backend-parity violation. Returns sorted `"<stem> (<backend> only)"` lines.
pub fn parity_mismatch(sqlite: &[String], postgres: &[String]) -> Vec<String> {
    let set = |names: &[String]| -> BTreeSet<String> {
        names.iter().map(|n| stem(n).to_string()).collect()
    };
    let s = set(sqlite);
    let p = set(postgres);
    let mut out = Vec::new();
    out.extend(s.difference(&p).map(|x| format!("{x} (sqlite only)")));
    out.extend(p.difference(&s).map(|x| format!("{x} (postgres only)")));
    out.sort();
    out
}

/// One greater than the maximum leading number across `filenames`; `0` when none
/// have a number. Monotonic — never reuses a gap left by a deleted file.
pub fn next_number(filenames: &[String]) -> u32 {
    filenames
        .iter()
        .filter_map(|n| leading_number(n))
        .max()
        .map_or(0, |m| m + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_number_parses_both_separators() {
        assert_eq!(leading_number("0034-foo.md"), Some(34));
        assert_eq!(leading_number("0023_create_x.sql"), Some(23));
        assert_eq!(leading_number("README.md"), None);
        assert_eq!(leading_number("template.md"), None);
    }

    #[test]
    fn duplicate_prefixes_reports_only_collisions_sorted() {
        let files = vec![
            "0034-bar.md".to_string(),
            "0034-foo.md".to_string(),
            "0033-solo.md".to_string(),
            "notes.md".to_string(),
        ];
        let dups = duplicate_prefixes(&files);
        assert_eq!(
            dups,
            vec![(
                34,
                vec!["0034-bar.md".to_string(), "0034-foo.md".to_string()]
            )]
        );
    }

    #[test]
    fn duplicate_prefixes_empty_when_unique() {
        let files = vec!["0001_a.sql".to_string(), "0002_b.sql".to_string()];
        assert!(duplicate_prefixes(&files).is_empty());
    }

    #[test]
    fn parity_mismatch_flags_each_side() {
        let sqlite = vec!["0001_a.sql".to_string(), "0002_only_sqlite.sql".to_string()];
        let postgres = vec!["0001_a.sql".to_string(), "0003_only_pg.sql".to_string()];
        let m = parity_mismatch(&sqlite, &postgres);
        assert_eq!(
            m,
            vec![
                "0002_only_sqlite (sqlite only)".to_string(),
                "0003_only_pg (postgres only)".to_string(),
            ]
        );
    }

    #[test]
    fn parity_mismatch_empty_when_identical() {
        let s = vec!["0001_a.sql".to_string()];
        assert!(parity_mismatch(&s, &s).is_empty());
    }

    #[test]
    fn next_number_is_max_plus_one() {
        let files = vec!["0001_a.sql".to_string(), "0007_b.sql".to_string()];
        assert_eq!(next_number(&files), 8);
        assert_eq!(next_number(&[]), 0);
    }
}
