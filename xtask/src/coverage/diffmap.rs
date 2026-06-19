use std::collections::{HashMap, HashSet};

/// Maps old (HEAD) line numbers to new (working-tree) line numbers for one file.
/// Built by walking unified-diff hunks: context lines map 1:1 (with the running
/// offset), deleted lines map to None, added lines advance the new counter only.
#[derive(Default)]
pub struct LineMap {
    map: HashMap<u32, Option<u32>>,
    // For old lines outside any hunk, apply the cumulative offset that is in
    // effect at the start of an old line. We record, for each old line that
    // begins a new offset regime, the (old_line, cumulative_offset) so that any
    // old line at or after it (and before the next boundary) shifts by `off`.
    offset_after: Vec<(u32, i64)>, // (old_line_from, cumulative_offset)
    // New-side line numbers introduced by `+` hunks (no old preimage). These
    // are the "newly added" lines the classifier uses to tell a brand-new
    // uncovered line (new_uncovered) from a previously-covered one (regression).
    added: HashSet<u32>,
}

impl LineMap {
    pub fn map(&self, old_line: u32) -> Option<u32> {
        if let Some(v) = self.map.get(&old_line) {
            return *v;
        }
        // Outside all hunks: new = old + offset in effect at this old line.
        let mut offset = 0i64;
        for (from, off) in &self.offset_after {
            if old_line >= *from {
                offset = *off;
            } else {
                break;
            }
        }
        Some((old_line as i64 + offset) as u32)
    }

    /// New-side (working-tree) line numbers introduced by `+` hunks — lines that
    /// have no HEAD preimage. Under an identity/empty map this is empty.
    pub fn added_lines(&self) -> HashSet<u32> {
        self.added.clone()
    }

    /// Test-only: directly install an old→new mapping (or a deletion via `None`).
    #[cfg(test)]
    pub fn set_for_test(&mut self, old: u32, new: Option<u32>) {
        self.map.insert(old, new);
    }

    /// Test-only: mark a new-side line number as freshly added (no preimage).
    #[cfg(test)]
    pub fn set_added_for_test(&mut self, new: u32) {
        self.added.insert(new);
    }
}

impl LineMap {
    /// Build a map for an untracked/new file whose every line is freshly added
    /// (no HEAD preimage). `git diff HEAD` omits untracked files, so without this
    /// such a file falls back to the identity `empty_map`, whose `added_lines()`
    /// is empty — and its uncovered lines would be mislabeled `regression` rather
    /// than `new_uncovered`. The identity `map()` is fine: these lines have no old
    /// preimage, so only `added_lines()` matters for the classifier.
    pub fn all_added(lines: &[u32]) -> LineMap {
        LineMap {
            added: lines.iter().copied().collect(),
            ..LineMap::default()
        }
    }
}

pub fn empty_map() -> LineMap {
    LineMap::default()
}

pub fn parse_unified_diff(diff: &str) -> HashMap<String, LineMap> {
    let mut out: HashMap<String, LineMap> = HashMap::new();
    let mut cur_path: Option<String> = None;
    let mut lm = LineMap::default();
    let mut old_ln = 0u32;
    let mut new_ln = 0u32;
    let mut cum_offset = 0i64;

    let flush =
        |out: &mut HashMap<String, LineMap>, path: &mut Option<String>, lm: &mut LineMap| {
            if let Some(p) = path.take() {
                out.insert(p, std::mem::take(lm));
            }
        };

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ b/") {
            flush(&mut out, &mut cur_path, &mut lm);
            cur_path = Some(rest.to_string());
            old_ln = 0;
            new_ln = 0;
            cum_offset = 0;
            continue;
        }
        if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") {
            continue;
        }
        if let Some(h) = line.strip_prefix("@@") {
            // @@ -oldStart,oldCount +newStart,newCount @@
            // We set the old/new counters to the hunk starts but do NOT touch
            // cum_offset: it carries across hunks, and each insertion/deletion
            // inside the hunk records its own boundary anchored at the precise
            // old line where the change takes effect. (A hunk header by itself
            // changes no line's image; only its +/- body lines do.)
            if let Some((os, ns)) = parse_hunk_header(h) {
                old_ln = os;
                new_ln = ns;
            }
            continue;
        }
        if cur_path.is_none() {
            continue;
        }
        match line.chars().next() {
            Some(' ') => {
                lm.map.insert(old_ln, Some(new_ln));
                old_ln += 1;
                new_ln += 1;
            }
            Some('-') => {
                lm.map.insert(old_ln, None);
                old_ln += 1;
                cum_offset -= 1;
                lm.offset_after.push((old_ln, cum_offset));
            }
            Some('+') => {
                lm.added.insert(new_ln);
                new_ln += 1;
                cum_offset += 1;
                // An added line is inserted *after* the current old line, so it
                // only shifts old lines strictly greater than `old_ln`. Anchor
                // the new offset at `old_ln + 1`.
                lm.offset_after.push((old_ln + 1, cum_offset));
            }
            _ => {}
        }
    }
    flush(&mut out, &mut cur_path, &mut lm);
    out
}

