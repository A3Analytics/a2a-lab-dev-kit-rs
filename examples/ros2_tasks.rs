//! ROS 2 actions as lab tasks, using the in-process graph.
//!
//! `MemoryRos2` does not speak DDS or RCL. The example advertises an action,
//! starts it, then records an executing and a succeeded goal.

use a2a_lab_sdk::{
    GetTaskStatusRequest, JsonObject, ListTasksRequest, MemoryRos2, PageRequest, Ros2Action,
    Ros2GoalStatus, Ros2Tasks, SdkError, StartTaskRequest, TaskId, TaskProvider,
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

    let tasks = Ros2Tasks::new(graph.clone());
    let listed = tasks
        .list_tasks(ListTasksRequest {
            page: PageRequest::new(None, 10)?,
        })
        .await?;
    println!(
        "advertised {} ({})",
        listed.items()[0].id,
        listed.items()[0].semantic_id.as_deref().unwrap_or("")
    );

    let started = tasks
        .start(StartTaskRequest {
            task_id: TaskId::new("navigate_to_pose")?,
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
    let working = tasks
        .status(GetTaskStatusRequest {
            id: started.id.clone(),
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
    let finished = tasks
        .status(GetTaskStatusRequest { id: started.id })
        .await?;
    println!("goal {goal_id} {:?}", finished.state);
    Ok(())
}
