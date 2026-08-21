use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Rust-compiling host checks opt into `sccache`. Cargo incremental artifacts
/// are target-dir-local and non-cacheable, so these gate steps trade same-target
/// incremental reuse for cross-checkout `rustc` reuse.
const CARGO_COMPILE_ENV: &[(&str, &str)] =
    &[("RUSTC_WRAPPER", "sccache"), ("CARGO_INCREMENTAL", "0")];

pub fn cargo_compile_env() -> (Vec<(String, String)>, Option<String>) {
    let workspace_root = env::current_dir()
        .ok()
        .and_then(|path| path.canonicalize().ok())
        .or_else(|| env::current_dir().ok());
    match workspace_root.as_deref() {
        Some(root) => cargo_compile_env_for(root),
        None => (
            static_compile_cache_env(),
            Some(
                "sccache worktree discovery skipped: could not resolve current directory"
                    .to_string(),
            ),
        ),
    }
}

fn cargo_compile_env_for(workspace_root: &Path) -> (Vec<(String, String)>, Option<String>) {
    let (basedirs, warnings) = sccache_basedirs(workspace_root);
    let mut env = static_compile_cache_env();
    if !basedirs.is_empty() {
        env.push(("SCCACHE_BASEDIRS".to_string(), basedirs));
    }
    let detail = if warnings.is_empty() {
        None
    } else {
        Some(format!(
            "sccache worktree discovery warning: {}",
            warnings.join("; ")
        ))
    };
    (env, detail)
}

fn static_compile_cache_env() -> Vec<(String, String)> {
    CARGO_COMPILE_ENV
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

fn sccache_basedirs(current_root: &Path) -> (String, Vec<String>) {
    let mut warnings = Vec::new();
    let mut roots = BTreeSet::new();
    match Command::new("git")
        .arg("-C")
        .arg(current_root)
        .args(["worktree", "list", "--porcelain"])
        .output()
    {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            roots.extend(parse_worktree_roots(&stdout));
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warnings.push(format!(
                "`git worktree list --porcelain` failed: {}",
                stderr.trim()
            ));
        }
        Err(err) => warnings.push(format!("could not run `git worktree list`: {err}")),
    }
    if let Some(current_root) = normalized_existing_root(&current_root.display().to_string()) {
        roots.insert(current_root);
    }
    let separator = if cfg!(windows) { ";" } else { ":" };
    let basedirs = roots
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(separator);
    (basedirs, warnings)
}

fn parse_worktree_roots(output: &str) -> BTreeSet<PathBuf> {
    output
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .filter_map(normalized_existing_root)
        .collect()
}

fn normalized_existing_root(raw: &str) -> Option<PathBuf> {
    let path = PathBuf::from(raw);
    if !path.is_absolute() || !path.is_dir() {
        return None;
    }
    path.canonicalize().ok().or(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parse_worktree_roots_keeps_existing_absolute_worktrees() {
        let temp = tempfile::tempdir().unwrap();
        let root_a = temp.path().join("a");
        let root_b = temp.path().join("b");
        fs::create_dir_all(&root_a).unwrap();
        fs::create_dir_all(&root_b).unwrap();
        let output = format!(
            "worktree {}\nHEAD abc\n\nworktree {}\nHEAD def\n\nworktree /not/a/checkout\n",
            root_a.display(),
            root_b.display()
        );

        let roots = parse_worktree_roots(&output);

        assert_eq!(roots.len(), 2);
        assert!(roots.contains(&root_a));
        assert!(roots.contains(&root_b));
    }

    #[test]
    fn compile_cache_env_contains_wrapper_incremental_and_worktree_basedirs_only() {
        let (env, _detail) = cargo_compile_env();

        assert!(env.contains(&("RUSTC_WRAPPER".to_string(), "sccache".to_string())));
        assert!(env.contains(&("CARGO_INCREMENTAL".to_string(), "0".to_string())));
        assert!(env.iter().any(|(key, _)| key == "SCCACHE_BASEDIRS"));
        assert!(
            !env.iter()
                .any(|(key, _)| key == "CARGO_PROFILE_DEV_DEBUG"
                    || key == "CARGO_PROFILE_TEST_DEBUG")
        );
    }
}
