use std::path::Path;

use super::allowlist::{ALLOWLIST, Allowed, Category, allowlist_self_problems, entry_matches};
use super::approve_set::{
    APPROVED_FOREIGN, ApproveSet, DECLARATION_ROOTS, Root, collect_declarations,
};
use super::macros_audit::{MACROS_LIB, macro_enumeration_problems};
use super::scan::{DecodeSite, decodes};
use crate::files;
use crate::result::{CommandResult, StepResult};

/// Source root scanned recursively for `.rs` files.
pub(super) const POLICED_ROOT: &str = "storage/src";

/// The failure detail for every unjustified decode and every allowlist entry whose
/// declared count no longer matches the tree, or `None` when the population is exactly
/// accounted for. Pure given the `(path, source)` pairs, so it is unit-tested directly.
fn problems(scanned: &[(String, String)], approve: &ApproveSet) -> Option<String> {
    problems_with_allowlist(scanned, approve, ALLOWLIST)
}

fn problems_with_allowlist(
    scanned: &[(String, String)],
    approve: &ApproveSet,
    allowlist: &[Allowed],
) -> Option<String> {
    let mut found: Vec<(String, DecodeSite)> = Vec::new();
    let mut lines = Vec::new();
    for (path, source) in scanned {
        match decodes(source, approve) {
            Ok(scan) => {
                found.extend(scan.sites.into_iter().map(|d| (path.clone(), d)));
                for (line, what) in scan.unreadable_fields {
                    lines.push(format!(
                        "{path}:{line}: `{what}` decodes into a struct-literal field with no type \
                         written at the call. Add a turbofish — `row.try_get::<T, _>({what})` — so \
                         this gate can read the target. It will not follow the field to the \
                         struct's definition: that declaration is only policed when the struct \
                         derives `FromRow`, and for a plain struct nothing checks it at all."
                    ));
                }
            }
            Err(e) => lines.push(format!(
                "{path}: {e} — an unparsed file is invisible to this gate, which is exactly the \
                 blind spot it exists to close. Fix the file or the parser; do not skip it."
            )),
        }
    }

    // Unjustified decodes: nothing in the allowlist names them.
    for (path, d) in &found {
        if !allowlist.iter().any(|e| entry_matches(e, path, d)) {
            lines.push(format!(
                "{path}:{}: `{}` decodes into `{}`, whose leaf type(s) {} are not approved column \
                 types. If the column holds a domain value, decode it straight into its type — the \
                 ADR-0071 bridge makes `query_scalar::<_, PostId>` work — and delete any hand \
                 re-wrap. If it is genuinely untyped, add an ALLOWLIST entry with a written \
                 reason. This gate reads no SQL, so it cannot tell which; that judgement is yours \
                 to record.",
                d.line,
                d.what,
                d.target,
                d.unapproved.join(", ")
            ));
        }
    }

    // Stale or drifted entries: an allowlist that stops tracking the tree is an
    // allowlist that has silently become a region exemption.
    for e in allowlist {
        let seen = found
            .iter()
            .filter(|(path, d)| entry_matches(e, path, d))
            .count();
        if seen != e.count {
            lines.push(format!(
                "{}::{}: allowlist entry for `{}` declares {} site(s), the tree has {}. {}",
                e.file,
                e.function,
                e.target,
                e.count,
                seen,
                if seen == 0 {
                    "The decode is gone — delete the entry."
                } else {
                    "Re-justify each site, then update the count."
                }
            ));
        }
    }

    lines.extend(allowlist_self_problems(allowlist));

    if lines.is_empty() {
        return None;
    }
    lines.push(
        "  recovery: this gate enumerates rather than searching — it has no idea which columns \
         hold domain values, and deliberately so, because every audit that searched for the \
         id-ish spelling missed the sites spelled another way (#686, #715). So a decode passes \
         only when every leaf of its target is an APPROVED type — one declared with a \
         bridge-emitting macro, or a composite whose fields this gate polices — and every other \
         decode is either typed or listed below. Currently exempt, by rationale:"
            .to_string(),
    );
    for category in Category::ALL {
        let mut group = allowlist
            .iter()
            .filter(|a| a.category == *category)
            .peekable();
        if group.peek().is_none() {
            continue;
        }
        lines.push(format!("    [{}]", category.label()));
        for a in group {
            lines.push(format!(
                "      - {}::{} `{}` ×{}: {}",
                a.file, a.function, a.target, a.count, a.reason
            ));
        }
    }
    Some(lines.join("\n"))
}

