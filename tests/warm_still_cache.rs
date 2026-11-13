//! Focused warm still cache lifecycle and retention tests.

use a2a_lab_dev_kit::videos::WarmStillCache;

const JPEG: &[u8] = &[0xFF, 0xD8, 0xAA, 0xFF, 0xD9];

#[test]
fn remembers_only_newer_sequences() {
    let cache = WarmStillCache::new();
    cache.remember("cam", 1, JPEG.to_vec());
    cache.remember("cam", 0, vec![0xFF]);
    let still = cache.latest("cam").expect("still");
    assert_eq!(still.sequence(), 1);
    cache.remember("cam", 2, vec![JPEG[0], 0xD8, 0xBB, 0xFF, 0xD9]);
    assert_eq!(cache.latest("cam").unwrap().sequence(), 2);
}

#[test]
fn invalidate_blocks_late_writes() {
    let cache = WarmStillCache::new();
    cache.remember("cam", 1, JPEG.to_vec());
    cache.invalidate("cam");
    cache.remember("cam", 5, JPEG.to_vec());
    assert!(cache.latest("cam").is_none());
}

#[test]
fn reset_source_accepts_frames_again() {
    let cache = WarmStillCache::new();
    cache.remember("cam", 1, JPEG.to_vec());
    cache.invalidate("cam");
    cache.reset_source("cam");
    cache.remember("cam", 1, JPEG.to_vec());
    assert!(cache.latest("cam").is_some());
}

#[test]
fn clear_all_isolates_cases() {
    let cache = WarmStillCache::new();
    cache.remember("a", 1, JPEG.to_vec());
    cache.remember("b", 1, JPEG.to_vec());
    cache.clear_all();
    assert!(cache.latest("a").is_none());
    assert!(cache.latest("b").is_none());
}
