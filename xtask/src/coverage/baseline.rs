use std::collections::BTreeMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::coverage::FileCoverage;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gap {
    pub line: u32,
    pub text: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Baseline {
    files: BTreeMap<String, Vec<Gap>>,
}

impl Baseline {
    pub fn gaps(&self, path: &str) -> &[Gap] {
        self.files.get(path).map(|v| v.as_slice()).unwrap_or(&[])
    }
    pub fn set_gaps(&mut self, path: &str, mut gaps: Vec<Gap>) {
        gaps.sort_by_key(|g| g.line);
        if gaps.is_empty() {
            self.files.remove(path);
        } else {
            self.files.insert(path.to_string(), gaps);
        }
    }
    pub fn paths(&self) -> impl Iterator<Item = &String> {
        self.files.keys()
    }
    pub fn from_files(files: &[FileCoverage]) -> Self {
        let mut b = Baseline::default();
        for f in files {
            let gaps: Vec<Gap> = f
                .lines
                .iter()
                .filter(|l| !l.covered)
                .map(|l| Gap {
                    line: l.line,
                    text: l.text.clone(),
                })
                .collect();
            b.set_gaps(&f.path, gaps);
        }
        b
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap()
    }
    pub fn from_json(s: &str) -> Result<Self> {
        Ok(serde_json::from_str(s)?)
    }
    pub fn load(path: &str) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::from_json(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Baseline::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, path: &str) -> Result<()> {
        std::fs::write(path, self.to_json())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::{FileCoverage, LineCov};

    #[test]
    fn from_files_collects_uncovered_lines_with_text() {
        let files = vec![FileCoverage {
            path: "a.rs".into(),
            lines: vec![
                LineCov {
                    line: 1,
                    covered: true,
                    text: "ok".into(),
                },
                LineCov {
                    line: 2,
                    covered: false,
                    text: "gap".into(),
                },
            ],
        }];
        let b = Baseline::from_files(&files);
        assert_eq!(
            b.gaps("a.rs"),
            &[Gap {
                line: 2,
                text: "gap".into()
            }]
        );
        assert_eq!(b.gaps("missing.rs"), &[] as &[Gap]);
    }

    #[test]
    fn round_trips_through_json_stably() {
        let mut b = Baseline::default();
        b.set_gaps(
            "z.rs",
            vec![Gap {
                line: 3,
                text: "x".into(),
            }],
        );
        b.set_gaps(
            "a.rs",
            vec![Gap {
                line: 1,
                text: "y".into(),
            }],
        );
        let json = b.to_json();
        // keys sorted for stable diffs
        assert!(json.find("\"a.rs\"").unwrap() < json.find("\"z.rs\"").unwrap());
        let b2 = Baseline::from_json(&json).unwrap();
        assert_eq!(b2.gaps("a.rs"), b.gaps("a.rs"));
    }
}
