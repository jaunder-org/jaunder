//! Public shared Just leaves preserve argv, environment and ordinary streams.
#[cfg(unix)]
mod unix {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

    fn fixture(programs: &[&str], script: &str) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("bin")).unwrap();
        for program in programs {
            let path = root.path().join("bin").join(program);
            fs::write(&path, script).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        root
    }

    fn recipe(root: &Path) -> Command {
        let mut paths = vec![root.join("bin")];
        paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
        let mut command = Command::new("just");
        command
            .arg("--justfile")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../recipes/static.just"))
            .arg("--working-directory")
            .arg(root)
            .env("PATH", std::env::join_paths(paths).unwrap());
        command
    }

    #[test]
    fn tsc_leaf_preserves_exact_arguments_streams_and_status_in_both_modes() {
        let root = fixture(
            &["tsc"],
            "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$STATIC_ARGV\"\nprintf 'compiler stdout\\n'\nprintf 'compiler stderr\\n' >&2\nexit \"$STATIC_EXIT\"\n",
        );
        for mode in ["verify", "fix"] {
            for exit in ["0", "23"] {
                fs::write(root.path().join("argv"), "").unwrap();
                let output = recipe(root.path())
                    .args(["tsc", mode])
                    .env("STATIC_ARGV", root.path().join("argv"))
                    .env("STATIC_EXIT", exit)
                    .output()
                    .unwrap();
                assert_eq!(output.status.success(), exit == "0");
                assert_eq!(
                    fs::read_to_string(root.path().join("argv")).unwrap(),
                    "--noEmit\n-p\nend2end/tsconfig.json\n"
                );
                assert!(String::from_utf8_lossy(&output.stdout).contains("compiler stdout"));
                assert!(String::from_utf8_lossy(&output.stderr).contains("compiler stderr"));
            }
        }
    }

    #[test]
    fn cargo_leaves_keep_workspace_arguments_targets_and_sandbox_policy() {
        let root = fixture(
            &["cargo"],
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$STATIC_ARGV\"\nprintf '%s\\n' \"$CARGO_HOME\" \"$CARGO_NET_OFFLINE\" > \"$STATIC_ENV\"\nexit \"${STATIC_EXIT:-0}\"\n",
        );
        for (leaf, expected) in [
            (
                "tools-fmt",
                "--offline\nfmt\n--manifest-path\ntools/Cargo.toml\n--all\n--check\n",
            ),
            (
                "product-core-clippy",
                "--offline\nclippy\n--workspace\n--exclude\nweb\n--exclude\nclient\n--exclude\ncsr\n--all-targets\n--\n-D\nwarnings\n",
            ),
            (
                "web-server-clippy",
                "--offline\nclippy\n-p\nweb\n--features\nserver\n--all-targets\n--\n-D\nwarnings\n",
            ),
            (
                "web-no-server-clippy",
                "--offline\nclippy\n-p\nweb\n--no-default-features\n--all-targets\n--\n-D\nwarnings\n",
            ),
            (
                "client-host-clippy",
                "--offline\nclippy\n-p\nclient\n--features\ncsr\n--all-targets\n--\n-D\nwarnings\n",
            ),
            (
                "tools-clippy",
                "--offline\nclippy\n--manifest-path\ntools/Cargo.toml\n--workspace\n--all-targets\n--\n-D\nwarnings\n",
            ),
            (
                "cargo-deny",
                "--offline\ndeny\ncheck\nbans\nlicenses\nsources\n",
            ),
            (
                "wasm-clippy",
                "--offline\nclippy\n-p\nweb\n-p\nclient\n-p\ncsr\n--features\ncsr\n--target\nwasm32-unknown-unknown\n--\n-D\nwarnings\n",
            ),
        ] {
            let run = |home: &str, exit: &str| {
                recipe(root.path())
                    .args([leaf, "verify", "sandbox"])
                    .env("STATIC_ARGV", root.path().join("argv"))
                    .env("STATIC_EXIT", exit)
                    .env("STATIC_ENV", root.path().join("environment"))
                    .env(
                        "JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME",
                        if leaf.starts_with("tools-") {
                            "unused-product"
                        } else {
                            home
                        },
                    )
                    .env(
                        "JAUNDER_DEVTOOL_TOOLS_CARGO_HOME",
                        if leaf.starts_with("tools-") {
                            home
                        } else {
                            "unused-tools"
                        },
                    )
                    .output()
                    .unwrap()
            };
            assert!(!run("", "0").status.success());
            assert!(!root.path().join("argv").exists());
            assert!(run("home", "0").status.success());
            assert_eq!(
                fs::read_to_string(root.path().join("argv")).unwrap(),
                expected
            );
            assert_eq!(
                fs::read_to_string(root.path().join("environment")).unwrap(),
                "home\ntrue\n"
            );
            assert!(!run("home", "7").status.success());
            fs::remove_file(root.path().join("argv")).unwrap();
        }
    }

    #[test]
    fn groups_admit_all_homes_before_work_and_preserve_independent_order() {
        let root = fixture(
            &[
                "cargo",
                "scoped-leptosfmt",
                "emacs",
                "ast-grep",
                "tsc",
                "devtool",
            ],
            "#!/bin/sh\nprintf '%s %s\\n' \"${0##*/}\" \"$*\" >> \"$STATIC_LOG\"\n",
        );
        let code = "emacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-fix\nast-grep test --config sgconfig.yml\nast-grep scan --config sgconfig.yml --filter ^no-full-reload$ web/src client/src\nemacs --batch -Q -l elisp/scripts/byte-compile.el\n";
        let clippy = "cargo --offline deny check bans licenses sources\ncargo --offline clippy --workspace --exclude web --exclude client --exclude csr --all-targets -- -D warnings\ncargo --offline clippy -p web --features server --all-targets -- -D warnings\ncargo --offline clippy -p web --no-default-features --all-targets -- -D warnings\ncargo --offline clippy -p client --features csr --all-targets -- -D warnings\ncargo --offline clippy -p web -p client -p csr --features csr --target wasm32-unknown-unknown -- -D warnings\ncargo --offline clippy --manifest-path tools/Cargo.toml --workspace --all-targets -- -D warnings\n";
        for (group, expected) in [
            ("rust-format", "cargo --offline fmt\nscoped-leptosfmt fix\ncargo --offline fmt --manifest-path tools/Cargo.toml --all\n".to_owned()),
            ("code", format!("{code}{clippy}")),
            ("all", format!("cargo --offline fmt\nscoped-leptosfmt fix\nemacs --batch -Q -l elisp/scripts/format.el -f jaunder-fmt-fix\ncargo --offline fmt --manifest-path tools/Cargo.toml --all\nast-grep test --config sgconfig.yml\nast-grep scan --config sgconfig.yml --filter ^no-full-reload$ web/src client/src\nemacs --batch -Q -l elisp/scripts/byte-compile.el\ntsc --noEmit -p end2end/tsconfig.json\n{clippy}devtool check ert\n")),
        ] {
        for (product_home, tools_home) in [("", "tools-home"), ("product-home", ""), ("product-home", "tools-home")] {
            let output = recipe(root.path())
                .args([group, "fix", "sandbox"])
                .env("STATIC_LOG", root.path().join("log"))
                .env("JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME", product_home)
                .env("JAUNDER_DEVTOOL_TOOLS_CARGO_HOME", tools_home)
                .output()
                .unwrap();
            if product_home.is_empty() || tools_home.is_empty() {
                assert!(!output.status.success());
                assert!(!root.path().join("log").exists());
            } else {
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert_eq!(
                    fs::read_to_string(root.path().join("log")).unwrap(),
                    expected
                );
                fs::remove_file(root.path().join("log")).unwrap();
            }
        }
        }
    }

    #[test]
    fn fmt_shares_host_verify_and_sandbox_fix_definition() {
        let root = fixture(
            &["cargo"],
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$STATIC_ARGV\"\nprintf '%s\\n' \"${CARGO_HOME-unset}\" \"${CARGO_NET_OFFLINE-unset}\" > \"$STATIC_ENV\"\nprintf 'workload stdout\\n'\nprintf 'workload stderr\\n' >&2\n",
        );
        for (mode, lane, expected, home, offline) in [
            ("verify", "host", "fmt\n--check\n", "unset", "host-policy"),
            ("fix", "sandbox", "--offline\nfmt\n", "sandbox-home", "true"),
        ] {
            let output = recipe(root.path())
                .args(["fmt", mode, lane])
                .env("STATIC_ARGV", root.path().join("argv"))
                .env("STATIC_ENV", root.path().join("environment"))
                .env_remove("CARGO_HOME")
                .env("CARGO_NET_OFFLINE", "host-policy")
                .env("JAUNDER_DEVTOOL_PRODUCT_CARGO_HOME", "sandbox-home")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                fs::read_to_string(root.path().join("argv")).unwrap(),
                expected
            );
            assert_eq!(
                fs::read_to_string(root.path().join("environment")).unwrap(),
                format!("{home}\n{offline}\n")
            );
            assert!(
                String::from_utf8(output.stdout)
                    .unwrap()
                    .contains("workload stdout")
            );
            assert!(
                String::from_utf8(output.stderr)
                    .unwrap()
                    .contains("workload stderr")
            );
        }
    }
}
