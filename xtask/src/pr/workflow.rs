//! Immutable GitHub Actions workflow graphs used to classify PR checks.
//!
//! The parser deliberately accepts only the workflow constructs whose dependency
//! semantics this observer can prove. Unknown expression-driven topology is an error
//! rather than a guess that could hide a required failure.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use serde_yaml::Value as YamlValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    Direct,
    Transitive,
    Optional,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RuntimeJob {
    pub name: String,
    /// The stable GitHub Actions check-run/job identity from the current attempt.
    #[serde(default)]
    pub check_run_id: Option<u64>,
    #[serde(default)]
    pub job_key: Option<String>,
    #[serde(default)]
    pub matrix: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    MissingJobs,
    InvalidWorkflow { detail: String },
    MissingJobName { job_key: String },
    MissingWorkflowSource { path: String },
    UnsupportedExpression { value: String },
    UnsupportedRemoteReusableWorkflow { uses: String },
    InvalidMatrix { job_key: String, value: String },
    MissingNeed { job_key: String, need: String },
    DependencyCycle { job_key: String },
    MissingJoin { name: String },
    AmbiguousJoin { name: String },
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingJobs => f.write_str("workflow has no jobs"),
            Self::InvalidWorkflow { detail } => write!(f, "invalid workflow: {detail}"),
            Self::MissingJobName { job_key } => write!(f, "job {job_key} has no name"),
            Self::MissingWorkflowSource { path } => {
                write!(f, "missing local reusable workflow {path}")
            }
            Self::UnsupportedExpression { value } => {
                write!(f, "unsupported workflow expression {value}")
            }
            Self::UnsupportedRemoteReusableWorkflow { uses } => {
                write!(f, "unsupported remote reusable workflow {uses}")
            }
            Self::InvalidMatrix { job_key, value } => {
                write!(f, "invalid matrix for job {job_key}: {value}")
            }
            Self::MissingNeed { job_key, need } => {
                write!(f, "job {job_key} needs missing job {need}")
            }
            Self::DependencyCycle { job_key } => {
                write!(f, "workflow dependency cycle includes job {job_key}")
            }
            Self::MissingJoin { name } => write!(f, "no workflow job matches {name}"),
            Self::AmbiguousJoin { name } => write!(f, "multiple workflow jobs match {name}"),
        }
    }
}

impl std::error::Error for GraphError {}

#[derive(Debug, Clone)]
struct Node {
    job_key: String,
    display_name: String,
    matrix: BTreeMap<String, String>,
    needs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct WorkflowGraph {
    nodes: Vec<Node>,
}

impl WorkflowGraph {
    /// Returns local reusable-workflow references from job-level `uses` entries.
    ///
    /// Steps also carry `uses`, but they invoke actions rather than reusable
    /// workflows and therefore do not contribute dependency-graph nodes. Deserializing
    /// only the top-level `jobs.*.uses` surface keeps those two concepts separate.
    pub fn local_reusable_workflow_paths(source: &str) -> Result<BTreeSet<String>, GraphError> {
        parse_jobs(source).map(|jobs| {
            jobs.into_iter()
                .filter_map(|job| job.uses)
                .filter(|uses| uses.starts_with("./"))
                .collect()
        })
    }

    /// Parses one immutable workflow source and expands its statically-known matrix.
    ///
    /// Local reusable workflows need their immutable sources to establish runtime
    /// identities, so this entry point rejects them rather than guessing.
    pub fn parse(source: &str) -> Result<Self, GraphError> {
        Self::parse_with_local_sources(source, &BTreeMap::new())
    }

    /// Parses a workflow while proving every local reusable-workflow reference is
    /// available from the same immutable source set. The caller supplies sources by
    /// relative path because fetching them belongs to the later GitHub boundary.
    pub fn parse_with_local_sources(
        source: &str,
        local_sources: &BTreeMap<String, String>,
    ) -> Result<Self, GraphError> {
        let graph = Self::parse_inner(source, local_sources)?;
        graph.validate_dependencies()?;
        Ok(graph)
    }