fn parse_hunk_header(h: &str) -> Option<(u32, u32)> {
    // h like " -1,4 +1,4 @@ ..."
    let h = h.trim_start();
    let mut it = h.split_whitespace();
    let old = it.next()?.strip_prefix('-')?;
    let new = it.next()?.strip_prefix('+')?;
    let old_start = old.split(',').next()?.parse().ok()?;
    let new_start = new.split(',').next()?.parse().ok()?;
    Some((old_start, new_start))
}

#[cfg(test)]
mod tests {
    use super::*;

    // One line deleted at old line 2, one added after old line 3.
    const DIFF: &str = "\
diff --git a/server/src/x.rs b/server/src/x.rs
--- a/server/src/x.rs
+++ b/server/src/x.rs
@@ -1,4 +1,4 @@
 line1
-line2_old
 line3
+line_new
 line4
";

    #[test]
    fn maps_unchanged_deleted_and_shifts() {
        let maps = parse_unified_diff(DIFF);
        let m = maps.get("server/src/x.rs").unwrap();
        assert_eq!(m.map(1), Some(1)); // unchanged context
        assert_eq!(m.map(2), None); // deleted
                                    // The plan's starter test asserted Some(3) here, but that is wrong: the
                                    // deletion of old line 2 shifts old line 3's content up to new line 2
                                    // *before* the insertion below it. New file is line1/line3/line_new/line4,
                                    // so old line 3 ("line3") is content-accurately at new line 2. Verified
                                    // against real `git diff` (and `git diff --unified=0`, which yields the
                                    // same image via the offset walk).
        assert_eq!(m.map(3), Some(2)); // content moved up by the deletion above
        assert_eq!(m.map(4), Some(4)); // net +0 around it (-1 delete, +1 insert)
    }

    #[test]
    fn empty_map_is_identity() {
        assert_eq!(empty_map().map(42), Some(42));
    }

    #[test]
    fn added_lines_collects_plus_hunk_new_numbers() {
        // The DIFF inserts one line: new file is line1/line3/line_new/line4, so
        // "line_new" is at new line 3.
        let maps = parse_unified_diff(DIFF);
        let m = maps.get("server/src/x.rs").unwrap();
        let added: Vec<u32> = {
            let mut v: Vec<u32> = m.added_lines().into_iter().collect();
            v.sort();
            v
        };
        assert_eq!(added, vec![3]);
    }

    #[test]
    fn empty_map_has_no_added_lines() {
        assert!(empty_map().added_lines().is_empty());
    }

    // ---- additional tests ----

    // Two hunks in one file. Hunk 1 (around old line 2) inserts one line;
    // hunk 2 (around old line 10) deletes one line. Offsets must accumulate.
    const MULTI_HUNK: &str = "\
diff --git a/m.rs b/m.rs
--- a/m.rs
+++ b/m.rs
@@ -1,3 +1,4 @@
 a1
 a2
+inserted
 a3
@@ -9,3 +10,2 @@
 b1
-deleted
 b2
";

    #[test]
    fn multiple_hunks_accumulate_offsets() {
        let maps = parse_unified_diff(MULTI_HUNK);
        let m = maps.get("m.rs").unwrap();
        // Hunk 1: lines 1,2,3 are context; insertion after old line 2.
        assert_eq!(m.map(1), Some(1));
        assert_eq!(m.map(2), Some(2));
        assert_eq!(m.map(3), Some(4)); // shifted +1 by the insertion
                                       // Between the hunks (old lines 4..8): only the +1 from hunk 1 applies.
        assert_eq!(m.map(4), Some(5));
        assert_eq!(m.map(8), Some(9));
        // Hunk 2 starts at old line 9: b1=9 context, deleted=10, b2=11.
        assert_eq!(m.map(9), Some(10)); // +1 net so far
        assert_eq!(m.map(10), None); // deleted
        assert_eq!(m.map(11), Some(11)); // +1 (insert) -1 (delete) = 0 net
                                         // After both hunks: net offset 0.
        assert_eq!(m.map(12), Some(12));
        assert_eq!(m.map(100), Some(100));
    }

