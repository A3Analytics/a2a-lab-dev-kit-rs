//! Versioned behavioral contract shared by A2A-LAB compliance runners.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use schemars::{JsonSchema, Schema, schema_for};
use serde::{Deserialize, Serialize};

use crate::{ImageId, ImageSourceId, JsonObject, MetricId, RunId, SourceId, TaskId, TimeRange};

#[path = "compliance_runner.rs"]
mod runner;

pub use runner::{
    ComplianceEndpoint, ComplianceRunError, ComplianceRunner, ComplianceRunnerConfig,
    run_compliance,
};

/// Current reusable A2A-LAB compliance profile.
pub const COMPLIANCE_PROFILE_VERSION: &str = "1.1.0";

/// Stable identifier of the JSON result schema.
pub const COMPLIANCE_RESULT_SCHEMA_ID: &str =
    "https://a2a-lab.dev/schemas/compliance-result-1.1.0.json";

/// Named scope selected for a compliance run.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ComplianceSuite {
    /// Stable operation-level cases from profile 1.0.0.
    #[default]
    Basic,
    /// Every basic case plus linked capability scenarios.
    Full,
}

/// Independently recoverable checks enabled for a run.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ComplianceCheck {
    /// Operation-level basic cases.
    BasicOperations,
    /// Linked logs, metrics, tasks, and images scenarios.
    LinkedScenarios,
    /// Optional A2A `agent-message` check backed by an LLM.
    A2aAgentMessage,
}

/// Interface exercised by the compliance suite.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ComplianceInterface {
    /// Agent2Agent.
    A2a,
    /// Model Context Protocol.
    Mcp,
}

/// One of the twelve A2A-LAB operations.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ComplianceOperation {
    /// List log sources.
    ListLogSources,
    /// Query log records.
    QueryLogs,
    /// List metric descriptors.
    ListMetrics,
    /// Query metric points.
    QueryMetric,
    /// List task definitions.
    ListTasks,
    /// Start a task.
    StartTask,
    /// Read a task run.
    GetTaskStatus,
    /// List image sources.
    ListImageSources,
    /// List image descriptors.
    ListImages,
    /// Search image descriptors.
    SearchImages,
    /// Read one image.
    GetImage,
    /// Read the current image.
    GetCurrentImage,
}

/// Behavior represented by a contract case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComplianceCaseKind {
    /// Representative successful request.
    Success,
    /// Pagination or half-open range boundary.
    Boundary,
    /// Invalid request.
    Invalid,
    /// Missing fixture identifier.
    NotFound,
    /// Capability explicitly declared unavailable.
    Unavailable,
}

/// Static definition of one compliance case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComplianceCase {
    /// Stable case identifier.
    pub id: &'static str,
    /// Operation under test.
    pub operation: ComplianceOperation,
    /// Expected behavior.
    pub kind: ComplianceCaseKind,
}

/// Capability exercised by a linked full-suite scenario.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ComplianceScenarioCapability {
    /// Discover a log source, then query it.
    Logs,
    /// Discover a metric, then query it.
    Metrics,
    /// Discover and start a task, then read or poll the returned run.
    Tasks,
    /// Discover, list or search, retrieve, and request a current image.
    Images,
    /// Optional A2A-only agent-message behavior.
    A2aAgentMessage,
}

/// Minimal assertion allowed for a linked scenario.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ComplianceScenarioAssertion {
    /// A returned identifier is passed to the next operation.
    MeaningfulLinkage,
    /// Producer and consumer accept the same identifier type.
    CompatibleIdentifier,
    /// The linked operation succeeds and returns available content.
    SuccessfulAvailability,
    /// A returned task run reaches or makes progress toward a terminal state.
    TaskTerminalProgress,
}

/// Static definition of one full-suite scenario case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComplianceScenario {
    /// Stable scenario identifier.
    pub id: &'static str,
    /// Capability under test.
    pub capability: ComplianceScenarioCapability,
    /// Interface producing the identifier handed off by the scenario.
    pub producer: ComplianceInterface,
    /// Interface consuming the returned identifier.
    pub consumer: ComplianceInterface,
    /// Ordered operation names in the linked flow.
    pub steps: &'static [&'static str],
    /// Assertions permitted for this scenario.
    pub assertions: &'static [ComplianceScenarioAssertion],
}

