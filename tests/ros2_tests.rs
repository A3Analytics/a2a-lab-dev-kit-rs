use a2a_lab_dev_kit::{
    GetTaskStatusRequest, JsonObject, ListTasksRequest, MemoryRos2, PageRequest, Ros2Action,
    Ros2GoalStatus, Ros2Tasks, TaskId, TaskProvider, TaskState,
};

#[tokio::test]
async fn lists_actions_and_maps_goal_status() {
    let graph = MemoryRos2::new();
    graph
        .advertise(Ros2Action {
            name: "/navigate_to_pose".to_owned(),
            type_name: "nav2_msgs/action/NavigateToPose".to_owned(),
        })
        .await;
    graph
        .advertise(Ros2Action {
            name: "/arm/follow_joint_trajectory".to_owned(),
            type_name: "control_msgs/action/FollowJointTrajectory".to_owned(),
        })
        .await;
    let tasks = Ros2Tasks::new(graph.clone());
    let page = tasks
        .list_tasks(ListTasksRequest {
            page: PageRequest::new(None, 10).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(page.items()[0].id.as_str(), "arm:follow_joint_trajectory");
    assert_eq!(
        page.items()[1].semantic_id.as_deref(),
        Some("nav2_msgs/action/NavigateToPose")
    );

    let started = tasks
        .start(a2a_lab_dev_kit::StartTaskRequest::new(
            TaskId::new("navigate_to_pose").unwrap(),
            JsonObject::parse(r#"{"pose":"dock"}"#).unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(started.state, TaskState::Submitted);
    graph
        .set_status(
            started.id.as_str().strip_prefix("ros2-").unwrap(),
            Ros2GoalStatus::Executing,
            Some("moving".to_owned()),
        )
        .await
        .unwrap();
    let status = tasks
        .status(GetTaskStatusRequest {
            id: started.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(status.state, TaskState::Working);
    assert_eq!(status.message.as_deref(), Some("moving"));
    graph
        .set_status(
            started.id.as_str().strip_prefix("ros2-").unwrap(),
            Ros2GoalStatus::Succeeded,
            None,
        )
        .await
        .unwrap();
    let finished = tasks
        .status(GetTaskStatusRequest { id: started.id })
        .await
        .unwrap();
    assert_eq!(finished.state, TaskState::Completed);

    let missing = tasks
        .start(a2a_lab_dev_kit::StartTaskRequest::new(
            TaskId::new("missing").unwrap(),
            JsonObject::empty(),
        ))
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");
}
