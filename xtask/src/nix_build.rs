//! Shared host-side Nix build helpers. Observation reads dry-run and local
//! path-info JSON only; completed-build path selection reads stdout only, so the
//! #224 stderr leak (`…-user-environment` on stderr) cannot recur.

use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::{self, Value};

use crate::result::{NixRealization, NixReport};

/// The stdout and exit status returned by an injected Nix command.
pub(crate) struct NixCommandOutput {
    success: bool,
    stdout: String,
}

impl NixCommandOutput {
    #[cfg(test)]
    fn success(stdout: String) -> Self {
        Self {
            success: true,
            stdout,
        }
    }

    #[cfg(test)]
    fn failure(stdout: &str) -> Self {
        Self {
            success: false,
            stdout: stdout.into(),
        }
    }
}

/// The derivation and output paths selected by one evaluated installable.
#[derive(Debug, PartialEq, Eq)]
struct NixSelection {
    derivation: String,
    outputs: Vec<String>,
}

/// Best-effort evidence from observing an installable without realizing it.
pub(crate) struct NixBuildObservation {
    pub(crate) derivation: Option<String>,
    selection: Option<NixSelection>,
    selected_outputs: Option<Vec<bool>>,
}

impl NixBuildObservation {
    /// Complete a successful gate-owned build observation with its post-build
    /// counterpart and turn both snapshots into one result-envelope report.
    pub(crate) fn finish(self, installable: &str) -> NixReport {
        self.finish_with(installable, observe)
    }

    fn finish_with(
        self,
        installable: &str,
        observe: impl FnOnce(&str) -> NixBuildObservation,
    ) -> NixReport {
        let after = observe(installable);
        report(installable, &self, &after)
    }
}

