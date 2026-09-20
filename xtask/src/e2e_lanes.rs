//! Validated ownership catalog for distributed browser E2E lanes.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

const CATALOG: &str = include_str!("../../end2end/e2e-lanes.json");

#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u8,
    #[serde(rename = "experimentalProjectDependencies")]
    pub(crate) experimental_project_dependencies: BTreeMap<String, Vec<String>>,
    pub(crate) lanes: Vec<Lane>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Lane {
    pub(crate) backend: String,
    pub(crate) browser: String,
    pub(crate) partition: String,
    #[serde(rename = "shardIndex")]
    pub(crate) shard_index: Option<u8>,
    #[serde(rename = "shardCount")]
    pub(crate) shard_count: Option<u8>,
    pub(crate) identity: String,
    #[serde(rename = "traceDigit")]
    pub(crate) trace_digit: String,
    pub(crate) projects: Vec<String>,
    pub(crate) enabled: bool,
}

pub fn catalog() -> Result<Catalog, String> {
    let value = serde_json::from_str(CATALOG)
        .map_err(|error| format!("parsing E2E lane catalog: {error}"))?;
    validate(&value)?;
    Ok(value)
}

fn identity(lane: &Lane) -> String {
    match (lane.partition.as_str(), lane.shard_index, lane.shard_count) {
        ("unsplit", None, None) | ("serial-special", None, None) => {
            format!("{}-{}-{}", lane.backend, lane.browser, lane.partition)
        }
        ("ordinary", Some(index), Some(count)) => format!(
            "{}-{}-ordinary-{}-of-{}",
            lane.backend, lane.browser, index, count
        ),
        _ => String::new(),
    }
}

fn validate_candidate_group(
    backend: &str,
    browser: &str,
    lanes: &[&Lane],
) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let ordinary = lanes
        .iter()
        .copied()
        .filter(|lane| lane.partition == "ordinary")
        .collect::<Vec<_>>();
    let special = lanes
        .iter()
        .copied()
        .filter(|lane| lane.partition == "serial-special")
        .collect::<Vec<_>>();
    if ordinary.is_empty() || special.len() != 1 {
        return Err(format!(
            "{backend}/{browser} must have ordinary shards and one serial-special lane"
        ));
    }
    let shard_count = ordinary[0]
        .shard_count
        .ok_or_else(|| format!("{backend}/{browser} ordinary lanes lack shard metadata"))?;
    let indices = ordinary
        .iter()
        .filter_map(|lane| lane.shard_index)
        .collect::<BTreeSet<_>>();
    let expected_indices = (1..=shard_count).collect::<BTreeSet<_>>();
    if usize::from(shard_count) != ordinary.len()
        || indices != expected_indices
        || ordinary
            .iter()
            .any(|lane| lane.shard_count != Some(shard_count))
    {
        return Err(format!(
            "{backend}/{browser} ordinary shard indices must completely cover their declared count"
        ));
    }
    let ordinary_projects = &ordinary[0].projects;
    if ordinary
        .iter()
        .any(|lane| lane.projects != *ordinary_projects)
    {
        return Err(format!(
            "{backend}/{browser} ordinary shards must select the same projects"
        ));
    }
    let special_projects = special[0].projects.iter().collect::<BTreeSet<_>>();
    if ordinary_projects
        .iter()
        .any(|project| special_projects.contains(project))
    {
        return Err(format!(
            "{backend}/{browser} ordinary and serial-special projects overlap"
        ));
    }

    let mut owners = BTreeMap::<String, BTreeSet<String>>::new();
    for lane in lanes {
        for project in &lane.projects {
            owners
                .entry(project.clone())
                .or_default()
                .insert(lane.identity.clone());
        }
    }
    Ok(owners)
}

