//! Shared checkout admission for retained xtask consumers.
use anyhow::Result;
pub(crate) use repository_policy::checkout::admit_untracked_inputs;

/// Preserve the selected xtask source predicate before hook preparation.
pub fn prepare_checkout() -> Result<()> {
    repository_policy::checkout::prepare_checkout(
        "xtask/Cargo.toml",
        (!env!("JAUNDER_XTASK_NIX_OUTPUT").is_empty()).then_some(".#xtask.sourceAdmits"),
    )
}

#[cfg(test)]
use repository_policy::checkout::{ADMIT_INPUTS, Candidate};
#[cfg(test)]
use std::{path::Path, process::Command};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_flake_admission_refuses_only_omitted_relevant_inputs() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let git = |args: &[&str]| {
            assert!(crate::git::at(root).args(args).status().unwrap().success());
        };
        git(&["init", "--quiet"]);
        // A dependency-free flake proves the real Git/Nix admission boundary
        // without realizing a toolchain or substituting a test verdict.
        std::fs::write(
            root.join("flake.nix"),
            r#"{
          outputs = { self }: { packages = builtins.listToAttrs (map (name: {
            inherit name;
            value.csrBundle = {
              sourceAdmits = path: kind: path == "new.rs";
              sourceText = builtins.readFile ./new.rs;
            };
          }) [ "x86_64-linux" "aarch64-linux" "aarch64-darwin" ]); };
        }"#,
        )
        .unwrap();
        git(&["add", "flake.nix"]);
        std::fs::write(root.join("notes.md"), "harmless").unwrap();
        admit_untracked_inputs(root, ".#csrBundle.sourceAdmits").unwrap();
        std::fs::write(root.join("new.rs"), "first source").unwrap();
        let error = admit_untracked_inputs(root, ".#csrBundle.sourceAdmits").unwrap_err();
        assert!(error.to_string().contains("new.rs"));
        assert!(error.to_string().contains("git add -N"));
        let index = crate::git::at(root)
            .args(["ls-files", "new.rs"])
            .output()
            .unwrap();
        assert!(index.stdout.is_empty(), "admission must not stage input");
        let selected_text = || {
            let output = Command::new("nix")
                .args(["eval", "--raw", ".#csrBundle.sourceText"])
                .current_dir(root)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap()
        };
        git(&["add", "-N", "new.rs"]);
        admit_untracked_inputs(root, ".#csrBundle.sourceAdmits").unwrap();
        assert_eq!(selected_text(), "first source");
        git(&["add", "new.rs"]);
        std::fs::write(root.join("new.rs"), "dirty source").unwrap();
        admit_untracked_inputs(root, ".#csrBundle.sourceAdmits").unwrap();
        assert_eq!(selected_text(), "dirty source");
        std::fs::write(root.join("flake.nix"), "invalid Nix").unwrap();
        assert!(admit_untracked_inputs(root, ".#csrBundle.sourceAdmits").is_err());
    }

    #[test]
    fn executable_selections_admit_host_seed_public_and_csr_inputs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let relevant = [
            "server/src/new.rs",
            "test-support/src/new.rs",
            "public/new.svg",
            "csr/src/new.rs",
            ".cargo/config.toml",
        ];
        let candidates = relevant
            .iter()
            .chain(["notes.md"].iter())
            .map(|path| Candidate {
                path: (*path).into(),
                kind: "regular",
            })
            .collect::<Vec<_>>();
        for attr in [
            ".#sandboxArtifacts.sourceAdmits",
            ".#sandboxArtifactsReleaseCsr.sourceAdmits",
        ] {
            let output = Command::new("nix")
                .args([
                    "eval",
                    "--impure",
                    "--json",
                    "--no-update-lock-file",
                    "--no-write-lock-file",
                    attr,
                    "--apply",
                    ADMIT_INPUTS,
                ])
                .current_dir(root)
                .env(
                    "JAUNDER_XTASK_UNTRACKED_INPUTS",
                    serde_json::to_string(&candidates).unwrap(),
                )
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let admitted: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(admitted, relevant);
        }
    }

    #[test]
    fn csr_admission_uses_the_real_producer_source_declarations() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let relevant = [
            "csr/src/new.rs",
            "web/src/new.rs",
            "client/src/new.rs",
            "common/src/new.rs",
            "macros/src/new.rs",
            "storage/src/new.rs",
            "tools/devtool/src/new.rs",
            "csr/index.html",
            "csr/entry.js",
            "vite.config.js",
            "package.json",
            "package-lock.json",
            "scripts/build-csr-assets.sh",
            "Justfile",
            "server/assets/jaunder.css",
            "host/system_theme_sources/new.css",
            "storage/migrations/new.sql",
            "tools/Cargo.lock",
            "Cargo.lock",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            "flake.nix",
            "flake.lock",
            "nix/new.nix",
        ];
        let candidates = relevant
            .iter()
            .chain(["notes.md", "server/src/unrelated.rs"].iter())
            .map(|path| Candidate {
                path: (*path).into(),
                kind: "regular",
            })
            .collect::<Vec<_>>();
        let output = Command::new("nix")
            .args([
                "eval",
                "--impure",
                "--json",
                ".#csrBundle.sourceAdmits",
                "--apply",
                ADMIT_INPUTS,
            ])
            .current_dir(root)
            .env(
                "JAUNDER_XTASK_UNTRACKED_INPUTS",
                serde_json::to_string(&candidates).unwrap(),
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let admitted: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(admitted, relevant);
    }
}
