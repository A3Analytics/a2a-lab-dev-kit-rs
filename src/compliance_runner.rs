//! Execution of the versioned A2A-LAB compliance contract.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::time::Instant;

use crate::a2a::{A2aClient, AgentMessageResponse};
use crate::compliance::{
    ComplianceCase, ComplianceCaseKind, ComplianceCaseResult, ComplianceCheck, ComplianceFixtures,
    ComplianceInterface, ComplianceOperation, ComplianceOutcome, ComplianceResult,
    ComplianceScenario, ComplianceScenarioCapability, ComplianceScenarioResult, ComplianceSuite,
    FixtureCapability, ImplementationIdentity, compliance_suite,
};
use crate::error::A2aLabError;
use crate::images::{
    GetCurrentImageRequest, GetImageRequest, ListImageSourcesRequest, ListImagesRequest,
    SearchImagesRequest, TckMalformedImageRequests,
};
use crate::logs::{ListLogSourcesRequest, QueryLogsRequest};
use crate::mcp::McpLab;
use crate::metrics::{ListMetricsRequest, QueryMetricRequest};
use crate::page::PageRequest;
use crate::service::{A2aLabApi, A2aLabCommand, A2aLabFuture, A2aLabResult};
use crate::tasks::{GetTaskStatusRequest, ListTasksRequest, StartTaskRequest};
use crate::{ImageId, ImageSourceId, JsonObject, MetricId, RunId, SourceId, TaskId, TimeRange};

/// Configuration shared by the library runner and command.
#[derive(Debug, Clone)]
pub struct ComplianceRunnerConfig {
    /// A2A agent origin.
    pub a2a_url: String,
    /// MCP Streamable HTTP endpoint.
    pub mcp_url: String,
    /// Deterministic implementation fixtures.
    pub fixtures: ComplianceFixtures,
    /// Identity written to the result document.
    pub implementation: ImplementationIdentity,
    /// Maximum time allowed for one interface call.
    pub case_timeout: Duration,
    /// Named profile suite to execute.
    pub suite: ComplianceSuite,
    /// Whether full runs execute the optional LLM-backed agent-message check.
    pub agent_message_check: bool,
}