    fn parse_inner(
        source: &str,
        local_sources: &BTreeMap<String, String>,
    ) -> Result<Self, GraphError> {
        let jobs = parse_jobs(source)?;
        let mut nodes = Vec::new();
        let mut reusable_terminals = BTreeMap::new();
        for job in jobs {
            let needs = job.needs.clone();
            if let Some(uses) = &job.uses {
                if uses.contains("${{") {
                    return Err(GraphError::UnsupportedExpression {
                        value: uses.clone(),
                    });
                }
                if !uses.starts_with("./") {
                    return Err(GraphError::UnsupportedRemoteReusableWorkflow {
                        uses: uses.clone(),
                    });
                }
                let reusable = local_sources
                    .get(uses)
                    .ok_or_else(|| GraphError::MissingWorkflowSource { path: uses.clone() })?;
                // GitHub reports the nested jobs, prefixed by the calling job key,
                // instead of a separate check run for the reusable-workflow caller.
                let nested = Self::parse_inner(reusable, local_sources)?;
                let (nested_nodes, terminals) = nested.inline_under(&job.key, &job.needs);
                nodes.extend(nested_nodes);
                reusable_terminals.insert(job.key.clone(), terminals);
                continue;
            }
            let name = job.name.ok_or_else(|| GraphError::MissingJobName {
                job_key: job.key.clone(),
            })?;
            if contains_unsupported_expression(&name) {
                return Err(GraphError::UnsupportedExpression { value: name });
            }
            let matrixes = expand_matrix(&job.key, &job.matrix)?;
            for matrix in matrixes {
                let display_name = render_matrix_name(&name, &matrix)?;
                nodes.push(Node {
                    job_key: job.key.clone(),
                    display_name,
                    matrix,
                    needs: needs.clone(),
                });
            }
        }
        for node in &mut nodes {
            node.needs = expand_reusable_needs(&node.needs, &reusable_terminals);
        }
        Ok(Self { nodes })
    }

    /// Names nested jobs under their calling job and returns the terminal nested jobs
    /// that satisfy the caller's downstream `needs`. The caller's existing `needs`
    /// become roots of the nested graph, so ancestors outside and inside the reusable
    /// workflow remain visible to one traversal without a synthetic caller check.
    fn inline_under(
        mut self,
        caller_key: &str,
        caller_needs: &[String],
    ) -> (Vec<Node>, Vec<String>) {
        let old_keys = self
            .nodes
            .iter()
            .map(|node| node.job_key.clone())
            .collect::<Vec<_>>();
        let roots = self
            .nodes
            .iter()
            .filter(|node| node.needs.is_empty())
            .map(|node| node.job_key.clone())
            .collect::<BTreeSet<_>>();
        for node in &mut self.nodes {
            let original_key = node.job_key.clone();
            node.job_key = format!("{caller_key}/{original_key}");
            node.display_name = format!("{caller_key} / {}", node.display_name);
            node.needs = node
                .needs
                .iter()
                .map(|need| format!("{caller_key}/{need}"))
                .collect();
            if roots.contains(&original_key) {
                node.needs.extend(caller_needs.iter().cloned());
            }
        }
        let nested_needs = self
            .nodes
            .iter()
            .flat_map(|node| node.needs.iter())
            .collect::<BTreeSet<_>>();
        let terminals = old_keys
            .into_iter()
            .map(|key| format!("{caller_key}/{key}"))
            .filter(|key| !nested_needs.contains(key))
            .collect();
        (self.nodes, terminals)
    }