/// Scan every Rust file under [`POLICED_ROOT`] and push the result step. A missing
/// root is a hard failure, so a moved or renamed tree can never quietly disable the
/// guard.
pub fn run(result: &mut CommandResult) {
    let files = match files::with_extension(Path::new(POLICED_ROOT), "rs") {
        Ok(files) => files,
        Err(e) => {
            result.push(
                StepResult::fail("sqlx-newtype-decode")
                    .detail(format!("cannot scan {POLICED_ROOT}: {e}")),
            );
            return;
        }
    };
    // A file that cannot be READ is as invisible as one that cannot be PARSED, so it
    // fails the same way. `read_to_string(p).ok()` would have dropped it from the
    // population silently — the precise failure this gate exists to prevent, committed
    // by the gate itself.
    let mut scanned: Vec<(String, String)> = Vec::with_capacity(files.len());
    let mut unreadable = Vec::new();
    for p in &files {
        let path = p.display().to_string();
        match std::fs::read_to_string(p) {
            Ok(s) => scanned.push((path, s)),
            Err(e) => unreadable.push(format!(
                "{path}: cannot read: {e} — an unread file is invisible to this gate, so it \
                 fails rather than shrinking the population."
            )),
        }
    }

    // The derive crate is read the same way and fails the same way: this gate's model of
    // the newtype families comes from it, so a file it cannot read is a model it cannot
    // check.
    match std::fs::read_to_string(MACROS_LIB) {
        Ok(s) => unreadable.extend(macro_enumeration_problems(&s)),
        Err(e) => unreadable.push(format!(
            "{MACROS_LIB}: cannot read: {e} — this gate's approved-type set is derived from the \
             derives declared there, so it fails rather than assuming its own list is current."
        )),
    }

    // The approve-set is built from a WIDER set of roots than the policed one: a
    // `storage` decode targets types declared in `common`. Same read-and-parse discipline
    // — a file missed here would silently shrink what the gate accepts, which changes the
    // rule rather than the population, and is worse.
    let mut approve = ApproveSet::default();
    // Delegation is only sound where composite policing runs, and that link is a *string*
    // match between the two consts. Check it rather than assume it: a `DECLARATION_ROOTS`
    // that spells the policed root differently would silently stop collecting composites.
    // That direction fails closed (every composite target becomes unrecognised and the
    // gate goes loudly red), so this is about naming the cause, not preventing a silent
    // hole.
    if !DECLARATION_ROOTS.contains(&POLICED_ROOT) {
        unreadable.push(format!(
            "DECLARATION_ROOTS does not contain POLICED_ROOT ({POLICED_ROOT}) — composite \
             delegation is scoped by matching the two, so nothing would be approved by \
             delegation and every row-struct target would fail as unrecognised."
        ));
    }
    for root in DECLARATION_ROOTS {
        let kind = if *root == POLICED_ROOT {
            Root::Policed
        } else {
            Root::DeclarationsOnly
        };
        match files::with_extension(Path::new(root), "rs") {
            Ok(decls) => {
                for p in &decls {
                    let path = p.display().to_string();
                    match std::fs::read_to_string(p) {
                        Ok(s) => {
                            if let Err(e) = collect_declarations(&s, kind, &mut approve) {
                                unreadable.push(format!(
                                    "{path}: {e} — this gate's approved-type set is built from \
                                     the declarations here, so an unparsed file shrinks what it \
                                     accepts."
                                ));
                            }
                        }
                        Err(e) => unreadable.push(format!(
                            "{path}: cannot read: {e} — a declaration file this gate cannot read \
                             is an approve-set it cannot trust."
                        )),
                    }
                }
            }
            Err(e) => unreadable.push(format!("cannot scan declaration root {root}: {e}")),
        }
    }
    approve
        .approved
        .extend(APPROVED_FOREIGN.iter().map(|(n, _)| (*n).to_string()));

    let detail = match (problems(&scanned, &approve), unreadable.is_empty()) {
        (None, true) => {
            result.push(StepResult::ok("sqlx-newtype-decode"));
            return;
        }
        (found, _) => {
            let mut lines = unreadable;
            lines.extend(found);
            lines.join("\n")
        }
    };
    result.push(StepResult::fail("sqlx-newtype-decode").detail(detail));
}

#[cfg(test)]
mod tests {
    use super::super::approve_set::approve;
    use super::*;

    /// [`problems`] against the synthetic approve-set.
    fn problems_of(scanned: &[(String, String)]) -> Option<String> {
        problems(scanned, &approve())
    }

    fn problems_of_with_allowlist(
        scanned: &[(String, String)],
        allowlist: &[Allowed],
    ) -> Option<String> {
        problems_with_allowlist(scanned, &approve(), allowlist)
    }

