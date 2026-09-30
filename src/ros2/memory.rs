//! In-process ROS 2 action graph for tests and embedded adapters.

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::error::SdkError;
use crate::json_object::JsonObject;
use crate::ros2::{Ros2Action, Ros2Goal, Ros2GoalStatus, Ros2Graph};

#[derive(Default)]
struct GraphState {
    actions: Vec<Ros2Action>,
    goals: BTreeMap<String, Ros2Goal>,
    next: u64,
}

/// ROS 2 graph whose actions and goal statuses are updated by the caller.
#[derive(Clone, Default)]
pub struct MemoryRos2 {
    inner: Arc<Mutex<GraphState>>,
}

impl MemoryRos2 {
    /// Creates an empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advertises an action server. A second call with the same name replaces it.
    pub async fn advertise(&self, action: Ros2Action) {
        let mut state = self.inner.lock().await;
        if let Some(existing) = state
            .actions
            .iter_mut()
            .find(|current| current.name == action.name)
        {
            *existing = action;
        } else {
            state.actions.push(action);
        }
    }

    /// Updates a goal status after the action server reports a transition.
    pub async fn set_status(
        &self,
        goal_id: &str,
        status: Ros2GoalStatus,
        message: Option<String>,
    ) -> Result<(), SdkError> {
        let mut state = self.inner.lock().await;
        let goal = state
            .goals
            .get_mut(goal_id)
            .ok_or_else(|| SdkError::not_found("ros2 goal", goal_id))?;
        goal.status = status;
        goal.message = message;
        Ok(())
    }
}

impl Ros2Graph for MemoryRos2 {
    async fn actions(&self) -> Result<Vec<Ros2Action>, SdkError> {
        Ok(self.inner.lock().await.actions.clone())
    }

    async fn send_goal(&self, action_name: &str, _goal: JsonObject) -> Result<Ros2Goal, SdkError> {
        let mut state = self.inner.lock().await;
        if !state
            .actions
            .iter()
            .any(|action| action.name == action_name)
        {
            return Err(SdkError::not_found("ros2 action", action_name));
        }
        state.next += 1;
        let goal = Ros2Goal {
            id: format!("goal-{}", state.next),
            action_name: action_name.to_owned(),
            status: Ros2GoalStatus::Accepted,
            message: None,
        };
        state.goals.insert(goal.id.clone(), goal.clone());
        Ok(goal)
    }

    async fn goal(&self, goal_id: &str) -> Result<Ros2Goal, SdkError> {
        self.inner
            .lock()
            .await
            .goals
            .get(goal_id)
            .cloned()
            .ok_or_else(|| SdkError::not_found("ros2 goal", goal_id))
    }
}
