use std::path::{Path, PathBuf};
use std::process::Command;

const ACTION: &str = include_str!("../action.yml");
const EXAMPLE: &str = include_str!("../.github/examples/a2a-lab-compliance.yml");

#[test]
fn action_documents_inputs_and_outputs() {
    for input in [
        "a2a-url:",
        "mcp-url:",
        "fixtures:",
        "profile:",
        "suite:",
        "llm-check:",
        "report-path:",
        "timeout-milliseconds:",
    ] {
        assert!(ACTION.contains(input), "missing action input {input}");
    }
    for output in [
        "profile:",
        "suite:",
        "llm-check:",
        "compliant:",
        "report-path:",
    ] {
        assert!(ACTION.contains(output), "missing action output {output}");
    }
    assert!(ACTION.contains("default: full"));
    assert!(ACTION.contains(r#"default: "true""#));
}

#[test]
fn action_runner_forwards_local_command_controls() {
    let runner = std::fs::read_to_string(runner_path()).unwrap();
    for argument in [
        "--a2a-url",
        "--mcp-url",
        "--fixtures",
        "--profile",
        "--suite",
        "--report",
        "--timeout-milliseconds",
    ] {
        assert!(
            runner.contains(argument),
            "missing runner argument {argument}"
        );
    }
    assert!(runner.contains("--no-llm-check"));
    assert!(runner.contains("--bin a2a-lab-compliance"));
}

#[test]
fn action_rejects_invalid_input_before_running_suite() {
    let directory = temporary_directory("invalid");
    let fixture = directory.join("fixtures.json");
    let output = directory.join("output");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(&fixture, "{}").unwrap();

    let result = action_command(&fixture, &directory.join("report.json"), &output)
        .env("INPUT_A2A_URL", "not-a-url")
        .output()
        .unwrap();

    assert_eq!(result.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&result.stderr).contains("absolute HTTP(S) URL"));
    assert!(!output.exists());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn action_rejects_invalid_suite_and_llm_check_before_running_suite() {
    for (name, value, diagnostic) in [
        ("INPUT_SUITE", "extended", "suite must be basic or full"),
        ("INPUT_LLM_CHECK", "yes", "llm-check must be true or false"),
    ] {
        let directory = temporary_directory(value);
        let fixture = directory.join("fixtures.json");
        let output = directory.join("output");
        let marker = directory.join("runner-called");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&fixture, r#"{"compliant":true}"#).unwrap();

        let result = action_command(&fixture, &directory.join("report.json"), &output)
            .env(name, value)
            .env("A2A_LAB_COMPLIANCE_BIN", &marker)
            .output()
            .unwrap();

        assert_eq!(result.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&result.stderr).contains(diagnostic));
        assert!(!output.exists());
        assert!(!marker.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn action_defaults_to_full_with_llm_check_enabled() {
    assert_action_outcome(true, 0, "full", true);
}

#[test]
fn action_forwards_explicit_basic_selection() {
    assert_action_outcome(true, 0, "basic", true);
}

#[test]
fn action_forwards_explicit_llm_opt_out_without_credentials() {
    assert_action_outcome(true, 0, "full", false);
}

#[test]
fn action_preserves_compliant_report_and_outputs() {
    assert_action_outcome(true, 0, "full", true);
}

#[test]
fn action_preserves_failing_report_outputs_and_exit_status() {
    assert_action_outcome(false, 1, "full", true);
}

#[test]
fn consumer_example_uses_full_sha_and_stable_evidence_semantics() {
    assert!(EXAMPLE.contains("name: A2A-LAB Compliance"));
    assert!(EXAMPLE.contains("if: always()"));
    assert!(EXAMPLE.contains("actions/upload-artifact@v4"));
    assert!(EXAMPLE.contains("/actions/workflows/a2a-lab-compliance.yml/badge.svg?branch=main"));
    assert!(EXAMPLE.contains("not certification"));
    assert!(EXAMPLE.contains("named repository commit"));
    assert!(EXAMPLE.contains("full suite with profile 1.1.0 and LLM checks enabled"));
    assert!(EXAMPLE.contains("suite: full"));
    assert!(EXAMPLE.contains(r#"llm-check: "true""#));

    let action_reference = EXAMPLE
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("uses: A3Analytics/a2a-lab-dev-kit-rs@")
        })
        .expect("consumer action reference");
    assert_eq!(action_reference.len(), 40);
    assert!(
        action_reference
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    );
}

fn assert_action_outcome(compliant: bool, expected_status: i32, suite: &str, llm_check: bool) {
    let suffix = if compliant { "compliant" } else { "failing" };
    let directory = temporary_directory(suffix);
    let fixture = directory.join("fixtures.json");
    let report = directory.join("reports/result.json");
    let output = directory.join("output");
    let arguments = directory.join("arguments");
    let runner = directory.join("runner.sh");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(&fixture, format!(r#"{{"compliant":{compliant}}}"#)).unwrap();
    write_fake_runner(&runner);

    let mut command = action_command(&fixture, &report, &output);
    command
        .env("INPUT_SUITE", suite)
        .env("INPUT_LLM_CHECK", llm_check.to_string())
        .env("A2A_LAB_COMPLIANCE_BIN", &runner)
        .env("ACTION_TEST_ARGUMENTS", &arguments);
    let result = command.output().unwrap();

    assert_eq!(
        result.status.code(),
        Some(expected_status),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        report.exists(),
        "report must remain available after the step"
    );
    assert_report_and_outputs(&report, &output, suite, llm_check, compliant);
    assert_forwarded_arguments(&arguments, suite, llm_check);
    std::fs::remove_dir_all(directory).unwrap();
}

fn assert_report_and_outputs(
    report: &Path,
    output: &Path,
    suite: &str,
    llm_check: bool,
    compliant: bool,
) {
    let report_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(report).unwrap()).unwrap();
    assert_eq!(report_json["suite_version"], "1.1.0");
    assert_eq!(report_json["selected_suite"], suite);
    assert_eq!(
        report_json["enabled_checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check == "a2a_agent_message"),
        suite == "full" && llm_check
    );
    assert_eq!(report_json["compliant"], compliant);
    let outputs = std::fs::read_to_string(output).unwrap();
    assert!(outputs.contains("profile=1.1.0"));
    assert!(outputs.contains(&format!("suite={suite}")));
    assert!(outputs.contains(&format!("llm-check={llm_check}")));
    assert!(outputs.contains(&format!("compliant={compliant}")));
    assert!(outputs.contains(&format!("report-path={}", report.display())));
}

fn assert_forwarded_arguments(arguments: &Path, suite: &str, llm_check: bool) {
    let arguments = std::fs::read_to_string(arguments).unwrap();
    for expected in [
        "--a2a-url",
        "--mcp-url",
        "--fixtures",
        "--profile",
        "--suite",
        "--report",
        "--timeout-milliseconds",
        "--implementation-name",
        "--implementation-version",
    ] {
        assert!(arguments.lines().any(|line| line == expected));
    }
    assert_eq!(argument_value(&arguments, "--suite"), suite);
    assert_eq!(
        arguments.lines().any(|line| line == "--no-llm-check"),
        !llm_check
    );
}

fn action_command(fixtures: &Path, report: &Path, output: &Path) -> Command {
    let mut command = Command::new("bash");
    command
        .arg(runner_path())
        .env("INPUT_A2A_URL", "http://127.0.0.1:3000")
        .env("INPUT_MCP_URL", "http://127.0.0.1:3001/mcp")
        .env("INPUT_FIXTURES", fixtures)
        .env("INPUT_PROFILE", "1.1.0")
        .env("INPUT_SUITE", "full")
        .env("INPUT_LLM_CHECK", "true")
        .env("INPUT_REPORT_PATH", report)
        .env("INPUT_TIMEOUT_MILLISECONDS", "30000")
        .env("INPUT_IMPLEMENTATION_NAME", "fixture-agent")
        .env("INPUT_IMPLEMENTATION_VERSION", "test-revision")
        .env("GITHUB_OUTPUT", output)
        .env("GITHUB_ACTION_PATH", env!("CARGO_MANIFEST_DIR"));
    command
}

fn write_fake_runner(path: &Path) {
    std::fs::write(
        path,
        r#"#!/usr/bin/env bash
set -eu
printf '%s\n' "$@" > "$ACTION_TEST_ARGUMENTS"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --fixtures) fixtures="$2"; shift 2 ;;
    --report) report="$2"; shift 2 ;;
    --suite) suite="$2"; shift 2 ;;
    --no-llm-check) llm_check=false; shift ;;
    *) shift ;;
  esac
done
llm_check="${llm_check:-true}"
python3 - "$fixtures" "$report" "$suite" "$llm_check" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as fixture:
    compliant = json.load(fixture)["compliant"]
enabled_checks = ["basic_operations"]
if sys.argv[3] == "full":
    enabled_checks.append("linked_scenarios")
    if sys.argv[4] == "true":
        enabled_checks.append("a2a_agent_message")
with open(sys.argv[2], "w", encoding="utf-8") as report:
    json.dump({
        "suite_version": "1.1.0",
        "selected_suite": sys.argv[3],
        "enabled_checks": enabled_checks,
        "compliant": compliant,
    }, report)
raise SystemExit(0 if compliant else 1)
PY
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn argument_value<'a>(arguments: &'a str, flag: &str) -> &'a str {
    let mut lines = arguments.lines();
    while let Some(argument) = lines.next() {
        if argument == flag {
            return lines.next().expect("argument value");
        }
    }
    panic!("missing argument {flag}");
}

fn runner_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/actions/a2a-lab-compliance/run.sh")
}

fn temporary_directory(suffix: &str) -> PathBuf {
    std::env::temp_dir().join(format!("a2a-lab-action-{}-{suffix}", std::process::id()))
}