pub fn validate(catalog: &Catalog) -> Result<(), String> {
    if catalog.schema_version != 1 {
        return Err("unsupported E2E lane catalog schema; expected 1".into());
    }
    if catalog.lanes.is_empty() {
        return Err("E2E lane catalog has no lanes".into());
    }

    let mut identities = BTreeSet::new();
    let mut trace_digits = BTreeSet::new();
    let mut enabled_unsplit_pairs = BTreeSet::<(String, String)>::new();
    let mut split_lanes = BTreeMap::<(String, String), Vec<&Lane>>::new();
    for lane in &catalog.lanes {
        let project_count = lane.projects.iter().collect::<BTreeSet<_>>().len();
        if lane.backend.is_empty()
            || lane.browser.is_empty()
            || lane.projects.is_empty()
            || project_count != lane.projects.len()
            || lane.trace_digit.chars().count() != 1
            || !lane
                .trace_digit
                .chars()
                .all(|digit| digit.is_ascii_hexdigit())
            || lane.identity != identity(lane)
        {
            return Err(format!("malformed E2E lane `{}`", lane.identity));
        }
        if !identities.insert(lane.identity.clone()) {
            return Err(format!("duplicate E2E lane identity `{}`", lane.identity));
        }
        if !trace_digits.insert(lane.trace_digit.clone()) {
            return Err(format!("duplicate E2E trace digit `{}`", lane.trace_digit));
        }
        match lane.partition.as_str() {
            "unsplit" if lane.enabled => {
                if !enabled_unsplit_pairs.insert((lane.backend.clone(), lane.browser.clone())) {
                    return Err(format!(
                        "duplicate enabled unsplit E2E combination {}/{}",
                        lane.backend, lane.browser
                    ));
                }
            }
            "unsplit" if !lane.enabled && lane.browser == "firefox" => {}
            "ordinary" if lane.enabled => {
                let (Some(index), Some(count)) = (lane.shard_index, lane.shard_count) else {
                    return Err(format!(
                        "ordinary lane `{}` lacks shard metadata",
                        lane.identity
                    ));
                };
                if count < 2 || index == 0 || index > count {
                    return Err(format!(
                        "ordinary lane `{}` has invalid shard metadata",
                        lane.identity
                    ));
                }
                split_lanes
                    .entry((lane.backend.clone(), lane.browser.clone()))
                    .or_default()
                    .push(lane);
            }
            "serial-special" if lane.enabled => {
                split_lanes
                    .entry((lane.backend.clone(), lane.browser.clone()))
                    .or_default()
                    .push(lane);
            }
            _ => return Err(format!("invalid retained E2E lane `{}`", lane.identity)),
        }
    }
    let chromium_backends = enabled_unsplit_pairs
        .iter()
        .filter(|(_, browser)| browser == "chromium")
        .map(|(backend, _)| backend.clone())
        .collect::<BTreeSet<_>>();
    if chromium_backends.len() != 2 || enabled_unsplit_pairs.len() != 2 {
        return Err(
            "the production catalog must contain exactly two enabled Chromium unsplit lanes".into(),
        );
    }
    let split_backends = split_lanes
        .keys()
        .filter(|(_, browser)| browser == "firefox")
        .map(|(backend, _)| backend.clone())
        .collect::<BTreeSet<_>>();
    if split_backends != chromium_backends || split_lanes.len() != 2 {
        return Err("retained Firefox split lanes must cover every Chromium backend".into());
    }

    let mut owner_maps = Vec::new();
    for ((backend, browser), lanes) in &split_lanes {
        owner_maps.push((lanes, validate_candidate_group(backend, browser, lanes)?));
    }
    let declared_projects = catalog
        .experimental_project_dependencies
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    for (_, owners) in &owner_maps {
        if owners.keys().cloned().collect::<BTreeSet<_>>() != declared_projects {
            return Err(
                "Firefox split dependency keys must equal the retained project union".into(),
            );
        }
    }
    for (project, dependencies) in &catalog.experimental_project_dependencies {
        if dependencies.iter().collect::<BTreeSet<_>>().len() != dependencies.len() {
            return Err(format!("project `{project}` has duplicate dependencies"));
        }
        for (lanes, owners) in &owner_maps {
            let project_owners = owners
                .get(project)
                .ok_or_else(|| format!("candidate project `{project}` has no owner"))?;
            for dependency in dependencies {
                if dependency == project {
                    return Err(format!("project `{project}` depends on itself"));
                }
                if owners.get(dependency) != Some(project_owners) {
                    return Err(format!(
                        "dependency `{dependency}` crosses the lane owning `{project}`"
                    ));
                }
                for lane in lanes.iter().filter(|lane| lane.projects.contains(project)) {
                    let dependency_index = lane
                        .projects
                        .iter()
                        .position(|candidate| candidate == dependency)
                        .ok_or_else(|| {
                            format!("lane `{}` lacks dependency `{dependency}`", lane.identity)
                        })?;
                    let project_index = lane
                        .projects
                        .iter()
                        .position(|candidate| candidate == project)
                        .expect("owned project is present");
                    if dependency_index >= project_index {
                        return Err(format!(
                            "lane `{}` orders `{dependency}` after `{project}`",
                            lane.identity
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{catalog, validate};

    #[test]
    fn checked_in_catalog_has_a_valid_relational_topology() {
        validate(&catalog().unwrap()).unwrap();
    }

    #[test]
    fn rejects_cross_lane_dependency_and_bad_shard() {
        let mut candidate = catalog().unwrap();
        let ordinary = candidate
            .lanes
            .iter()
            .find(|lane| lane.partition == "ordinary")
            .unwrap()
            .projects[0]
            .clone();
        let special = candidate
            .lanes
            .iter()
            .find(|lane| lane.partition == "serial-special")
            .unwrap()
            .projects[0]
            .clone();
        candidate
            .experimental_project_dependencies
            .insert(ordinary, vec![special]);
        assert!(validate(&candidate).is_err());

        let mut candidate = catalog().unwrap();
        let lane = candidate
            .lanes
            .iter_mut()
            .find(|lane| lane.partition == "ordinary")
            .unwrap();
        lane.shard_index = lane.shard_count.map(|count| count + 1);
        lane.identity = super::identity(lane);
        assert!(validate(&candidate).is_err());
    }

    #[test]
    fn rejects_identity_collision_and_incomplete_shard_set() {
        let mut candidate = catalog().unwrap();
        candidate.lanes[1].identity = candidate.lanes[0].identity.clone();
        assert!(validate(&candidate).is_err());

        let mut candidate = catalog().unwrap();
        let duplicate_index = candidate
            .lanes
            .iter()
            .find(|lane| lane.partition == "ordinary")
            .unwrap()
            .shard_index;
        let lane = candidate
            .lanes
            .iter_mut()
            .filter(|lane| lane.partition == "ordinary")
            .nth(1)
            .unwrap();
        lane.shard_index = duplicate_index;
        lane.identity = super::identity(lane);
        assert!(validate(&candidate).is_err());
    }
}
