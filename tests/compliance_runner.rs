use std::sync::{Arc, Mutex};
use std::time::Duration;

use a2a_lab_dev_kit::{
    A2aLabCommand, A2aLabError, A2aLabFuture, A2aLabResult, A2aLabService, A2aServer,
    AgentMessageResponse, COMPLIANCE_PROFILE_VERSION, ComplianceCheck, ComplianceEndpoint,
    ComplianceFixtures, ComplianceOutcome, ComplianceResult, ComplianceRunner,
    ComplianceRunnerConfig, ComplianceSuite, FixtureCapability, GetTaskStatusRequest, Image,
    ImageDescriptor, ImageFixtures, ImageSource, ImplementationIdentity, JsonObject, LogFixtures,
    LogSource, McpServer, MemoryLogs, MemoryMetrics, MemoryTasks, MetricDescriptor, MetricFixtures,
    Page, RunId, TaskDefinition, TaskFixtures, TaskRun, TaskState, TimeRange, UtcTimestamp,
    bind_local, compliance_profile, compliance_suite,
};

fn fixtures() -> ComplianceFixtures {
    ComplianceFixtures {
        suite_version: COMPLIANCE_PROFILE_VERSION.to_owned(),
        range: TimeRange::new(
            timestamp("2026-01-01T00:00:00Z"),
            timestamp("2026-01-02T00:00:00Z"),
        )
        .unwrap(),
        logs: FixtureCapability::Required(LogFixtures {
            source_id: id("logs.primary"),
            alternate_source_id: id("logs.secondary"),
            missing_source_id: id("logs.missing"),
        }),
        metrics: FixtureCapability::Required(MetricFixtures {
            metric_id: id("metrics.primary"),
            alternate_metric_id: id("metrics.secondary"),
            missing_metric_id: id("metrics.missing"),
        }),
        tasks: FixtureCapability::Required(TaskFixtures {
            task_id: id("tasks.primary"),
            alternate_task_id: id("tasks.secondary"),
            run_id: id("runs.primary"),
            missing_task_id: id("tasks.missing"),
            missing_run_id: id("runs.missing"),
            input: JsonObject::parse(r#"{"duration":1}"#).unwrap(),
        }),
        images: FixtureCapability::Required(ImageFixtures {
            source_id: id("images.sources.primary"),
            alternate_source_id: id("images.sources.secondary"),
            image_id: id("images.primary"),
            alternate_image_id: id("images.secondary"),
            missing_source_id: id("images.sources.missing"),
            missing_image_id: id("images.missing"),
        }),
    }
}

fn config(fixtures: ComplianceFixtures) -> ComplianceRunnerConfig {
    config_for(fixtures, ComplianceSuite::Basic)
}

fn config_for(fixtures: ComplianceFixtures, suite: ComplianceSuite) -> ComplianceRunnerConfig {
    ComplianceRunnerConfig {
        a2a_url: "http://a2a.test".to_owned(),
        mcp_url: "http://mcp.test/mcp".to_owned(),
        fixtures,
        implementation: ImplementationIdentity {
            name: "fixture-agent".to_owned(),
            version: "1.0.0".to_owned(),
        },
        case_timeout: Duration::from_millis(50),
        suite,
        agent_message_check: false,
    }
}

fn runner(
    a2a: Arc<dyn ComplianceEndpoint>,
    mcp: Arc<dyn ComplianceEndpoint>,
    fixtures: ComplianceFixtures,
) -> ComplianceRunner {
    ComplianceRunner::new(a2a, mcp, config(fixtures))
}

fn full_runner(
    a2a: Arc<dyn ComplianceEndpoint>,
    mcp: Arc<dyn ComplianceEndpoint>,
    fixtures: ComplianceFixtures,
) -> ComplianceRunner {
    ComplianceRunner::new(a2a, mcp, config_for(fixtures, ComplianceSuite::Full))
}

fn full_llm_runner(
    a2a: Arc<dyn ComplianceEndpoint>,
    mcp: Arc<dyn ComplianceEndpoint>,
) -> ComplianceRunner {
    let mut config = config_for(fixtures(), ComplianceSuite::Full);
    config.agent_message_check = true;
    ComplianceRunner::new(a2a, mcp, config)
}

#[tokio::test]
async fn runs_all_required_cases_in_profile_order() {
    let endpoint: Arc<dyn ComplianceEndpoint> = Arc::new(FixtureEndpoint);
    let result = runner(Arc::clone(&endpoint), endpoint, fixtures())
        .run()
        .await
        .unwrap();

    assert!(
        result.compliant,
        "{:?}",
        result
            .cases
            .iter()
            .filter(|case| case.outcome == ComplianceOutcome::Fail)
            .collect::<Vec<_>>()
    );
    assert_eq!(result.selected_suite, ComplianceSuite::Basic);
    assert_eq!(
        result.enabled_checks,
        [ComplianceCheck::BasicOperations].into_iter().collect()
    );
    assert!(result.scenarios.is_empty());
    assert_eq!(result.cases.len(), compliance_profile().cases.len());
    assert_eq!(
        result
            .cases
            .iter()
            .map(|case| case.case_id.as_str())
            .collect::<Vec<_>>(),
        compliance_profile()
            .cases
            .iter()
            .map(|case| case.id)
            .collect::<Vec<_>>()
    );
    assert!(result.cases.iter().all(|case| {
        if case.required {
            case.outcome == ComplianceOutcome::Pass
        } else {
            case.outcome == ComplianceOutcome::Skip
        }
    }));
}

#[tokio::test]
async fn unavailable_capabilities_skip_only_nonrequired_cases() {
    let fixtures = ComplianceFixtures {
        logs: FixtureCapability::Unavailable,
        metrics: FixtureCapability::Unavailable,
        tasks: FixtureCapability::Unavailable,
        images: FixtureCapability::Unavailable,
        ..fixtures()
    };
    let endpoint: Arc<dyn ComplianceEndpoint> = Arc::new(UnavailableEndpoint);
    let result = runner(Arc::clone(&endpoint), endpoint, fixtures)
        .run()
        .await
        .unwrap();

    assert!(result.compliant);
    assert_eq!(result.cases.iter().filter(|case| case.required).count(), 12);
    assert!(result.cases.iter().all(|case| {
        if case.required {
            case.outcome == ComplianceOutcome::Pass
        } else {
            case.outcome == ComplianceOutcome::Skip
        }
    }));
}

#[tokio::test]
async fn parity_failure_names_case_and_normalized_outputs() {
    let result = runner(
        Arc::new(FixtureEndpoint),
        Arc::new(DifferentEndpoint),
        fixtures(),
    )
    .run()
    .await
    .unwrap();
    let failure = result
        .cases
        .iter()
        .find(|case| case.outcome == ComplianceOutcome::Fail)
        .unwrap();
    let diagnostic = failure.diagnostics.join(" ");
    assert!(diagnostic.contains(&failure.case_id));
    assert!(diagnostic.contains("a2a="));
    assert!(diagnostic.contains("mcp="));
    assert!(!result.compliant);
}

#[tokio::test]
async fn timeout_and_endpoint_failure_are_reported_without_aborting() {
    let timeout = runner(
        Arc::new(PendingEndpoint),
        Arc::new(FixtureEndpoint),
        fixtures(),
    )
    .run()
    .await
    .unwrap();
    assert!(!timeout.compliant);
    assert!(
        timeout.cases[0]
            .diagnostics
            .iter()
            .any(|message| message.contains("timeout"))
    );

    let failed = runner(
        Arc::new(FailedEndpoint),
        Arc::new(FixtureEndpoint),
        fixtures(),
    )
    .run()
    .await
    .unwrap();
    assert!(!failed.compliant);
    assert!(
        failed.cases[0]
            .diagnostics
            .iter()
            .any(|message| message.contains("transport"))
    );
}

#[tokio::test]
async fn compliant_and_noncompliant_reports_validate_after_json_round_trip() {
    let endpoint: Arc<dyn ComplianceEndpoint> = Arc::new(FixtureEndpoint);
    let compliant = runner(Arc::clone(&endpoint), endpoint, fixtures())
        .run()
        .await
        .unwrap();
    let noncompliant = runner(
        Arc::new(FailedEndpoint),
        Arc::new(FixtureEndpoint),
        fixtures(),
    )
    .run()
    .await
    .unwrap();

    for report in [compliant, noncompliant] {
        let json = serde_json::to_string(&report).unwrap();
        ComplianceResult::from_json(&json).unwrap();
    }
}

#[tokio::test]
async fn full_runs_basic_cases_and_all_non_llm_linked_scenarios() {
    let a2a_commands = Arc::new(Mutex::new(Vec::new()));
    let mcp_commands = Arc::new(Mutex::new(Vec::new()));
    let result = full_runner(
        Arc::new(RecordingEndpoint(Arc::clone(&a2a_commands))),
        Arc::new(RecordingEndpoint(Arc::clone(&mcp_commands))),
        fixtures(),
    )
    .run()
    .await
    .unwrap();

    assert!(result.compliant, "{:?}", result.scenarios);
    assert_eq!(result.selected_suite, ComplianceSuite::Full);
    assert_eq!(result.cases.len(), compliance_profile().cases.len());
    assert_eq!(
        result.scenarios.len(),
        compliance_suite(ComplianceSuite::Full).scenarios.len() - 1
    );
    assert!(result.scenarios.iter().all(|scenario| {
        scenario.outcome == ComplianceOutcome::Pass
            && scenario
                .scenario_id
                .split('.')
                .next()
                .is_some_and(|capability| capability != "agent-message")
    }));
    assert_handoff(
        &result,
        "logs.link.a2a-mcp",
        [
            a2a_lab_dev_kit::ComplianceInterface::A2a,
            a2a_lab_dev_kit::ComplianceInterface::Mcp,
        ],
    );
    assert_handoff(
        &result,
        "metrics.link.mcp-a2a",
        [
            a2a_lab_dev_kit::ComplianceInterface::Mcp,
            a2a_lab_dev_kit::ComplianceInterface::A2a,
        ],
    );
    assert_handoff(
        &result,
        "tasks.link.a2a-mcp",
        [
            a2a_lab_dev_kit::ComplianceInterface::A2a,
            a2a_lab_dev_kit::ComplianceInterface::Mcp,
        ],
    );
    assert_handoff(
        &result,
        "images.link.mcp-a2a",
        [
            a2a_lab_dev_kit::ComplianceInterface::Mcp,
            a2a_lab_dev_kit::ComplianceInterface::A2a,
        ],
    );

    let a2a = a2a_commands.lock().unwrap();
    let mcp = mcp_commands.lock().unwrap();
    assert!(has_query_log(&mcp, "logs.discovered"));
    assert!(has_query_metric(&a2a, "metrics.discovered"));
    assert!(has_task_status(&mcp, "runs.started"));
    assert!(has_get_image(&a2a, "images.discovered"));
}

#[tokio::test]
async fn task_scenarios_poll_the_returned_run_until_terminal() {
    let reads = Arc::new(Mutex::new(0));
    let endpoint: Arc<dyn ComplianceEndpoint> = Arc::new(PollingEndpoint(Arc::clone(&reads)));
    let result = full_runner(Arc::clone(&endpoint), endpoint, fixtures())
        .run()
        .await
        .unwrap();

    assert!(result.compliant, "{:?}", result.scenarios);
    assert!(*reads.lock().unwrap() >= 5);
}

#[tokio::test]
async fn empty_discovery_fails_only_affected_scenarios_and_completes_report() {
    let result = full_runner(
        Arc::new(EmptyLogDiscoveryEndpoint),
        Arc::new(RecordingEndpoint(Arc::new(Mutex::new(Vec::new())))),
        fixtures(),
    )
    .run()
    .await
    .unwrap();

    assert!(!result.compliant);
    let failed = result
        .scenarios
        .iter()
        .filter(|scenario| scenario.outcome == ComplianceOutcome::Fail)
        .collect::<Vec<_>>();
    assert_eq!(failed.len(), 2);
    assert!(
        failed
            .iter()
            .all(|scenario| scenario.scenario_id.starts_with("logs."))
    );
    assert!(failed.iter().all(|scenario| {
        scenario.diagnostics.iter().any(|message| {
            message.contains("list_log_sources") && message.contains("no log source")
        })
    }));
    result.validate().unwrap();
}

#[tokio::test]
async fn llm_check_uses_mcp_task_id_and_deterministic_agent_reply() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let a2a: Arc<dyn ComplianceEndpoint> =
        Arc::new(AgentMessageEndpoint::grounded(Arc::clone(&calls)));
    let mcp: Arc<dyn ComplianceEndpoint> = Arc::new(FixtureEndpoint);
    let result = full_llm_runner(a2a, mcp).run().await.unwrap();

    assert!(result.compliant, "{:?}", result.scenarios);
    assert!(
        result
            .enabled_checks
            .contains(&ComplianceCheck::A2aAgentMessage)
    );
    let scenario = result
        .scenarios
        .iter()
        .find(|scenario| scenario.scenario_id == "agent-message.a2a")
        .unwrap();
    assert_eq!(scenario.outcome, ComplianceOutcome::Pass);
    let prompts = calls.lock().unwrap();
    assert_eq!(prompts.len(), 1);
    assert!(prompts[0].contains("tasks.discovered"));
    assert!(prompts[0].contains("task tools"));
}

