use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use a2a_lab_dev_kit::{
    A2aLabError, DvrFrameAtRequest, DvrFrameAtResult, DvrMediaCoverage, DvrMediaPayload,
    DvrMediaSegment, DvrMediaStore, DvrMediaTrack, DvrRetentionBounds, DvrTrackId, PageRequest,
    ReadDvrMediaWindowRequest, TimeRange, UtcTimestamp, VideoCodec, VideoSourceId,
};

const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xD9];
const H264_INIT: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x67];
const H265_INIT: &[u8] = &[0x00, 0x00, 0x00, 0x01, 0x40];

fn timestamp(value: &str) -> UtcTimestamp {
    UtcTimestamp::parse(value).expect("timestamp")
}

fn source_id(id: &str) -> VideoSourceId {
    VideoSourceId::new(id).unwrap()
}

fn track_id(id: &str) -> DvrTrackId {
    DvrTrackId::new(id).unwrap()
}

fn range(start: &str, end: &str) -> TimeRange {
    TimeRange::new(timestamp(start), timestamp(end)).unwrap()
}

#[derive(Clone)]
struct StoredSegment {
    track_id: DvrTrackId,
    range: TimeRange,
    keyframe: bool,
    payload: DvrMediaPayload,
}

pub struct FakeDvrMediaStore {
    source_id: VideoSourceId,
    tracks: Vec<DvrMediaTrack>,
    segments: Vec<StoredSegment>,
    retention: DvrRetentionBounds,
    coverage: DvrMediaCoverage,
    unavailable: bool,
    segments_examined: Arc<AtomicUsize>,
}

