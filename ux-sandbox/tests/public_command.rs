//! Exercise only the public helper and Just argv boundary.
#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use std::{fs, path::Path};

    fn executable(path: &Path, text: &str) {
        fs::write(path, text).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn just_forwards_literal_argv_without_interpolation() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("target/debug")).unwrap();
        fs::create_dir(temp.path().join("bin")).unwrap();
        executable(
            &temp.path().join("bin/cargo"),
            "#!/bin/sh\n[ \"$CARGO_TARGET_DIR\" = target ] || exit 97\nprintf '%s\\0' \"$@\" > cargo-argv\n",
        );
        executable(
            &temp.path().join("target/debug/ux-sandbox"),
            "#!/bin/sh\nprintf '%s\\0' \"$@\" > helper-argv\n",
        );
        let args = ["space here", "", "--", "$(touch injected); * ' \\"];
        let output = Command::new("just")
            .args([
                "--justfile",
                concat!(env!("CARGO_MANIFEST_DIR"), "/../Justfile"),
                "--working-directory",
            ])
            .arg(temp.path())
            .arg("sandbox")
            .args(args)
            .env("CARGO_TARGET_DIR", temp.path().join("stale-alternate"))
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    temp.path().join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let expected: Vec<u8> = args.iter().flat_map(|s| s.bytes().chain([0])).collect();
        assert_eq!(fs::read(temp.path().join("helper-argv")).unwrap(), expected);
        assert_eq!(
            fs::read(temp.path().join("cargo-argv")).unwrap(),
            b"build\0--locked\0-p\0ux-sandbox\0"
        );
        assert!(!temp.path().join("injected").exists());
    }

    #[test]
    fn deferred_modes_are_rejected_before_preparation() {
        for args in [
            vec!["writing-flow"],
            vec!["--profile", "demo"],
            vec!["--reset"],
            vec!["--", "backup"],
            vec![""],
        ] {
            let result = Command::new(env!("CARGO_BIN_EXE_ux-sandbox"))
                .args(&args)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(2), "{args:?}: {result:?}");
            assert!(String::from_utf8_lossy(&result.stderr).contains("only disposable empty mode"));
        }
    }
}
