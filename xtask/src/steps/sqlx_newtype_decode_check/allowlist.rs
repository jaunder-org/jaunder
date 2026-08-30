use super::report::POLICED_ROOT;
use super::scan::DecodeSite;

/// A decode exempt from the guard, keyed by (file, function, target, what) — all
/// reflow-stable, none positional — plus how many identical sites that key covers.
///
/// **The count is load-bearing, not decoration.** `sqlx-newtype-bind`'s substring
/// needles exempt "every matching line under the policed root, not one site" (its own
/// doc says so), which is a region-scoped exemption: a new violation inside the reach
/// passes silently. The population here really does contain byte-identical decode
/// pairs that no key can separate — two `COUNT(*) FROM {table}` calls in one `match`,
/// two `query_scalar(sql)` arms in one helper. Declaring the multiplicity means
/// gaining a third is a mismatch and a failure, not a silent absorption.
pub(super) struct Allowed {
    /// Path suffix under [`POLICED_ROOT`], e.g. `backup.rs` or `sqlite/mod.rs`.
    pub(super) file: &'static str,
    /// Enclosing function name.
    pub(super) function: &'static str,
    /// Rendered decode target, whitespace-stripped, e.g. `i64` or `Option<i64>`.
    pub(super) target: &'static str,
    /// Rendered first argument of the decode call, whitespace-stripped — the SQL
    /// literal, the column name, or the expression that produced it. A **key only**:
    /// nothing in the rule branches on it.
    pub(super) what: &'static str,
    /// How many identical decodes this entry covers.
    pub(super) count: usize,
    /// What kind of exemption this is. Grouping only — see [`Category`].
    pub(super) category: Category,
    /// Why this decode legitimately yields a primitive.
    pub(super) reason: &'static str,
}

/// What kind of exemption an [`Allowed`] entry is, so the failure output can be read by
/// rationale instead of by file.
///
/// **Nothing in the matching rule or the count check branches on this.** It exists because
/// the allowlist's whole value is that a human reads it, and a flat list where a third of
/// the entries are variations of "a name out of `information_schema`" is a list people skim
/// — which is how a region exemption sneaks in wearing a dozen costumes. An enum rather
/// than a string so a typo is a compile error and the grouping order is total.
///
/// [`Category::DeferredNewtype`] is the one that carries an obligation: it means "this
/// *should* be a domain type and is not yet", so its reason must name a tracking issue —
/// enforced in [`problems`]. That makes the allowlist a worklist rather than a graveyard:
/// `DeferredNewtype` entries are the remaining unwrapped storage values, and the gate's own
/// staleness check deletes each one as it gets typed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Category {
    /// `COUNT(*)`, `SELECT EXISTS(…)`, and other cardinality probes.
    CountOrExists,
    /// A flag or counter whose primitive **is** the whole domain — a `bool` with two
    /// meaningful states, an integer retry count. A newtype would wrap nothing.
    ///
    /// Distinct from [`Category::CountOrExists`], which is about a *query shape*: reading
    /// `email_verified` is not a cardinality probe, and filing it under one would dilute
    /// exactly the by-rationale reading these categories exist for.
    FlagOrCounter,
    /// Names and versions read out of the database's own catalog.
    SchemaIntrospection,
    /// A blob the storage layer deliberately does not interpret — raw JSON, a cached
    /// response body.
    OpaquePayload,
    /// A value that is *deliberately* stored lossily, so the domain type would claim more
    /// than the column holds.
    DeliberateLossy,
    /// Not an sqlx decode at all — the gate's population is defined structurally, so it
    /// reaches a few constructs that are not row reads. See the module doc's over-bite
    /// note.
    NotADecodeTarget,
    /// Test scaffolding whose type comes from a generic helper's signature.
    TestScaffolding,
    /// A query target whose hand-written `FromRow` decoder cannot satisfy the narrow
    /// direct-column-get proof. Each exact target is listed here.
    ///
    /// A decoder that satisfies the proof is approved by delegation, while the scanner
    /// still polices each direct typed column get in its body.
    HandWrittenFromRow,
    /// **Residue, not a verdict.** This should be a domain type; the fix is a vertical
    /// tracked elsewhere. The reason must name the issue.
    DeferredNewtype,
}

