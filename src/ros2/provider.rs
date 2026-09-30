//! ROS 2 actions exposed as lab workflows.

use crate::error::SdkError;
use crate::id::{RunId, WorkflowId};
use crate::page::{Page, slice_page};
use crate::ros2::{Ros2Action, Ros2Graph};
use crate::workflows::{
    GetWorkflowStatusRequest, ListWorkflowsRequest, StartWorkflowRequest, WorkflowDefinition,
    WorkflowProvider, WorkflowRun,
};

/// Workflow provider backed by ROS 2 action servers.
pub struct Ros2Workflows<G> {
    graph: G,
}

impl<G> Ros2Workflows<G> {
    /// Creates a provider over a ROS 2 action graph.
    #[must_use]
    pub const fn new(graph: G) -> Self {
        Self { graph }
    }

    /// Returns the graph so a caller can advertise actions or update goals.
    #[must_use]
    pub const fn graph(&self) -> &G {
        &self.graph
    }
}

impl<G: Ros2Graph> WorkflowProvider for Ros2Workflows<G> {
    async fn list_workflows(
        &self,
        request: ListWorkflowsRequest,
    ) -> Result<Page<WorkflowDefinition>, SdkError> {
        let mut actions = self.graph.actions().await?;
        actions.sort_by(|left, right| left.name.cmp(&right.name));
        let mut workflows = Vec::new();
        for action in actions {
            workflows.push(definition(&action)?);
        }
        slice_page(&workflows, &request.page)
    }

    async fn start(&self, request: StartWorkflowRequest) -> Result<WorkflowRun, SdkError> {
        let action_name = action_name(request.workflow_id.as_str());
        let goal = self
            .graph
            .send_goal(&action_name, request.input.clone())
            .await?;
        Ok(WorkflowRun {
            id: RunId::new(goal_token(&goal.id))?,
            workflow_id: request.workflow_id,
            state: goal.status.to_run_state(),
            input: request.input,
            message: goal.message,
        })
    }

    async fn status(&self, request: GetWorkflowStatusRequest) -> Result<WorkflowRun, SdkError> {
        let goal = self.graph.goal(&goal_name(request.run_id.as_str())).await?;
        Ok(WorkflowRun {
            id: request.run_id,
            workflow_id: WorkflowId::new(action_token(&goal.action_name))?,
            state: goal.status.to_run_state(),
            input: crate::json_object::JsonObject::empty(),
            message: goal.message,
        })
    }
}

fn definition(action: &Ros2Action) -> Result<WorkflowDefinition, SdkError> {
    Ok(WorkflowDefinition {
        id: WorkflowId::new(action_token(&action.name))?,
        name: action.name.clone(),
        description: action.type_name.clone(),
        asset_id: None,
        semantic_id: Some(action.type_name.clone()),
    })
}

fn action_token(name: &str) -> String {
    name.trim_matches('/').replace('/', ":").replace(' ', "_")
}

fn action_name(token: &str) -> String {
    format!("/{}", token.replace(':', "/"))
}

fn goal_token(goal_id: &str) -> String {
    format!("ros2-{goal_id}")
}

fn goal_name(token: &str) -> String {
    token.strip_prefix("ros2-").unwrap_or(token).to_owned()
}
