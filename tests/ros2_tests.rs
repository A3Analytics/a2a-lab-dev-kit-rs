use a2a_lab_sdk::{
    GetWorkflowStatusRequest, JsonObject, ListWorkflowsRequest, MemoryRos2, PageRequest,
    Ros2Action, Ros2GoalStatus, Ros2Workflows, RunState, WorkflowId, WorkflowProvider,
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
    let workflows = Ros2Workflows::new(graph.clone());
    let page = workflows
        .list_workflows(ListWorkflowsRequest {
            page: PageRequest::new(None, 10).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(page.items()[0].id.as_str(), "arm:follow_joint_trajectory");
    assert_eq!(
        page.items()[1].semantic_id.as_deref(),
        Some("nav2_msgs/action/NavigateToPose")
    );

    let started = workflows
        .start(a2a_lab_sdk::StartWorkflowRequest {
            workflow_id: WorkflowId::new("navigate_to_pose").unwrap(),
            input: JsonObject::parse(r#"{"pose":"dock"}"#).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(started.state, RunState::Submitted);
    graph
        .set_status(
            started.id.as_str().strip_prefix("ros2-").unwrap(),
            Ros2GoalStatus::Executing,
            Some("moving".to_owned()),
        )
        .await
        .unwrap();
    let status = workflows
        .status(GetWorkflowStatusRequest {
            run_id: started.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(status.state, RunState::Working);
    assert_eq!(status.message.as_deref(), Some("moving"));
    graph
        .set_status(
            started.id.as_str().strip_prefix("ros2-").unwrap(),
            Ros2GoalStatus::Succeeded,
            None,
        )
        .await
        .unwrap();
    let finished = workflows
        .status(GetWorkflowStatusRequest { run_id: started.id })
        .await
        .unwrap();
    assert_eq!(finished.state, RunState::Completed);

    let missing = workflows
        .start(a2a_lab_sdk::StartWorkflowRequest {
            workflow_id: WorkflowId::new("missing").unwrap(),
            input: JsonObject::empty(),
        })
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");
}
