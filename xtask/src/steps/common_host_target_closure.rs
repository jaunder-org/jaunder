//! Cargo-metadata gate for the host floor and CSR runtime closure.
//!
//! The rule is graph-based because source text cannot reveal Cargo's resolved
//! target and feature graph. The adapter roots metadata in a temporary package
//! that depends only on `csr`, avoiding feature unification from unrelated
//! workspace members.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use xshell::{Shell, cmd};

use crate::git;
use crate::result::{CommandResult, StepResult};

const CSR_TARGET: &str = "wasm32-unknown-unknown";

#[derive(Clone, Debug)]
struct Graph {
    packages: BTreeMap<String, Package>,
    nodes: BTreeMap<String, Node>,
    root: String,
}

#[derive(Clone, Debug)]
struct Package {
    name: String,
    workspace: bool,
    proc_macro: bool,
}

#[derive(Clone, Debug)]
struct Node {
    features: BTreeSet<String>,
    dependencies: Vec<Dependency>,
}

#[derive(Clone, Debug)]
struct Dependency {
    package: String,
    runtime: bool,
}

/// Evaluates the two target-resolved crate-boundary invariants.
///
/// The resolves are deliberately isolated: host-only feature activation must not
/// contaminate CSR's feature closure. The returned diagnostic contains the
/// direct host edge or complete CSR path that requires repair.
fn evaluate(host_graph: &Graph, csr: &Graph) -> std::result::Result<(), String> {
    let host = host_graph
        .nodes
        .get(&host_graph.root)
        .ok_or_else(|| "Cargo metadata omitted the `host` resolve node".to_owned())?;

    for dependency in host
        .dependencies
        .iter()
        .filter(|dependency| dependency.runtime)
    {
        let package = host_graph
            .packages
            .get(&dependency.package)
            .ok_or_else(|| format!("Cargo metadata omitted package `{}`", dependency.package))?;
        if package.workspace
            && package.name != "common"
            && !(package.name == "macros" && package.proc_macro)
        {
            return Err(format!(
                "host runtime workspace dependency violates the host floor: host -> {}",
                package.name
            ));
        }
    }

    let mut queue = VecDeque::from([(csr.root.clone(), vec!["csr".to_owned()])]);
    let mut visited = BTreeSet::new();
    while let Some((package_id, path)) = queue.pop_front() {
        if !visited.insert(package_id.clone()) {
            continue;
        }
        let package = csr
            .packages
            .get(&package_id)
            .ok_or_else(|| format!("Cargo metadata omitted package `{package_id}`"))?;
        let node = csr
            .nodes
            .get(&package_id)
            .ok_or_else(|| format!("Cargo metadata omitted resolve node `{}`", package.name))?;

        if matches!(package.name.as_str(), "host" | "storage" | "server") {
            return Err(format!(
                "CSR wasm closure contains `{}` via {}",
                package.name,
                path.join(" -> ")
            ));
        }
        if package.name == "common" && node.features.contains("sqlx") {
            return Err(format!(
                "CSR wasm closure contains `common/sqlx` via {}/sqlx",
                path.join(" -> ")
            ));
        }

        for dependency in node
            .dependencies
            .iter()
            .filter(|dependency| dependency.runtime)
        {
            let next = csr.packages.get(&dependency.package).ok_or_else(|| {
                format!("Cargo metadata omitted package `{}`", dependency.package)
            })?;
            let mut next_path = path.clone();
            next_path.push(next.name.clone());
            queue.push_back((dependency.package.clone(), next_path));
        }
    }

    Ok(())
}

#[derive(Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
    resolve: CargoResolve,
}

#[derive(Deserialize)]
struct CargoPackage {
    id: String,
    name: String,
    source: Option<String>,
    targets: Vec<CargoTarget>,
}

#[derive(Deserialize)]
struct CargoTarget {
    kind: Vec<String>,
}

#[derive(Deserialize)]
struct CargoResolve {
    nodes: Vec<CargoNode>,
}

#[derive(Deserialize)]
struct CargoNode {
    id: String,
    features: Vec<String>,
    deps: Vec<CargoDependency>,
}

#[derive(Deserialize)]
struct CargoDependency {
    pkg: String,
    dep_kinds: Vec<CargoDependencyKind>,
}

#[derive(Deserialize)]
struct CargoDependencyKind {
    kind: Option<String>,
}

/// Converts Cargo's structured metadata into the evaluator's target graph.
fn graph_from_metadata(metadata: CargoMetadata, root_name: &str) -> Result<Graph> {
    let mut root = None;
    let mut packages = BTreeMap::new();
    for package in metadata.packages {
        if package.name == root_name {
            root = Some(package.id.clone());
        }
        packages.insert(
            package.id,
            Package {
                name: package.name,
                workspace: package.source.is_none(),
                proc_macro: package
                    .targets
                    .iter()
                    .any(|target| target.kind.iter().any(|kind| kind == "proc-macro")),
            },
        );
    }
    let nodes = metadata
        .resolve
        .nodes
        .into_iter()
        .map(|node| {
            (
                node.id,
                Node {
                    features: node.features.into_iter().collect(),
                    dependencies: node
                        .deps
                        .into_iter()
                        .map(|dependency| Dependency {
                            package: dependency.pkg,
                            runtime: dependency.dep_kinds.iter().any(|kind| kind.kind.is_none()),
                        })
                        .collect(),
                },
            )
        })
        .collect();
    Ok(Graph {
        packages,
        nodes,
        root: root.with_context(|| format!("Cargo metadata omitted the `{root_name}` package"))?,
    })
}