#[allow(dead_code)]
impl FakeDvrMediaStore {
    pub fn continuous_mjpeg() -> Self {
        let source = source_id("cam");
        let track = track_id("mjpeg");
        let bounds = range("2024-01-01T00:00:00Z", "2024-01-01T00:00:05Z");
        let track_meta =
            DvrMediaTrack::new(track.clone(), VideoCodec::Mjpeg, Vec::new(), bounds).unwrap();
        let segments = (0..5)
            .map(|second| {
                let start = timestamp(&format!("2024-01-01T00:00:0{second}Z"));
                let end = start.saturating_add_micros(1_000_000);
                StoredSegment {
                    track_id: track.clone(),
                    range: TimeRange::new(start, end).unwrap(),
                    keyframe: true,
                    payload: DvrMediaPayload::MjpegFrame(JPEG.to_vec()),
                }
            })
            .collect();
        let retention = DvrRetentionBounds::new(bounds.start(), bounds.end()).unwrap();
        let coverage = DvrMediaCoverage::new(vec![bounds]).unwrap();
        Self {
            source_id: source,
            tracks: vec![track_meta],
            segments,
            retention,
            coverage,
            unavailable: false,
            segments_examined: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn gapped_h264() -> Self {
        let source = source_id("cam");
        let track = track_id("h264");
        let first = range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z");
        let second = range("2024-01-01T00:00:04Z", "2024-01-01T00:00:06Z");
        let bounds = range("2024-01-01T00:00:00Z", "2024-01-01T00:00:06Z");
        let track_meta =
            DvrMediaTrack::new(track.clone(), VideoCodec::H264, H264_INIT.to_vec(), bounds)
                .unwrap();
        let mut segments = Vec::new();
        for (index, start_text) in ["2024-01-01T00:00:00Z", "2024-01-01T00:00:01Z"]
            .iter()
            .enumerate()
        {
            let start = timestamp(start_text);
            let end = start.saturating_add_micros(1_000_000);
            segments.push(StoredSegment {
                track_id: track.clone(),
                range: TimeRange::new(start, end).unwrap(),
                keyframe: index == 0,
                payload: DvrMediaPayload::EncodedSample {
                    codec: VideoCodec::H264,
                    bytes: vec![0x00, u8::try_from(index).unwrap()],
                },
            });
        }
        for (index, start_text) in ["2024-01-01T00:00:04Z", "2024-01-01T00:00:05Z"]
            .iter()
            .enumerate()
        {
            let start = timestamp(start_text);
            let end = start.saturating_add_micros(1_000_000);
            segments.push(StoredSegment {
                track_id: track.clone(),
                range: TimeRange::new(start, end).unwrap(),
                keyframe: index == 0,
                payload: DvrMediaPayload::EncodedSample {
                    codec: VideoCodec::H264,
                    bytes: vec![0x10, u8::try_from(index).unwrap()],
                },
            });
        }
        let retention = DvrRetentionBounds::new(first.start(), second.end()).unwrap();
        let coverage = DvrMediaCoverage::new(vec![first, second]).unwrap();
        Self {
            source_id: source,
            tracks: vec![track_meta],
            segments,
            retention,
            coverage,
            unavailable: false,
            segments_examined: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn trimmed_retention() -> Self {
        let mut store = Self::long_range_mjpeg();
        store.retention = DvrRetentionBounds::new(
            timestamp("2024-01-01T01:00:00Z"),
            timestamp("2024-01-01T02:00:00Z"),
        )
        .unwrap();
        store
    }

    fn long_range_mjpeg() -> Self {
        let source = source_id("cam");
        let track = track_id("mjpeg");
        let start = timestamp("2024-01-01T00:00:00Z");
        let end = start.saturating_add_micros(7_200_000_000);
        let bounds = TimeRange::new(start, end).unwrap();
        let track_meta =
            DvrMediaTrack::new(track.clone(), VideoCodec::Mjpeg, Vec::new(), bounds).unwrap();
        let segments = (0..7_200)
            .map(|second| {
                let at = start.saturating_add_micros(i64::from(second) * 1_000_000);
                StoredSegment {
                    track_id: track.clone(),
                    range: TimeRange::new(at, at.saturating_add_micros(1_000_000)).unwrap(),
                    keyframe: true,
                    payload: DvrMediaPayload::MjpegFrame(JPEG.to_vec()),
                }
            })
            .collect();
        let retention = DvrRetentionBounds::new(start, end).unwrap();
        let coverage = DvrMediaCoverage::new(vec![bounds]).unwrap();
        Self {
            source_id: source,
            tracks: vec![track_meta],
            segments,
            retention,
            coverage,
            unavailable: false,
            segments_examined: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn h265_track() -> Self {
        let source = source_id("cam");
        let track = track_id("h265");
        let bounds = range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z");
        let track_meta =
            DvrMediaTrack::new(track.clone(), VideoCodec::H265, H265_INIT.to_vec(), bounds)
                .unwrap();
        let segments = vec![StoredSegment {
            track_id: track.clone(),
            range: bounds,
            keyframe: true,
            payload: DvrMediaPayload::EncodedSample {
                codec: VideoCodec::H265,
                bytes: vec![0xAA, 0xBB],
            },
        }];
        let retention = DvrRetentionBounds::new(bounds.start(), bounds.end()).unwrap();
        let coverage = DvrMediaCoverage::new(vec![bounds]).unwrap();
        Self {
            source_id: source,
            tracks: vec![track_meta],
            segments,
            retention,
            coverage,
            unavailable: false,
            segments_examined: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn unavailable() -> Self {
        let mut store = Self::continuous_mjpeg();
        store.unavailable = true;
        store
    }

    pub fn long_range_h264() -> Self {
        let source = source_id("cam");
        let track = track_id("h264");
        let start = timestamp("2024-01-01T00:00:00Z");
        let end = start.saturating_add_micros(7_200_000_000);
        let bounds = TimeRange::new(start, end).unwrap();
        let track_meta =
            DvrMediaTrack::new(track.clone(), VideoCodec::H264, H264_INIT.to_vec(), bounds)
                .unwrap();
        let segments = (0..7_200)
            .map(|second| {
                let at = start.saturating_add_micros(i64::from(second) * 1_000_000);
                StoredSegment {
                    track_id: track.clone(),
                    range: TimeRange::new(at, at.saturating_add_micros(1_000_000)).unwrap(),
                    keyframe: second % 30 == 0,
                    payload: DvrMediaPayload::EncodedSample {
                        codec: VideoCodec::H264,
                        bytes: vec![0x01, u8::try_from(second % 255).unwrap()],
                    },
                }
            })
            .collect();
        let retention = DvrRetentionBounds::new(start, end).unwrap();
        let coverage = DvrMediaCoverage::new(vec![bounds]).unwrap();
        Self {
            source_id: source,
            tracks: vec![track_meta],
            segments,
            retention,
            coverage,
            unavailable: false,
            segments_examined: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn malformed_h264_init() -> Self {
        let mut store = Self::gapped_h264();
        store.tracks[0] = DvrMediaTrack::new(
            track_id("h264"),
            VideoCodec::H264,
            Vec::new(),
            store.tracks[0].bounds(),
        )
        .unwrap();
        store
    }

    pub fn segment_read_hole() -> Self {
        let mut store = Self::gapped_h264();
        store.segments.remove(1);
        store
    }

    fn segments_examined(&self) -> usize {
        self.segments_examined.load(Ordering::SeqCst)
    }

    fn matching_segments(&self, track_id: &DvrTrackId, window: TimeRange) -> Vec<DvrMediaSegment> {
        self.segments_examined.store(0, Ordering::SeqCst);
        let start_index = self
            .segments
            .partition_point(|segment| segment.range.end() <= window.start());
        let mut matched = Vec::new();
        for segment in &self.segments[start_index..] {
            self.segments_examined.fetch_add(1, Ordering::SeqCst);
            if segment.track_id != *track_id {
                continue;
            }
            if segment.range.start() >= window.end() {
                break;
            }
            if ranges_overlap(segment.range, window) {
                matched.push(
                    DvrMediaSegment::new(
                        segment.track_id.clone(),
                        segment.range,
                        segment.keyframe,
                        segment.payload.clone(),
                    )
                    .unwrap(),
                );
            }
        }
        matched
    }
}

fn ranges_overlap(left: TimeRange, right: TimeRange) -> bool {
    left.start() < right.end() && right.start() < left.end()
}

impl DvrMediaStore for FakeDvrMediaStore {
    fn retention_bounds(
        &self,
        source_id: &VideoSourceId,
    ) -> impl std::future::Future<Output = Result<DvrRetentionBounds, A2aLabError>> + Send {
        let result = self.require_ready(source_id).map(|()| self.retention);
        std::future::ready(result)
    }

    fn media_coverage(
        &self,
        source_id: &VideoSourceId,
    ) -> impl std::future::Future<Output = Result<DvrMediaCoverage, A2aLabError>> + Send {
        let result = self
            .require_ready(source_id)
            .map(|()| self.coverage.clone());
        std::future::ready(result)
    }

    fn list_tracks(
        &self,
        source_id: &VideoSourceId,
    ) -> impl std::future::Future<Output = Result<Vec<DvrMediaTrack>, A2aLabError>> + Send {
        let result = self.require_ready(source_id).map(|()| self.tracks.clone());
        std::future::ready(result)
    }

    fn read_media_window(
        &self,
        request: ReadDvrMediaWindowRequest,
    ) -> impl std::future::Future<
        Output = Result<a2a_lab_dev_kit::Page<DvrMediaSegment>, A2aLabError>,
    > + Send {
        let result = (|| {
            self.require_ready(request.source_id())?;
            let matched = self.matching_segments(request.track_id(), request.window());
            paginate_segments(&matched, request.page())
        })();
        std::future::ready(result)
    }

    fn frame_at(
        &self,
        request: DvrFrameAtRequest,
    ) -> impl std::future::Future<Output = Result<DvrFrameAtResult, A2aLabError>> + Send {
        let result = self.frame_at_sync(&request);
        std::future::ready(result)
    }
}

impl FakeDvrMediaStore {
    fn frame_at_sync(&self, request: &DvrFrameAtRequest) -> Result<DvrFrameAtResult, A2aLabError> {
        self.require_ready(request.source_id())?;
        let at = request.at();
        if !self.covered(at) {
            if self.in_gap(at) {
                return Err(A2aLabError::recording_gap("requested instant is in a gap"));
            }
            return Err(A2aLabError::no_recording(
                "requested instant is not retained",
            ));
        }
        let candidates: Vec<_> = self
            .segments
            .iter()
            .filter(|segment| segment.track_id == *request.track_id())
            .filter(|segment| segment.range.contains(at) || segment.range.start() <= at)
            .collect();
        let selected = candidates
            .iter()
            .min_by(|left, right| {
                segment_delta(left, at)
                    .cmp(&segment_delta(right, at))
                    .then_with(|| left.range.start().cmp(&right.range.start()))
            })
            .ok_or_else(|| A2aLabError::not_found("DVR frame", at.to_string()))?;
        if let Some(limit) = request.max_delta_us()
            && segment_delta(selected, at) > limit
        {
            return Err(A2aLabError::not_found("DVR frame", at.to_string()));
        }
        Ok(DvrFrameAtResult::new(
            at,
            selected.range.start(),
            selected.payload.clone(),
        ))
    }
}

impl FakeDvrMediaStore {
    fn require_ready(&self, source_id: &VideoSourceId) -> Result<(), A2aLabError> {
        if self.unavailable {
            return Err(A2aLabError::unavailable("fake DVR store is unavailable"));
        }
        if source_id != &self.source_id {
            return Err(A2aLabError::not_found(
                "video source",
                source_id.to_string(),
            ));
        }
        Ok(())
    }

    fn covered(&self, at: UtcTimestamp) -> bool {
        self.coverage
            .intervals()
            .iter()
            .any(|interval| interval.contains(at))
    }

    fn in_gap(&self, at: UtcTimestamp) -> bool {
        self.coverage
            .discontinuities()
            .iter()
            .any(|gap| gap.gap().contains(at))
    }
}

fn segment_delta(segment: &StoredSegment, at: UtcTimestamp) -> u64 {
    segment.range.start().as_micros().abs_diff(at.as_micros())
}

fn paginate_segments(
    items: &[DvrMediaSegment],
    page: &PageRequest,
) -> Result<a2a_lab_dev_kit::Page<DvrMediaSegment>, A2aLabError> {
    page.check()?;
    let start = match page.cursor() {
        None => 0,
        Some(cursor) => cursor
            .parse::<usize>()
            .map_err(|_| A2aLabError::invalid("cursor", "is not a valid page cursor"))?,
    };
    if start > items.len() {
        return Err(A2aLabError::invalid("cursor", "is past the end"));
    }
    let limit = usize::try_from(page.limit()).unwrap_or(usize::MAX);
    let end = start.saturating_add(limit).min(items.len());
    let next_cursor = (end < items.len()).then(|| end.to_string());
    Ok(a2a_lab_dev_kit::Page::new(
        items[start..end].to_vec(),
        next_cursor,
    ))
}

async fn run_dvr_media_store_contract_suite(store: &impl DvrMediaStore) {
    let source = source_id("cam");
    let retention = store.retention_bounds(&source).await.unwrap();
    assert!(retention.oldest() <= retention.newest());

    let coverage = store.media_coverage(&source).await.unwrap();
    assert!(!coverage.intervals().is_empty());

    let tracks = store.list_tracks(&source).await.unwrap();
    assert!(!tracks.is_empty());
    for track in &tracks {
        assert!(track.bounds().check().is_ok());
        match track.codec() {
            VideoCodec::Mjpeg => assert!(track.codec_initialization().is_empty()),
            VideoCodec::H264 | VideoCodec::H265 => {
                assert!(!track.codec_initialization().is_empty());
            }
        }
    }
}

#[tokio::test]
async fn dvr_media_store_contract_is_storage_only_not_memory_videos() {
    use a2a_lab_dev_kit::MemoryVideos;
    use std::any::{Any, TypeId};
    let store = FakeDvrMediaStore::continuous_mjpeg();
    assert_ne!(
        TypeId::of::<FakeDvrMediaStore>(),
        TypeId::of::<MemoryVideos>()
    );
    let _: &dyn Any = &store;
}

#[tokio::test]
async fn dvr_media_store_contract_continuous_coverage_and_frame_at() {
    let store = FakeDvrMediaStore::continuous_mjpeg();
    run_dvr_media_store_contract_suite(&store).await;

    let source = source_id("cam");
    let track = track_id("mjpeg");
    let coverage = store.media_coverage(&source).await.unwrap();
    assert_eq!(coverage.intervals().len(), 1);
    assert!(coverage.discontinuities().is_empty());

    let frame = store
        .frame_at(DvrFrameAtRequest::new(
            source.clone(),
            track.clone(),
            timestamp("2024-01-01T00:00:02.500000Z"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(frame.actual_at(), timestamp("2024-01-01T00:00:02Z"));
    assert!(matches!(frame.payload(), DvrMediaPayload::MjpegFrame(_)));
}

#[tokio::test]
async fn dvr_media_store_contract_gaps_and_keyframes() {
    let store = FakeDvrMediaStore::gapped_h264();
    let source = source_id("cam");
    let track = track_id("h264");
    let coverage = store.media_coverage(&source).await.unwrap();
    assert_eq!(coverage.intervals().len(), 2);
    assert_eq!(coverage.discontinuities().len(), 1);

    let gap_error = store
        .frame_at(DvrFrameAtRequest::new(
            source.clone(),
            track.clone(),
            timestamp("2024-01-01T00:00:03Z"),
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(gap_error.code(), "recording_gap");

    let page = store
        .read_media_window(
            ReadDvrMediaWindowRequest::new(
                source,
                track,
                range("2024-01-01T00:00:00Z", "2024-01-01T00:00:02Z"),
                PageRequest::new(None, 10).unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.items().len(), 2);
    assert!(page.items()[0].keyframe());
    assert!(!page.items()[1].keyframe());
}

#[tokio::test]
async fn dvr_media_store_contract_retention_trimming() {
    let store = FakeDvrMediaStore::trimmed_retention();
    let source = source_id("cam");
    let retention = store.retention_bounds(&source).await.unwrap();
    assert_eq!(retention.oldest(), timestamp("2024-01-01T01:00:00Z"));
    assert_eq!(retention.newest(), timestamp("2024-01-01T02:00:00Z"));
}

#[tokio::test]
async fn dvr_media_store_contract_long_range_window_reads() {
    let store = FakeDvrMediaStore::long_range_mjpeg();
    let source = source_id("cam");
    let track = track_id("mjpeg");
    let window = range("2024-01-01T01:00:00Z", "2024-01-01T01:00:05Z");
    let page = store
        .read_media_window(
            ReadDvrMediaWindowRequest::new(
                source,
                track,
                window,
                PageRequest::new(None, 3).unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.items().len(), 3);
    assert!(page.next_cursor().is_some());
    assert!(
        store.segments_examined() < store.segments.len(),
        "window read must not require examining every retained segment"
    );
}

#[tokio::test]
async fn dvr_media_store_contract_encoded_codecs_without_persistence_prescription() {
    let h265 = FakeDvrMediaStore::h265_track();
    run_dvr_media_store_contract_suite(&h265).await;
    let track = h265.list_tracks(&source_id("cam")).await.unwrap()[0].clone();
    assert_eq!(track.codec(), VideoCodec::H265);
    assert_eq!(track.codec_initialization(), H265_INIT);
}

#[tokio::test]
async fn dvr_media_store_contract_stable_missing_and_unavailable_errors() {
    let store = FakeDvrMediaStore::continuous_mjpeg();
    let missing = store
        .retention_bounds(&source_id("missing"))
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "not_found");

    let unavailable = FakeDvrMediaStore::unavailable()
        .retention_bounds(&source_id("cam"))
        .await
        .unwrap_err();
    assert_eq!(unavailable.code(), "unavailable");
    assert!(unavailable.availability().is_none());
}
