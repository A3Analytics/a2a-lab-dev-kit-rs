//! ROS 2 actions as lab workflows, using the in-process graph.
//!
//! `MemoryRos2` does not speak DDS or RCL. The example advertises an action,
//! starts it, then records an executing and a succeeded goal.

use a2a_lab_sdk::{
    GetWorkflowStatusRequest, JsonObject, ListWorkflowsRequest, MemoryRos2, PageRequest,
    Ros2Action, Ros2GoalStatus, Ros2Workflows, SdkError, StartWorkflowRequest, WorkflowId,
    WorkflowProvider,
};

#[tokio::main]
async fn main() -> Result<(), SdkError> {
    let graph = MemoryRos2::new();
    graph
        .advertise(Ros2Action {
            name: "/navigate_to_pose".to_owned(),
            type_name: "nav2_msgs/action/NavigateToPose".to_owned(),
        })
        .await;

    let workflows = Ros2Workflows::new(graph.clone());
    let listed = workflows
        .list_workflows(ListWorkflowsRequest {
            page: PageRequest::new(None, 10)?,
        })
        .await?;
    println!(
        "advertised {} ({})",
        listed.items()[0].id,
        listed.items()[0].semantic_id.as_deref().unwrap_or("")
    );

    let started = workflows
        .start(StartWorkflowRequest {
            workflow_id: WorkflowId::new("navigate_to_pose")?,
            input: JsonObject::parse(r#"{"pose":"dock"}"#)?,
        })
        .await?;
    let goal_id = started
        .id
        .as_str()
        .strip_prefix("ros2-")
        .ok_or_else(|| SdkError::protocol("run id is missing the ros2 prefix"))?
        .to_owned();
    println!("started {} {:?}", started.id, started.state);

    graph
        .set_status(
            &goal_id,
            Ros2GoalStatus::Executing,
            Some("moving".to_owned()),
        )
        .await?;
    let working = workflows
        .status(GetWorkflowStatusRequest {
            run_id: started.id.clone(),
        })
        .await?;
    println!(
        "goal {goal_id} {:?} message {}",
        working.state,
        working.message.as_deref().unwrap_or("")
    );

    graph
        .set_status(&goal_id, Ros2GoalStatus::Succeeded, None)
        .await?;
    let finished = workflows
        .status(GetWorkflowStatusRequest { run_id: started.id })
        .await?;
    println!("goal {goal_id} {:?}", finished.state);
    Ok(())
}