/// Static definition of a named suite.
#[derive(Debug, Clone, Copy)]
pub struct ComplianceSuiteContract {
    /// Suite name.
    pub name: ComplianceSuite,
    /// Stable operation-level cases.
    pub cases: &'static [ComplianceCase],
    /// Linked scenario cases added by this suite.
    pub scenarios: &'static [ComplianceScenario],
}

/// Static contract for an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationContract {
    /// Operation under test.
    pub operation: ComplianceOperation,
    /// Interfaces on which the operation must exhibit the same lab behavior.
    pub interfaces: &'static [ComplianceInterface],
    /// Result fields compared after protocol envelopes are removed.
    pub observable_fields: &'static [&'static str],
    /// Expected lab error when the primitive is declared unavailable.
    pub unavailable_error_code: &'static str,
}

/// Versioned contract consumed by compliance runners.
#[derive(Debug, Clone, Copy)]
pub struct ComplianceProfile {
    /// Profile version.
    pub version: &'static str,
    /// Operations that must be exercised over both interfaces.
    pub operations: &'static [OperationContract],
    /// Success and failure cases in deterministic order.
    pub cases: &'static [ComplianceCase],
    /// Named basic and full suite contracts.
    pub suites: &'static [ComplianceSuiteContract],
    /// Intentional protocol-level differences excluded from result comparison.
    pub intentional_envelope_differences: &'static [&'static str],
}

/// Returns the current compliance profile.
#[must_use]
pub const fn compliance_profile() -> ComplianceProfile {
    ComplianceProfile {
        version: COMPLIANCE_PROFILE_VERSION,
        operations: OPERATIONS,
        cases: CASES,
        suites: SUITES,
        intentional_envelope_differences: INTENTIONAL_ENVELOPE_DIFFERENCES,
    }
}

/// Returns one named suite from the current profile.
#[must_use]
pub const fn compliance_suite(suite: ComplianceSuite) -> ComplianceSuiteContract {
    match suite {
        ComplianceSuite::Basic => BASIC_SUITE,
        ComplianceSuite::Full => FULL_SUITE,
    }
}

const PAGE_FIELDS: &[&str] = &["items", "next_cursor"];
const RUN_FIELDS: &[&str] = &[
    "id",
    "task_id",
    "state",
    "input",
    "message",
    "result",
    "progress",
    "error_kind",
    "error_identifier",
];
const IMAGE_FIELDS: &[&str] = &["descriptor", "data"];
const BOTH_INTERFACES: &[ComplianceInterface] =
    &[ComplianceInterface::A2a, ComplianceInterface::Mcp];

const OPERATIONS: &[OperationContract] = &[
    operation(ComplianceOperation::ListLogSources, PAGE_FIELDS),
    operation(ComplianceOperation::QueryLogs, PAGE_FIELDS),
    operation(ComplianceOperation::ListMetrics, PAGE_FIELDS),
    operation(ComplianceOperation::QueryMetric, PAGE_FIELDS),
    operation(ComplianceOperation::ListTasks, PAGE_FIELDS),
    operation(ComplianceOperation::StartTask, RUN_FIELDS),
    operation(ComplianceOperation::GetTaskStatus, RUN_FIELDS),
    operation(ComplianceOperation::ListImageSources, PAGE_FIELDS),
    operation(ComplianceOperation::ListImages, PAGE_FIELDS),
    operation(ComplianceOperation::SearchImages, PAGE_FIELDS),
    operation(ComplianceOperation::GetImage, IMAGE_FIELDS),
    operation(ComplianceOperation::GetCurrentImage, IMAGE_FIELDS),
];

const fn operation(
    operation: ComplianceOperation,
    observable_fields: &'static [&'static str],
) -> OperationContract {
    OperationContract {
        operation,
        interfaces: BOTH_INTERFACES,
        observable_fields,
        unavailable_error_code: "unavailable",
    }
}

const INTENTIONAL_ENVELOPE_DIFFERENCES: &[&str] = &[
    "A2A returns a task artifact; MCP returns structured tool content",
    "A2A and MCP use their native error envelopes and status transport",
    "MCP may repeat image base64 in text and structured content",
    "protocol task and context identifiers are not A2A-LAB result fields",
];

