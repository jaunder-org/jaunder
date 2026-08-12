//! Provision `end2end/node_modules` for the offline `tsc --noEmit` type-check gate.
//!
//! Why this exists: `end2end/node_modules` is gitignored, so it is absent from every
//! fresh checkout and every git worktree. The type-dep closure tsc needs
//! (`@types/node`, `undici-types`, `typescript`, `playwright`/`-core`, and
//! `@playwright/test`) all come from the Nix `e2ePackage` / `playwright-test` store
//! paths, which the devShell exports as `E2E_TYPES_NODE_MODULES` and
//! `E2E_PLAYWRIGHT_TEST`.
//!
//! Two callers, both with the repo/worktree root as cwd: the devShell `shellHook`
//! (interactive IDE support) invokes `devtool provision-node-modules`, and
//! [`crate::check`] calls [`run`] in-process before `tsc`, so `cargo xtask
//! check|validate` self-heals in a worktree where the shellHook never fired for that
//! cwd (#229).

use anyhow::{Context, Result, bail};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// The two Nix store paths provisioning needs. They are always resolved together and
/// always consumed together, so they travel as one value rather than as a pair of
/// arguments every caller has to order correctly.
pub struct StorePaths {
    /// The tsc type-dep closure (`${e2ePackage}/node_modules`).
    types_node_modules: PathBuf,
    /// The nix-matched `@playwright/test`.
    playwright_test: PathBuf,
}

impl StorePaths {
    /// Resolve both paths: an explicit flag wins, else the devShell's env var.
    ///
    /// The flag→variable pairing lives here alone, so the subcommand and
    /// [`crate::check`] cannot drift apart and there is exactly one place that
    /// produces the unset-variable message.
    pub fn resolve(
        types_node_modules: Option<PathBuf>,
        playwright_test: Option<PathBuf>,
    ) -> Result<Self> {
        Ok(Self {
            types_node_modules: resolve_store_path(types_node_modules, "E2E_TYPES_NODE_MODULES")?,
            playwright_test: resolve_store_path(playwright_test, "E2E_PLAYWRIGHT_TEST")?,
        })
    }
}

/// Resolve one store path: the explicit flag wins, else the devShell's `var`.
///
/// Reads the fallback as an `OsString` because these are paths, which need not be
/// UTF-8, and treats an empty value as unset — matching the bash it replaces, whose
/// `: "${VAR:?}"` guard rejected empty. Letting `""` through would defer the failure
/// to an obscure `read_dir` error instead of naming the variable.
fn resolve_store_path(flag: Option<PathBuf>, var: &str) -> Result<PathBuf> {
    if let Some(path) = flag {
        return Ok(path);
    }
    match std::env::var_os(var) {
        Some(value) if !value.is_empty() => Ok(PathBuf::from(value)),
        _ => bail!("{var} is unset — run inside the Nix devShell (nix develop)"),
    }
}

/// Delete whatever is at `path`, symlink or directory or file, like `rm -rf`.
///
/// Uses `symlink_metadata`, which does **not** follow links: a stale symlink into the
/// Nix store must be unlinked, never recursed into and deleted.
fn remove_any(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path)
            .with_context(|| format!("removing directory {}", path.display()))?,
        Ok(_) => {
            fs::remove_file(path).with_context(|| format!("removing {}", path.display()))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("inspecting {}", path.display())),
    }
    Ok(())
}