#[tokio::test]
async fn llm_opt_out_omits_evidence_and_never_contacts_agent_message() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let a2a: Arc<dyn ComplianceEndpoint> =
        Arc::new(AgentMessageEndpoint::grounded(Arc::clone(&calls)));
    let mcp: Arc<dyn ComplianceEndpoint> = Arc::new(FixtureEndpoint);
    let result = full_runner(a2a, mcp, fixtures()).run().await.unwrap();

    assert!(result.compliant);
    assert!(
        !result
            .enabled_checks
            .contains(&ComplianceCheck::A2aAgentMessage)
    );
    assert!(
        result
            .scenarios
            .iter()
            .all(|scenario| scenario.scenario_id != "agent-message.a2a")
    );
    assert!(calls.lock().unwrap().is_empty());

    let mut basic_config = config_for(fixtures(), ComplianceSuite::Basic);
    basic_config.agent_message_check = true;
    let basic = ComplianceRunner::new(
        Arc::new(AgentMessageEndpoint::grounded(Arc::clone(&calls))),
        Arc::new(FixtureEndpoint),
        basic_config,
    )
    .run()
    .await
    .unwrap();
    assert!(basic.compliant);
    assert_eq!(
        basic.enabled_checks,
        [ComplianceCheck::BasicOperations].into_iter().collect()
    );
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn llm_failures_are_actionable_and_do_not_prevent_reports() {
    for (endpoint, expected) in [
        (AgentMessageEndpoint::unadvertised(), "does not advertise"),
        (
            AgentMessageEndpoint::model_failure(),
            "model or tool failed",
        ),
        (AgentMessageEndpoint::ungrounded(), "did not preserve"),
        (AgentMessageEndpoint::broken_context(), "missing context"),
    ] {
        let result = full_llm_runner(Arc::new(endpoint), Arc::new(FixtureEndpoint))
            .run()
            .await
            .unwrap();
        let failure = result
            .scenarios
            .iter()
            .find(|scenario| scenario.scenario_id == "agent-message.a2a")
            .unwrap();
        assert_eq!(failure.outcome, ComplianceOutcome::Fail);
        assert!(
            failure
                .diagnostics
                .iter()
                .any(|message| message.contains(expected)),
            "{:?}",
            failure.diagnostics
        );
        assert!(!result.compliant);
        result.validate().unwrap();
    }

    let tool_failure = full_llm_runner(
        Arc::new(AgentMessageEndpoint::grounded(Arc::new(Mutex::new(
            Vec::new(),
        )))),
        Arc::new(FailedEndpoint),
    )
    .run()
    .await
    .unwrap();
    let failure = tool_failure
        .scenarios
        .iter()
        .find(|scenario| scenario.scenario_id == "agent-message.a2a")
        .unwrap();
    assert!(
        failure.diagnostics[0].contains("list_tasks")
            && failure.diagnostics[0].contains("transport")
    );
}

