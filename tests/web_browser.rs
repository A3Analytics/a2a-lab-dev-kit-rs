#![allow(
    clippy::duplicate_mod,
    reason = "Browser and playback harness modules define tests scoped to each original suite"
)]

mod web_ui_agent;
mod web_ui_browser;
mod web_ui_chat;
mod web_ui_chrome;
mod web_ui_h264_live;
mod web_ui_hls_mse;
mod web_ui_images;
mod web_ui_images_closeout;
mod web_ui_images_visual;
mod web_ui_loading;
mod web_ui_logs;
mod web_ui_metrics;
mod web_ui_parity;
mod web_ui_tasks_ui;
mod web_ui_video_playback;
mod web_ui_video_speed;
mod web_ui_videos;
mod web_ui_videos_closeout;
mod web_ui_videos_e2e;
mod web_ui_videos_interactions;
mod web_ui_videos_route;
mod web_ui_videos_semantic;
mod web_ui_videos_visual;
mod web_ui_visual;