macro_rules! cases {
    ($(($id:literal, $operation:ident, $kind:ident)),+ $(,)?) => {
        &[$(ComplianceCase {
            id: $id,
            operation: ComplianceOperation::$operation,
            kind: ComplianceCaseKind::$kind,
        }),+]
    };
}

const CASES: &[ComplianceCase] = cases![
    ("logs.list.success", ListLogSources, Success),
    ("logs.list.pagination", ListLogSources, Boundary),
    ("logs.list.invalid-limit", ListLogSources, Invalid),
    ("logs.list.unavailable", ListLogSources, Unavailable),
    ("logs.query.success", QueryLogs, Success),
    ("logs.query.half-open-range", QueryLogs, Boundary),
    ("logs.query.missing-source", QueryLogs, NotFound),
    ("logs.query.invalid-range", QueryLogs, Invalid),
    ("logs.query.unavailable", QueryLogs, Unavailable),
    ("metrics.list.success", ListMetrics, Success),
    ("metrics.list.pagination", ListMetrics, Boundary),
    ("metrics.list.invalid-limit", ListMetrics, Invalid),
    ("metrics.list.unavailable", ListMetrics, Unavailable),
    ("metrics.query.success", QueryMetric, Success),
    ("metrics.query.half-open-range", QueryMetric, Boundary),
    ("metrics.query.missing-metric", QueryMetric, NotFound),
    ("metrics.query.invalid-range", QueryMetric, Invalid),
    ("metrics.query.unavailable", QueryMetric, Unavailable),
    ("tasks.list.success", ListTasks, Success),
    ("tasks.list.pagination", ListTasks, Boundary),
    ("tasks.list.invalid-limit", ListTasks, Invalid),
    ("tasks.list.unavailable", ListTasks, Unavailable),
    ("tasks.start.success", StartTask, Success),
    ("tasks.start.missing-task", StartTask, NotFound),
    ("tasks.start.unavailable", StartTask, Unavailable),
    ("tasks.status.success", GetTaskStatus, Success),
    ("tasks.status.missing-run", GetTaskStatus, NotFound),
    ("tasks.status.unavailable", GetTaskStatus, Unavailable),
    ("images.sources.success", ListImageSources, Success),
    ("images.sources.pagination", ListImageSources, Boundary),
    ("images.sources.invalid-limit", ListImageSources, Invalid),
    ("images.sources.unavailable", ListImageSources, Unavailable),
    ("images.list.success", ListImages, Success),
    ("images.list.pagination", ListImages, Boundary),
    ("images.list.missing-source", ListImages, NotFound),
    ("images.list.invalid-limit", ListImages, Invalid),
    ("images.list.unavailable", ListImages, Unavailable),
    ("images.search.success", SearchImages, Success),
    ("images.search.half-open-range", SearchImages, Boundary),
    ("images.search.invalid-criteria", SearchImages, Invalid),
    ("images.search.unavailable", SearchImages, Unavailable),
    ("images.get.success", GetImage, Success),
    ("images.get.missing-image", GetImage, NotFound),
    ("images.get.unavailable", GetImage, Unavailable),
    ("images.current.success", GetCurrentImage, Success),
    ("images.current.missing-source", GetCurrentImage, NotFound),
    ("images.current.unavailable", GetCurrentImage, Unavailable),
];

const LINK_ASSERTIONS: &[ComplianceScenarioAssertion] = &[
    ComplianceScenarioAssertion::MeaningfulLinkage,
    ComplianceScenarioAssertion::CompatibleIdentifier,
    ComplianceScenarioAssertion::SuccessfulAvailability,
];
const TASK_ASSERTIONS: &[ComplianceScenarioAssertion] = &[
    ComplianceScenarioAssertion::MeaningfulLinkage,
    ComplianceScenarioAssertion::CompatibleIdentifier,
    ComplianceScenarioAssertion::SuccessfulAvailability,
    ComplianceScenarioAssertion::TaskTerminalProgress,
];
const AGENT_MESSAGE_ASSERTIONS: &[ComplianceScenarioAssertion] =
    &[ComplianceScenarioAssertion::SuccessfulAvailability];
const LOG_STEPS: &[&str] = &["list_log_sources", "query_logs"];
const METRIC_STEPS: &[&str] = &["list_metrics", "query_metric"];
const TASK_STEPS: &[&str] = &["list_tasks", "start_task", "get_task_status"];
const IMAGE_STEPS: &[&str] = &[
    "list_image_sources",
    "list_or_search_images",
    "get_image",
    "get_current_image",
];
const AGENT_MESSAGE_STEPS: &[&str] = &["a2a_agent_message"];

