//! Consumer entrypoint and source-local stateless verdict.

use std::{collections::BTreeMap, fs, path::Path};

use super::{
    lcov,
    model::{
        CoverageError, CoverageFailure, CoverageReport, ModuleCensus, PointKind, ProducerOutcome,
        ProducerStatus, STATUS_SCHEMA,
    },
    source,
};

/// Consume one fixed producer artifact directory against an explicit repository root.
///
/// `artifact_dir` is the directory containing `lcov.info`, `summary.txt`, and
/// `status.json` (normally `$out/elisp-coverage`). The explicit paths keep this
/// seam deterministic and independent of xtask command/orchestration state.
pub fn consume(repo_root: &Path, artifact_dir: &Path) -> Result<CoverageReport, CoverageError> {
    let status_path = artifact_dir.join("status.json");
    let summary_path = artifact_dir.join("summary.txt");
    let lcov_path = artifact_dir.join("lcov.info");
    let status_text = read_artifact(&status_path)?;
    read_artifact(&summary_path)?;
    let status: ProducerStatus =
        serde_json::from_str(&status_text).map_err(|error| CoverageError::Status {
            message: format!("invalid status.json: {error}"),
        })?;
    if status.schema != STATUS_SCHEMA {
        return Err(CoverageError::Status {
            message: format!("unknown status schema {:?}", status.schema),
        });
    }
    if status.outcome != ProducerOutcome::Success {
        return Err(CoverageError::Status {
            message: format!("producer outcome is {}", outcome_name(&status.outcome)),
        });
    }

    let lcov = lcov::parse(&lcov_path, repo_root)?;
    let sources = discover_modules(repo_root)?;
    let status_modules = census_modules(status.modules)?;
    if sources.keys().collect::<Vec<_>>() != status_modules.keys().collect::<Vec<_>>() {
        return Err(CoverageError::Census {
            message: format!(
                "producer modules do not match current source: expected {:?}, got {:?}",
                sources.keys().collect::<Vec<_>>(),
                status_modules.keys().collect::<Vec<_>>()
            ),
        });
    }
    if !lcov.keys().all(|path| sources.contains_key(path)) {
        return Err(CoverageError::Lcov {
            message: "LCOV names a module outside the production census".to_owned(),
        });
    }

    let mut report = CoverageReport::default();
    let mut failures = Vec::new();
    for (path, source_path) in sources {
        let module = &status_modules[&path];
        let source_text = source::assert_forms(&source_path, &module.forms)?;
        let points = census_points(&path, module)?;
        reject_unknown_lcov(&path, lcov.get(&path), &points)?;
        let markers = markers(&source_text, &path)?;
        for &line in markers.keys() {
            let point = points.get(&line).ok_or_else(|| CoverageError::Census {
                message: format!("{path}:{line} has a cov:ignore marker but is not a census point"),
            })?;
            if point.kind == PointKind::Ordinary && lcov_hits(&path, lcov.get(&path), line)? > 0 {
                return Err(CoverageError::Census {
                    message: format!("{path}:{line} has a cov:ignore marker on a covered point"),
                });
            }
        }
        for (line, point) in points {
            let covered = match point.kind {
                PointKind::Ordinary => lcov_hits(&path, lcov.get(&path), line)? > 0,
                PointKind::Synthetic => false,
            };
            if covered {
                report.covered_points += 1;
            } else if markers.contains_key(&line) {
                report.ignored_points += 1;
            } else {
                failures.push(CoverageFailure {
                    path: path.clone(),
                    line,
                    message: match point.kind {
                        PointKind::Ordinary => "uncovered executable point".to_owned(),
                        PointKind::Synthetic => "uninstrumented synthetic point".to_owned(),
                    },
                });
            }
        }
    }
    if failures.is_empty() {
        Ok(report)
    } else {
        Err(CoverageError::Verdict { failures })
    }
}

fn read_artifact(path: &Path) -> Result<String, CoverageError> {
    fs::read_to_string(path).map_err(|error| CoverageError::Artifact {
        path: path.to_owned(),
        message: error.to_string(),
    })
}

fn discover_modules(
    repo_root: &Path,
) -> Result<BTreeMap<String, std::path::PathBuf>, CoverageError> {
    let directory = repo_root.join("elisp");
    let entries = fs::read_dir(&directory).map_err(|error| CoverageError::Source {
        path: directory,
        message: error.to_string(),
    })?;
    let mut modules = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|error| CoverageError::Source {
            path: repo_root.join("elisp"),
            message: error.to_string(),
        })?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|extension| extension == "el") {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| CoverageError::Source {
                    path: path.clone(),
                    message: "module name is not UTF-8".to_owned(),
                })?;
            modules.insert(format!("elisp/{name}"), path);
        }
    }
    Ok(modules)
}

