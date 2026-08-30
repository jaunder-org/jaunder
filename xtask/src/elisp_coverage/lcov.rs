//! Strict LCOV `SF`/`DA` reader for the producer's line observations.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use super::model::CoverageError;

pub(crate) type Lcov = BTreeMap<String, BTreeMap<u32, Vec<u64>>>;

pub(crate) fn parse(path: &Path, repo_root: &Path) -> Result<Lcov, CoverageError> {
    let text = fs::read_to_string(path).map_err(|error| CoverageError::Artifact {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    let mut lcov = Lcov::new();
    let mut current = None;
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if let Some(source) = line.strip_prefix("SF:") {
            if current.is_some() {
                return Err(error(format!("line {line_number}: nested SF record")));
            }
            let source = normalize_source(source, repo_root).map_err(error)?;
            lcov.entry(source.clone()).or_default();
            current = Some(source);
        } else if let Some(data) = line.strip_prefix("DA:") {
            let source = current
                .as_ref()
                .ok_or_else(|| error(format!("line {line_number}: DA without SF")))?;
            let (line, hits) = data
                .split_once(',')
                .ok_or_else(|| error(format!("line {line_number}: malformed DA record")))?;
            let line = line
                .parse()
                .map_err(|_| error(format!("line {line_number}: invalid DA line")))?;
            let hits = hits
                .parse()
                .map_err(|_| error(format!("line {line_number}: invalid DA hit count")))?;
            lcov.entry(source.clone())
                .or_default()
                .entry(line)
                .or_default()
                .push(hits);
        } else if line == "end_of_record" {
            if current.take().is_none() {
                return Err(error(format!(
                    "line {line_number}: end_of_record without SF"
                )));
            }
        } else if line.is_empty()
            || line.starts_with("TN:")
            || line.starts_with("LF:")
            || line.starts_with("LH:")
        {
            continue;
        } else {
            return Err(error(format!(
                "line {line_number}: unsupported LCOV record"
            )));
        }
    }
    if current.is_some() {
        return Err(error("missing end_of_record".to_owned()));
    }
    Ok(lcov)
}

fn normalize_source(value: &str, repo_root: &Path) -> Result<String, String> {
    let source = PathBuf::from(value);
    let relative = if source.is_absolute() {
        source
            .strip_prefix(repo_root)
            .map_err(|_| "SF path is outside the repository")?
    } else {
        source.as_path()
    };
    let relative = relative.to_str().ok_or("SF path is not UTF-8")?;
    if !relative.starts_with("elisp/") || !relative.ends_with(".el") || relative.contains("..") {
        return Err("SF path is not a flat production elisp module".to_owned());
    }
    Ok(relative.to_owned())
}

fn error(message: String) -> CoverageError {
    CoverageError::Lcov { message }
}
