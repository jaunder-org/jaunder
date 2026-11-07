//! Command-level proofs of the Just/gh issue boundary; no real tracker writes.

use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::{Command, Output};

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new(responses: Value) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("responses.json"), responses.to_string()).unwrap();
        std::fs::write(dir.path().join("index"), "0").unwrap();
        let gh = dir.path().join("gh");
        std::fs::write(&gh, r#"#!/usr/bin/env bash
set -euo pipefail
input=''
if [[ " $* " == *' --input - '* ]]; then input=$(</dev/stdin); fi
jq -cn --arg input "$input" --args '{args:$ARGS.positional,input:$input}' -- "$@" >> "$FIXTURE/calls.jsonl"
# Inventory replies model gh --paginate --slurp, never an unpaginated request.
for arg in "$@"; do
    case "$arg" in
        repos/*/milestones\?*|repos/*/issues\?*|repos/*/dependencies/blocked_by\?*)
            [[ " $* " == *' --paginate '* && " $* " == *' --slurp '* ]] || {
                echo 'inventory requires --paginate --slurp' >&2; exit 19;
            } ;;
    esac
done
n=$(<"$FIXTURE/index")
printf '%s' "$((n+1))" > "$FIXTURE/index"
reply=$(jq -ce --argjson n "$n" '.[$n] // error("unexpected gh call")' "$FIXTURE/responses.json")
if [[ $(jq -r '.exit // 0' <<< "$reply") != 0 ]]; then
    jq -c '.body' <<< "$reply"
    printf 'simulated gh failure\n' >&2
    exit 1
fi
jq -c '.body' <<< "$reply"
"#).unwrap();
        std::fs::set_permissions(gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.path().join("body.md"), "Keep <placeholder> exactly.\n").unwrap();
        Self { dir }
    }

    fn run(&self, recipe: &str, number: Option<&str>) -> Output {
        self.run_with_blockers(recipe, number, &[])
    }

    fn run_with_blockers(&self, recipe: &str, number: Option<&str>, blockers: &[&str]) -> Output {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut command = Command::new("just");
        command.current_dir(root).arg(recipe);
        if let Some(number) = number {
            command.arg(number);
        }
        command
            .args([
                "fix(tooling): preserve metadata",
                "Task",
                "3",
                "P2",
                "tooling,dx",
            ])
            .arg(self.dir.path().join("body.md"))
            .args(blockers);
        command
            .env("FIXTURE", self.dir.path())
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.dir.path().display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .output()
            .unwrap()
    }

    fn discover(&self) -> Output {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        Command::new("just")
            .current_dir(root)
            .args(["issue-discover", "3"])
            .env("FIXTURE", self.dir.path())
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.dir.path().display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .output()
            .unwrap()
    }

    fn calls(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.path().join("calls.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn readback() -> Value {
    json!({"number":731,"html_url":"https://github.com/jaunder-org/jaunder/issues/731",
        "title":"fix(tooling): preserve metadata","body":"Keep <placeholder> exactly.\n",
        "type":{"name":"Task"},"milestone":{"number":3},
        "labels":[{"name":"dx"},{"name":"tooling"}]})
}

fn responses() -> Value {
    json!([
        {"body":[[{"number":3,"title":"Developer tooling & DX"}]]},
        {"body":{"number":731,"html_url":"https://github.com/jaunder-org/jaunder/issues/731"}},
        {"body":{"id":"PVTI_1"}},
        {"body":{"id":"PVTI_1"}},
        {"body":readback()},
        {"body":{"data":{"repository":{"issue":{"projectItems":{
            "nodes":[{"id":"PVTI_1","project":{"id":"PVT_kwDOECw7os4BblPP"},"priority":{"name":"P2"}}],
            "pageInfo":{"hasNextPage":false}
        }}}}}}
    ])
}

#[test]
fn edit_frontdoor_replaces_explicit_metadata_without_changing_issue_identity() {
    let fixture = Fixture::new(responses());
    let output = fixture.run("issue-edit", Some("731"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fixture.calls()[1]["args"],
        json!([
            "api",
            "--method",
            "PATCH",
            "repos/jaunder-org/jaunder/issues/731",
            "--input",
            "-"
        ])
    );
    let fixture = Fixture::new(responses());
    let output = fixture.run("issue-edit", Some("732"));
    assert!(
        !output.status.success(),
        "wrong issue identity must be refused"
    );
    assert_eq!(
        fixture.calls().len(),
        2,
        "do not file a different issue in Backlog"
    );
}

#[test]
fn mutation_frontdoors_fail_visibly_on_partial_malformed_or_mangled_evidence() {
    for recipe in ["issue-create", "issue-edit"] {
        let number = (recipe == "issue-edit").then_some("731");
        for (index, bad) in [
            (
                0,
                json!({"body":[[{"number":3,"title":"Developer tooling & DX"}],[{"number":3,"title":"duplicate"}]]}),
            ),
            (0, json!({"body":{"message":"not pages"}})),
            (1, json!({"body":{}})),
            (2, json!({"body":{}})),
            (3, json!({"exit":1})),
            (3, json!({"body":{}})),
            (4, json!({"body":{"number":731,"body":"partial"}})),
            (
                4,
                json!({"body":{ "number":731,"title":"fix(tooling): preserve metadata", "body":"Keep  exactly.\n", "type":{"name":"Task"},"milestone":{"number":3},"labels":[{"name":"dx"},{"name":"tooling"}]}}),
            ),
            (
                5,
                json!({"body":{"errors":[{"message":"GraphQL failure"}]}}),
            ),
            (
                5,
                json!({"body":{"data":{"repository":{"issue":{"projectItems":{"nodes":[],"pageInfo":{"hasNextPage":true}}}}}}}),
            ),
        ] {
            let mut replies = responses();
            replies[index] = bad;
            let fixture = Fixture::new(replies);
            let output = fixture.run(recipe, number);
            assert!(
                !output.status.success(),
                "{recipe} accepted bad step {index}"
            );
            assert!(
                output.stdout.is_empty(),
                "must not emit a partial-success report"
            );
            assert!(String::from_utf8_lossy(&output.stderr).contains("failed"));
        }
    }
}

#[test]
fn explicit_dependencies_use_native_database_ids_and_verified_readback() {
    let mut replies = responses().as_array().unwrap().clone();
    replies.splice(
        4..4,
        [
            json!({"body":{"id":9001,"number":42,"state":"open"}}),
            json!({"body":[[{"id":9000,"number":40,"state":"closed"}],[{"id":9002,"number":43,"state":"open"}]]}),
            json!({"body":{"id":9001,"number":42,"state":"open"}}),
            json!({"body":[[{"id":9000,"number":40,"state":"closed"}],[{"id":9002,"number":43,"state":"open"},{"id":9001,"number":42,"state":"open"}]]}),
        ],
    );
    let fixture = Fixture::new(Value::Array(replies));
    let output = fixture.run_with_blockers("issue-create", None, &["42"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fixture.calls();
    assert_eq!(
        calls[6]["args"],
        json!([
            "api",
            "--method",
            "POST",
            "repos/jaunder-org/jaunder/issues/731/dependencies/blocked_by",
            "-F",
            "issue_id=9001"
        ])
    );
}

#[test]
fn missing_dependency_readback_cannot_claim_success() {
    let mut replies = responses().as_array().unwrap().clone();
    replies.splice(
        4..4,
        [
            json!({"body":{"id":9001,"number":42,"state":"open"}}),
            json!({"body":[[]]}),
            json!({"body":{"id":9001,"number":42,"state":"open"}}),
            json!({"body":[[]]}),
        ],
    );
    let fixture = Fixture::new(Value::Array(replies));
    let output = fixture.run_with_blockers("issue-create", None, &["42"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn empty_body_is_refused_before_mutation() {
    let fixture = Fixture::new(responses());
    std::fs::write(fixture.dir.path().join("body.md"), " \n").unwrap();
    let output = fixture.run("issue-create", None);
    assert!(!output.status.success());
    assert_eq!(fixture.calls().len(), 1, "only read milestone metadata");
}

fn discovery_responses() -> Value {
    let project = |status: &str| {
        json!({"body":{"data":{"repository":{"issue":{"projectItems":{
            "nodes":[{"id":"PVTI_1","project":{"id":"PVT_kwDOECw7os4BblPP"},"status":{"name":status}}],
            "pageInfo":{"hasNextPage":false}
        }}}}}})
    };
    json!([
        {"body":[[{"number":3,"title":"Developer tooling & DX"}]]},
        {"body":[[
            {"number":731,"title":"available","html_url":"https://github.com/jaunder-org/jaunder/issues/731","labels":[{"name":"tooling"}]},
            {"number":732,"title":"claimed","html_url":"https://github.com/jaunder-org/jaunder/issues/732","labels":[]},
            {"number":733,"title":"blocked","html_url":"https://github.com/jaunder-org/jaunder/issues/733","labels":[]},
            {"number":734,"pull_request":{}}
        ]]},
        project("Todo"), {"body":[[]]},
        project("In Progress"), {"body":[[]]},
        project("Todo"), {"body":[[{"number":42,"state":"open"}]]}
    ])
}

#[test]
fn discovery_is_read_only_and_skips_claimed_blocked_and_pull_requests() {
    let fixture = Fixture::new(discovery_responses());
    let output = fixture.discover();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(report["candidates"][0]["number"], 731);
    assert_eq!(report["skipped"][0]["reason"], "claimed");
    assert_eq!(report["skipped"][1]["reason"], "blocked");
    for call in fixture.calls() {
        let args = call["args"].as_array().unwrap();
        assert!(!args.contains(&json!("--method")));
        assert!(
            args.iter()
                .all(|value| !value.as_str().unwrap().starts_with("query=mutation"))
        );
    }
}

#[test]
fn discovery_consumes_all_inventory_pages_before_filtering_closed_prerequisites() {
    let mut replies = discovery_responses();
    replies[0]["body"] = json!([[{"number":2,"title":"Other milestone"}],[{"number":3,"title":"Developer tooling & DX"}]]);
    let issues = replies[1]["body"][0].as_array().unwrap().clone();
    replies[1]["body"] = json!([[issues[0]], issues[1..]]);
    replies[7]["body"] = json!([[{"number":40,"state":"closed"}],[{"number":42,"state":"open"}]]);
    let fixture = Fixture::new(replies);
    let output = fixture.discover();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(report["skipped"].as_array().unwrap().len(), 2);
    assert_eq!(report["skipped"][1]["number"], 733);
    assert_eq!(report["skipped"][1]["blocked_by"], json!([42]));
}

#[test]
fn repeated_identities_across_inventory_pages_are_refused_before_filtering() {
    for (index, row) in [
        (0, json!({"number":3,"title":"Developer tooling & DX"})),
        (
            1,
            json!({"number":731,"title":"available","html_url":"https://github.com/jaunder-org/jaunder/issues/731","labels":[]}),
        ),
        (3, json!({"number":42,"state":"closed"})),
    ] {
        let mut replies = discovery_responses();
        replies[index]["body"] = json!([[row.clone()], [row]]);
        let fixture = Fixture::new(replies);
        let output = fixture.discover();
        assert!(
            !output.status.success(),
            "accepted duplicate at step {index}"
        );
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("invalid/repeated issue identity")
        );
    }
}

#[test]
fn discovery_refuses_late_page_failure_and_incomplete_or_malformed_evidence() {
    for (index, bad) in [
        (1, json!({"exit":1,"body":[[]]})),
        (1, json!({"body":[[],{"message":"bad later page"}]})),
        (1, json!({"body":[[{"number":731,"pull_request":null}]]})),
        (
            2,
            json!({"body":{"data":{"repository":{"issue":{"projectItems":{"nodes":[],"pageInfo":{"hasNextPage":true}}}}}}}),
        ),
        (
            2,
            json!({"body":{"data":{"repository":{"issue":{"projectItems":{"nodes":[{"id":"PVTI_1","project":{"id":"PVT_kwDOECw7os4BblPP"}}],"pageInfo":{"hasNextPage":false}}}}}}}),
        ),
        (3, json!({"body":[[{"number":42,"state":"unknown"}]]})),
    ] {
        let mut replies = discovery_responses();
        replies[index] = bad;
        let fixture = Fixture::new(replies);
        let output = fixture.discover();
        assert!(
            !output.status.success(),
            "accepted bad discovery step {index}"
        );
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn discovery_preserves_branch_docs_outline_progress_and_open_pr_resume_evidence() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut replies = discovery_responses().as_array().unwrap().clone();
    replies.insert(4, json!({"body":[{"number":900,"url":"https://github.com/jaunder-org/jaunder/pull/900","state":"OPEN"}]}));
    let fixture = Fixture::new(Value::Array(replies));
    symlink(root.join("scripts"), fixture.dir.path().join("scripts")).unwrap();
    for kind in ["specs", "plans"] {
        let directory = fixture.dir.path().join("docs/superpowers").join(kind);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("test-issue-731-resume.md"),
            "- [x] first\n- [X] second\n- [ ] third\n",
        )
        .unwrap();
    }
    let git = fixture.dir.path().join("git");
    std::fs::write(&git, "#!/usr/bin/env bash\nprintf 'issue-731-resume\\n'\n").unwrap();
    std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::new("bash")
        .arg(root.join("scripts/issue-discover.sh"))
        .arg("3")
        .current_dir(fixture.dir.path())
        .env("FIXTURE", fixture.dir.path())
        .env(
            "PATH",
            format!(
                "{}:{}",
                fixture.dir.path().display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["candidates"][0]["local"],
        json!({
            "branch":"issue-731-resume", "spec":"docs/superpowers/specs/test-issue-731-resume.md",
            "plan":"docs/superpowers/plans/test-issue-731-resume.md", "plan_progress":{"done":2,"total":3},
            "open_pr":{"number":900,"url":"https://github.com/jaunder-org/jaunder/pull/900","state":"OPEN"}
        })
    );
}

#[test]
fn missing_or_invalid_decisions_cannot_trigger_an_issue_mutation() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (issue_type, priority, labels) in [
        ("", "P2", "tooling"),
        ("Task", "P0", "tooling"),
        ("Task", "P2", ""),
        ("Task", "P2", "tooling,tooling"),
    ] {
        let fixture = Fixture::new(responses());
        let output = Command::new("just")
            .current_dir(&root)
            .args([
                "issue-create",
                "fix(tooling): preserve metadata",
                issue_type,
                "3",
                priority,
                labels,
            ])
            .arg(fixture.dir.path().join("body.md"))
            .env("FIXTURE", fixture.dir.path())
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    fixture.dir.path().display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .output()
            .unwrap();
        assert!(!output.status.success());
        if fixture.dir.path().join("calls.jsonl").exists() {
            assert!(
                fixture.calls().iter().all(|call| !call["args"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("--method"))),
                "{issue_type}/{priority}/{labels}: {:?}",
                fixture.calls()
            );
        }
    }
}

#[test]
fn create_frontdoor_applies_explicit_metadata_and_checks_readback() {
    let fixture = Fixture::new(responses());
    let output = fixture.run("issue-create", None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["number"], 731);
    assert_eq!(report["backlog"]["priority"]["name"], "P2");
    let calls = fixture.calls();
    let input: Value = serde_json::from_str(calls[1]["input"].as_str().unwrap()).unwrap();
    assert_eq!(
        input,
        json!({"title":"fix(tooling): preserve metadata","type":"Task",
        "milestone":3,"labels":["tooling","dx"],"body":"Keep <placeholder> exactly.\n"})
    );
    assert!(
        calls[2]["args"]
            .as_array()
            .unwrap()
            .contains(&json!("item-add"))
    );
    assert!(
        calls[3]["args"]
            .as_array()
            .unwrap()
            .contains(&json!("be01f9da"))
    );
}
