//! Exercise the executable's read-only command surface without GitHub credentials.

use std::process::Command;

fn observer(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pr-observer"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn watch_help_documents_the_read_only_modes() {
    let output = observer(&["watch", "--help"]);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("Never merges or mutates"));
    assert!(help.contains("--once"));
    assert!(help.contains("--until"));
    assert!(help.contains("--expected-head"));
    assert!(help.contains("--acknowledged-ejection"));
    assert!(help.contains("--observed-queued-head"));
    assert!(help.contains("[default: 30]"));
    assert!(help.contains("[default: 90]"));
}

#[test]
fn mismatched_queue_history_is_refused_before_network_observation() {
    let output = observer(&[
        "watch",
        "731",
        "--expected-head",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "--observed-queued-head",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("queued head must equal expected head")
    );
}

#[test]
fn mutation_commands_and_unsafe_watch_arguments_are_refused_before_observation() {
    for args in [
        vec!["land", "731"],
        vec!["cleanup", "731"],
        vec!["watch", "731", "--reenqueue"],
        vec!["watch", "731", "--once", "--until", "merged"],
        vec!["watch", "731", "--interval", "0"],
        vec!["watch", "731", "--timeout", "0"],
        vec!["watch", "731", "--expected-head", "abc"],
        vec![
            "watch",
            "731",
            "--observed-queued-head",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ],
        vec![
            "watch",
            "731",
            "--acknowledged-ejection",
            "https://github.com/a/b/actions/runs/1",
        ],
        vec![
            "watch",
            "731",
            "--expected-head",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--acknowledged-ejection",
            "not-a-url",
        ],
    ] {
        let output = observer(&args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "no observer report for {args:?}");
        assert!(String::from_utf8(output.stderr).unwrap().contains("error:"));
    }
}