macro_rules! scenario {
    ($id:literal, $capability:ident, $producer:ident, $consumer:ident, $steps:ident, $assertions:ident) => {
        ComplianceScenario {
            id: $id,
            capability: ComplianceScenarioCapability::$capability,
            producer: ComplianceInterface::$producer,
            consumer: ComplianceInterface::$consumer,
            steps: $steps,
            assertions: $assertions,
        }
    };
}

const FULL_SCENARIOS: &[ComplianceScenario] = &[
    scenario!(
        "logs.link.a2a-a2a",
        Logs,
        A2a,
        A2a,
        LOG_STEPS,
        LINK_ASSERTIONS
    ),
    scenario!(
        "logs.link.mcp-mcp",
        Logs,
        Mcp,
        Mcp,
        LOG_STEPS,
        LINK_ASSERTIONS
    ),
    scenario!(
        "logs.link.a2a-mcp",
        Logs,
        A2a,
        Mcp,
        LOG_STEPS,
        LINK_ASSERTIONS
    ),
    scenario!(
        "logs.link.mcp-a2a",
        Logs,
        Mcp,
        A2a,
        LOG_STEPS,
        LINK_ASSERTIONS
    ),
    scenario!(
        "metrics.link.a2a-a2a",
        Metrics,
        A2a,
        A2a,
        METRIC_STEPS,
        LINK_ASSERTIONS
    ),
    scenario!(
        "metrics.link.mcp-mcp",
        Metrics,
        Mcp,
        Mcp,
        METRIC_STEPS,
        LINK_ASSERTIONS
    ),
    scenario!(
        "metrics.link.a2a-mcp",
        Metrics,
        A2a,
        Mcp,
        METRIC_STEPS,
        LINK_ASSERTIONS
    ),
    scenario!(
        "metrics.link.mcp-a2a",
        Metrics,
        Mcp,
        A2a,
        METRIC_STEPS,
        LINK_ASSERTIONS
    ),
    scenario!(
        "tasks.link.a2a-a2a",
        Tasks,
        A2a,
        A2a,
        TASK_STEPS,
        TASK_ASSERTIONS
    ),
    scenario!(
        "tasks.link.mcp-mcp",
        Tasks,
        Mcp,
        Mcp,
        TASK_STEPS,
        TASK_ASSERTIONS
    ),
    scenario!(
        "tasks.link.a2a-mcp",
        Tasks,
        A2a,
        Mcp,
        TASK_STEPS,
        TASK_ASSERTIONS
    ),
    scenario!(
        "tasks.link.mcp-a2a",
        Tasks,
        Mcp,
        A2a,
        TASK_STEPS,
        TASK_ASSERTIONS
    ),
    scenario!(
        "images.link.a2a-a2a",
        Images,
        A2a,
        A2a,
        IMAGE_STEPS,
        LINK_ASSERTIONS
    ),
    scenario!(
        "images.link.mcp-mcp",
        Images,
        Mcp,
        Mcp,
        IMAGE_STEPS,
        LINK_ASSERTIONS
    ),
    scenario!(
        "images.link.a2a-mcp",
        Images,
        A2a,
        Mcp,
        IMAGE_STEPS,
        LINK_ASSERTIONS
    ),
    scenario!(
        "images.link.mcp-a2a",
        Images,
        Mcp,
        A2a,
        IMAGE_STEPS,
        LINK_ASSERTIONS
    ),
    scenario!(
        "agent-message.a2a",
        A2aAgentMessage,
        A2a,
        A2a,
        AGENT_MESSAGE_STEPS,
        AGENT_MESSAGE_ASSERTIONS
    ),
];

const BASIC_SUITE: ComplianceSuiteContract = ComplianceSuiteContract {
    name: ComplianceSuite::Basic,
    cases: CASES,
    scenarios: &[],
};
const FULL_SUITE: ComplianceSuiteContract = ComplianceSuiteContract {
    name: ComplianceSuite::Full,
    cases: CASES,
    scenarios: FULL_SCENARIOS,
};
const SUITES: &[ComplianceSuiteContract] = &[BASIC_SUITE, FULL_SUITE];