    fn validate_dependencies(&self) -> Result<(), GraphError> {
        let keys = self
            .nodes
            .iter()
            .map(|node| node.job_key.clone())
            .collect::<BTreeSet<_>>();
        for node in &self.nodes {
            for need in &node.needs {
                if !keys.contains(need) {
                    return Err(GraphError::MissingNeed {
                        job_key: node.job_key.clone(),
                        need: need.clone(),
                    });
                }
            }
        }
        let dependencies = self
            .nodes
            .iter()
            .map(|node| (node.job_key.clone(), node.needs.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut states = BTreeMap::new();
        for key in dependencies.keys() {
            visit_dependency(key, &dependencies, &mut states)?;
        }
        Ok(())
    }

    /// Classifies one runtime job using exact display-name and matrix identity joins.
    ///
    /// Direct ruleset contexts take precedence. Other jobs are transitive only when
    /// their graph node is an ancestor of a uniquely correlated required context.
    pub fn classify(
        &self,
        runtime: &RuntimeJob,
        required_contexts: &[String],
    ) -> Result<Requirement, GraphError> {
        let node = self.correlate(runtime)?;
        if required_contexts.iter().any(|name| name == &runtime.name) {
            return Ok(Requirement::Direct);
        }
        let required = required_contexts
            .iter()
            .map(|name| {
                self.correlate(&RuntimeJob {
                    name: name.clone(),
                    check_run_id: None,
                    job_key: None,
                    matrix: BTreeMap::new(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if required
            .iter()
            .any(|target| self.is_ancestor(node, *target))
        {
            Ok(Requirement::Transitive)
        } else {
            Ok(Requirement::Optional)
        }
    }

    fn correlate(&self, runtime: &RuntimeJob) -> Result<usize, GraphError> {
        let matches = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.display_name == runtime.name
                    && runtime
                        .job_key
                        .as_ref()
                        .is_none_or(|key| key == &node.job_key)
                    && (runtime.matrix.is_empty() || runtime.matrix == node.matrix)
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [index] => Ok(*index),
            [] => Err(GraphError::MissingJoin {
                name: runtime.name.clone(),
            }),
            _ => Err(GraphError::AmbiguousJoin {
                name: runtime.name.clone(),
            }),
        }
    }

    fn is_ancestor(&self, candidate: usize, target: usize) -> bool {
        let mut pending = vec![target];
        let mut seen = BTreeSet::new();
        while let Some(index) = pending.pop() {
            if !seen.insert(index) {
                continue;
            }
            for need in &self.nodes[index].needs {
                for (parent, node) in self.nodes.iter().enumerate() {
                    if node.job_key == *need {
                        if parent == candidate {
                            return true;
                        }
                        pending.push(parent);
                    }
                }
            }
        }
        false
    }
}

fn expand_reusable_needs(
    needs: &[String],
    reusable_terminals: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    needs
        .iter()
        .flat_map(|need| {
            reusable_terminals
                .get(need)
                .cloned()
                .unwrap_or_else(|| vec![need.clone()])
        })
        .collect()
}

fn visit_dependency(
    job_key: &str,
    dependencies: &BTreeMap<String, Vec<String>>,
    states: &mut BTreeMap<String, u8>,
) -> Result<(), GraphError> {
    match states.get(job_key).copied().unwrap_or_default() {
        1 => {
            return Err(GraphError::DependencyCycle {
                job_key: job_key.to_string(),
            });
        }
        2 => return Ok(()),
        _ => {}
    }
    states.insert(job_key.to_string(), 1);
    if let Some(needs) = dependencies.get(job_key) {
        for need in needs {
            visit_dependency(need, dependencies, states)?;
        }
    }
    states.insert(job_key.to_string(), 2);
    Ok(())
}

#[derive(Debug, Clone)]
struct Job {
    key: String,
    name: Option<String>,
    needs: Vec<String>,
    matrix: BTreeMap<String, Vec<String>>,
    uses: Option<String>,
}

#[derive(Deserialize)]
struct Workflow {
    jobs: Option<BTreeMap<String, JobDefinition>>,
}

#[derive(Deserialize)]
struct JobDefinition {
    name: Option<String>,
    #[serde(default)]
    needs: Needs,
    strategy: Option<Strategy>,
    uses: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(untagged)]
enum Needs {
    #[default]
    None,
    One(String),
    Many(Vec<String>),
}

impl Needs {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::None => Vec::new(),
            Self::One(need) => vec![need],
            Self::Many(needs) => needs,
        }
    }
}

#[derive(Deserialize)]
struct Strategy {
    matrix: Option<BTreeMap<String, Vec<YamlValue>>>,
}

fn parse_jobs(source: &str) -> Result<Vec<Job>, GraphError> {
    let workflow: Workflow =
        serde_yaml::from_str(source).map_err(|error| GraphError::InvalidWorkflow {
            detail: error.to_string(),
        })?;
    let jobs = workflow.jobs.ok_or(GraphError::MissingJobs)?;
    if jobs.is_empty() {
        return Err(GraphError::MissingJobs);
    }
    jobs.into_iter()
        .map(|(key, job)| {
            let matrix = job
                .strategy
                .and_then(|strategy| strategy.matrix)
                .unwrap_or_default()
                .into_iter()
                .map(|(name, values)| {
                    values
                        .into_iter()
                        .map(|value| yaml_scalar(&key, value))
                        .collect::<Result<Vec<_>, _>>()
                        .map(|values| (name, values))
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            Ok(Job {
                key,
                name: job.name,
                needs: job.needs.into_vec(),
                matrix,
                uses: job.uses,
            })
        })
        .collect()
}

fn yaml_scalar(job_key: &str, value: YamlValue) -> Result<String, GraphError> {
    match value {
        YamlValue::String(value) => Ok(value),
        YamlValue::Number(value) => Ok(value.to_string()),
        YamlValue::Bool(value) => Ok(value.to_string()),
        value => Err(GraphError::InvalidMatrix {
            job_key: job_key.to_string(),
            value: format!("{value:?}"),
        }),
    }
}

fn expand_matrix(
    job_key: &str,
    matrix: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<BTreeMap<String, String>>, GraphError> {
    let mut result = vec![BTreeMap::new()];
    for (key, values) in matrix {
        if values.is_empty() {
            return Err(GraphError::InvalidMatrix {
                job_key: job_key.to_string(),
                value: key.clone(),
            });
        }
        result = result
            .into_iter()
            .flat_map(|partial| {
                values.iter().map(move |value| {
                    let mut expanded = partial.clone();
                    expanded.insert(key.clone(), value.clone());
                    expanded
                })
            })
            .collect();
    }
    Ok(result)
}

fn contains_unsupported_expression(value: &str) -> bool {
    value.contains("${{") && !value.contains("${{ matrix.")
}

fn render_matrix_name(
    template: &str,
    matrix: &BTreeMap<String, String>,
) -> Result<String, GraphError> {
    let mut rendered = template.to_string();
    while let Some(start) = rendered.find("${{") {
        let Some(end) = rendered[start..].find("}}") else {
            return Err(GraphError::UnsupportedExpression {
                value: template.to_string(),
            });
        };
        let end = start + end + 2;
        let expression = rendered[start + 3..end - 2].trim();
        let Some(key) = expression.strip_prefix("matrix.") else {
            return Err(GraphError::UnsupportedExpression {
                value: template.to_string(),
            });
        };
        let Some(value) = matrix.get(key.trim()) else {
            return Err(GraphError::UnsupportedExpression {
                value: template.to_string(),
            });
        };
        rendered.replace_range(start..end, value);
    }
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/src/pr/testdata/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap_or_else(|error| panic!("fixture {name}: {error}"))
    }

    fn runtime(name: &str) -> RuntimeJob {
        RuntimeJob {
            name: name.into(),
            check_run_id: None,
            job_key: None,
            matrix: BTreeMap::new(),
        }
    }

    fn captured_runtime_jobs() -> Vec<RuntimeJob> {
        serde_json::from_str(&fixture("workflow-runtime-jobs.json"))
            .unwrap_or_else(|error| panic!("runtime fixture: {error}"))
    }

    #[test]
    fn production_parser_classifies_renamed_ancestor_and_matrix_jobs() {
        let graph = WorkflowGraph::parse(&fixture("workflow-graph.yml")).unwrap();
        let required = vec!["Aggregate verdict".into()];
        assert_eq!(
            graph.classify(&runtime("Aggregate verdict"), &required),
            Ok(Requirement::Direct)
        );
        let jobs = captured_runtime_jobs();
        assert_eq!(
            graph.classify(&jobs[0], &required),
            Ok(Requirement::Transitive)
        );
        assert_eq!(
            graph.classify(&jobs[1], &required),
            Ok(Requirement::Transitive)
        );
        assert_eq!(
            graph.classify(&jobs[2], &required),
            Ok(Requirement::Optional)
        );
    }

    #[test]
    fn local_reusable_workflow_jobs_use_actions_caller_prefixed_identity() {
        let sources = BTreeMap::from([(
            "./.github/workflows/reusable.yml".into(),
            fixture("workflow-reusable.yml"),
        )]);
        let graph =
            WorkflowGraph::parse_with_local_sources(&fixture("workflow-local-reuse.yml"), &sources)
                .unwrap();
        let runtime_jobs =
            serde_json::from_str::<Vec<RuntimeJob>>(&fixture("workflow-local-reuse-runtime.json"))
                .unwrap_or_else(|error| panic!("local reusable runtime fixture: {error}"));
        assert_eq!(
            graph.classify(&runtime_jobs[0], &["Aggregate".into()]),
            Ok(Requirement::Transitive)
        );
        assert_eq!(
            graph.classify(&runtime_jobs[1], &["Aggregate".into()]),
            Ok(Requirement::Transitive)
        );
        assert_eq!(
            graph.classify(&runtime("Preparation"), &["Aggregate".into()]),
            Ok(Requirement::Transitive)
        );
        assert!(matches!(
            graph.classify(&runtime("package"), &["Aggregate".into()]),
            Err(GraphError::MissingJoin { .. })
        ));
    }

    #[test]
    fn local_reusable_references_exclude_step_level_local_actions() {
        // This is the CI shape: a job-level reusable workflow and a step-level
        // repository action both use local paths, but only the job builds graph
        // ancestry and therefore needs workflow source evidence.
        let source = r#"
            jobs:
              reusable:
                uses: "./.github/workflows/reusable.yml"
              ordinary:
                name: ordinary
                runs-on: ubuntu-24.04
                steps:
                  - uses: './.github/actions/setup-ci'
        "#;
        assert_eq!(
            WorkflowGraph::local_reusable_workflow_paths(source).unwrap(),
            BTreeSet::from(["./.github/workflows/reusable.yml".into()])
        );
    }

    #[test]
    fn missing_local_reusable_source_fails_closed() {
        assert!(matches!(
            WorkflowGraph::parse_with_local_sources(
                &fixture("workflow-local-reuse.yml"),
                &BTreeMap::new()
            ),
            Err(GraphError::MissingWorkflowSource { .. })
        ));
    }

    #[test]
    fn remote_reuse_and_unsupported_expressions_fail_closed() {
        assert!(matches!(
            WorkflowGraph::parse(&fixture("workflow-remote-reuse.yml")),
            Err(GraphError::UnsupportedRemoteReusableWorkflow { .. })
        ));
        assert!(matches!(
            WorkflowGraph::parse(&fixture("workflow-unsupported-expression.yml")),
            Err(GraphError::UnsupportedExpression { .. })
        ));
    }

    #[test]
    fn malformed_needs_and_dependency_cycles_fail_closed() {
        assert!(matches!(
            WorkflowGraph::parse(&fixture("workflow-missing-need.yml")),
            Err(GraphError::MissingNeed { .. })
        ));
        assert!(matches!(
            WorkflowGraph::parse(&fixture("workflow-cycle.yml")),
            Err(GraphError::DependencyCycle { .. })
        ));
    }

    #[test]
    fn missing_and_ambiguous_runtime_joins_fail_closed() {
        let graph = WorkflowGraph::parse(&fixture("workflow-ambiguous.yml")).unwrap();
        assert!(matches!(
            graph.classify(&runtime("missing"), &["Aggregate".into()]),
            Err(GraphError::MissingJoin { .. })
        ));
        assert!(matches!(
            graph.classify(&runtime("duplicate"), &["Aggregate".into()]),
            Err(GraphError::AmbiguousJoin { .. })
        ));
    }
}
