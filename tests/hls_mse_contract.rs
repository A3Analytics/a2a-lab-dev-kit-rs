//! Packaged init segments must expose avcC/hvcC for browser codec probing.

use a2a_lab_dev_kit::{
    DvrMediaPackager, DvrTrackId, HLS_INIT_RELATIVE_PATH, HLS_PLAYLIST_RELATIVE_PATH, UtcTimestamp,
    VideoSourceId,
};

#[path = "support/dvr_store_contract.rs"]
mod dvr_store_contract;

use dvr_store_contract::FakeDvrMediaStore;

#[tokio::test]
async fn packaged_h264_init_carries_avcc_for_mse_probe() {
    let store = FakeDvrMediaStore::gapped_h264();
    let session = DvrMediaPackager::default()
        .package_hls_at(
            &store,
            "/media/video-clips/probe/",
            &VideoSourceId::new("cam").unwrap(),
            &DvrTrackId::new("h264").unwrap(),
            UtcTimestamp::parse("2024-01-01T00:00:00Z").unwrap(),
        )
        .await
        .unwrap();
    let init = DvrMediaPackager::default()
        .load_hls_asset(&store, &session, HLS_INIT_RELATIVE_PATH)
        .await
        .unwrap();
    let bytes = init.bytes();
    assert!(bytes.windows(4).any(|window| window == b"avcC"));
    let playlist = DvrMediaPackager::default()
        .load_hls_asset(&store, &session, HLS_PLAYLIST_RELATIVE_PATH)
        .await
        .unwrap();
    let text = std::str::from_utf8(playlist.bytes()).unwrap();
    assert!(text.contains("init.mp4"));
    assert!(text.contains(".m4s"));
}
