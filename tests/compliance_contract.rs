use std::collections::BTreeSet;

use a2a_lab_dev_kit::{
    COMPLIANCE_PROFILE_VERSION, COMPLIANCE_RESULT_SCHEMA_ID, ComplianceCaseKind,
    ComplianceCaseResult, ComplianceCheck, ComplianceFixtures, ComplianceInterface,
    ComplianceOperation, ComplianceOutcome, ComplianceResult, ComplianceScenarioAssertion,
    ComplianceScenarioCapability, ComplianceScenarioResult, ComplianceSuite, FixtureCapability,
    ImageFixtures, ImplementationIdentity, JsonObject, LogFixtures, MetricFixtures, TaskFixtures,
    TimeRange, UtcTimestamp, cases_by_operation, compliance_profile, compliance_result_schema,
    compliance_suite,
};
use serde_json::json;

fn interfaces() -> BTreeSet<ComplianceInterface> {
    [ComplianceInterface::A2a, ComplianceInterface::Mcp]
        .into_iter()
        .collect()
}

fn fixtures() -> ComplianceFixtures {
    ComplianceFixtures {
        suite_version: COMPLIANCE_PROFILE_VERSION.to_owned(),
        range: TimeRange::new(
            UtcTimestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            UtcTimestamp::parse("2026-01-02T00:00:00Z").unwrap(),
        )
        .unwrap(),
        logs: FixtureCapability::Required(LogFixtures {
            source_id: a2a_lab_dev_kit::SourceId::new("logs.primary").unwrap(),
            alternate_source_id: a2a_lab_dev_kit::SourceId::new("logs.secondary").unwrap(),
            missing_source_id: a2a_lab_dev_kit::SourceId::new("logs.missing").unwrap(),
        }),
        metrics: FixtureCapability::Required(MetricFixtures {
            metric_id: a2a_lab_dev_kit::MetricId::new("metrics.primary").unwrap(),
            alternate_metric_id: a2a_lab_dev_kit::MetricId::new("metrics.secondary").unwrap(),
            missing_metric_id: a2a_lab_dev_kit::MetricId::new("metrics.missing").unwrap(),
        }),
        tasks: FixtureCapability::Required(TaskFixtures {
            task_id: a2a_lab_dev_kit::TaskId::new("tasks.primary").unwrap(),
            alternate_task_id: a2a_lab_dev_kit::TaskId::new("tasks.secondary").unwrap(),
            run_id: a2a_lab_dev_kit::RunId::new("runs.primary").unwrap(),
            missing_task_id: a2a_lab_dev_kit::TaskId::new("tasks.missing").unwrap(),
            missing_run_id: a2a_lab_dev_kit::RunId::new("runs.missing").unwrap(),
            input: JsonObject::parse(r#"{"duration":1}"#).unwrap(),
        }),
        images: FixtureCapability::Required(ImageFixtures {
            source_id: a2a_lab_dev_kit::ImageSourceId::new("images.sources.primary").unwrap(),
            alternate_source_id: a2a_lab_dev_kit::ImageSourceId::new("images.sources.secondary")
                .unwrap(),
            image_id: a2a_lab_dev_kit::ImageId::new("images.primary").unwrap(),
            alternate_image_id: a2a_lab_dev_kit::ImageId::new("images.secondary").unwrap(),
            missing_source_id: a2a_lab_dev_kit::ImageSourceId::new("images.sources.missing")
                .unwrap(),
            missing_image_id: a2a_lab_dev_kit::ImageId::new("images.missing").unwrap(),
        }),
    }
}

fn compliant_result() -> ComplianceResult {
    ComplianceResult {
        suite_version: COMPLIANCE_PROFILE_VERSION.to_owned(),
        selected_suite: ComplianceSuite::Basic,
        enabled_checks: [ComplianceCheck::BasicOperations].into_iter().collect(),
        implementation: ImplementationIdentity {
            name: "fixture-agent".to_owned(),
            version: "1.2.3".to_owned(),
        },
        tested_interfaces: interfaces(),
        cases: compliance_profile()
            .cases
            .iter()
            .map(|case| ComplianceCaseResult {
                case_id: case.id.to_owned(),
                required: true,
                interfaces: interfaces(),
                outcome: ComplianceOutcome::Pass,
                diagnostics: Vec::new(),
            })
            .collect(),
        scenarios: Vec::new(),
        compliant: true,
    }
}

fn compliant_full_result(agent_message: bool) -> ComplianceResult {
    let mut result = compliant_result();
    result.selected_suite = ComplianceSuite::Full;
    result
        .enabled_checks
        .insert(ComplianceCheck::LinkedScenarios);
    if agent_message {
        result
            .enabled_checks
            .insert(ComplianceCheck::A2aAgentMessage);
    }
    result.scenarios = compliance_suite(ComplianceSuite::Full)
        .scenarios
        .iter()
        .filter(|scenario| {
            scenario.capability != ComplianceScenarioCapability::A2aAgentMessage || agent_message
        })
        .map(|scenario| ComplianceScenarioResult {
            scenario_id: scenario.id.to_owned(),
            required: true,
            interfaces: [scenario.producer, scenario.consumer].into_iter().collect(),
            handoff_path: vec![scenario.producer, scenario.consumer],
            outcome: ComplianceOutcome::Pass,
            diagnostics: Vec::new(),
        })
        .collect();
    result
}

#[test]
fn profile_covers_every_operation_and_behavior_family() {
    let profile = compliance_profile();
    assert_eq!(profile.version, COMPLIANCE_PROFILE_VERSION);
    assert_eq!(profile.operations.len(), 12);
    assert!(profile.operations.iter().all(|operation| {
        operation.interfaces == [ComplianceInterface::A2a, ComplianceInterface::Mcp]
            && operation.unavailable_error_code == "unavailable"
            && !operation.observable_fields.is_empty()
    }));

    let grouped = cases_by_operation();
    for operation in [
        ComplianceOperation::ListLogSources,
        ComplianceOperation::QueryLogs,
        ComplianceOperation::ListMetrics,
        ComplianceOperation::QueryMetric,
        ComplianceOperation::ListTasks,
        ComplianceOperation::StartTask,
        ComplianceOperation::GetTaskStatus,
        ComplianceOperation::ListImageSources,
        ComplianceOperation::ListImages,
        ComplianceOperation::SearchImages,
        ComplianceOperation::GetImage,
        ComplianceOperation::GetCurrentImage,
    ] {
        let cases = grouped.get(&operation).unwrap();
        assert!(
            cases
                .iter()
                .any(|case| case.kind == ComplianceCaseKind::Success)
        );
        assert!(cases.iter().any(|case| {
            matches!(
                case.kind,
                ComplianceCaseKind::Boundary
                    | ComplianceCaseKind::Invalid
                    | ComplianceCaseKind::NotFound
            )
        }));
        assert!(
            cases
                .iter()
                .any(|case| case.kind == ComplianceCaseKind::Unavailable)
        );
    }
}

#[test]
fn profile_exposes_basic_and_strictly_larger_full_suite() {
    assert_eq!(COMPLIANCE_PROFILE_VERSION, "1.1.0");
    let profile = compliance_profile();
    assert_eq!(
        profile
            .suites
            .iter()
            .map(|suite| suite.name)
            .collect::<Vec<_>>(),
        [ComplianceSuite::Basic, ComplianceSuite::Full]
    );
    let basic = compliance_suite(ComplianceSuite::Basic);
    let full = compliance_suite(ComplianceSuite::Full);
    assert_eq!(basic.cases, full.cases);
    assert_eq!(basic.cases.len(), 47);
    assert_eq!(basic.cases[0].id, "logs.list.success");
    assert_eq!(basic.cases[46].id, "images.current.unavailable");
    assert!(basic.scenarios.is_empty());
    assert!(!full.scenarios.is_empty());
}

#[test]
fn full_suite_defines_bounded_linked_interface_matrix() {
    let full = compliance_suite(ComplianceSuite::Full);
    for capability in [
        ComplianceScenarioCapability::Logs,
        ComplianceScenarioCapability::Metrics,
        ComplianceScenarioCapability::Tasks,
        ComplianceScenarioCapability::Images,
    ] {
        let scenarios: Vec<_> = full
            .scenarios
            .iter()
            .filter(|scenario| scenario.capability == capability)
            .collect();
        assert_eq!(scenarios.len(), 4);
        let paths: BTreeSet<_> = scenarios
            .iter()
            .map(|scenario| (scenario.producer, scenario.consumer))
            .collect();
        assert_eq!(
            paths,
            [
                (ComplianceInterface::A2a, ComplianceInterface::A2a),
                (ComplianceInterface::A2a, ComplianceInterface::Mcp),
                (ComplianceInterface::Mcp, ComplianceInterface::A2a),
                (ComplianceInterface::Mcp, ComplianceInterface::Mcp),
            ]
            .into_iter()
            .collect()
        );
        assert!(scenarios.iter().all(|scenario| {
            scenario
                .assertions
                .contains(&ComplianceScenarioAssertion::MeaningfulLinkage)
                && scenario
                    .assertions
                    .contains(&ComplianceScenarioAssertion::CompatibleIdentifier)
                && scenario
                    .assertions
                    .contains(&ComplianceScenarioAssertion::SuccessfulAvailability)
        }));
    }

    let tasks = full
        .scenarios
        .iter()
        .find(|scenario| scenario.capability == ComplianceScenarioCapability::Tasks)
        .unwrap();
    assert_eq!(tasks.steps, ["list_tasks", "start_task", "get_task_status"]);
    assert!(
        tasks
            .assertions
            .contains(&ComplianceScenarioAssertion::TaskTerminalProgress)
    );
    let images = full
        .scenarios
        .iter()
        .find(|scenario| scenario.capability == ComplianceScenarioCapability::Images)
        .unwrap();
    assert_eq!(
        images.steps,
        [
            "list_image_sources",
            "list_or_search_images",
            "get_image",
            "get_current_image"
        ]
    );
}

#[test]
fn agent_message_is_the_only_optional_llm_surface() {
    let full = compliance_suite(ComplianceSuite::Full);
    let llm: Vec<_> = full
        .scenarios
        .iter()
        .filter(|scenario| scenario.capability == ComplianceScenarioCapability::A2aAgentMessage)
        .collect();
    assert_eq!(llm.len(), 1);
    assert_eq!(llm[0].id, "agent-message.a2a");
    assert_eq!(llm[0].producer, ComplianceInterface::A2a);
    assert_eq!(llm[0].consumer, ComplianceInterface::A2a);
    assert!(
        full.scenarios
            .iter()
            .all(|scenario| !scenario.id.contains("mcp-agent-message"))
    );
}

#[test]
fn fixture_declaration_round_trips_and_rejects_duplicate_ids() {
    let fixtures = fixtures();
    fixtures.validate().unwrap();
    let encoded = serde_json::to_string(&fixtures).unwrap();
    assert_eq!(ComplianceFixtures::from_json(&encoded).unwrap(), fixtures);

    let mut malformed = fixtures;
    let FixtureCapability::Required(logs) = &mut malformed.logs else {
        unreachable!()
    };
    logs.alternate_source_id = logs.source_id.clone();
    let errors = malformed.validate().unwrap_err();
    assert_eq!(errors.errors()[0].field, "logs");
}

#[test]
fn malformed_fixture_json_reports_the_field() {
    let error = ComplianceFixtures::from_json(r#"{"suite_version":"1.0.0"}"#).unwrap_err();
    assert_eq!(error.errors()[0].field, "range");
}

#[test]
fn result_schema_is_versioned_and_result_round_trips() {
    let schema = serde_json::to_value(compliance_result_schema()).unwrap();
    assert_eq!(schema["$id"], COMPLIANCE_RESULT_SCHEMA_ID);
    assert_eq!(schema["title"], "ComplianceResult");

    let result = compliant_result();
    result.validate().unwrap();
    let encoded = serde_json::to_string(&result).unwrap();
    assert_eq!(ComplianceResult::from_json(&encoded).unwrap(), result);
}

#[test]
fn selected_suite_checks_and_scenario_evidence_round_trip() {
    assert_eq!(ComplianceSuite::default(), ComplianceSuite::Basic);
    for result in [
        compliant_result(),
        compliant_full_result(false),
        compliant_full_result(true),
    ] {
        result.validate().unwrap();
        let encoded = serde_json::to_string(&result).unwrap();
        assert_eq!(ComplianceResult::from_json(&encoded).unwrap(), result);
    }
}

#[test]
fn full_result_rejects_missing_or_failed_enabled_scenarios() {
    let mut missing = compliant_full_result(false);
    missing.scenarios.remove(0);
    let errors = missing.validate().unwrap_err();
    assert!(errors.errors().iter().any(|error| {
        error.field == "scenarios" && error.message.contains("missing enabled scenario")
    }));

    let mut failed = compliant_full_result(false);
    failed.scenarios[0].outcome = ComplianceOutcome::Fail;
    failed.scenarios[0].diagnostics = vec!["consumer rejected producer identifier".to_owned()];
    let errors = failed.validate().unwrap_err();
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| error.field == "scenarios[0].outcome")
    );
}