    // Pure-addition file: with --unified=0 git emits @@ -N,0 +M,K @@; old lines
    // before and after the insertion must still map correctly.
    const PURE_ADD: &str = "\
diff --git a/add.rs b/add.rs
--- a/add.rs
+++ b/add.rs
@@ -2,0 +3,2 @@ ctx
+new_a
+new_b
";

    #[test]
    fn pure_addition_shifts_following_lines() {
        let maps = parse_unified_diff(PURE_ADD);
        let m = maps.get("add.rs").unwrap();
        // Insertion happens after old line 2 (old has 0 lines in this hunk).
        // Old lines 1,2 are before the insertion → unchanged.
        assert_eq!(m.map(1), Some(1));
        assert_eq!(m.map(2), Some(2));
        // Old lines 3+ are after the 2-line insertion → shifted by +2.
        assert_eq!(m.map(3), Some(5));
        assert_eq!(m.map(50), Some(52));
    }

    // Pure-deletion: with --unified=0 git emits @@ -N,K +M,0 @@.
    const PURE_DEL: &str = "\
diff --git a/del.rs b/del.rs
--- a/del.rs
+++ b/del.rs
@@ -3,2 +2,0 @@ ctx
-gone_a
-gone_b
";

    #[test]
    fn pure_deletion_maps_to_none_and_shifts_up() {
        let maps = parse_unified_diff(PURE_DEL);
        let m = maps.get("del.rs").unwrap();
        // Old lines 1,2 before the deletion → unchanged.
        assert_eq!(m.map(1), Some(1));
        assert_eq!(m.map(2), Some(2));
        // Deleted old lines 3,4.
        assert_eq!(m.map(3), None);
        assert_eq!(m.map(4), None);
        // Old lines after the deletion shift up by 2.
        assert_eq!(m.map(5), Some(3));
        assert_eq!(m.map(50), Some(48));
    }

    // A line after a net-insertion hunk must shift down; after a net-deletion
    // hunk must shift up. Covered with two separate single-hunk diffs.
    const NET_INSERT: &str = "\
diff --git a/ins.rs b/ins.rs
--- a/ins.rs
+++ b/ins.rs
@@ -5,0 +6,3 @@ ctx
+x
+y
+z
";

    #[test]
    fn line_after_net_insertion_shifts_down() {
        let maps = parse_unified_diff(NET_INSERT);
        let m = maps.get("ins.rs").unwrap();
        assert_eq!(m.map(5), Some(5)); // before insertion
        assert_eq!(m.map(6), Some(9)); // after 3-line insertion → +3
    }

    const NET_DELETE: &str = "\
diff --git a/d.rs b/d.rs
--- a/d.rs
+++ b/d.rs
@@ -6,3 +5,0 @@ ctx
-a
-b
-c
";

    #[test]
    fn line_after_net_deletion_shifts_up() {
        let maps = parse_unified_diff(NET_DELETE);
        let m = maps.get("d.rs").unwrap();
        assert_eq!(m.map(5), Some(5)); // before deletion
        assert_eq!(m.map(6), None); // deleted
        assert_eq!(m.map(9), Some(6)); // after 3-line deletion → -3
    }

    // Two files in one diff → independent maps.
    const TWO_FILES: &str = "\
diff --git a/f1.rs b/f1.rs
--- a/f1.rs
+++ b/f1.rs
@@ -1,2 +1,3 @@
 keep
+added
 tail
diff --git a/f2.rs b/f2.rs
--- a/f2.rs
+++ b/f2.rs
@@ -1,2 +1,1 @@
 keep
-removed
";

    #[test]
    fn two_files_have_independent_maps() {
        let maps = parse_unified_diff(TWO_FILES);
        let f1 = maps.get("f1.rs").unwrap();
        let f2 = maps.get("f2.rs").unwrap();
        // f1: insertion after line 1 → line 2 shifts down.
        assert_eq!(f1.map(1), Some(1));
        assert_eq!(f1.map(2), Some(3));
        assert_eq!(f1.map(10), Some(11));
        // f2: deletion of line 2 → line 1 unchanged, 2 gone, rest shifts up.
        assert_eq!(f2.map(1), Some(1));
        assert_eq!(f2.map(2), None);
        assert_eq!(f2.map(3), Some(2));
        assert_eq!(f2.map(10), Some(9));
    }
}
