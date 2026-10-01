//! ROS 2 actions exposed as lab tasks.

use crate::error::A2aLabError;
use crate::id::{RunId, TaskId};
use crate::page::{Page, slice_page};
use crate::ros2::{Ros2Action, Ros2Graph};
use crate::tasks::{
    GetTaskStatusRequest, ListTasksRequest, StartTaskRequest, TaskDefinition, TaskProvider, TaskRun,
};

/// Task provider backed by ROS 2 action servers.
pub struct Ros2Tasks<G> {
    graph: G,
}

impl<G> Ros2Tasks<G> {
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

impl<G: Ros2Graph> TaskProvider for Ros2Tasks<G> {
    async fn list_tasks(
        &self,
        request: ListTasksRequest,
    ) -> Result<Page<TaskDefinition>, A2aLabError> {
        let mut actions = self.graph.actions().await?;
        actions.sort_by(|left, right| left.name.cmp(&right.name));
        let mut tasks = Vec::new();
        for action in actions {
            tasks.push(definition(&action)?);
        }
        slice_page(&tasks, &request.page)
    }

    async fn start(&self, request: StartTaskRequest) -> Result<TaskRun, A2aLabError> {
        let action_name = action_name(request.task_id.as_str());
        let goal = self
            .graph
            .send_goal(&action_name, request.input.clone())
            .await?;
        Ok(TaskRun {
            id: RunId::new(goal_token(&goal.id))?,
            task_id: request.task_id,
            state: goal.status.to_task_state(),
            input: request.input,
            message: goal.message,
        })
    }

    async fn status(&self, request: GetTaskStatusRequest) -> Result<TaskRun, A2aLabError> {
        let goal = self.graph.goal(&goal_name(request.id.as_str())).await?;
        Ok(TaskRun {
            id: request.id,
            task_id: TaskId::new(action_token(&goal.action_name))?,
            state: goal.status.to_task_state(),
            input: crate::json_object::JsonObject::empty(),
            message: goal.message,
        })
    }
}

fn definition(action: &Ros2Action) -> Result<TaskDefinition, A2aLabError> {
    Ok(TaskDefinition {
        id: TaskId::new(action_token(&action.name))?,
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