    /// A source with `n` identical allowlisted `COUNT(*)` decodes.
    fn identical_sites(n: usize) -> String {
        let decodes: String = (0..n)
            .map(|_| {
                r#"sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
                )
                .fetch_one(pool)
                .await?;
"#
                .to_string()
            })
            .collect();
        format!(
            "async fn backup_covers_every_table_or_deliberately_excludes_it() -> Result<(), sqlx::Error> {{ {decodes} Ok(()) }}"
        )
    }

    fn backup_count_entry() -> Allowed {
        Allowed {
            file: "backup.rs",
            function: "backup_covers_every_table_or_deliberately_excludes_it",
            target: "i64",
            what: "\"SELECTCOUNT(*)FROMsqlite_masterWHEREtype='table'ANDnameNOTLIKE'sqlite_%'\"",
            count: 1,
            category: Category::CountOrExists,
            reason: "COUNT(*) of live SQLite tables, checked against the backup manifest",
        }
    }

    #[test]
    fn an_entry_count_passes_on_one_and_fails_on_two() {
        // The property that stops an entry becoming a region exemption. The real
        // backup-manifest entry declares 1; a second identical decode must NOT be
        // silently absorbed by it.
        //
        // Scanning one file in isolation legitimately makes the other nine entries
        // stale, so the assertion is scoped to this entry's own message rather than to
        // `problems` returning `None`.
        let one = vec![("storage/src/backup.rs".to_string(), identical_sites(1))];
        let allowlist = [backup_count_entry()];
        let one_detail = problems_of_with_allowlist(&one, &allowlist).unwrap_or_default();
        // Match the failure phrasing, not the bare key — the recovery footer lists
        // every entry by key, including this one.
        assert!(
            !one_detail.contains(
                "backup.rs::backup_covers_every_table_or_deliberately_excludes_it: allowlist entry"
            ),
            "one site matches the declared count, so this entry must not complain: {one_detail}"
        );

        let two = vec![("storage/src/backup.rs".to_string(), identical_sites(2))];
        let detail = problems_of_with_allowlist(&two, &allowlist)
            .expect("a second identical decode must fail");
        assert!(
            detail.contains("declares 1 site(s), the tree has 2"),
            "{detail}"
        );
    }

    #[test]
    fn an_entry_exempts_only_the_decode_it_names() {
        // A different `i64` decode in the same allowlisted function is still a failure —
        // the entry covers one decode, never a region.
        let src = format!(
            "{} {}",
            identical_sites(1),
            "async fn backup_covers_every_table_or_deliberately_excludes_it_extra() -> Result<(), sqlx::Error> { \
             let _: i64 = sqlx::query_scalar(\"SELECT owner_id FROM t\").fetch_one(pool).await?; \
             Ok(()) \
             }"
        );
        let allowlist = [backup_count_entry()];
        let detail =
            problems_of_with_allowlist(&[("storage/src/backup.rs".to_string(), src)], &allowlist)
                .expect("the unlisted sibling decode must fail");
        assert!(detail.contains("SELECTowner_idFROMt"), "{detail}");
    }

    #[test]
    fn an_unallowlisted_id_decode_is_flagged() {
        // The headline case: reverting any swept site must fail the gate.
        let src = r#"
            impl S {
                async fn create(&self) -> Result<UserId, E> {
                    let id = sqlx::query_scalar::<_, i64>("INSERT INTO users RETURNING user_id")
                        .fetch_one(&self.pool).await?;
                    Ok(UserId::from(id))
                }
            }
        "#;
        let detail = problems_of(&[("storage/src/users.rs".to_string(), src.to_string())])
            .expect("a bare i64 id decode must fail");
        assert!(detail.contains("storage/src/users.rs"), "{detail}");
        assert!(detail.contains("decodes into `i64`"), "{detail}");
    }

    #[test]
    fn a_stale_entry_with_no_matching_site_is_reported() {
        // An allowlist that stops tracking the tree has quietly become a region
        // exemption, so a vanished site is a failure too, not a free pass.
        let detail = problems_of(&[("storage/src/users.rs".to_string(), String::new())])
            .expect("every entry is now stale");
        assert!(
            detail.contains("The decode is gone — delete the entry."),
            "{detail}"
        );
    }

    #[test]
    fn an_unparseable_file_is_a_failure_not_a_skip() {
        let detail = problems_of(&[("storage/src/broken.rs".to_string(), "fn f( {{{".to_string())])
            .expect("an unparsed file must fail");
        assert!(detail.contains("invisible to this gate"), "{detail}");
    }
}