impl Category {
    /// Every variant, so the failure footer can group in a stable, total order without a
    /// `HashMap` iteration order or a hand-kept second list.
    pub(super) const ALL: &'static [Self] = &[
        Self::CountOrExists,
        Self::FlagOrCounter,
        Self::SchemaIntrospection,
        Self::OpaquePayload,
        Self::DeliberateLossy,
        Self::NotADecodeTarget,
        Self::TestScaffolding,
        Self::HandWrittenFromRow,
        Self::DeferredNewtype,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::CountOrExists => "count-or-exists",
            Self::FlagOrCounter => "flag-or-counter",
            Self::SchemaIntrospection => "schema-introspection",
            Self::OpaquePayload => "opaque-payload",
            Self::DeliberateLossy => "deliberate-lossy",
            Self::NotADecodeTarget => "not-a-decode-target",
            Self::TestScaffolding => "test-scaffolding",
            Self::HandWrittenFromRow => "hand-written-fromrow",
            Self::DeferredNewtype => "deferred-newtype",
        }
    }
}

/// Whether `reason` names a tracking issue (`#` followed by at least one digit).
///
/// Only [`Category::DeferredNewtype`] requires one. A deferred entry whose reason names no
/// issue is a TODO with no owner — the shape that turns an allowlist into a graveyard.
fn names_an_issue(reason: &str) -> bool {
    reason
        .split('#')
        .skip(1)
        .any(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// No decode leaf remains exempt while Task 4 removes this mechanism.
pub(super) const ALLOWLIST: &[Allowed] = &[];

/// Whether `path` is exactly `POLICED_ROOT/relative`.
///
/// **Exact, not a suffix match.** Three files under the root are named `backup.rs`,
/// and two of them declare a function called `schema_version`, so a suffix match would
/// let the `backup.rs` entry reach decodes in `postgres/backup.rs` — a region-scoped
/// exemption of exactly the kind the site-scoping rule exists to stop. The other key
/// fields happen to keep it honest today; that is luck, not design.
fn file_matches(path: &str, relative: &str) -> bool {
    path.strip_prefix(POLICED_ROOT)
        .map(|rest| rest.trim_start_matches('/'))
        == Some(relative)
}

/// Whether `entry` names `decode` in `path`.
pub(super) fn entry_matches(entry: &Allowed, path: &str, decode: &DecodeSite) -> bool {
    file_matches(path, entry.file)
        && entry.function == decode.function
        && entry.target == decode.target
        && entry.what == decode.what
}

/// Faults in an allowlist itself, independent of the tree.
///
/// A gate that polices the source but not its own exemption list is blind in the one place
/// it can least afford to be — the same rule as failing on an unparseable file, applied
/// inward (ADR-0085 principle 6).
///
/// Takes the list rather than reading [`ALLOWLIST`] directly so the tests drive *this*
/// function with synthetic entries instead of re-implementing the rule beside it.
pub(super) fn allowlist_self_problems(allowlist: &[Allowed]) -> Vec<String> {
    let mut lines = Vec::new();

    // Duplicate keys. Matching is `.any(…)` and the count check is per-entry, so two
    // entries with the same key each declaring 1 would BOTH pass while double-covering a
    // single site — and deleting the decode would then need two edits to go green, which
    // is exactly how a stale exemption survives.
    for (i, a) in allowlist.iter().enumerate() {
        if let Some(dup) = allowlist[..i].iter().find(|b| {
            (b.file, b.function, b.target, b.what) == (a.file, a.function, a.target, a.what)
        }) {
            lines.push(format!(
                "{}::{} `{}`: two allowlist entries share one key ({} and {}). Merge them into \
                 one entry and state the combined multiplicity in `count` — two entries covering \
                 one site can never go stale together.",
                a.file, a.function, a.target, dup.reason, a.reason
            ));
        }
    }

    // A deferred entry with no issue is a TODO with no owner.
    for a in allowlist {
        if a.category == Category::DeferredNewtype && !names_an_issue(a.reason) {
            lines.push(format!(
                "{}::{} `{}`: a `deferred-newtype` entry must name the issue tracking the fix \
                 (e.g. \"…, deferred to #750\"). Without one this is not deferred work, it is an \
                 exemption with a sympathetic label.",
                a.file, a.function, a.target
            ));
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An entry with everything but the field under test held fixed.
    fn entry(what: &'static str, category: Category, reason: &'static str) -> Allowed {
        Allowed {
            file: "users.rs",
            function: "f",
            target: "i64",
            what,
            count: 1,
            category,
            reason,
        }
    }

    #[test]
    fn the_category_field_changes_nothing_about_matching_or_counting() {
        // A8's falsifiable form. "No code path branches on `category`" cannot be asserted
        // from a test, but its observable consequence can: two entries that differ ONLY in
        // category must behave identically for both the match and the duplicate check.
        let a = entry("\"SELECTCOUNT(*)\"", Category::CountOrExists, "a count");
        let b = entry("\"SELECTCOUNT(*)\"", Category::OpaquePayload, "a count");
        let site = DecodeSite {
            function: "f".to_string(),
            target: "i64".to_string(),
            what: "\"SELECTCOUNT(*)\"".to_string(),
            unapproved: vec!["i64".to_string()],
            line: 1,
        };
        let path = "storage/src/users.rs";
        assert_eq!(
            entry_matches(&a, path, &site),
            entry_matches(&b, path, &site),
            "category must not affect whether an entry covers a site"
        );
        // …and differing only in category does NOT make two entries distinct, so it cannot
        // be used to sneak a second entry past the duplicate check.
        assert_eq!(
            allowlist_self_problems(&[a, b]).len(),
            1,
            "same key, still a duplicate"
        );
    }

    #[test]
    fn category_drives_only_the_deferred_obligation() {
        // The precise claim: `category` is inert for matching and counting, but it is NOT
        // decoration — `DeferredNewtype` alone carries the name-your-issue obligation.
        let ok = entry("\"a\"", Category::CountOrExists, "a count");
        assert!(allowlist_self_problems(std::slice::from_ref(&ok)).is_empty());
        let deferred = entry("\"a\"", Category::DeferredNewtype, "should be a newtype");
        assert_eq!(
            allowlist_self_problems(std::slice::from_ref(&deferred)).len(),
            1,
            "a deferred entry naming no issue must fail"
        );
    }

    #[test]
    fn two_entries_with_one_key_are_a_failure() {
        let a = entry("\"SELECTCOUNT(*)\"", Category::CountOrExists, "first");
        let b = entry("\"SELECTCOUNT(*)\"", Category::CountOrExists, "second");
        assert_eq!(allowlist_self_problems(&[a, b]).len(), 1);
    }

    #[test]
    fn distinct_keys_are_not_duplicates() {
        let a = entry("\"SELECTCOUNT(*)\"", Category::CountOrExists, "first");
        let b = entry("\"SELECTMAX(v)\"", Category::CountOrExists, "second");
        assert!(allowlist_self_problems(&[a, b]).is_empty());
    }

    #[test]
    fn the_shipped_allowlist_has_no_self_faults() {
        // The self-checks run on every gate invocation, so a bad entry would fail the gate
        // on a clean tree. Pin it here too, where the message is about the allowlist rather
        // than about whatever else was failing.
        assert!(
            allowlist_self_problems(ALLOWLIST).is_empty(),
            "{:?}",
            allowlist_self_problems(ALLOWLIST)
        );
    }

    #[test]
    fn a_deferred_newtype_entry_must_name_its_issue() {
        assert!(names_an_issue(
            "subscriber_ref should be a newtype, deferred to #750"
        ));
        assert!(!names_an_issue(
            "subscriber_ref should be a newtype one day"
        ));
        // A bare `#` with no number is a false positive waiting to happen — a reason that
        // says "the # column" must not count as a tracking reference.
        assert!(!names_an_issue("the # column is opaque"));
    }
    #[test]
    fn an_entry_does_not_reach_a_same_named_file_in_a_subdirectory() {
        // `backup.rs`, `postgres/backup.rs` and `sqlite/backup.rs` all exist, and the
        // last two both declare `schema_version`. A suffix match would let one entry
        // exempt decodes in a sibling file — a region exemption by the back door.
        assert!(file_matches("storage/src/backup.rs", "backup.rs"));
        assert!(!file_matches("storage/src/postgres/backup.rs", "backup.rs"));
        assert!(file_matches(
            "storage/src/postgres/backup.rs",
            "postgres/backup.rs"
        ));
        assert!(!file_matches("storage/src/sqlite/mod.rs", "mod.rs"));
    }
}