/// Required deterministic fixtures or an explicitly unavailable capability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "availability", content = "fixtures", rename_all = "snake_case")]
pub enum FixtureCapability<T> {
    /// Operations for this primitive are required and use these fixtures.
    Required(T),
    /// Operations must consistently return `unavailable`.
    Unavailable,
}

/// Deterministic log fixture identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogFixtures {
    /// Primary existing source.
    pub source_id: SourceId,
    /// Second existing source used for pagination.
    pub alternate_source_id: SourceId,
    /// Identifier guaranteed not to exist.
    pub missing_source_id: SourceId,
}

/// Deterministic metric fixture identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MetricFixtures {
    /// Primary existing metric.
    pub metric_id: MetricId,
    /// Second existing metric used for pagination.
    pub alternate_metric_id: MetricId,
    /// Identifier guaranteed not to exist.
    pub missing_metric_id: MetricId,
}

/// Deterministic task and run fixtures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskFixtures {
    /// Primary existing task definition.
    pub task_id: TaskId,
    /// Second existing task definition used for pagination.
    pub alternate_task_id: TaskId,
    /// Existing run readable by `get_task_status`.
    pub run_id: RunId,
    /// Identifier guaranteed not to name a task definition.
    pub missing_task_id: TaskId,
    /// Identifier guaranteed not to name a run.
    pub missing_run_id: RunId,
    /// Deterministic input accepted by `task_id`.
    pub input: JsonObject,
}

/// Deterministic image fixture identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageFixtures {
    /// Primary existing image source.
    pub source_id: ImageSourceId,
    /// Second existing image source used for pagination.
    pub alternate_source_id: ImageSourceId,
    /// Primary existing image.
    pub image_id: ImageId,
    /// Second existing image used for pagination.
    pub alternate_image_id: ImageId,
    /// Identifier guaranteed not to name an image source.
    pub missing_source_id: ImageSourceId,
    /// Identifier guaranteed not to name an image.
    pub missing_image_id: ImageId,
}

/// Hardware-independent declaration consumed by a compliance runner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComplianceFixtures {
    /// Compliance profile version targeted by this declaration.
    pub suite_version: String,
    /// Half-open range containing deterministic log, metric, and image records.
    pub range: TimeRange,
    /// Log capability and fixtures.
    pub logs: FixtureCapability<LogFixtures>,
    /// Metric capability and fixtures.
    pub metrics: FixtureCapability<MetricFixtures>,
    /// Task capability and fixtures.
    pub tasks: FixtureCapability<TaskFixtures>,
    /// Image capability and fixtures.
    pub images: FixtureCapability<ImageFixtures>,
}

impl ComplianceFixtures {
    /// Validates version, range, and fixture uniqueness.
    pub fn validate(&self) -> Result<(), ComplianceValidationErrors> {
        let mut errors = Vec::new();
        check_version(&self.suite_version, &mut errors);
        if let Err(error) = self.range.check() {
            errors.push(field_error("range", error));
        }
        if let FixtureCapability::Required(fixtures) = &self.logs {
            unique(
                "logs",
                [
                    fixtures.source_id.as_str(),
                    fixtures.alternate_source_id.as_str(),
                    fixtures.missing_source_id.as_str(),
                ],
                &mut errors,
            );
        }
        if let FixtureCapability::Required(fixtures) = &self.metrics {
            unique(
                "metrics",
                [
                    fixtures.metric_id.as_str(),
                    fixtures.alternate_metric_id.as_str(),
                    fixtures.missing_metric_id.as_str(),
                ],
                &mut errors,
            );
        }
        if let FixtureCapability::Required(fixtures) = &self.tasks {
            unique(
                "tasks.definitions",
                [
                    fixtures.task_id.as_str(),
                    fixtures.alternate_task_id.as_str(),
                    fixtures.missing_task_id.as_str(),
                ],
                &mut errors,
            );
            unique(
                "tasks.runs",
                [fixtures.run_id.as_str(), fixtures.missing_run_id.as_str()],
                &mut errors,
            );
        }
        if let FixtureCapability::Required(fixtures) = &self.images {
            unique(
                "images.sources",
                [
                    fixtures.source_id.as_str(),
                    fixtures.alternate_source_id.as_str(),
                    fixtures.missing_source_id.as_str(),
                ],
                &mut errors,
            );
            unique(
                "images.images",
                [
                    fixtures.image_id.as_str(),
                    fixtures.alternate_image_id.as_str(),
                    fixtures.missing_image_id.as_str(),
                ],
                &mut errors,
            );
        }
        finish(errors)
    }

