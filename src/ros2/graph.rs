//! ROS 2 action graph used by the workflow provider.

use crate::error::SdkError;
use crate::json_object::JsonObject;
use crate::workflows::RunState;

/// A ROS 2 action server advertised on the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ros2Action {
    /// Action name, such as `/navigate_to_pose`.
    pub name: String,
    /// Action type, such as `nav2_msgs/action/NavigateToPose`.
    pub type_name: String,
}

/// Status reported by a ROS 2 action goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ros2GoalStatus {
    /// The goal was accepted and has not started.
    Accepted,
    /// The goal is executing.
    Executing,
    /// The goal is canceling.
    Canceling,
    /// The goal succeeded.
    Succeeded,
    /// The goal was aborted.
    Aborted,
    /// The goal was canceled.
    Canceled,
}

impl Ros2GoalStatus {
    /// Maps a ROS 2 goal status onto a lab run state.
    #[must_use]
    pub const fn to_run_state(self) -> RunState {
        match self {
            Self::Accepted => RunState::Submitted,
            Self::Executing | Self::Canceling => RunState::Working,
            Self::Succeeded => RunState::Completed,
            Self::Aborted => RunState::Failed,
            Self::Canceled => RunState::Canceled,
        }
    }
}

/// A goal accepted by an action server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ros2Goal {
    /// Goal identifier assigned by the action server.
    pub id: String,
    /// Action that accepted the goal.
    pub action_name: String,
    /// Current goal status.
    pub status: Ros2GoalStatus,
    /// Optional feedback or result detail.
    pub message: Option<String>,
}

/// Live view of ROS 2 action servers and goals.
pub trait Ros2Graph: Send + Sync {
    /// Returns the action servers currently advertised.
    fn actions(&self) -> impl Future<Output = Result<Vec<Ros2Action>, SdkError>> + Send;

    /// Sends a JSON goal to an action.
    fn send_goal(
        &self,
        action_name: &str,
        goal: JsonObject,
    ) -> impl Future<Output = Result<Ros2Goal, SdkError>> + Send;

    /// Returns the current status of a goal.
    fn goal(&self, goal_id: &str) -> impl Future<Output = Result<Ros2Goal, SdkError>> + Send;
}