/// Run the non-realizing Nix observation commands for an installable.
pub(crate) fn observe(installable: &str) -> NixBuildObservation {
    observe_with(installable, |arguments| {
        Command::new("nix")
            .args(arguments)
            .output()
            .map(|output| NixCommandOutput {
                success: output.status.success(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            })
    })
}

/// Run an observation through an injected Nix command boundary.
///
/// Every failure is deliberately represented as incomplete evidence: observation
/// must not turn a successful later build into a gate failure.
fn observe_with(
    installable: &str,
    mut run: impl FnMut(&[String]) -> std::io::Result<NixCommandOutput>,
) -> NixBuildObservation {
    let dry_run = vec![
        "build".into(),
        "--dry-run".into(),
        "--json".into(),
        "--no-link".into(),
        "--accept-flake-config".into(),
        installable.into(),
    ];
    let Ok(dry_run) = run(&dry_run) else {
        return indeterminate_observation();
    };
    if !dry_run.success {
        return indeterminate_observation();
    }
    let Some(selection) = parse_dry_run(&dry_run.stdout) else {
        return indeterminate_observation();
    };
    let derivation = Some(selection.derivation.clone());
    let path_info = [
        vec![
            "path-info".into(),
            "--offline".into(),
            "--json".into(),
            "--json-format".into(),
            "2".into(),
        ],
        selection.outputs.clone(),
    ]
    .concat();
    let selected_outputs = run(&path_info)
        .ok()
        .filter(|output| output.success)
        .and_then(|output| parse_path_info(&output.stdout, &selection.outputs));
    NixBuildObservation {
        derivation,
        selection: Some(selection),
        selected_outputs,
    }
}

fn indeterminate_observation() -> NixBuildObservation {
    NixBuildObservation {
        derivation: None,
        selection: None,
        selected_outputs: None,
    }
}

/// Parse the selected derivation and its output paths from dry-run JSON stdout.
fn parse_dry_run(document: &str) -> Option<NixSelection> {
    let Value::Array(entries) = serde_json::from_str(document).ok()? else {
        return None;
    };
    let [entry] = entries.as_slice() else {
        return None;
    };
    let derivation = entry.get("drvPath")?.as_str()?.to_owned();
    let outputs = entry
        .get("outputs")?
        .as_object()?
        .values()
        .map(Value::as_str)
        .collect::<Option<Vec<_>>>()?;
    if derivation.starts_with("/nix/store/") && derivation.ends_with(".drv") && !outputs.is_empty()
    {
        let mut outputs = outputs.into_iter().map(str::to_owned).collect::<Vec<_>>();
        outputs.sort_unstable();
        outputs
            .iter()
            .all(|path| path.starts_with("/nix/store/"))
            .then_some(NixSelection {
                derivation,
                outputs,
            })
    } else {
        None
    }
}

/// Parse v2 `nix path-info` JSON, where object `info` entries mean valid and null
/// entries mean invalid. Missing, malformed, or unrelated paths make the probe
/// indeterminate.
fn parse_path_info(document: &str, selected: &[String]) -> Option<Vec<bool>> {
    let Value::Object(document) = serde_json::from_str(document).ok()? else {
        return None;
    };
    if document.get("version")?.as_u64()? != 2 {
        return None;
    }
    let store_dir = document.get("storeDir")?.as_str()?;
    let info = document.get("info")?.as_object()?;
    if info.len() != selected.len() {
        return None;
    }
    selected
        .iter()
        .map(|selected_path| {
            let name = selected_path.strip_prefix(store_dir)?.strip_prefix('/')?;
            if name.contains('/') {
                return None;
            }
            match info.get(name)? {
                Value::Object(_) => Some(true),
                Value::Null => Some(false),
                _ => None,
            }
        })
        .collect()
}

/// Turn paired observations into the structured evidence carried by a successful
/// gate-owned build step.
fn report(
    installable: &str,
    before: &NixBuildObservation,
    after: &NixBuildObservation,
) -> NixReport {
    NixReport {
        installable: installable.to_owned(),
        derivation: before
            .derivation
            .clone()
            .or_else(|| after.derivation.clone()),
        realization: classify_realization(before, after),
    }
}

/// Classify local-store evidence from immediately before and after a build.
pub(crate) fn classify_realization(
    before: &NixBuildObservation,
    after: &NixBuildObservation,
) -> NixRealization {
    let (Some(before_selection), Some(before_validity)) =
        (before.selection.as_ref(), before.selected_outputs.as_ref())
    else {
        return NixRealization::Unknown;
    };
    if before_validity.iter().all(|valid| *valid) {
        return NixRealization::Reused;
    }
    let (Some(after_selection), Some(after_validity)) =
        (after.selection.as_ref(), after.selected_outputs.as_ref())
    else {
        return NixRealization::Unknown;
    };
    if before_selection != after_selection {
        return NixRealization::Unknown;
    }
    if after_validity.iter().all(|valid| *valid) {
        NixRealization::Realized
    } else {
        NixRealization::Unknown
    }
}

/// Pull the built store path out of `nix build --print-out-paths` output. Nix may
/// print warnings interleaved, so we take the *last* `/nix/store/` line (the
/// realized output).
pub fn parse_store_path(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rfind(|l| l.starts_with("/nix/store/"))
        .map(str::to_string)
}

/// Select the store path from a completed `nix build`'s streams. Parses **stdout
/// only** — `stderr` is used solely for the error message, never parsed — so a
/// `…-user-environment` (or any other) line nix writes to stderr can never be
/// selected as the result (#224).
pub fn store_path_from_streams(stdout: &str, stderr: &str) -> Result<String> {
    parse_store_path(stdout).with_context(|| {
        format!("could not parse a /nix/store path from nix stdout; stderr:\n{stderr}")
    })
}

/// `nix build .#<attr> --no-link --print-out-paths`; captures both streams, bails
/// with the captured stderr on non-zero status, else selects the store path from
/// stdout via [`store_path_from_streams`].
pub fn build_out_path(attr: &str) -> Result<String> {
    let flake_ref = format!(".#{attr}");
    let out = Command::new("nix")
        .args(["build", &flake_ref, "--no-link", "--print-out-paths"])
        .output()
        .with_context(|| format!("spawning `nix build {flake_ref}`"))?;
    if !out.status.success() {
        bail!(
            "`nix build {flake_ref}` failed ({}):\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    store_path_from_streams(
        &String::from_utf8_lossy(&out.stdout),
        &String::from_utf8_lossy(&out.stderr),
    )
    .with_context(|| format!("`nix build {flake_ref}` produced no store path"))
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;
    use crate::result::NixRealization;

    const INSTALLABLE: &str = ".#site";
    const DERIVATION: &str = "/nix/store/aaa-site.drv";
    const OUT: &str = "/nix/store/bbb-site";
    const DEV: &str = "/nix/store/ccc-site-dev";

    fn dry_run(outputs: &str) -> String {
        format!(r#"[{{"drvPath":"{DERIVATION}","outputs":{outputs}}}]"#)
    }

    fn path_info(entries: &str) -> String {
        format!(r#"{{"version":2,"storeDir":"/nix/store","info":{{{entries}}}}}"#)
    }

    fn observation_from(
        dry_run: Result<NixCommandOutput, io::Error>,
        path_info: Result<NixCommandOutput, io::Error>,
    ) -> NixBuildObservation {
        let mut commands = [dry_run, path_info].into_iter();
        observe_with(INSTALLABLE, |_| commands.next().unwrap())
    }

    #[test]
    fn parse_store_path_takes_last_store_line() {
        let out = "warning: ignoring\n/nix/store/aaa-x\n  /nix/store/bbb-jaunder-site  \n";
        assert_eq!(
            parse_store_path(out).as_deref(),
            Some("/nix/store/bbb-jaunder-site")
        );
    }

    #[test]
    fn parse_store_path_none_when_no_store_line() {
        assert_eq!(parse_store_path("no paths here\n"), None);
    }

    #[test]
    fn store_path_from_streams_ignores_stderr() {
        let result =
            store_path_from_streams("no result here\n", "/nix/store/zzz-user-environment\n");
        assert!(result.is_err(), "stderr store path must not be selected");
    }

    #[test]
    fn store_path_from_streams_takes_stdout() {
        assert_eq!(
            store_path_from_streams("/nix/store/aaa-e2e\n", "junk stderr").unwrap(),
            "/nix/store/aaa-e2e"
        );
    }

    #[test]
    fn parse_dry_run_selects_every_output_and_derivation() {
        assert_eq!(
            parse_dry_run(&dry_run(&format!(r#"{{"out":"{OUT}","dev":"{DEV}"}}"#))),
            Some(NixSelection {
                derivation: DERIVATION.into(),
                outputs: vec![OUT.into(), DEV.into()],
            })
        );
    }

    #[test]
    fn parse_dry_run_rejects_missing_or_malformed_documents() {
        assert_eq!(parse_dry_run("[]"), None);
        assert_eq!(
            parse_dry_run(r#"[{"outputs":{"out":"/nix/store/bbb"}}]"#),
            None
        );
        assert_eq!(parse_dry_run(r#"[{"drvPath":"/nix/store/a.drv"}]"#), None);
        assert_eq!(parse_dry_run("not json"), None);
        assert_eq!(
            parse_dry_run(
                r#"[{"drvPath":"/nix/store/not-a-derivation","outputs":{"out":"/nix/store/bbb"}}]"#
            ),
            None
        );
    }

    #[test]
    fn parse_path_info_v2_distinguishes_valid_and_invalid_outputs() {
        let selected = vec![OUT.into(), DEV.into()];
        assert_eq!(
            parse_path_info(
                &path_info(r#""bbb-site":{"narHash":"sha256-a"},"ccc-site-dev":null"#),
                &selected,
            ),
            Some(vec![true, false])
        );
    }

    #[test]
    fn parse_path_info_rejects_missing_malformed_or_inconsistent_documents() {
        let selected = vec![OUT.into(), DEV.into()];
        assert_eq!(
            parse_path_info(&path_info(r#""bbb-site":{}"#), &selected),
            None
        );
        assert_eq!(
            parse_path_info(
                &path_info(r#""bbb-site":"bad","ccc-site-dev":{}"#),
                &selected
            ),
            None
        );
        assert_eq!(
            parse_path_info(r#"{"version":2,"storeDir":"/nix/store"}"#, &selected),
            None
        );
        assert_eq!(parse_path_info("not json", &selected), None);
        assert_eq!(
            parse_path_info(&path_info(r#""bbb-site":{},"other":{}"#), &selected),
            None
        );
    }

    #[test]
    fn unavailable_or_failed_commands_are_indeterminate() {
        let unavailable = observation_from(
            Err(io::Error::other("nix unavailable")),
            unreachable_command(),
        );
        let failed_evaluation = observation_from(
            Ok(NixCommandOutput::failure("evaluation failed")),
            unreachable_command(),
        );
        let failed_probe = observation_from(
            Ok(NixCommandOutput::success(dry_run(&format!(
                r#"{{"out":"{OUT}"}}"#
            )))),
            Ok(NixCommandOutput::failure("path-info failed")),
        );
        assert!(unavailable.selected_outputs.is_none());
        assert!(failed_evaluation.selected_outputs.is_none());
        assert!(failed_probe.selected_outputs.is_none());
    }

    #[test]
    fn incomplete_probe_is_indeterminate() {
        let observation = observation_from(
            Ok(NixCommandOutput::success(dry_run(&format!(
                r#"{{"out":"{OUT}","dev":"{DEV}"}}"#
            )))),
            Ok(NixCommandOutput::success(path_info(r#""bbb-site":{}"#))),
        );
        assert!(observation.selected_outputs.is_none());
    }

    #[test]
    fn classify_reused_when_all_outputs_were_already_valid() {
        let observation = observation_from(
            Ok(NixCommandOutput::success(dry_run(&format!(
                r#"{{"out":"{OUT}","dev":"{DEV}"}}"#
            )))),
            Ok(NixCommandOutput::success(path_info(
                r#""bbb-site":{},"ccc-site-dev":{}"#,
            ))),
        );
        assert_eq!(
            classify_realization(&observation, &observation),
            NixRealization::Reused
        );
    }

    #[test]
    fn classify_realized_when_any_missing_output_becomes_valid() {
        let before = observation_from(
            Ok(NixCommandOutput::success(dry_run(&format!(
                r#"{{"out":"{OUT}","dev":"{DEV}"}}"#
            )))),
            Ok(NixCommandOutput::success(path_info(
                r#""bbb-site":{},"ccc-site-dev":null"#,
            ))),
        );
        let after = observation_from(
            Ok(NixCommandOutput::success(dry_run(&format!(
                r#"{{"out":"{OUT}","dev":"{DEV}"}}"#
            )))),
            Ok(NixCommandOutput::success(path_info(
                r#""bbb-site":{},"ccc-site-dev":{}"#,
            ))),
        );
        assert_eq!(
            classify_realization(&before, &after),
            NixRealization::Realized
        );
    }

    #[test]
    fn classify_reused_without_a_post_build_observation() {
        let before = observation_from(
            Ok(NixCommandOutput::success(dry_run(&format!(
                r#"{{"out":"{OUT}"}}"#
            )))),
            Ok(NixCommandOutput::success(path_info(r#""bbb-site":{}"#))),
        );
        let after = indeterminate_observation();

        assert_eq!(
            classify_realization(&before, &after),
            NixRealization::Reused
        );
    }

    #[test]
    fn finish_prefers_pre_build_derivation_and_falls_back_to_post_build_identity() {
        let before = observation_from(
            Ok(NixCommandOutput::success(dry_run(&format!(
                r#"{{"out":"{OUT}"}}"#
            )))),
            Ok(NixCommandOutput::success(path_info(r#""bbb-site":{}"#))),
        );
        let reused = before.finish_with(INSTALLABLE, |_| indeterminate_observation());
        assert_eq!(reused.derivation.as_deref(), Some(DERIVATION));
        assert_eq!(reused.realization, NixRealization::Reused);

        let unavailable = indeterminate_observation();
        let fallback = unavailable.finish_with(INSTALLABLE, |_| {
            observation_from(
                Ok(NixCommandOutput::success(dry_run(&format!(
                    r#"{{"out":"{OUT}"}}"#
                )))),
                Ok(NixCommandOutput::success(path_info(r#""bbb-site":{}"#))),
            )
        });
        assert_eq!(fallback.derivation.as_deref(), Some(DERIVATION));
        assert_eq!(fallback.realization, NixRealization::Unknown);
    }

    #[test]
    fn classify_unknown_for_incomplete_or_changed_observations() {
        let incomplete = observation_from(
            Ok(NixCommandOutput::success(dry_run(&format!(
                r#"{{"out":"{OUT}"}}"#
            )))),
            Ok(NixCommandOutput::success("[]".into())),
        );
        let changed = observation_from(
            Ok(NixCommandOutput::success(dry_run(&format!(
                r#"{{"out":"{DEV}"}}"#
            )))),
            Ok(NixCommandOutput::success(path_info(r#""ccc-site-dev":{}"#))),
        );
        assert_eq!(
            classify_realization(&incomplete, &changed),
            NixRealization::Unknown
        );
    }

    fn unreachable_command() -> Result<NixCommandOutput, io::Error> {
        Err(io::Error::other("command should not be called"))
    }
}