    /// Parses and validates a fixture declaration.
    pub fn from_json(json: &str) -> Result<Self, ComplianceValidationErrors> {
        let fixtures: Self = parse_json(json)?;
        fixtures.validate()?;
        Ok(fixtures)
    }
}

/// Identity reported by the implementation under test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImplementationIdentity {
    /// Implementation or product name.
    pub name: String,
    /// Implementation version.
    pub version: String,
}

/// Outcome of one case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComplianceOutcome {
    /// Required behavior matched.
    Pass,
    /// Required behavior differed.
    Fail,
    /// Case was not executed.
    Skip,
}

/// Machine-readable result for one contract case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComplianceCaseResult {
    /// Stable identifier from [`ComplianceProfile::cases`].
    pub case_id: String,
    /// Whether this case is required for this fixture declaration.
    pub required: bool,
    /// Interfaces exercised by this case.
    pub interfaces: BTreeSet<ComplianceInterface>,
    /// Case outcome.
    pub outcome: ComplianceOutcome,
    /// Actionable messages or normalized differences.
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

/// Machine-readable result for one linked scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComplianceScenarioResult {
    /// Stable identifier from [`ComplianceSuiteContract::scenarios`].
    pub scenario_id: String,
    /// Whether this scenario is required for this fixture declaration.
    pub required: bool,
    /// Interfaces exercised by the scenario.
    pub interfaces: BTreeSet<ComplianceInterface>,
    /// Ordered producer-to-consumer interface handoff.
    pub handoff_path: Vec<ComplianceInterface>,
    /// Scenario outcome.
    pub outcome: ComplianceOutcome,
    /// Actionable messages or linkage failures.
    #[serde(default)]
    pub diagnostics: Vec<String>,
}

/// Versioned, machine-readable compliance report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComplianceResult {
    /// Compliance profile version.
    pub suite_version: String,
    /// Named suite selected for this run.
    pub selected_suite: ComplianceSuite,
    /// Checks enabled for this run; absence records explicit opt-outs.
    pub enabled_checks: BTreeSet<ComplianceCheck>,
    /// Implementation under test.
    pub implementation: ImplementationIdentity,
    /// Interfaces covered by this report.
    pub tested_interfaces: BTreeSet<ComplianceInterface>,
    /// Results keyed by stable case order.
    pub cases: Vec<ComplianceCaseResult>,
    /// Linked outcomes keyed by stable full-suite scenario order.
    pub scenarios: Vec<ComplianceScenarioResult>,
    /// True only when every required case passed on both interfaces.
    pub compliant: bool,
}

impl ComplianceResult {
    /// Validates report structure and compliance consistency.
    pub fn validate(&self) -> Result<(), ComplianceValidationErrors> {
        let mut errors = Vec::new();
        check_version(&self.suite_version, &mut errors);
        check_nonblank(
            "implementation.name",
            &self.implementation.name,
            &mut errors,
        );
        check_nonblank(
            "implementation.version",
            &self.implementation.version,
            &mut errors,
        );
        self.validate_enabled_checks(&mut errors);
        let both = both_interfaces();
        if self.tested_interfaces != both {
            errors.push(ComplianceValidationError::new(
                "tested_interfaces",
                "must contain both a2a and mcp",
            ));
        }

        let known: BTreeSet<_> = CASES.iter().map(|case| case.id).collect();
        let mut seen = BTreeSet::new();
        for (index, case) in self.cases.iter().enumerate() {
            let field = format!("cases[{index}].case_id");
            if !known.contains(case.case_id.as_str()) {
                errors.push(ComplianceValidationError::new(
                    field,
                    "is not a profile case",
                ));
            } else if !seen.insert(case.case_id.as_str()) {
                errors.push(ComplianceValidationError::new(field, "is duplicated"));
            }
            if case.interfaces != both {
                errors.push(ComplianceValidationError::new(
                    format!("cases[{index}].interfaces"),
                    "must contain both a2a and mcp",
                ));
            }
            if case.outcome != ComplianceOutcome::Pass && case.diagnostics.is_empty() {
                errors.push(ComplianceValidationError::new(
                    format!("cases[{index}].diagnostics"),
                    "must explain a failed or skipped case",
                ));
            }
            if self.compliant && case.required && case.outcome != ComplianceOutcome::Pass {
                errors.push(ComplianceValidationError::new(
                    format!("cases[{index}].outcome"),
                    "required cases must pass when compliant is true",
                ));
            }
        }
        for missing in known.difference(&seen) {
            errors.push(ComplianceValidationError::new(
                "cases",
                format!("missing profile case `{missing}`"),
            ));
        }
        if self.compliant && self.cases.iter().all(|case| !case.required) {
            errors.push(ComplianceValidationError::new(
                "compliant",
                "requires at least one required case",
            ));
        }
        self.validate_scenarios(&mut errors);
        finish(errors)
    }