#[test]
fn enabled_agent_message_requires_a2a_only_evidence() {
    let mut result = compliant_full_result(false);
    result
        .enabled_checks
        .insert(ComplianceCheck::A2aAgentMessage);
    let errors = result.validate().unwrap_err();
    assert!(errors.errors().iter().any(|error| {
        error.field == "scenarios" && error.message.contains("agent-message.a2a")
    }));
}

#[test]
fn skipped_required_case_cannot_be_compliant() {
    let mut result = compliant_result();
    result.cases[0].outcome = ComplianceOutcome::Skip;
    result.cases[0].diagnostics = vec!["endpoint did not expose fixture".to_owned()];
    let errors = result.validate().unwrap_err();
    assert!(
        errors
            .errors()
            .iter()
            .any(|error| error.field == "cases[0].outcome")
    );
}

#[test]
fn malformed_results_report_specific_fields() {
    let malformed = json!({
        "suite_version": "0.0.0",
        "selected_suite": "basic",
        "enabled_checks": ["basic_operations"],
        "implementation": {"name": "", "version": "1"},
        "tested_interfaces": ["a2a"],
        "cases": [],
        "scenarios": [],
        "compliant": true
    });
    let errors = ComplianceResult::from_json(&malformed.to_string()).unwrap_err();
    let fields: BTreeSet<_> = errors
        .errors()
        .iter()
        .map(|error| error.field.as_str())
        .collect();
    assert!(fields.contains("suite_version"));
    assert!(fields.contains("implementation.name"));
    assert!(fields.contains("tested_interfaces"));
    assert!(fields.contains("cases"));
    assert!(fields.contains("compliant"));
}
