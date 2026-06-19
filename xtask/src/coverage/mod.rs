// Scaffold module: the parser API below is consumed by later coverage tasks
// (diffmap/baseline/classify/crap/run). Suppress dead-code noise until wired.
#![allow(dead_code)]

pub mod baseline;
pub mod diffmap;
pub mod report;

#[derive(Clone, Debug, PartialEq)]
pub struct LineCov {
    pub line: u32,
    pub covered: bool,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FileCoverage {
    pub path: String,
    pub lines: Vec<LineCov>,
}