    fn validate_enabled_checks(&self, errors: &mut Vec<ComplianceValidationError>) {
        if !self
            .enabled_checks
            .contains(&ComplianceCheck::BasicOperations)
        {
            errors.push(ComplianceValidationError::new(
                "enabled_checks",
                "must enable basic_operations",
            ));
        }
        let linked = self
            .enabled_checks
            .contains(&ComplianceCheck::LinkedScenarios);
        let agent_message = self
            .enabled_checks
            .contains(&ComplianceCheck::A2aAgentMessage);
        match self.selected_suite {
            ComplianceSuite::Basic if linked || agent_message => {
                errors.push(ComplianceValidationError::new(
                    "enabled_checks",
                    "basic suite cannot enable full-suite scenario checks",
                ));
            }
            ComplianceSuite::Full if !linked => {
                errors.push(ComplianceValidationError::new(
                    "enabled_checks",
                    "full suite must enable linked_scenarios",
                ));
            }
            ComplianceSuite::Basic | ComplianceSuite::Full => {}
        }
    }

    fn validate_scenarios(&self, errors: &mut Vec<ComplianceValidationError>) {
        if self.selected_suite == ComplianceSuite::Basic {
            if !self.scenarios.is_empty() {
                errors.push(ComplianceValidationError::new(
                    "scenarios",
                    "basic suite cannot contain scenario outcomes",
                ));
            }
            return;
        }

        let definitions: BTreeMap<_, _> = FULL_SCENARIOS
            .iter()
            .map(|scenario| (scenario.id, scenario))
            .collect();
        let agent_enabled = self
            .enabled_checks
            .contains(&ComplianceCheck::A2aAgentMessage);
        let mut seen = BTreeSet::new();
        for (index, scenario) in self.scenarios.iter().enumerate() {
            let field = format!("scenarios[{index}].scenario_id");
            let Some(definition) = definitions.get(scenario.scenario_id.as_str()) else {
                errors.push(ComplianceValidationError::new(
                    field,
                    "is not a full-suite scenario",
                ));
                continue;
            };
            if !seen.insert(scenario.scenario_id.as_str()) {
                errors.push(ComplianceValidationError::new(field, "is duplicated"));
            }
            let is_agent = definition.capability == ComplianceScenarioCapability::A2aAgentMessage;
            if is_agent && !agent_enabled {
                errors.push(ComplianceValidationError::new(
                    format!("scenarios[{index}]"),
                    "agent-message outcome requires a2a_agent_message to be enabled",
                ));
            }
            let expected_path = [definition.producer, definition.consumer];
            if scenario.handoff_path != expected_path {
                errors.push(ComplianceValidationError::new(
                    format!("scenarios[{index}].handoff_path"),
                    "must match the defined producer-to-consumer path",
                ));
            }
            let expected_interfaces: BTreeSet<_> = expected_path.into_iter().collect();
            if scenario.interfaces != expected_interfaces {
                errors.push(ComplianceValidationError::new(
                    format!("scenarios[{index}].interfaces"),
                    "must match the defined handoff interfaces",
                ));
            }
            if scenario.outcome != ComplianceOutcome::Pass && scenario.diagnostics.is_empty() {
                errors.push(ComplianceValidationError::new(
                    format!("scenarios[{index}].diagnostics"),
                    "must explain a failed or skipped scenario",
                ));
            }
            if self.compliant && scenario.required && scenario.outcome != ComplianceOutcome::Pass {
                errors.push(ComplianceValidationError::new(
                    format!("scenarios[{index}].outcome"),
                    "required scenarios must pass when compliant is true",
                ));
            }
        }
        for definition in FULL_SCENARIOS {
            let required_outcome = definition.capability
                != ComplianceScenarioCapability::A2aAgentMessage
                || agent_enabled;
            if required_outcome && !seen.contains(definition.id) {
                errors.push(ComplianceValidationError::new(
                    "scenarios",
                    format!("missing enabled scenario `{}`", definition.id),
                ));
            }
        }
    }

