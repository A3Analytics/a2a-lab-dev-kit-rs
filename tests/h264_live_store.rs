//! Bounded in-memory H.264 live storage independent of DVR persistence pressure.

use std::sync::Arc;
use std::thread;

use a2a_lab_dev_kit::{A2aLabError, DvrTrackId};
use a2a_lab_dev_kit::{
    DvrEnqueueStatus, DvrPersistenceQueue, DvrPictureProperties, H264LiveStore,
    H264LiveStoreLimits, UtcTimestamp, live_h264_edge_anchor, load_live_hls_asset,
    package_h264_live_at,
};

const INIT_A: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67, 0x42];
const INIT_B: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67, 0x64];

fn track(id: &str) -> DvrTrackId {
    DvrTrackId::new(id).expect("track")
}

fn at(second: u32) -> UtcTimestamp {
    UtcTimestamp::parse(&format!("2024-06-01T00:00:{second:02}Z")).expect("timestamp")
}

fn picture() -> DvrPictureProperties {
    DvrPictureProperties::new(640, 480).expect("picture")
}

fn open_store(samples: usize, bytes: usize, retention_us: u64) -> H264LiveStore {
    let store =
        H264LiveStore::new(H264LiveStoreLimits::new(samples, bytes, retention_us).expect("limits"));
    store
        .open_epoch(track("epoch-a"), INIT_A.to_vec(), picture())
        .expect("epoch");
    store
}

#[test]
fn retains_samples_with_epoch_initialization() {
    let store = open_store(8, 256, 60_000_000);
    store
        .append_sample(&track("epoch-a"), at(1), vec![0x11], true)
        .expect("append");
    let snapshot = store.snapshot().expect("snapshot");
    assert_eq!(snapshot.epochs().len(), 1);
    assert_eq!(snapshot.epochs()[0].initialization(), INIT_A);
    assert_eq!(snapshot.samples().len(), 1);
    assert_eq!(snapshot.samples()[0].track_id(), &track("epoch-a"));
    assert_eq!(snapshot.samples()[0].captured_at(), at(1));
}

#[test]
fn evicts_oldest_by_sample_and_byte_bounds() {
    let store = open_store(2, 6, 60_000_000);
    store
        .append_sample(&track("epoch-a"), at(1), vec![0x01, 0x02], true)
        .expect("first");
    let stats = store
        .append_sample(&track("epoch-a"), at(2), vec![0x03, 0x04], true)
        .expect("second");
    assert_eq!(stats.evicted_samples(), 0);
    let stats = store
        .append_sample(&track("epoch-a"), at(3), vec![0x05, 0x06], true)
        .expect("third");
    assert!(stats.evicted_samples() >= 1);
    let snapshot = store.snapshot().expect("snapshot");
    assert_eq!(snapshot.samples().len(), 2);
    assert_eq!(snapshot.retained_bytes(), 4);
    assert_eq!(snapshot.samples()[0].captured_at(), at(2));
}

#[test]
fn evicts_oldest_by_retention_window() {
    let store = open_store(8, 256, 1_500_000);
    store
        .append_sample(&track("epoch-a"), at(1), vec![0x01], true)
        .expect("first");
    let stats = store
        .append_sample(&track("epoch-a"), at(4), vec![0x02], true)
        .expect("second");
    assert_eq!(stats.evicted_samples(), 1);
    let snapshot = store.snapshot().expect("snapshot");
    assert_eq!(snapshot.samples().len(), 1);
    assert_eq!(snapshot.samples()[0].captured_at(), at(4));
}

#[test]
fn live_writes_ignore_saturated_sparse_and_stopped_dvr_queue() {
    let store = open_store(4, 64, 60_000_000);
    let dvr = DvrPersistenceQueue::new(1).expect("queue");
    assert!(dvr.is_sparse().expect("sparse"));
    assert_eq!(
        dvr.try_enqueue(at(1), vec![0xAA]).expect("first"),
        DvrEnqueueStatus::Accepted
    );
    assert_eq!(
        dvr.try_enqueue(at(2), vec![0xBB]).expect("full"),
        DvrEnqueueStatus::Full
    );
    store
        .append_sample(&track("epoch-a"), at(1), vec![0x01], true)
        .expect("live while full");
    dvr.stop();
    assert_eq!(
        dvr.try_enqueue(at(3), vec![0xCC]).expect("stopped"),
        DvrEnqueueStatus::Stopped
    );
    store
        .append_sample(&track("epoch-a"), at(2), vec![0x02], true)
        .expect("live while stopped");
    assert_eq!(store.snapshot().expect("snapshot").samples().len(), 2);
}