fn census_modules(
    modules: Vec<ModuleCensus>,
) -> Result<BTreeMap<String, ModuleCensus>, CoverageError> {
    let mut result = BTreeMap::new();
    for module in modules {
        if !module.path.starts_with("elisp/")
            || module.path["elisp/".len()..].contains('/')
            || !module.path.ends_with(".el")
        {
            return Err(CoverageError::Census {
                message: format!("invalid module path {:?}", module.path),
            });
        }
        if result.insert(module.path.clone(), module).is_some() {
            return Err(CoverageError::Census {
                message: "duplicate module in census".to_owned(),
            });
        }
    }
    Ok(result)
}

fn census_points<'a>(
    path: &str,
    module: &'a ModuleCensus,
) -> Result<BTreeMap<u32, &'a super::model::PointCensus>, CoverageError> {
    let mut points = BTreeMap::new();
    for form in &module.forms {
        let synthetic = form
            .points
            .iter()
            .filter(|point| point.kind == PointKind::Synthetic)
            .count();
        if form.start_line == 0 || form.points.is_empty() {
            return Err(CoverageError::Census {
                message: format!("{path}:{} form has no valid points", form.start_line),
            });
        }
        if synthetic > 0
            && (synthetic != 1 || form.points.len() != 1 || form.points[0].line != form.start_line)
        {
            return Err(CoverageError::Census {
                message: format!(
                    "{path}:{} synthetic census point must be the sole opening-line point",
                    form.start_line
                ),
            });
        }
        for point in &form.points {
            if point.line == 0 || points.insert(point.line, point).is_some() {
                return Err(CoverageError::Census {
                    message: format!("{path}: duplicate or invalid census point {}", point.line),
                });
            }
        }
    }
    Ok(points)
}

fn reject_unknown_lcov(
    path: &str,
    records: Option<&BTreeMap<u32, Vec<u64>>>,
    points: &BTreeMap<u32, &super::model::PointCensus>,
) -> Result<(), CoverageError> {
    for line in records.into_iter().flat_map(|records| records.keys()) {
        let point = points.get(line).ok_or_else(|| CoverageError::Lcov {
            message: format!("{path}:{line} is not a census point"),
        })?;
        if point.kind == PointKind::Synthetic {
            return Err(CoverageError::Lcov {
                message: format!("{path}:{line} is synthetic but has an LCOV record"),
            });
        }
    }
    Ok(())
}

fn lcov_hits(
    path: &str,
    records: Option<&BTreeMap<u32, Vec<u64>>>,
    line: u32,
) -> Result<u64, CoverageError> {
    let records = records
        .and_then(|records| records.get(&line))
        .ok_or_else(|| CoverageError::Lcov {
            message: format!("{path}:{line} is missing an LCOV record"),
        })?;
    if records.len() != 1 {
        return Err(CoverageError::Lcov {
            message: format!("{path}:{line} has {} LCOV records", records.len()),
        });
    }
    Ok(records[0])
}

/// Return valid source-local marker lines. Any spelling of `cov:ignore` outside
/// the exact trailing comment grammar is rejected rather than ignored.
fn markers(source: &str, path: &str) -> Result<BTreeMap<u32, ()>, CoverageError> {
    let mut result = BTreeMap::new();
    for (index, line) in source.lines().enumerate() {
        let line_number = (index + 1) as u32;
        let Some(start) = comment_start(line) else {
            continue;
        };
        let comment = line[start + 2..].trim_start();
        if !comment.contains("cov:ignore") {
            continue;
        }
        let valid = comment
            .strip_prefix("cov:ignore:")
            .is_some_and(|reason| !reason.trim().is_empty());
        if !valid {
            return Err(CoverageError::Census {
                message: format!("{path}:{line_number} has a malformed cov:ignore marker"),
            });
        }
        result.insert(line_number, ());
    }
    Ok(result)
}

fn comment_start(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut index = 0;
    let mut string = false;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if string => index += 2,
            b'"' => {
                string = !string;
                index += 1;
            }
            b';' if !string && bytes.get(index + 1) == Some(&b';') => return Some(index),
            _ => index += 1,
        }
    }
    None
}

fn outcome_name(outcome: &ProducerOutcome) -> &'static str {
    match outcome {
        ProducerOutcome::Success => "success",
        ProducerOutcome::ErtFailure => "ert-failure",
        ProducerOutcome::InstrumentationFailure => "instrumentation-failure",
        ProducerOutcome::InvalidReport => "invalid-report",
    }
}
