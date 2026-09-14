use std::{io::Write, path::Path};

use anyhow::Context;

fn output_parent(output: &Path) -> &Path {
    output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn write_temporary(
    output: &Path,
    bytes: &[u8],
    artifact: &str,
) -> anyhow::Result<tempfile::NamedTempFile> {
    let parent = output_parent(output);
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("create temporary {artifact} beside {}", output.display()))?;
    temporary
        .write_all(bytes)
        .with_context(|| format!("write temporary {artifact} beside {}", output.display()))?;
    temporary
        .as_file_mut()
        .sync_all()
        .with_context(|| format!("sync temporary {artifact} beside {}", output.display()))?;
    Ok(temporary)
}

/// Atomically publishes bytes to a new destination without replacing an existing artifact.
pub(super) fn publish_new(output: &Path, bytes: &[u8], artifact: &str) -> anyhow::Result<()> {
    write_temporary(output, bytes, artifact)?
        .persist_noclobber(output)
        .map_err(|error| error.error)
        .with_context(|| {
            format!(
                "atomically publish new {artifact} without replacing {}",
                output.display()
            )
        })?;
    Ok(())
}

/// Atomically publishes bytes, replacing an existing artifact if present.
pub(super) fn publish_replace(output: &Path, bytes: &[u8], artifact: &str) -> anyhow::Result<()> {
    write_temporary(output, bytes, artifact)?
        .persist(output)
        .map_err(|error| error.error)
        .with_context(|| format!("atomically publish {artifact} to {}", output.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{output_parent, publish_replace};

    #[test]
    fn bare_output_filename_resolves_to_current_directory() {
        assert_eq!(
            output_parent(std::path::Path::new("preview.png")),
            std::path::Path::new(".")
        );
    }

    #[test]
    fn replace_publication_atomically_replaces_an_existing_artifact() {
        let directory = tempfile::tempdir().expect("artifact directory");
        let output = directory.path().join("preview.png");
        std::fs::write(&output, b"old").expect("existing artifact");

        publish_replace(&output, b"new", "thumbnail").expect("replace artifact");

        assert_eq!(std::fs::read(output).expect("published artifact"), b"new");
    }
}
