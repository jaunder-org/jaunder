use std::{io::Write, path::Path};

use anyhow::Context;

/// Validates and compiles a theme repository without opening application storage.
///
/// # Errors
///
/// Returns the filesystem, Theme Package, or CSS validation failure.
pub fn cmd_theme_check(repository: &Path) -> anyhow::Result<()> {
    host::theme_repository::accept_theme_repository(repository).with_context(|| {
        format!(
            "theme repository validation failed: {}",
            repository.display()
        )
    })?;
    println!("Theme repository is valid: {}", repository.display());
    Ok(())
}

/// Validates a repository and atomically publishes its canonical package without replacement.
///
/// # Errors
///
/// Returns an error if the repository is invalid, the destination already exists, or publication
/// cannot complete atomically.
pub fn cmd_theme_package(repository: &Path, output: &Path) -> anyhow::Result<()> {
    let accepted =
        host::theme_repository::accept_theme_repository(repository).with_context(|| {
            format!(
                "theme repository validation failed: {}",
                repository.display()
            )
        })?;
    publish_new(output, accepted.package_bytes())?;
    println!("Theme package created: {}", output.display());
    Ok(())
}

fn publish_new(output: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("create temporary theme package beside {}", output.display()))?;
    temporary
        .write_all(bytes)
        .with_context(|| format!("write temporary theme package beside {}", output.display()))?;
    temporary
        .as_file_mut()
        .sync_all()
        .with_context(|| format!("sync temporary theme package beside {}", output.display()))?;
    temporary
        .persist_noclobber(output)
        .map_err(|error| error.error)
        .with_context(|| {
            format!(
                "atomically publish new theme package without replacing {}",
                output.display()
            )
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const MANIFEST: &str =
        r#"{"schema":1,"name":"Paper","style_contract":1,"assets":{},"defaults":{}}"#;

    fn repository(css: &str) -> tempfile::TempDir {
        let repository = tempfile::tempdir().expect("repository");
        fs::write(repository.path().join("theme.json"), MANIFEST).expect("manifest");
        fs::write(repository.path().join("style.css"), css).expect("css");
        repository
    }

    #[test]
    fn check_and_package_reject_the_shared_css_compiler_failures_before_publication() {
        for css in [
            "@import url(https://example.test/theme.css);",
            ":scope { color: black; }",
            "body { background-image: url(https://example.test/image.png); }",
            "body { background-image: url(../escaped.png); }",
            "body { background-image: url(assets/missing.png); }",
        ] {
            let repository = repository(css);
            let output = repository.path().join("output.zip");
            let stylesheet = repository.path().join("style.css");
            let check = cmd_theme_check(repository.path()).expect_err(css);
            let check_message = format!("{check:#}");
            assert!(
                check_message.contains(&stylesheet.display().to_string()),
                "{check_message}"
            );
            let package = cmd_theme_package(repository.path(), &output).expect_err(css);
            let package_message = format!("{package:#}");
            assert!(
                package_message.contains(&stylesheet.display().to_string()),
                "{package_message}"
            );
            assert!(!output.exists(), "{css}");
        }
    }

    #[test]
    fn package_creates_a_valid_archive_once_without_replacing_a_destination() {
        let repository = repository("body { color: black; }");
        cmd_theme_check(repository.path()).expect("check");
        let output = repository.path().join("theme.zip");
        cmd_theme_package(repository.path(), &output).expect("package");
        let bytes = fs::read(&output).expect("package bytes");
        assert!(!bytes.is_empty());
        let original = b"already here";
        fs::write(&output, original).expect("replace output for test");
        assert!(cmd_theme_package(repository.path(), &output).is_err());
        assert_eq!(fs::read(&output).expect("unchanged output"), original);
    }
}
