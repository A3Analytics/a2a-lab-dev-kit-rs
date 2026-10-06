---
id: doc-15
title: Expose ROS 2 actions as tasks
type: guide
audience: public
created_date: '2026-10-06 20:11'
---

# Expose ROS 2 actions as tasks

## Run the example

Run the in-process Robot Operating System 2 (ROS 2) task example from the repository root. The program is `examples/ros2_tasks.rs`. It builds `MemoryRos2`, advertises `/navigate_to_pose`, and calls `Ros2Tasks`.

1. Change to the repository root.
2. Run this command:

   ```bash
   mise exec -- cargo run --example ros2_tasks
   ```

3. Confirm the example prints this output:

   ```text
   advertised navigate_to_pose (nav2_msgs/action/NavigateToPose)
   started ros2-goal-1 Submitted
   goal goal-1 Working message moving
   goal goal-1 Completed
   ```

The first line reports task `navigate_to_pose` and type `nav2_msgs/action/NavigateToPose`. The second line shows run `ros2-goal-1` in state `Submitted`. The third line shows goal `goal-1` in state `Working` with message `moving`. The fourth line shows goal `goal-1` in state `Completed`.

The ROS 2 reference defines task ids and goal status.

## Related

- [A2A-LAB devkit overview](<../../overview/doc-10 - Lab-dev-kit-overview.md>)
- [ROS 2](<../../reference/ros-2/doc-9 - ROS-2.md>)
- [ROS 2 task example](../../../../examples/ros2_tasks.rs)
