use std::process::Command;

use anyhow::{Context, Result};

/// A conservative whole-tree key: every git-tracked file's content (via the
/// blob SHAs in `git ls-files -s`), plus Cargo.lock and the exact toolchain.
/// Whole-tree (not per-crate) keeps it sound — coverage is a whole-suite property.
pub fn tree_key() -> Result<String> {
    let files = Command::new("git")
        .args(["ls-files", "-s"])
        .output()
        .context("git ls-files")?;
    let lock = std::fs::read("Cargo.lock").unwrap_or_default();
    let toolchain = Command::new("rustc")
        .arg("-Vv")
        .output()
        .context("rustc -Vv")?;

    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    files.stdout.hash(&mut hasher);
    lock.hash(&mut hasher);
    toolchain.stdout.hash(&mut hasher);
    Ok(format!("{:016x}", hasher.finish()))
}

pub fn last_green(command: &str) -> Option<String> {
    std::fs::read_to_string(format!(".xtask/green-{command}.key"))
        .ok()
        .map(|s| s.trim().to_string())
}

pub fn record_green(command: &str, key: &str) -> Result<()> {
    std::fs::create_dir_all(".xtask")?;
    std::fs::write(format!(".xtask/green-{command}.key"), key)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_key_is_stable_for_unchanged_inputs() {
        let a = tree_key().unwrap();
        let b = tree_key().unwrap();
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }
}