/// Symlink the type-dep closure into `<root>/end2end/node_modules`, then re-pin
/// `@playwright/test` to the nix-matched Playwright (browser-driver parity + IDE
/// support) instead of the closure's own npm copy.
///
/// Idempotent: each target is removed before it is linked, so a re-run overwrites
/// cleanly and the plain `@playwright` directory can replace an earlier symlink of the
/// same name. Dot-entries are skipped — linking `.bin` would change what tsc sees,
/// which is a separate decision.
pub fn run(root: &Path, paths: &StorePaths) -> Result<()> {
    let types_node_modules = paths.types_node_modules.as_path();
    let dest = root.join("end2end/node_modules");
    fs::create_dir_all(&dest).with_context(|| format!("creating {}", dest.display()))?;

    let entries = fs::read_dir(types_node_modules).with_context(|| {
        format!(
            "reading the type-dep closure {}",
            types_node_modules.display()
        )
    })?;
    for entry in entries {
        let entry = entry
            .with_context(|| format!("reading an entry of {}", types_node_modules.display()))?;
        let name = entry.file_name();
        // Compare bytes, not a lossy String: these are paths, which need not be UTF-8.
        if name.as_bytes().starts_with(b".") {
            continue;
        }
        let target = dest.join(&name);
        remove_any(&target)?;
        std::os::unix::fs::symlink(entry.path(), &target).with_context(|| {
            format!("linking {} -> {}", target.display(), entry.path().display())
        })?;
    }

    // After the loop, so this wins over the closure's own `@playwright` entry.
    let playwright_dir = dest.join("@playwright");
    remove_any(&playwright_dir)?;
    fs::create_dir_all(&playwright_dir)
        .with_context(|| format!("creating {}", playwright_dir.display()))?;
    let link = playwright_dir.join("test");
    let playwright_test = paths.playwright_test.as_path();
    std::os::unix::fs::symlink(playwright_test, &link).with_context(|| {
        format!(
            "linking {} -> {}",
            link.display(),
            playwright_test.display()
        )
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A fake `E2E_TYPES_NODE_MODULES` closure: the real entries tsc needs, plus
    /// the dot-entries a real npm tree carries and an `@playwright` that must lose
    /// to `--playwright-test`.
    fn fake_types_dir(base: &Path, marker: &str) -> PathBuf {
        let dir = base.join(marker);
        for name in [
            "@types",
            "typescript",
            "undici-types",
            "playwright",
            "playwright-core",
            "@playwright",
            ".bin",
        ] {
            fs::create_dir_all(dir.join(name)).unwrap();
        }
        fs::write(dir.join(".package-lock.json"), "{}").unwrap();
        dir
    }

    fn fake_playwright(base: &Path) -> PathBuf {
        let dir = base.join("playwright-test");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tmp() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("devtool-provision-")
            .tempdir()
            .unwrap()
    }

    /// Explicit paths, bypassing the env fallback — these tests are about what `run`
    /// writes, not about where the paths came from.
    fn paths(types_node_modules: &Path, playwright_test: &Path) -> StorePaths {
        StorePaths {
            types_node_modules: types_node_modules.to_path_buf(),
            playwright_test: playwright_test.to_path_buf(),
        }
    }

    #[test]
    fn provisions_visible_entries_as_symlinks() {
        let t = tmp();
        let types = fake_types_dir(t.path(), "store-a");
        let pw = fake_playwright(t.path());
        run(t.path(), &paths(&types, &pw)).unwrap();

        let dest = t.path().join("end2end/node_modules");
        for name in [
            "@types",
            "typescript",
            "undici-types",
            "playwright",
            "playwright-core",
        ] {
            let link = dest.join(name);
            assert!(
                fs::symlink_metadata(&link).unwrap().is_symlink(),
                "{name} should be a symlink"
            );
            assert_eq!(fs::read_link(&link).unwrap(), types.join(name));
        }
    }

    #[test]
    fn skips_dot_entries() {
        let t = tmp();
        let types = fake_types_dir(t.path(), "store-a");
        let pw = fake_playwright(t.path());
        run(t.path(), &paths(&types, &pw)).unwrap();

        let dest = t.path().join("end2end/node_modules");
        for name in [".bin", ".package-lock.json"] {
            assert!(
                fs::symlink_metadata(dest.join(name)).is_err(),
                "{name} must not be provisioned"
            );
        }
    }

    #[test]
    fn pins_playwright_test_over_e2e_package_copy() {
        let t = tmp();
        let types = fake_types_dir(t.path(), "store-a");
        let pw = fake_playwright(t.path());
        run(t.path(), &paths(&types, &pw)).unwrap();

        let at_pw = t.path().join("end2end/node_modules/@playwright");
        assert!(
            fs::symlink_metadata(&at_pw).unwrap().is_dir(),
            "@playwright must be a real directory, not a symlink to the closure's copy"
        );
        assert_eq!(fs::read_link(at_pw.join("test")).unwrap(), pw);
        let entries: Vec<_> = fs::read_dir(&at_pw)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("test")]);
    }

    #[test]
    fn is_idempotent_across_reruns() {
        let t = tmp();
        let types = fake_types_dir(t.path(), "store-a");
        let pw = fake_playwright(t.path());
        run(t.path(), &paths(&types, &pw)).unwrap();
        run(t.path(), &paths(&types, &pw)).unwrap();

        let dest = t.path().join("end2end/node_modules");
        assert_eq!(
            fs::read_link(dest.join("typescript")).unwrap(),
            types.join("typescript")
        );
        assert!(
            fs::symlink_metadata(dest.join("@playwright"))
                .unwrap()
                .is_dir()
        );
        assert_eq!(fs::read_link(dest.join("@playwright/test")).unwrap(), pw);
    }

    #[test]
    fn replaces_a_stale_playwright_symlink_with_the_dir() {
        let t = tmp();
        let types = fake_types_dir(t.path(), "store-a");
        let pw = fake_playwright(t.path());
        let dest = t.path().join("end2end/node_modules");
        fs::create_dir_all(&dest).unwrap();
        // A previous run (or a hand-rolled tree) left @playwright as a symlink.
        std::os::unix::fs::symlink(types.join("@playwright"), dest.join("@playwright")).unwrap();

        run(t.path(), &paths(&types, &pw)).unwrap();

        assert!(
            fs::symlink_metadata(dest.join("@playwright"))
                .unwrap()
                .is_dir()
        );
        assert_eq!(fs::read_link(dest.join("@playwright/test")).unwrap(), pw);
    }

    #[test]
    fn repoints_symlinks_when_the_store_path_changes() {
        let t = tmp();
        let old = fake_types_dir(t.path(), "store-a");
        let new = fake_types_dir(t.path(), "store-b");
        let pw = fake_playwright(t.path());
        run(t.path(), &paths(&old, &pw)).unwrap();
        run(t.path(), &paths(&new, &pw)).unwrap();

        let dest = t.path().join("end2end/node_modules");
        assert_eq!(
            fs::read_link(dest.join("typescript")).unwrap(),
            new.join("typescript")
        );
    }

    #[test]
    fn errors_when_types_dir_missing() {
        let t = tmp();
        let missing = t.path().join("no-such-store");
        let pw = fake_playwright(t.path());
        let err = run(t.path(), &paths(&missing, &pw)).unwrap_err();
        assert!(
            format!("{err:#}").contains(&missing.display().to_string()),
            "error should name the missing path, got: {err:#}"
        );
    }

    #[test]
    fn resolve_prefers_the_flag_over_the_environment() {
        let flag = PathBuf::from("/from/flag");
        // Deliberately a variable that is set in every environment.
        assert_eq!(
            resolve_store_path(Some(flag.clone()), "PATH").unwrap(),
            flag
        );
    }

    #[test]
    fn resolve_errors_name_the_variable_and_the_devshell() {
        let err = resolve_store_path(None, "JAUNDER_DEFINITELY_UNSET_FOR_TESTS").unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("JAUNDER_DEFINITELY_UNSET_FOR_TESTS"),
            "got: {msg}"
        );
        assert!(msg.contains("nix develop"), "got: {msg}");
    }
}