#[tokio::test]
async fn llm_timeout_is_an_isolated_reported_failure() {
    let result = full_llm_runner(
        Arc::new(PendingAgentMessageEndpoint),
        Arc::new(FixtureEndpoint),
    )
    .run()
    .await
    .unwrap();
    let failure = result
        .scenarios
        .iter()
        .find(|scenario| scenario.scenario_id == "agent-message.a2a")
        .unwrap();
    assert_eq!(failure.outcome, ComplianceOutcome::Fail);
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|message| message.contains("timed out"))
    );
    result.validate().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn command_writes_reports_and_maps_compliance_to_exit_status() {
    let logs = MemoryLogs::new();
    logs.set_unavailable("fixture unavailable").await;
    let metrics = MemoryMetrics::new();
    metrics.set_unavailable("fixture unavailable").await;
    let tasks = MemoryTasks::new();
    tasks.set_unavailable("fixture unavailable").await;
    let service = A2aLabService::new(logs, metrics, tasks).share();
    let (a2a_listener, a2a_address) = bind_local().await.unwrap();
    let (mcp_listener, mcp_address) = bind_local().await.unwrap();
    let a2a_service = Arc::clone(&service);
    let a2a = tokio::spawn(async move { A2aServer::new(&a2a_service).listen(a2a_listener).await });
    let mcp_service = Arc::clone(&service);
    let mcp =
        tokio::spawn(async move { McpServer::new(&mcp_service).serve_http(mcp_listener).await });

    let declaration = ComplianceFixtures {
        logs: FixtureCapability::Unavailable,
        metrics: FixtureCapability::Unavailable,
        tasks: FixtureCapability::Unavailable,
        images: FixtureCapability::Unavailable,
        ..fixtures()
    };
    let fixture_path = temporary_path("fixtures.json");
    let compliant_path = temporary_path("compliant.json");
    let failed_path = temporary_path("failed.json");
    std::fs::write(&fixture_path, serde_json::to_vec(&declaration).unwrap()).unwrap();

    let success = command(
        &a2a_address.to_string(),
        &format!("{mcp_address}/mcp"),
        &fixture_path,
        &compliant_path,
    );
    assert!(success.success());
    let compliant =
        ComplianceResult::from_json(&std::fs::read_to_string(&compliant_path).unwrap()).unwrap();
    assert!(compliant.compliant);
    assert_eq!(compliant.selected_suite, ComplianceSuite::Full);
    assert!(
        !compliant
            .enabled_checks
            .contains(&ComplianceCheck::A2aAgentMessage)
    );

    let llm_path = temporary_path("llm-default.json");
    let llm_status = command_with_default_llm(
        &a2a_address.to_string(),
        &format!("{mcp_address}/mcp"),
        &fixture_path,
        &llm_path,
    );
    assert!(!llm_status.success());
    let llm_report =
        ComplianceResult::from_json(&std::fs::read_to_string(&llm_path).unwrap()).unwrap();
    assert!(
        llm_report
            .enabled_checks
            .contains(&ComplianceCheck::A2aAgentMessage)
    );
    assert!(
        llm_report
            .scenarios
            .iter()
            .any(|scenario| scenario.scenario_id == "agent-message.a2a"
                && scenario.outcome == ComplianceOutcome::Fail)
    );

    let basic_path = temporary_path("basic.json");
    let basic = command_with_suite(
        &a2a_address.to_string(),
        &format!("{mcp_address}/mcp"),
        &fixture_path,
        &basic_path,
        "basic",
    );
    assert!(basic.success());
    let basic =
        ComplianceResult::from_json(&std::fs::read_to_string(&basic_path).unwrap()).unwrap();
    assert_eq!(basic.selected_suite, ComplianceSuite::Basic);
    assert!(basic.scenarios.is_empty());

    let failure = command(
        &a2a_address.to_string(),
        "127.0.0.1:1/mcp",
        &fixture_path,
        &failed_path,
    );
    assert!(!failure.success());
    let failed =
        ComplianceResult::from_json(&std::fs::read_to_string(&failed_path).unwrap()).unwrap();
    assert!(!failed.compliant);

    a2a.abort();
    mcp.abort();
    for path in [
        fixture_path,
        compliant_path,
        llm_path,
        basic_path,
        failed_path,
    ] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn command_rejects_unknown_suite_as_configuration_error() {
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_a2a-lab-compliance"))
        .args(["--suite", "extended"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(2));
}

struct FixtureEndpoint;

impl ComplianceEndpoint for FixtureEndpoint {
    fn execute(
        &self,
        command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabResult, A2aLabError>> {
        Box::pin(async move { fixture_result(command) })
    }
}

enum AgentMessageBehavior {
    Grounded,
    ModelFailure,
    Ungrounded,
    BrokenContext,
}

struct AgentMessageEndpoint {
    advertised: bool,
    behavior: AgentMessageBehavior,
    calls: Arc<Mutex<Vec<String>>>,
}

impl AgentMessageEndpoint {
    fn grounded(calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            advertised: true,
            behavior: AgentMessageBehavior::Grounded,
            calls,
        }
    }

    fn unadvertised() -> Self {
        Self {
            advertised: false,
            behavior: AgentMessageBehavior::Grounded,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn model_failure() -> Self {
        Self {
            advertised: true,
            behavior: AgentMessageBehavior::ModelFailure,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn ungrounded() -> Self {
        Self {
            advertised: true,
            behavior: AgentMessageBehavior::Ungrounded,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn broken_context() -> Self {
        Self {
            advertised: true,
            behavior: AgentMessageBehavior::BrokenContext,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl ComplianceEndpoint for AgentMessageEndpoint {
    fn execute(
        &self,
        command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabResult, A2aLabError>> {
        Box::pin(async move { fixture_result(command) })
    }

    fn agent_message_advertised(&self) -> A2aLabFuture<'_, Result<bool, A2aLabError>> {
        let advertised = self.advertised;
        Box::pin(async move { Ok(advertised) })
    }

    fn agent_message(
        &self,
        text: String,
    ) -> A2aLabFuture<'_, Result<AgentMessageResponse, A2aLabError>> {
        self.calls.lock().unwrap().push(text);
        Box::pin(async move {
            match self.behavior {
                AgentMessageBehavior::Grounded => Ok(AgentMessageResponse {
                    text: "The available task is tasks.discovered.".to_owned(),
                    task_id: Some("a2a-turn-1".to_owned()),
                    context_id: "compliance-context".to_owned(),
                }),
                AgentMessageBehavior::ModelFailure => {
                    Err(A2aLabError::unavailable("model or tool failed"))
                }
                AgentMessageBehavior::Ungrounded => Ok(AgentMessageResponse {
                    text: "A different task is available.".to_owned(),
                    task_id: Some("a2a-turn-1".to_owned()),
                    context_id: "compliance-context".to_owned(),
                }),
                AgentMessageBehavior::BrokenContext => Ok(AgentMessageResponse {
                    text: "The available task is tasks.discovered.".to_owned(),
                    task_id: Some("a2a-turn-1".to_owned()),
                    context_id: String::new(),
                }),
            }
        })
    }
}

struct PendingAgentMessageEndpoint;

impl ComplianceEndpoint for PendingAgentMessageEndpoint {
    fn execute(
        &self,
        command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabResult, A2aLabError>> {
        Box::pin(async move { fixture_result(command) })
    }

    fn agent_message_advertised(&self) -> A2aLabFuture<'_, Result<bool, A2aLabError>> {
        Box::pin(async { Ok(true) })
    }

    fn agent_message(
        &self,
        _text: String,
    ) -> A2aLabFuture<'_, Result<AgentMessageResponse, A2aLabError>> {
        Box::pin(std::future::pending())
    }
}

struct DifferentEndpoint;

impl ComplianceEndpoint for DifferentEndpoint {
    fn execute(
        &self,
        command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabResult, A2aLabError>> {
        Box::pin(async move {
            match command {
                A2aLabCommand::ListLogSources(request) if request.page.limit() != 0 => {
                    Ok(A2aLabResult::ListLogSources(Page::new(
                        Vec::new(),
                        Some("different".to_owned()),
                    )))
                }
                command => fixture_result(command),
            }
        })
    }
}

struct UnavailableEndpoint;

impl ComplianceEndpoint for UnavailableEndpoint {
    fn execute(
        &self,
        _command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabResult, A2aLabError>> {
        Box::pin(async { Err(A2aLabError::unavailable("fixture unavailable")) })
    }
}

struct FailedEndpoint;

impl ComplianceEndpoint for FailedEndpoint {
    fn execute(
        &self,
        _command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabResult, A2aLabError>> {
        Box::pin(async { Err(A2aLabError::transport("endpoint failed")) })
    }
}

struct PendingEndpoint;

impl ComplianceEndpoint for PendingEndpoint {
    fn execute(
        &self,
        _command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabResult, A2aLabError>> {
        Box::pin(std::future::pending())
    }
}

struct RecordingEndpoint(Arc<Mutex<Vec<A2aLabCommand>>>);

impl ComplianceEndpoint for RecordingEndpoint {
    fn execute(
        &self,
        command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabResult, A2aLabError>> {
        self.0.lock().unwrap().push(command.clone());
        Box::pin(async move { fixture_result(command) })
    }
}

struct EmptyLogDiscoveryEndpoint;

impl ComplianceEndpoint for EmptyLogDiscoveryEndpoint {
    fn execute(
        &self,
        command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabResult, A2aLabError>> {
        Box::pin(async move {
            if let A2aLabCommand::ListLogSources(request) = &command {
                request.page.check()?;
                Ok(A2aLabResult::ListLogSources(empty_page(&request.page)))
            } else {
                fixture_result(command)
            }
        })
    }
}

struct PollingEndpoint(Arc<Mutex<usize>>);

impl ComplianceEndpoint for PollingEndpoint {
    fn execute(
        &self,
        command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabResult, A2aLabError>> {
        if let A2aLabCommand::GetTaskStatus(request) = &command
            && request.id.as_str() == "runs.started"
        {
            let mut reads = self.0.lock().unwrap();
            *reads += 1;
            let state = if *reads == 1 {
                TaskState::Working
            } else {
                TaskState::Completed
            };
            let run_id = request.id.clone();
            return Box::pin(async move {
                Ok(A2aLabResult::GetTaskStatus(task_run(
                    run_id,
                    id("tasks.discovered"),
                    JsonObject::empty(),
                    state,
                )))
            });
        }
        Box::pin(async move { fixture_result(command) })
    }
}

fn fixture_result(command: A2aLabCommand) -> Result<A2aLabResult, A2aLabError> {
    match command {
        A2aLabCommand::ListLogSources(request) => {
            request.page.check()?;
            Ok(A2aLabResult::ListLogSources(item_page(
                LogSource {
                    id: id("logs.discovered"),
                    name: "Discovered logs".to_owned(),
                    description: "Linked scenario fixture".to_owned(),
                    asset_id: None,
                    semantic_id: None,
                },
                &request.page,
            )))
        }
        A2aLabCommand::QueryLogs(request) => {
            request.range.check()?;
            missing(request.source_id.as_str())?;
            Ok(A2aLabResult::QueryLogs(Page::new(Vec::new(), None)))
        }
        A2aLabCommand::ListMetrics(request) => {
            request.page.check()?;
            Ok(A2aLabResult::ListMetrics(item_page(
                MetricDescriptor {
                    id: id("metrics.discovered"),
                    name: "Discovered metric".to_owned(),
                    description: "Linked scenario fixture".to_owned(),
                    unit: "count".to_owned(),
                    asset_id: None,
                    semantic_id: None,
                },
                &request.page,
            )))
        }
        A2aLabCommand::QueryMetric(request) => {
            request.range.check()?;
            missing(request.metric_id.as_str())?;
            Ok(A2aLabResult::QueryMetric(Page::new(Vec::new(), None)))
        }
        A2aLabCommand::ListTasks(request) => {
            request.page.check()?;
            Ok(A2aLabResult::ListTasks(item_page(
                TaskDefinition {
                    id: id("tasks.discovered"),
                    name: "Discovered task".to_owned(),
                    description: "Linked scenario fixture".to_owned(),
                    asset_id: None,
                    semantic_id: None,
                    input_schema: None,
                    output_schema: None,
                },
                &request.page,
            )))
        }
        A2aLabCommand::StartTask(request) => {
            missing(request.task_id.as_str())?;
            Ok(A2aLabResult::StartTask(run(
                RunId::new("runs.started").unwrap(),
                request.task_id,
                request.input,
            )))
        }
        A2aLabCommand::GetTaskStatus(GetTaskStatusRequest { id: run_id }) => {
            missing(run_id.as_str())?;
            let task_id = if run_id.as_str() == "runs.started" {
                id("tasks.discovered")
            } else {
                id("tasks.primary")
            };
            Ok(A2aLabResult::GetTaskStatus(run(
                run_id,
                task_id,
                JsonObject::empty(),
            )))
        }
        command @ (A2aLabCommand::ListImageSources(_)
        | A2aLabCommand::ListImages(_)
        | A2aLabCommand::SearchImages(_)
        | A2aLabCommand::GetImage(_)
        | A2aLabCommand::GetCurrentImage(_)) => image_fixture_result(command),
    }
}

fn image_fixture_result(command: A2aLabCommand) -> Result<A2aLabResult, A2aLabError> {
    match command {
        A2aLabCommand::ListImageSources(request) => {
            request.page().check()?;
            Ok(A2aLabResult::ListImageSources(item_page(
                ImageSource {
                    id: id("images.sources.discovered"),
                    name: "Discovered camera".to_owned(),
                    description: "Linked scenario fixture".to_owned(),
                    asset_id: None,
                    semantic_id: None,
                },
                request.page(),
            )))
        }
        A2aLabCommand::ListImages(request) => {
            request.page().check()?;
            missing(request.source_id().as_str())?;
            let descriptor = if request.source_id().as_str() == "images.sources.discovered" {
                image_with("images.discovered", "images.sources.discovered", vec![2])
                    .descriptor()
                    .clone()
            } else {
                image().descriptor().clone()
            };
            Ok(A2aLabResult::ListImages(item_page(
                descriptor,
                request.page(),
            )))
        }
        A2aLabCommand::SearchImages(request) => {
            request.check()?;
            Ok(A2aLabResult::SearchImages(empty_page(request.page())))
        }
        A2aLabCommand::GetImage(request) => {
            missing(request.id().as_str())?;
            let image = if request.id().as_str() == "images.discovered" {
                image_with("images.discovered", "images.sources.discovered", vec![3])
            } else {
                image()
            };
            Ok(A2aLabResult::GetImage(image))
        }
        A2aLabCommand::GetCurrentImage(request) => {
            missing(request.source_id().as_str())?;
            let image = if request.source_id().as_str() == "images.sources.discovered" {
                image_with(
                    "images.current.different",
                    "images.sources.discovered",
                    vec![4],
                )
            } else {
                image()
            };
            Ok(A2aLabResult::GetCurrentImage(image))
        }
        _ => unreachable!("image fixture helper received a non-image command"),
    }
}

fn empty_page<T>(request: &a2a_lab_dev_kit::PageRequest) -> Page<T> {
    Page::new(Vec::new(), (request.limit() == 1).then(|| "1".to_owned()))
}

fn item_page<T>(item: T, request: &a2a_lab_dev_kit::PageRequest) -> Page<T> {
    Page::new(vec![item], (request.limit() == 1).then(|| "1".to_owned()))
}

fn missing(value: &str) -> Result<(), A2aLabError> {
    if value.contains("missing") {
        Err(A2aLabError::not_found("fixture", value))
    } else {
        Ok(())
    }
}

fn run(id: RunId, task_id: a2a_lab_dev_kit::TaskId, input: JsonObject) -> TaskRun {
    task_run(id, task_id, input, TaskState::Completed)
}

fn task_run(
    id: RunId,
    task_id: a2a_lab_dev_kit::TaskId,
    input: JsonObject,
    state: TaskState,
) -> TaskRun {
    TaskRun {
        id,
        task_id,
        state,
        input,
        message: None,
        result: None,
        progress: None,
        error_kind: None,
        error_identifier: None,
    }
}

fn image() -> Image {
    image_with("images.primary", "images.sources.primary", vec![1])
}

fn image_with(image_id: &str, source_id: &str, data: Vec<u8>) -> Image {
    Image::new(
        ImageDescriptor::new(
            id(image_id),
            id(source_id),
            timestamp("2026-01-01T00:00:00Z"),
            "image/png",
            1,
            1,
            None,
            JsonObject::empty(),
        )
        .unwrap(),
        data,
    )
    .unwrap()
}

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).unwrap()
}

fn assert_handoff(
    result: &ComplianceResult,
    scenario_id: &str,
    expected: [a2a_lab_dev_kit::ComplianceInterface; 2],
) {
    let scenario = result
        .scenarios
        .iter()
        .find(|scenario| scenario.scenario_id == scenario_id)
        .unwrap();
    assert_eq!(scenario.handoff_path, expected);
}

fn has_query_log(commands: &[A2aLabCommand], expected: &str) -> bool {
    commands.iter().any(|command| {
        matches!(
            command,
            A2aLabCommand::QueryLogs(request) if request.source_id.as_str() == expected
        )
    })
}

fn has_query_metric(commands: &[A2aLabCommand], expected: &str) -> bool {
    commands.iter().any(|command| {
        matches!(
            command,
            A2aLabCommand::QueryMetric(request) if request.metric_id.as_str() == expected
        )
    })
}

fn has_task_status(commands: &[A2aLabCommand], expected: &str) -> bool {
    commands.iter().any(|command| {
        matches!(
            command,
            A2aLabCommand::GetTaskStatus(request) if request.id.as_str() == expected
        )
    })
}

fn has_get_image(commands: &[A2aLabCommand], expected: &str) -> bool {
    commands.iter().any(|command| {
        matches!(
            command,
            A2aLabCommand::GetImage(request) if request.id().as_str() == expected
        )
    })
}

fn id<T>(value: &str) -> T
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(serde_json::Value::String(value.to_owned())).unwrap()
}

fn command(
    a2a_address: &str,
    mcp_address: &str,
    fixtures: &std::path::Path,
    report: &std::path::Path,
) -> std::process::ExitStatus {
    std::process::Command::new(env!("CARGO_BIN_EXE_a2a-lab-compliance"))
        .args([
            "--a2a-url",
            &format!("http://{a2a_address}"),
            "--mcp-url",
            &format!("http://{mcp_address}"),
            "--fixtures",
            fixtures.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
            "--implementation-name",
            "fixture-agent",
            "--implementation-version",
            "1.0.0",
            "--no-llm-check",
            "--timeout-milliseconds",
            "1000",
        ])
        .status()
        .unwrap()
}

fn command_with_default_llm(
    a2a_address: &str,
    mcp_address: &str,
    fixtures: &std::path::Path,
    report: &std::path::Path,
) -> std::process::ExitStatus {
    std::process::Command::new(env!("CARGO_BIN_EXE_a2a-lab-compliance"))
        .args([
            "--a2a-url",
            &format!("http://{a2a_address}"),
            "--mcp-url",
            &format!("http://{mcp_address}"),
            "--fixtures",
            fixtures.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
            "--implementation-name",
            "fixture-agent",
            "--implementation-version",
            "1.0.0",
            "--timeout-milliseconds",
            "1000",
        ])
        .status()
        .unwrap()
}

fn command_with_suite(
    a2a_address: &str,
    mcp_address: &str,
    fixtures: &std::path::Path,
    report: &std::path::Path,
    suite: &str,
) -> std::process::ExitStatus {
    std::process::Command::new(env!("CARGO_BIN_EXE_a2a-lab-compliance"))
        .args([
            "--a2a-url",
            &format!("http://{a2a_address}"),
            "--mcp-url",
            &format!("http://{mcp_address}"),
            "--fixtures",
            fixtures.to_str().unwrap(),
            "--report",
            report.to_str().unwrap(),
            "--implementation-name",
            "fixture-agent",
            "--implementation-version",
            "1.0.0",
            "--suite",
            suite,
            "--no-llm-check",
            "--timeout-milliseconds",
            "1000",
        ])
        .status()
        .unwrap()
}

fn temporary_path(suffix: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "a2a-lab-compliance-{}-{suffix}",
        std::process::id()
    ))
}
