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
        fs::create_dir(temp.path().join("bin")).unwrap();
        executable(
            &temp.path().join("bin/cargo"),
            "#!/bin/sh\ntouch cargo-called\nexit 97\n",
        );
        executable(
            &temp.path().join("bin/nix"),
            "#!/bin/sh\nprintf '%s\\0' \"$@\" > nix-argv\npwd > nix-cwd\n",
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
        let expected: Vec<u8> = [
            "run",
            "--no-update-lock-file",
            "--no-write-lock-file",
            ".#ux-sandbox",
            "--",
        ]
        .into_iter()
        .chain(args)
        .flat_map(|s| s.bytes().chain([0]))
        .collect();
        assert_eq!(fs::read(temp.path().join("nix-argv")).unwrap(), expected);
        assert_eq!(
            fs::read_to_string(temp.path().join("nix-cwd"))
                .unwrap()
                .trim(),
            temp.path().to_str().unwrap()
        );
        assert!(!temp.path().join("cargo-called").exists());
        assert!(!temp.path().join("injected").exists());
    }

    #[test]
    fn unsafe_and_non_utf8_names_are_rejected_before_workspace_access() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("bin")).unwrap();
        executable(
            &root.path().join("bin/cargo"),
            "#!/bin/sh\ntouch built\nexit 97\n",
        );
        let mut invalid: Vec<OsString> = [
            "",
            ".",
            "..",
            "../writing",
            "writing/next",
            "writing\\next",
            "Writing",
            "_writing",
            "writing next",
            "writíng",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        invalid.push(root.path().join("absolute").into_os_string());
        invalid.push(OsString::from_vec(vec![0xff]));
        for name in invalid {
            let output = Command::new(env!("CARGO_BIN_EXE_ux-sandbox"))
                .current_dir(root.path())
                .arg(&name)
                .env(
                    "PATH",
                    format!(
                        "{}:{}",
                        root.path().join("bin").display(),
                        std::env::var("PATH").unwrap()
                    ),
                )
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(2), "{name:?}: {output:?}");
        }
        assert!(!root.path().join("built").exists());
        assert!(!root.path().join(".xtask").exists());
    }

    #[test]
    fn non_utf8_storage_selectors_are_rejected_before_preparation() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("bin")).unwrap();
        executable(
            &root.path().join("bin/cargo"),
            "#!/bin/sh\ntouch built\nexit 97\n",
        );
        for prefix in [b"--storage-path=".as_slice(), b"--db=".as_slice()] {
            let argument = OsString::from_vec([prefix, &[0xff]].concat());
            let output = Command::new(env!("CARGO_BIN_EXE_ux-sandbox"))
                .current_dir(root.path())
                .args(["writing", "--", "backup"])
                .arg(argument)
                .env(
                    "PATH",
                    format!(
                        "{}:{}",
                        root.path().join("bin").display(),
                        std::env::var("PATH").unwrap()
                    ),
                )
                .output()
                .unwrap();
            assert_eq!(output.status.code(), Some(2), "{output:?}");
        }
        assert!(!root.path().join("built").exists());
    }

    #[test]
    fn invalid_modes_are_rejected_before_preparation() {
        for args in [
            vec!["../writing-flow"],
            vec!["--profile", "unknown"],
            vec!["--reset"],
            vec!["--", "backup"],
            vec!["writing", "--"],
            vec![""],
        ] {
            let result = Command::new(env!("CARGO_BIN_EXE_ux-sandbox"))
                .args(&args)
                .output()
                .unwrap();
            assert_eq!(result.status.code(), Some(2), "{args:?}: {result:?}");
            assert!(String::from_utf8_lossy(&result.stderr).contains("error:"));
        }
    }
}
