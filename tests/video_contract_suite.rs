#![allow(
    clippy::duplicate_mod,
    reason = "DVR fixture modules also define tests with identities tied to their original module"
)]

mod tck_malformed_videos;
mod video_backend_seams;
mod video_contracts;
mod video_tests;
mod video_trick_play_8x;