    /// Parses and validates a result document.
    pub fn from_json(json: &str) -> Result<Self, ComplianceValidationErrors> {
        let result: Self = parse_json(json)?;
        result.validate()?;
        Ok(result)
    }
}

/// Generates the versioned JSON Schema for [`ComplianceResult`].
#[must_use]
pub fn compliance_result_schema() -> Schema {
    let mut schema = schema_for!(ComplianceResult);
    schema.insert(
        "$id".to_owned(),
        serde_json::Value::String(COMPLIANCE_RESULT_SCHEMA_ID.to_owned()),
    );
    schema
}

/// One actionable validation error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ComplianceValidationError {
    /// Dotted field path.
    pub field: String,
    /// Validation failure.
    pub message: String,
}

impl ComplianceValidationError {
    fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

/// Field-level errors found in a fixture declaration or result document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplianceValidationErrors {
    errors: Vec<ComplianceValidationError>,
}

impl ComplianceValidationErrors {
    /// Returns every validation error.
    #[must_use]
    pub fn errors(&self) -> &[ComplianceValidationError] {
        &self.errors
    }
}

impl fmt::Display for ComplianceValidationErrors {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.errors.iter().enumerate() {
            if index > 0 {
                formatter.write_str("; ")?;
            }
            write!(formatter, "{}: {}", error.field, error.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for ComplianceValidationErrors {}

fn both_interfaces() -> BTreeSet<ComplianceInterface> {
    [ComplianceInterface::A2a, ComplianceInterface::Mcp]
        .into_iter()
        .collect()
}

fn check_version(version: &str, errors: &mut Vec<ComplianceValidationError>) {
    if version != COMPLIANCE_PROFILE_VERSION {
        errors.push(ComplianceValidationError::new(
            "suite_version",
            format!("must be {COMPLIANCE_PROFILE_VERSION}"),
        ));
    }
}

fn check_nonblank(field: &str, value: &str, errors: &mut Vec<ComplianceValidationError>) {
    if value.trim().is_empty() {
        errors.push(ComplianceValidationError::new(field, "must be nonblank"));
    }
}

fn unique<'a>(
    field: &str,
    values: impl IntoIterator<Item = &'a str>,
    errors: &mut Vec<ComplianceValidationError>,
) {
    let mut seen = BTreeSet::new();
    if values.into_iter().any(|value| !seen.insert(value)) {
        errors.push(ComplianceValidationError::new(
            field,
            "fixture identifiers must be distinct",
        ));
    }
}

fn field_error(field: &str, error: impl fmt::Display) -> ComplianceValidationError {
    ComplianceValidationError::new(field, error.to_string())
}

fn finish(errors: Vec<ComplianceValidationError>) -> Result<(), ComplianceValidationErrors> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(ComplianceValidationErrors { errors })
    }
}

fn parse_json<T>(json: &str) -> Result<T, ComplianceValidationErrors>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(json).map_err(|error| ComplianceValidationErrors {
        errors: vec![ComplianceValidationError::new(
            json_error_field(&error),
            error.to_string(),
        )],
    })
}

fn json_error_field(error: &serde_json::Error) -> String {
    let message = error.to_string();
    let Some(start) = message.find('`') else {
        return "$".to_owned();
    };
    let remaining = &message[start + 1..];
    remaining
        .find('`')
        .map_or_else(|| "$".to_owned(), |end| remaining[..end].to_owned())
}

/// Groups profile cases by operation for runner implementations.
#[must_use]
pub fn cases_by_operation() -> BTreeMap<ComplianceOperation, Vec<&'static ComplianceCase>> {
    let mut grouped = BTreeMap::new();
    for case in CASES {
        grouped
            .entry(case.operation)
            .or_insert_with(Vec::new)
            .push(case);
    }
    grouped
}