#[test]
fn concurrent_readers_and_epoch_transition_stay_deterministic() {
    let store = Arc::new(open_store(8, 128, 60_000_000));
    store
        .append_sample(&track("epoch-a"), at(1), vec![0x01], true)
        .expect("a1");
    store
        .open_epoch(track("epoch-b"), INIT_B.to_vec(), picture())
        .expect("epoch b");
    store
        .append_sample(&track("epoch-b"), at(2), vec![0x02], true)
        .expect("b1");
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let shared = Arc::clone(&store);
            thread::spawn(move || shared.snapshot().expect("snapshot"))
        })
        .collect();
    for handle in readers {
        let snapshot = handle.join().expect("join");
        assert_eq!(snapshot.epochs().len(), 2);
        assert_eq!(snapshot.samples().len(), 2);
        assert_eq!(snapshot.samples()[0].track_id(), &track("epoch-a"));
        assert_eq!(snapshot.samples()[1].track_id(), &track("epoch-b"));
        assert_eq!(snapshot.samples()[1].bytes(), &[0x02]);
    }
}

#[test]
fn reconnect_clears_empty_history_without_dropping_epochs() {
    let store = open_store(4, 64, 60_000_000);
    store
        .append_sample(&track("epoch-a"), at(1), vec![0x01], true)
        .expect("append");
    store.reconnect().expect("reconnect");
    let snapshot = store.snapshot().expect("snapshot");
    assert!(snapshot.samples().is_empty());
    assert_eq!(snapshot.epochs().len(), 1);
    assert_eq!(snapshot.retained_bytes(), 0);
    store
        .append_sample(&track("epoch-a"), at(3), vec![0x03], true)
        .expect("after reconnect");
    assert_eq!(store.snapshot().expect("snapshot").samples().len(), 1);
}

#[test]
fn empty_store_reads_without_samples() {
    let store = H264LiveStore::new(H264LiveStoreLimits::new(4, 64, 60_000_000).expect("limits"));
    let snapshot = store.snapshot().expect("snapshot");
    assert!(snapshot.epochs().is_empty());
    assert!(snapshot.samples().is_empty());
    store
        .open_epoch(track("epoch-a"), INIT_A.to_vec(), picture())
        .expect("open");
    assert!(store.snapshot().expect("snapshot").samples().is_empty());
}

#[test]
fn live_refresh_keeps_segment_identity_and_timeline() {
    let store = open_store(32, 1_000_000, 1_500_000);
    let origin = UtcTimestamp::parse("2024-06-01T00:00:00Z").expect("origin");
    let sample = vec![0, 0, 0, 1, 0x65];
    for step in 0..=5 {
        store
            .append_sample(
                &track("epoch-a"),
                origin.saturating_add_micros(step * 300_000),
                sample.clone(),
                true,
            )
            .expect("sample");
    }
    let snapshot = store.snapshot().expect("snapshot");
    let anchor = live_h264_edge_anchor(&snapshot).expect("anchor");
    let first = package_h264_live_at(&snapshot, anchor, 600_000).expect("playlist");
    let kept = origin.saturating_add_micros(1_200_000).as_micros();
    let kept_uri = format!("seg-0-{kept}.m4s");
    assert!(first.playlist().contains(&kept_uri));
    assert!(!first.playlist().contains(&format!(
        "seg-0-{}.m4s",
        origin.saturating_add_micros(1_500_000).as_micros()
    )));
    assert!(first.playlist().contains("#EXTINF:0.300"));

    store
        .append_sample(
            &track("epoch-a"),
            origin.saturating_add_micros(1_800_000),
            sample,
            true,
        )
        .expect("slide");
    let mut second = package_window(&store);
    let arrived = origin.saturating_add_micros(1_500_000).as_micros();
    let arrived_uri = format!("seg-0-{arrived}.m4s");
    assert!(second.playlist().contains(&kept_uri));
    assert!(second.playlist().contains(&arrived_uri));
    assert!(
        !second
            .playlist()
            .contains(&format!("seg-0-{}.m4s", origin.as_micros()))
    );
    assert_eq!(first.timeline_origin_us(), kept);
    second.pin_timeline_origin(first.timeline_origin_us());

    let snapshot = store.snapshot().expect("snapshot");
    let earlier = load_live_hls_asset(&snapshot, &first, &kept_uri).expect("earlier");
    let later = load_live_hls_asset(&snapshot, &second, &arrived_uri).expect("later");
    assert_eq!(
        decode_time_us(later.bytes()),
        decode_time_us(earlier.bytes()) + 300_000
    );
}

fn package_window(store: &H264LiveStore) -> a2a_lab_dev_kit::EpochHlsSession {
    let snapshot = store.snapshot().expect("snapshot");
    let anchor = live_h264_edge_anchor(&snapshot).expect("anchor");
    package_h264_live_at(&snapshot, anchor, 30_000_000).expect("playlist")
}

fn decode_time_us(bytes: &[u8]) -> u64 {
    let index = bytes
        .windows(4)
        .position(|window| window == b"tfdt")
        .expect("tfdt");
    let start = index + 8;
    u64::from_be_bytes(bytes[start..start + 8].try_into().expect("time"))
}

#[test]
fn rejects_immutable_epoch_replacement() {
    let store = open_store(4, 64, 60_000_000);
    let changed = DvrPictureProperties::new(1280, 720).expect("picture");
    let error = store
        .open_epoch(track("epoch-a"), INIT_B.to_vec(), changed)
        .expect_err("immutable");
    assert!(matches!(error, A2aLabError::Invalid { field: "id", .. }));
}
