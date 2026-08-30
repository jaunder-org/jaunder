use std::path::Path;

use super::approve_set::{
    APPROVED_FOREIGN, ApproveSet, DECLARATION_ROOTS, Root, collect_declarations,
};
use super::macros_audit::{MACROS_LIB, macro_enumeration_problems};
use super::scan::decodes;
use crate::files;
use crate::result::{CommandResult, StepResult};

/// Source root scanned recursively for `.rs` files.
pub(super) const POLICED_ROOT: &str = "storage/src";

/// The failure detail for structurally found decodes whose leaves are not declaration-backed
/// approvals, or `None` when every readable target is approved. Pure given the `(path, source)`
/// pairs, so it is unit-tested directly.
fn problems(scanned: &[(String, String)], approve: &ApproveSet) -> Option<String> {
    let mut found = Vec::new();
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

    for (path, d) in &found {
        lines.push(format!(
            "{path}:{}: `{}` decodes into `{}`, whose leaf type(s) {} are not approved column \
             types. Decode it into a declaration-backed type: the ADR-0071 bridge makes \
             `query_scalar::<_, PostId>` work, and explicit persisted role types make raw \
             storage values equally visible. This gate reads no SQL and has no site exemption; \
             its only recovery is an approved target type.",
            d.line,
            d.what,
            d.target,
            d.unapproved.join(", ")
        ));
    }

    (!lines.is_empty()).then(|| lines.join("\n"))
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

    #[test]
    fn a_novel_bare_primitive_decode_is_flagged_by_its_unapproved_type() {
        // `u128` is not a type the gate anticipates. It fails solely because it has no
        // declaration-backed approval, not because the call or SQL spelling is recognised.
        let src = r#"
            async fn read_unanticipated_measurement(pool: &sqlx::Pool<sqlx::Sqlite>) {
                let _: u128 = sqlx::query_scalar("SELECT unanticipated_measurement FROM t")
                    .fetch_one(pool).await.unwrap();
            }
        "#;
        let detail = problems_of(&[("storage/src/novel.rs".to_string(), src.to_string())])
            .expect("a bare primitive without approval must fail");
        assert!(detail.contains("decodes into `u128`"), "{detail}");
        assert!(
            detail.contains("leaf type(s) u128 are not approved"),
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