impl ComplianceRunnerConfig {
    /// Validates configuration before a run starts.
    pub fn validate(&self) -> Result<(), ComplianceRunError> {
        self.fixtures
            .validate()
            .map_err(|error| ComplianceRunError::Configuration(error.to_string()))?;
        check_url("a2a_url", &self.a2a_url)?;
        check_url("mcp_url", &self.mcp_url)?;
        if self.implementation.name.trim().is_empty() {
            return Err(ComplianceRunError::Configuration(
                "implementation.name must be nonblank".to_owned(),
            ));
        }
        if self.implementation.version.trim().is_empty() {
            return Err(ComplianceRunError::Configuration(
                "implementation.version must be nonblank".to_owned(),
            ));
        }
        if self.case_timeout.is_zero() {
            return Err(ComplianceRunError::Configuration(
                "case_timeout must be greater than zero".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Failure that prevents a compliance run from starting or producing a valid report.
#[derive(Debug, thiserror::Error)]
pub enum ComplianceRunError {
    /// Caller-supplied configuration is invalid.
    #[error("invalid compliance configuration: {0}")]
    Configuration(String),
    /// A generated report violated the versioned result contract.
    #[error("invalid generated compliance report: {0}")]
    Report(String),
}

/// Interface adapter used by the reusable runner.
pub trait ComplianceEndpoint: Send + Sync {
    /// Executes one typed A2A-LAB command.
    fn execute(
        &self,
        command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabResult, A2aLabError>>;

    /// Reports whether the A2A Agent Card advertises `agent-message`.
    fn agent_message_advertised(&self) -> A2aLabFuture<'_, Result<bool, A2aLabError>> {
        Box::pin(async {
            Err(A2aLabError::unavailable(
                "endpoint does not support A2A Agent Card inspection",
            ))
        })
    }

    /// Sends one plain-text request through the existing A2A agent-message endpoint.
    fn agent_message(
        &self,
        _text: String,
    ) -> A2aLabFuture<'_, Result<AgentMessageResponse, A2aLabError>> {
        Box::pin(async {
            Err(A2aLabError::unavailable(
                "endpoint does not support A2A agent messages",
            ))
        })
    }
}

/// Runs the selected profile against configured A2A and MCP URLs.
pub async fn run_compliance(
    config: ComplianceRunnerConfig,
) -> Result<ComplianceResult, ComplianceRunError> {
    config.validate()?;
    let a2a: Arc<dyn ComplianceEndpoint> = match A2aClient::new(&config.a2a_url) {
        Ok(client) => Arc::new(A2aEndpoint(client)),
        Err(error) => Arc::new(FailedEndpoint(error)),
    };
    let mcp: Arc<dyn ComplianceEndpoint> =
        match tokio::time::timeout(config.case_timeout, McpLab::connect(&config.mcp_url)).await {
            Ok(Ok(client)) => Arc::new(McpEndpoint(client)),
            Ok(Err(error)) => Arc::new(FailedEndpoint(error)),
            Err(_) => Arc::new(FailedEndpoint(A2aLabError::transport(format!(
                "mcp connection timed out after {} ms",
                config.case_timeout.as_millis()
            )))),
        };
    ComplianceRunner::new(a2a, mcp, config).run().await
}

/// Reusable contract runner over caller-supplied interface adapters.
pub struct ComplianceRunner {
    a2a: Arc<dyn ComplianceEndpoint>,
    mcp: Arc<dyn ComplianceEndpoint>,
    config: ComplianceRunnerConfig,
}

impl ComplianceRunner {
    /// Creates a runner. [`Self::run`] validates the configuration.
    #[must_use]
    pub fn new(
        a2a: Arc<dyn ComplianceEndpoint>,
        mcp: Arc<dyn ComplianceEndpoint>,
        config: ComplianceRunnerConfig,
    ) -> Self {
        Self { a2a, mcp, config }
    }

    /// Executes the selected suite in stable profile order and returns a validated report.
    pub async fn run(&self) -> Result<ComplianceResult, ComplianceRunError> {
        self.config.validate()?;
        let suite = compliance_suite(self.config.suite);
        let mut cases = Vec::with_capacity(suite.cases.len());
        for case in suite.cases {
            cases.push(self.run_case(case).await);
        }
        let scenarios = if self.config.suite == ComplianceSuite::Full {
            let definitions = suite.scenarios.iter().filter(|scenario| {
                scenario.capability != ComplianceScenarioCapability::A2aAgentMessage
                    || self.config.agent_message_check
            });
            let mut results = Vec::new();
            for scenario in definitions {
                results.push(self.run_scenario(scenario).await);
            }
            results
        } else {
            Vec::new()
        };
        let compliant = cases
            .iter()
            .filter(|case| case.required)
            .all(|case| case.outcome == ComplianceOutcome::Pass)
            && scenarios
                .iter()
                .filter(|scenario| scenario.required)
                .all(|scenario| scenario.outcome == ComplianceOutcome::Pass);
        let mut enabled_checks = BTreeSet::from([ComplianceCheck::BasicOperations]);
        if self.config.suite == ComplianceSuite::Full {
            enabled_checks.insert(ComplianceCheck::LinkedScenarios);
            if self.config.agent_message_check {
                enabled_checks.insert(ComplianceCheck::A2aAgentMessage);
            }
        }
        let result = ComplianceResult {
            suite_version: self.config.fixtures.suite_version.clone(),
            selected_suite: self.config.suite,
            enabled_checks,
            implementation: self.config.implementation.clone(),
            tested_interfaces: both_interfaces(),
            cases,
            scenarios,
            compliant,
        };
        result
            .validate()
            .map_err(|error| ComplianceRunError::Report(error.to_string()))?;
        Ok(result)
    }

    async fn run_case(&self, case: &ComplianceCase) -> ComplianceCaseResult {
        let required = case_required(case, &self.config.fixtures);
        if !required {
            return case_result(
                case,
                false,
                ComplianceOutcome::Skip,
                vec!["case is not required by the fixture availability declaration".to_owned()],
            );
        }

        let command = command_for(case, &self.config.fixtures);
        let (a2a, mcp) = tokio::join!(
            execute_with_timeout(&*self.a2a, command.clone(), self.config.case_timeout),
            execute_with_timeout(&*self.mcp, command, self.config.case_timeout)
        );
        let expected = expected_for(case);
        let mut diagnostics = Vec::new();
        check_expected("a2a", &a2a, expected, &mut diagnostics);
        check_expected("mcp", &mcp, expected, &mut diagnostics);
        if a2a != mcp {
            diagnostics.push(format!(
                "{}: normalized interface difference: a2a={} mcp={}",
                case.id,
                normalized_json(&a2a),
                normalized_json(&mcp)
            ));
        }
        let outcome = if diagnostics.is_empty() {
            ComplianceOutcome::Pass
        } else {
            ComplianceOutcome::Fail
        };
        case_result(case, true, outcome, diagnostics)
    }

    async fn run_scenario(&self, scenario: &ComplianceScenario) -> ComplianceScenarioResult {
        let required = scenario_required(scenario.capability, &self.config.fixtures);
        let diagnostics = if required {
            self.execute_scenario(scenario)
                .await
                .err()
                .into_iter()
                .collect()
        } else {
            vec!["scenario is not required by the fixture availability declaration".to_owned()]
        };
        ComplianceScenarioResult {
            scenario_id: scenario.id.to_owned(),
            required,
            interfaces: [scenario.producer, scenario.consumer].into_iter().collect(),
            handoff_path: vec![scenario.producer, scenario.consumer],
            outcome: if required && diagnostics.is_empty() {
                ComplianceOutcome::Pass
            } else if required {
                ComplianceOutcome::Fail
            } else {
                ComplianceOutcome::Skip
            },
            diagnostics,
        }
    }

    async fn execute_scenario(&self, scenario: &ComplianceScenario) -> Result<(), String> {
        match scenario.capability {
            ComplianceScenarioCapability::Logs => self.run_logs_scenario(scenario).await,
            ComplianceScenarioCapability::Metrics => self.run_metrics_scenario(scenario).await,
            ComplianceScenarioCapability::Tasks => self.run_tasks_scenario(scenario).await,
            ComplianceScenarioCapability::Images => self.run_images_scenario(scenario).await,
            ComplianceScenarioCapability::A2aAgentMessage => {
                self.run_agent_message_scenario().await
            }
        }
    }

    async fn run_agent_message_scenario(&self) -> Result<(), String> {
        let advertised = match tokio::time::timeout(
            self.config.case_timeout,
            self.a2a.agent_message_advertised(),
        )
        .await
        {
            Ok(Ok(advertised)) => advertised,
            Ok(Err(error)) => {
                return Err(format!(
                    "agent_card: A2a returned lab error `{}`: {error}",
                    error.code()
                ));
            }
            Err(_) => {
                return Err(format!(
                    "agent_card: A2a timed out after {} ms",
                    self.config.case_timeout.as_millis()
                ));
            }
        };
        if !advertised {
            return Err("agent_card: A2A Agent Card does not advertise `agent-message`".to_owned());
        }

        let listed = self
            .step(
                ComplianceInterface::Mcp,
                "list_tasks",
                A2aLabCommand::ListTasks(ListTasksRequest {
                    page: PageRequest::default(),
                }),
            )
            .await?;
        let A2aLabResult::ListTasks(page) = listed else {
            return Err("list_tasks: MCP returned an incompatible result".to_owned());
        };
        let task_id = page
            .items()
            .first()
            .ok_or_else(|| "list_tasks: MCP discovery returned no task identifier".to_owned())?
            .id
            .clone();
        let prompt = format!(
            "Use your lab task tools to inspect task `{task_id}`. Reply with the exact task identifier `{task_id}`."
        );
        let response =
            match tokio::time::timeout(self.config.case_timeout, self.a2a.agent_message(prompt))
                .await
            {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => {
                    return Err(format!(
                        "a2a_agent_message: A2a returned lab error `{}`: {error}",
                        error.code()
                    ));
                }
                Err(_) => {
                    return Err(format!(
                        "a2a_agent_message: A2a timed out after {} ms",
                        self.config.case_timeout.as_millis()
                    ));
                }
            };
        if response.context_id.trim().is_empty() {
            return Err("a2a_agent_message: reply is missing context data".to_owned());
        }
        if response
            .task_id
            .as_ref()
            .is_some_and(|id| id.trim().is_empty())
        {
            return Err("a2a_agent_message: reply contains a blank protocol task id".to_owned());
        }
        if !response.text.contains(task_id.as_str()) {
            return Err(format!(
                "a2a_agent_message: reply did not preserve MCP task identifier `{task_id}`"
            ));
        }
        Ok(())
    }

    async fn run_logs_scenario(&self, scenario: &ComplianceScenario) -> Result<(), String> {
        let listed = self
            .step(
                scenario.producer,
                "list_log_sources",
                A2aLabCommand::ListLogSources(ListLogSourcesRequest {
                    page: PageRequest::default(),
                }),
            )
            .await?;
        let A2aLabResult::ListLogSources(page) = listed else {
            return Err("list_log_sources: returned an incompatible result".to_owned());
        };
        let source_id = page
            .items()
            .first()
            .ok_or_else(|| "list_log_sources: discovery returned no log source".to_owned())?
            .id
            .clone();
        let queried = self
            .step(
                scenario.consumer,
                "query_logs",
                A2aLabCommand::QueryLogs(QueryLogsRequest {
                    source_id,
                    range: self.config.fixtures.range,
                    page: PageRequest::default(),
                }),
            )
            .await?;
        if matches!(queried, A2aLabResult::QueryLogs(_)) {
            Ok(())
        } else {
            Err("query_logs: returned an incompatible result".to_owned())
        }
    }

    async fn run_metrics_scenario(&self, scenario: &ComplianceScenario) -> Result<(), String> {
        let listed = self
            .step(
                scenario.producer,
                "list_metrics",
                A2aLabCommand::ListMetrics(ListMetricsRequest {
                    page: PageRequest::default(),
                }),
            )
            .await?;
        let A2aLabResult::ListMetrics(page) = listed else {
            return Err("list_metrics: returned an incompatible result".to_owned());
        };
        let metric_id = page
            .items()
            .first()
            .ok_or_else(|| "list_metrics: discovery returned no metric".to_owned())?
            .id
            .clone();
        let queried = self
            .step(
                scenario.consumer,
                "query_metric",
                A2aLabCommand::QueryMetric(QueryMetricRequest {
                    metric_id,
                    range: self.config.fixtures.range,
                    page: PageRequest::default(),
                }),
            )
            .await?;
        if matches!(queried, A2aLabResult::QueryMetric(_)) {
            Ok(())
        } else {
            Err("query_metric: returned an incompatible result".to_owned())
        }
    }

    async fn run_tasks_scenario(&self, scenario: &ComplianceScenario) -> Result<(), String> {
        let listed = self
            .step(
                scenario.producer,
                "list_tasks",
                A2aLabCommand::ListTasks(ListTasksRequest {
                    page: PageRequest::default(),
                }),
            )
            .await?;
        let A2aLabResult::ListTasks(page) = listed else {
            return Err("list_tasks: returned an incompatible result".to_owned());
        };
        let task_id = page
            .items()
            .first()
            .ok_or_else(|| "list_tasks: discovery returned no task".to_owned())?
            .id
            .clone();
        let input = match &self.config.fixtures.tasks {
            FixtureCapability::Required(fixtures) => fixtures.input.clone(),
            FixtureCapability::Unavailable => JsonObject::empty(),
        };
        let started = self
            .step(
                scenario.producer,
                "start_task",
                A2aLabCommand::StartTask(StartTaskRequest::new(task_id.clone(), input).immediate()),
            )
            .await?;
        let A2aLabResult::StartTask(run) = started else {
            return Err("start_task: returned an incompatible result".to_owned());
        };
        if run.task_id != task_id {
            return Err(
                "start_task: returned run references a different task identifier".to_owned(),
            );
        }
        let run_id = run.id;
        let deadline = Instant::now() + self.config.case_timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!(
                    "get_task_status: run `{run_id}` did not reach a terminal state within {} ms",
                    self.config.case_timeout.as_millis()
                ));
            }
            let status = self
                .step_with_timeout(
                    scenario.consumer,
                    "get_task_status",
                    A2aLabCommand::GetTaskStatus(GetTaskStatusRequest { id: run_id.clone() }),
                    remaining,
                )
                .await?;
            let A2aLabResult::GetTaskStatus(status) = status else {
                return Err("get_task_status: returned an incompatible result".to_owned());
            };
            if status.id != run_id {
                return Err(
                    "get_task_status: returned a different run identifier than requested"
                        .to_owned(),
                );
            }
            if status.task_id != task_id {
                return Err(
                    "get_task_status: returned run references a different task identifier"
                        .to_owned(),
                );
            }
            if status.state.is_terminal() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10).min(remaining)).await;
        }
    }

    async fn run_images_scenario(&self, scenario: &ComplianceScenario) -> Result<(), String> {
        let listed_sources = self
            .step(
                scenario.producer,
                "list_image_sources",
                A2aLabCommand::ListImageSources(
                    ListImageSourcesRequest::new(PageRequest::default())
                        .expect("default page is valid"),
                ),
            )
            .await?;
        let A2aLabResult::ListImageSources(page) = listed_sources else {
            return Err("list_image_sources: returned an incompatible result".to_owned());
        };
        let source_id = page
            .items()
            .first()
            .ok_or_else(|| "list_image_sources: discovery returned no image source".to_owned())?
            .id
            .clone();
        let listed_images = self
            .step(
                scenario.producer,
                "list_images",
                A2aLabCommand::ListImages(
                    ListImagesRequest::new(source_id.clone(), PageRequest::default())
                        .expect("default page is valid"),
                ),
            )
            .await?;
        let A2aLabResult::ListImages(page) = listed_images else {
            return Err("list_images: returned an incompatible result".to_owned());
        };
        let descriptor = page
            .items()
            .first()
            .ok_or_else(|| "list_images: discovery returned no image identifier".to_owned())?;
        if descriptor.source_id() != &source_id {
            return Err("list_images: returned image references a different source".to_owned());
        }
        let image_id = descriptor.id().clone();
        let image = self
            .step(
                scenario.consumer,
                "get_image",
                A2aLabCommand::GetImage(GetImageRequest::new(image_id.clone())),
            )
            .await?;
        let A2aLabResult::GetImage(image) = image else {
            return Err("get_image: returned an incompatible result".to_owned());
        };
        if image.descriptor().id() != &image_id {
            return Err(
                "get_image: returned a different image identifier than requested".to_owned(),
            );
        }
        if image.descriptor().source_id() != &source_id {
            return Err("get_image: returned image references a different source".to_owned());
        }
        let current = self
            .step(
                scenario.consumer,
                "get_current_image",
                A2aLabCommand::GetCurrentImage(GetCurrentImageRequest::new(source_id.clone())),
            )
            .await?;
        let A2aLabResult::GetCurrentImage(current) = current else {
            return Err("get_current_image: returned an incompatible result".to_owned());
        };
        if current.descriptor().source_id() == &source_id {
            Ok(())
        } else {
            Err("get_current_image: returned image references a different source".to_owned())
        }
    }

    async fn step(
        &self,
        interface: ComplianceInterface,
        name: &str,
        command: A2aLabCommand,
    ) -> Result<A2aLabResult, String> {
        self.step_with_timeout(interface, name, command, self.config.case_timeout)
            .await
    }

    async fn step_with_timeout(
        &self,
        interface: ComplianceInterface,
        name: &str,
        command: A2aLabCommand,
        timeout: Duration,
    ) -> Result<A2aLabResult, String> {
        let endpoint = match interface {
            ComplianceInterface::A2a => &*self.a2a,
            ComplianceInterface::Mcp => &*self.mcp,
        };
        match tokio::time::timeout(timeout, endpoint.execute(command)).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(error)) => Err(format!(
                "{name}: {interface:?} returned lab error `{}`: {error}",
                error.code()
            )),
            Err(_) => Err(format!(
                "{name}: {interface:?} timed out after {} ms",
                timeout.as_millis()
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum NormalizedOutcome {
    Result { value: serde_json::Value },
    Error { code: String },
    Timeout { milliseconds: u128 },
}

#[derive(Debug, Clone, Copy)]
enum Expected {
    Result,
    PaginatedResult,
    Error(&'static str),
}

async fn execute_with_timeout(
    endpoint: &dyn ComplianceEndpoint,
    command: A2aLabCommand,
    timeout: Duration,
) -> NormalizedOutcome {
    match tokio::time::timeout(timeout, endpoint.execute(command)).await {
        Ok(Ok(result)) => match serde_json::to_value(result) {
            Ok(value) => NormalizedOutcome::Result { value },
            Err(_) => NormalizedOutcome::Error {
                code: "protocol".to_owned(),
            },
        },
        Ok(Err(error)) => NormalizedOutcome::Error {
            code: error.code().to_owned(),
        },
        Err(_) => NormalizedOutcome::Timeout {
            milliseconds: timeout.as_millis(),
        },
    }
}

fn check_expected(
    interface: &str,
    actual: &NormalizedOutcome,
    expected: Expected,
    diagnostics: &mut Vec<String>,
) {
    let matches = match (expected, actual) {
        (Expected::Result, NormalizedOutcome::Result { .. }) => true,
        (Expected::PaginatedResult, NormalizedOutcome::Result { value }) => result_page(value)
            .is_some_and(|page| {
                page.get("next_cursor")
                    .is_some_and(|cursor| !cursor.is_null())
            }),
        (Expected::Error(expected), NormalizedOutcome::Error { code }) => code == expected,
        _ => false,
    };
    if !matches {
        diagnostics.push(format!(
            "{interface}: expected {}, got {}",
            expected_name(expected),
            normalized_json(actual)
        ));
    }
}

fn expected_for(case: &ComplianceCase) -> Expected {
    match (case.kind, case.operation) {
        (
            ComplianceCaseKind::Boundary,
            ComplianceOperation::ListLogSources
            | ComplianceOperation::ListMetrics
            | ComplianceOperation::ListTasks
            | ComplianceOperation::ListImageSources
            | ComplianceOperation::ListImages,
        ) => Expected::PaginatedResult,
        (ComplianceCaseKind::Success | ComplianceCaseKind::Boundary, _) => Expected::Result,
        (ComplianceCaseKind::Invalid, _) => Expected::Error("invalid"),
        (ComplianceCaseKind::NotFound, _) => Expected::Error("not_found"),
        (ComplianceCaseKind::Unavailable, _) => Expected::Error("unavailable"),
    }
}

fn expected_name(expected: Expected) -> &'static str {
    match expected {
        Expected::Result => "result",
        Expected::PaginatedResult => "paginated result",
        Expected::Error(code) => code,
    }
}

fn result_page(value: &serde_json::Value) -> Option<&serde_json::Value> {
    value.get("result")
}

fn normalized_json(outcome: &NormalizedOutcome) -> String {
    serde_json::to_string(outcome).unwrap_or_else(|_| r#"{"outcome":"protocol"}"#.to_owned())
}

fn case_result(
    case: &ComplianceCase,
    required: bool,
    outcome: ComplianceOutcome,
    diagnostics: Vec<String>,
) -> ComplianceCaseResult {
    ComplianceCaseResult {
        case_id: case.id.to_owned(),
        required,
        interfaces: both_interfaces(),
        outcome,
        diagnostics,
    }
}

fn both_interfaces() -> BTreeSet<ComplianceInterface> {
    [ComplianceInterface::A2a, ComplianceInterface::Mcp]
        .into_iter()
        .collect()
}

fn scenario_required(
    capability: ComplianceScenarioCapability,
    fixtures: &ComplianceFixtures,
) -> bool {
    match capability {
        ComplianceScenarioCapability::Logs => {
            matches!(fixtures.logs, FixtureCapability::Required(_))
        }
        ComplianceScenarioCapability::Metrics => {
            matches!(fixtures.metrics, FixtureCapability::Required(_))
        }
        ComplianceScenarioCapability::Tasks => {
            matches!(fixtures.tasks, FixtureCapability::Required(_))
        }
        ComplianceScenarioCapability::Images => {
            matches!(fixtures.images, FixtureCapability::Required(_))
        }
        ComplianceScenarioCapability::A2aAgentMessage => true,
    }
}

fn case_required(case: &ComplianceCase, fixtures: &ComplianceFixtures) -> bool {
    let available = match case.operation {
        ComplianceOperation::ListLogSources | ComplianceOperation::QueryLogs => {
            matches!(fixtures.logs, FixtureCapability::Required(_))
        }
        ComplianceOperation::ListMetrics | ComplianceOperation::QueryMetric => {
            matches!(fixtures.metrics, FixtureCapability::Required(_))
        }
        ComplianceOperation::ListTasks
        | ComplianceOperation::StartTask
        | ComplianceOperation::GetTaskStatus => {
            matches!(fixtures.tasks, FixtureCapability::Required(_))
        }
        ComplianceOperation::ListImageSources
        | ComplianceOperation::ListImages
        | ComplianceOperation::SearchImages
        | ComplianceOperation::GetImage
        | ComplianceOperation::GetCurrentImage => {
            matches!(fixtures.images, FixtureCapability::Required(_))
        }
    };
    available != (case.kind == ComplianceCaseKind::Unavailable)
}

fn command_for(case: &ComplianceCase, fixtures: &ComplianceFixtures) -> A2aLabCommand {
    let invalid_page = || {
        serde_json::from_value(serde_json::json!({"limit": 0}))
            .expect("unchecked invalid page fixture")
    };
    let page = || match case.kind {
        ComplianceCaseKind::Boundary => PageRequest::new(None, 1).expect("valid page"),
        ComplianceCaseKind::Invalid => invalid_page(),
        _ => PageRequest::default(),
    };
    let range = || match case.kind {
        ComplianceCaseKind::Invalid => reversed_range(fixtures.range),
        _ => fixtures.range,
    };
    match case.operation {
        ComplianceOperation::ListLogSources => {
            A2aLabCommand::ListLogSources(ListLogSourcesRequest { page: page() })
        }
        ComplianceOperation::QueryLogs => {
            let source_id = log_source(case.kind, fixtures);
            A2aLabCommand::QueryLogs(QueryLogsRequest {
                source_id,
                range: range(),
                page: PageRequest::default(),
            })
        }
        ComplianceOperation::ListMetrics => {
            A2aLabCommand::ListMetrics(ListMetricsRequest { page: page() })
        }
        ComplianceOperation::QueryMetric => {
            let metric_id = metric_id(case.kind, fixtures);
            A2aLabCommand::QueryMetric(QueryMetricRequest {
                metric_id,
                range: range(),
                page: PageRequest::default(),
            })
        }
        ComplianceOperation::ListTasks => {
            A2aLabCommand::ListTasks(ListTasksRequest { page: page() })
        }
        ComplianceOperation::StartTask => {
            let (task_id, input) = task_start(case.kind, fixtures);
            A2aLabCommand::StartTask(StartTaskRequest::new(task_id, input))
        }
        ComplianceOperation::GetTaskStatus => A2aLabCommand::GetTaskStatus(GetTaskStatusRequest {
            id: run_id(case.kind, fixtures),
        }),
        ComplianceOperation::ListImageSources => {
            let request = if case.kind == ComplianceCaseKind::Invalid {
                TckMalformedImageRequests::list_image_sources_zero_limit()
            } else {
                ListImageSourcesRequest::new(page()).expect("page request")
            };
            A2aLabCommand::ListImageSources(request)
        }
        ComplianceOperation::ListImages => {
            let source_id = image_source(case.kind, fixtures);
            let request = if case.kind == ComplianceCaseKind::Invalid {
                TckMalformedImageRequests::list_images_zero_limit(source_id)
            } else {
                ListImagesRequest::new(source_id, page()).expect("image list request")
            };
            A2aLabCommand::ListImages(request)
        }
        ComplianceOperation::SearchImages => {
            let request = if case.kind == ComplianceCaseKind::Invalid {
                TckMalformedImageRequests::search_images_without_criterion()
            } else {
                SearchImagesRequest::new(
                    Some(image_source(case.kind, fixtures)),
                    Some(fixtures.range),
                    None,
                    page(),
                )
                .expect("image search request")
            };
            A2aLabCommand::SearchImages(request)
        }
        ComplianceOperation::GetImage => {
            A2aLabCommand::GetImage(GetImageRequest::new(image_id(case.kind, fixtures)))
        }
        ComplianceOperation::GetCurrentImage => A2aLabCommand::GetCurrentImage(
            GetCurrentImageRequest::new(image_source(case.kind, fixtures)),
        ),
    }
}

fn log_source(kind: ComplianceCaseKind, fixtures: &ComplianceFixtures) -> SourceId {
    match (&fixtures.logs, kind) {
        (FixtureCapability::Required(value), ComplianceCaseKind::NotFound) => {
            value.missing_source_id.clone()
        }
        (FixtureCapability::Required(value), _) => value.source_id.clone(),
        (FixtureCapability::Unavailable, _) => SourceId::new("compliance.unavailable").unwrap(),
    }
}

fn metric_id(kind: ComplianceCaseKind, fixtures: &ComplianceFixtures) -> MetricId {
    match (&fixtures.metrics, kind) {
        (FixtureCapability::Required(value), ComplianceCaseKind::NotFound) => {
            value.missing_metric_id.clone()
        }
        (FixtureCapability::Required(value), _) => value.metric_id.clone(),
        (FixtureCapability::Unavailable, _) => MetricId::new("compliance.unavailable").unwrap(),
    }
}

fn task_start(kind: ComplianceCaseKind, fixtures: &ComplianceFixtures) -> (TaskId, JsonObject) {
    match (&fixtures.tasks, kind) {
        (FixtureCapability::Required(value), ComplianceCaseKind::NotFound) => {
            (value.missing_task_id.clone(), value.input.clone())
        }
        (FixtureCapability::Required(value), _) => (value.task_id.clone(), value.input.clone()),
        (FixtureCapability::Unavailable, _) => (
            TaskId::new("compliance.unavailable").unwrap(),
            JsonObject::empty(),
        ),
    }
}

fn run_id(kind: ComplianceCaseKind, fixtures: &ComplianceFixtures) -> RunId {
    match (&fixtures.tasks, kind) {
        (FixtureCapability::Required(value), ComplianceCaseKind::NotFound) => {
            value.missing_run_id.clone()
        }
        (FixtureCapability::Required(value), _) => value.run_id.clone(),
        (FixtureCapability::Unavailable, _) => RunId::new("compliance.unavailable").unwrap(),
    }
}

fn image_source(kind: ComplianceCaseKind, fixtures: &ComplianceFixtures) -> ImageSourceId {
    match (&fixtures.images, kind) {
        (FixtureCapability::Required(value), ComplianceCaseKind::NotFound) => {
            value.missing_source_id.clone()
        }
        (FixtureCapability::Required(value), _) => value.source_id.clone(),
        (FixtureCapability::Unavailable, _) => {
            ImageSourceId::new("compliance.unavailable").unwrap()
        }
    }
}

fn image_id(kind: ComplianceCaseKind, fixtures: &ComplianceFixtures) -> ImageId {
    match (&fixtures.images, kind) {
        (FixtureCapability::Required(value), ComplianceCaseKind::NotFound) => {
            value.missing_image_id.clone()
        }
        (FixtureCapability::Required(value), _) => value.image_id.clone(),
        (FixtureCapability::Unavailable, _) => ImageId::new("compliance.unavailable").unwrap(),
    }
}

fn reversed_range(range: TimeRange) -> TimeRange {
    serde_json::from_value(serde_json::json!({
        "start": range.end(),
        "end": range.start()
    }))
    .expect("unchecked invalid range fixture")
}

fn check_url(field: &str, value: &str) -> Result<(), ComplianceRunError> {
    let valid = (value.starts_with("http://") || value.starts_with("https://"))
        && value
            .split_once("://")
            .is_some_and(|(_, authority)| !authority.is_empty());
    if valid {
        Ok(())
    } else {
        Err(ComplianceRunError::Configuration(format!(
            "{field} must be an absolute HTTP(S) URL"
        )))
    }
}

struct A2aEndpoint(A2aClient);

impl ComplianceEndpoint for A2aEndpoint {
    fn execute(
        &self,
        command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabResult, A2aLabError>> {
        Box::pin(async move {
            match command {
                A2aLabCommand::ListLogSources(request) => self
                    .0
                    .list_log_sources(request)
                    .await
                    .map(A2aLabResult::ListLogSources),
                A2aLabCommand::QueryLogs(request) => self
                    .0
                    .query_logs(request)
                    .await
                    .map(A2aLabResult::QueryLogs),
                A2aLabCommand::ListMetrics(request) => self
                    .0
                    .list_metrics(request)
                    .await
                    .map(A2aLabResult::ListMetrics),
                A2aLabCommand::QueryMetric(request) => self
                    .0
                    .query_metric(request)
                    .await
                    .map(A2aLabResult::QueryMetric),
                A2aLabCommand::ListTasks(request) => self
                    .0
                    .list_tasks(request)
                    .await
                    .map(A2aLabResult::ListTasks),
                A2aLabCommand::StartTask(request) => self
                    .0
                    .start_task(request)
                    .await
                    .map(|snapshot| snapshot.result),
                A2aLabCommand::GetTaskStatus(request) => self
                    .0
                    .task_status(request)
                    .await
                    .map(A2aLabResult::GetTaskStatus),
                A2aLabCommand::ListImageSources(request) => self
                    .0
                    .list_image_sources(request)
                    .await
                    .map(A2aLabResult::ListImageSources),
                A2aLabCommand::ListImages(request) => self
                    .0
                    .list_images(request)
                    .await
                    .map(A2aLabResult::ListImages),
                A2aLabCommand::SearchImages(request) => self
                    .0
                    .search_images(request)
                    .await
                    .map(A2aLabResult::SearchImages),
                A2aLabCommand::GetImage(request) => {
                    self.0.get_image(request).await.map(A2aLabResult::GetImage)
                }
                A2aLabCommand::GetCurrentImage(request) => self
                    .0
                    .get_current_image(request)
                    .await
                    .map(A2aLabResult::GetCurrentImage),
            }
        })
    }

    fn agent_message_advertised(&self) -> A2aLabFuture<'_, Result<bool, A2aLabError>> {
        Box::pin(async move {
            self.0
                .agent_card()
                .await
                .map(|card| card.skills.iter().any(|skill| skill.id == "agent-message"))
        })
    }

    fn agent_message(
        &self,
        text: String,
    ) -> A2aLabFuture<'_, Result<AgentMessageResponse, A2aLabError>> {
        Box::pin(async move { self.0.agent_message(&text, None).await })
    }
}

struct McpEndpoint(McpLab);

impl ComplianceEndpoint for McpEndpoint {
    fn execute(
        &self,
        command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabResult, A2aLabError>> {
        Box::pin(async move {
            self.0
                .execute(command)
                .await
                .map(|outcome| outcome.task.result)
        })
    }
}

struct FailedEndpoint(A2aLabError);

impl ComplianceEndpoint for FailedEndpoint {
    fn execute(
        &self,
        _command: A2aLabCommand,
    ) -> A2aLabFuture<'_, Result<A2aLabResult, A2aLabError>> {
        let error = self.0.clone();
        Box::pin(async move { Err(error) })
    }
}