/// Obtains metadata for one target root, avoiding feature unification between
/// the host floor and the exact CSR wasm closure.
fn load_graph(sh: &Shell, package: &str, target: Option<&str>) -> Result<Graph> {
    let root = git::toplevel(Path::new("."))?;
    let temporary = tempfile::tempdir().context("creating temporary metadata root")?;
    let manifest = temporary.path().join("Cargo.toml");
    let source = temporary.path().join("src");
    fs::create_dir(&source).context("creating temporary metadata source directory")?;
    fs::write(source.join("lib.rs"), "pub fn graph_root() {}\n")
        .context("writing temporary metadata source")?;
    let package_path = serde_json::to_string(&Path::new(&root).join(package).to_string_lossy())
        .context("encoding package path for temporary manifest")?;
    fs::write(
        &manifest,
        format!(
            "[package]\nname = \"target-closure-root\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
             [dependencies]\n{package} = {{ path = {package_path} }}\n"
        ),
    )
    .context("writing temporary metadata manifest")?;

    let manifest = manifest.to_string_lossy().into_owned();
    let mut command = cmd!(
        sh,
        "cargo metadata --format-version=1 --manifest-path {manifest}"
    );
    if let Some(target) = target {
        command = command.args(["--filter-platform", target]);
    }
    let json = command
        .read()
        .with_context(|| format!("resolving the {package} dependency graph with cargo metadata"))?;
    let metadata = serde_json::from_str(&json).context("parsing Cargo metadata JSON")?;
    graph_from_metadata(metadata, package)
}

/// Runs the repository-shape gate.
pub fn run(result: &mut CommandResult) {
    let start = std::time::Instant::now();
    let shell = Shell::new().expect("constructing a shell cannot fail");
    let step = load_graph(&shell, "host", None)
        .and_then(|host| {
            load_graph(&shell, "csr", Some(CSR_TARGET))
                .and_then(|csr| evaluate(&host, &csr).map_err(|error| anyhow!(error)))
        })
        .map_or_else(
            |error| StepResult::fail("common-host-target-closure").detail(error.to_string()),
            |_| StepResult::ok("common-host-target-closure"),
        );
    result.push(step.with_duration(start.elapsed()));
}
#[cfg(test)]
mod tests {
    use super::{Dependency, Graph, Node, Package, evaluate};
    use std::collections::{BTreeMap, BTreeSet};

    fn graph(root: &str, edges: &[(&str, &str)], common_features: &[&str]) -> Graph {
        let packages = [
            ("csr", true, false),
            ("web", true, false),
            ("common", true, false),
            ("host", true, false),
            ("storage", true, false),
            ("server", true, false),
            ("macros", true, true),
            ("external", false, false),
        ]
        .into_iter()
        .map(|(name, workspace, proc_macro)| {
            (
                name.to_owned(),
                Package {
                    name: name.to_owned(),
                    workspace,
                    proc_macro,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
        let mut nodes: BTreeMap<_, Node> = packages
            .keys()
            .map(|name| {
                (
                    name.clone(),
                    Node {
                        features: BTreeSet::new(),
                        dependencies: Vec::new(),
                    },
                )
            })
            .collect();
        nodes.get_mut("common").unwrap().features = common_features
            .iter()
            .map(|feature| (*feature).to_owned())
            .collect();
        for (from, to) in edges {
            nodes.get_mut(*from).unwrap().dependencies.push(Dependency {
                package: (*to).to_owned(),
                runtime: true,
            });
        }
        Graph {
            packages,
            nodes,
            root: root.to_owned(),
        }
    }

    fn clean_host() -> Graph {
        graph(
            "host",
            &[("host", "common"), ("host", "macros"), ("host", "external")],
            &["sqlx"],
        )
    }

    fn clean_csr() -> Graph {
        let mut graph = graph("csr", &[("csr", "common"), ("csr", "external")], &[]);
        graph.packages.remove("host");
        graph.nodes.remove("host");
        graph
    }

    #[test]
    fn rejects_host_runtime_workspace_dependency_with_its_edge() {
        let error = evaluate(&graph("host", &[("host", "storage")], &[]), &clean_csr())
            .expect_err("host must remain the workspace floor");
        assert!(error.contains("host -> storage"));
    }

    #[test]
    fn rejects_host_dependency_in_csr_closure_with_the_path() {
        let error = evaluate(
            &clean_host(),
            &graph("csr", &[("csr", "web"), ("web", "host")], &[]),
        )
        .expect_err("host cannot enter CSR");
        assert!(error.contains("csr -> web -> host"));
    }

    #[test]
    fn rejects_storage_dependency_in_csr_closure_with_the_path() {
        let error = evaluate(
            &clean_host(),
            &graph("csr", &[("csr", "web"), ("web", "storage")], &[]),
        )
        .expect_err("storage cannot enter CSR");
        assert!(error.contains("csr -> web -> storage"));
    }

    #[test]
    fn rejects_server_dependency_in_csr_closure_with_the_path() {
        let error = evaluate(
            &clean_host(),
            &graph("csr", &[("csr", "web"), ("web", "server")], &[]),
        )
        .expect_err("server cannot enter CSR");
        assert!(error.contains("csr -> web -> server"));
    }

    #[test]
    fn rejects_common_sqlx_in_csr_closure_with_the_path() {
        let error = evaluate(
            &clean_host(),
            &graph("csr", &[("csr", "common")], &["sqlx"]),
        )
        .expect_err("common/sqlx cannot enter CSR");
        assert!(error.contains("csr -> common/sqlx"));
    }

    #[test]
    fn permits_an_absent_host_in_csr_and_host_only_common_sqlx() {
        evaluate(&clean_host(), &clean_csr())
            .expect("host-only feature activation cannot contaminate the CSR closure");
    }
}
