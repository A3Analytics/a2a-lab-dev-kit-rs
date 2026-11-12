//! Independent live and DVR cadence selection before color conversion.

use a2a_lab_dev_kit::videos::{
    CadenceDispatch, CaptureClock, CaptureIdentity, CaptureTiming, CapturedColorFrame, ColorMatrix,
    ColorProperties, ColorRange, LiveDvrCadenceSelector, OUTPUT_5FPS_PERIOD_NS,
};
use a2a_lab_dev_kit::{A2aLabError, UtcTimestamp, VideoSourceId};

fn timestamp(seconds: u32) -> UtcTimestamp {
    UtcTimestamp::parse(&format!("2020-01-01T00:00:{seconds:02}Z")).expect("timestamp")
}

fn frame(sequence: u64, utc_seconds: u32, monotonic_ns: u64) -> CapturedColorFrame {
    CapturedColorFrame::yuy2(
        CaptureIdentity::new(VideoSourceId::new("cam-1").expect("source"), sequence),
        CaptureTiming::new(
            timestamp(utc_seconds),
            CaptureClock::monotonic(monotonic_ns),
        ),
        2,
        2,
        4,
        &[16, 128, 16, 128, 16, 128, 16, 128],
        ColorProperties::new(ColorMatrix::Bt709, ColorRange::Limited),
        None,
    )
    .expect("frame")
}

fn conversion_count(dispatches: &[CadenceDispatch]) -> usize {
    dispatches
        .iter()
        .filter(|dispatch| dispatch.requests_conversion())
        .count()
}

fn destinations(dispatches: &[CadenceDispatch]) -> Vec<(bool, bool)> {
    dispatches
        .iter()
        .map(|dispatch| {
            let targets = dispatch.destinations();
            (targets.live(), targets.dvr())
        })
        .collect()
}

fn monotonic_times(dispatches: &[CadenceDispatch]) -> Vec<u64> {
    dispatches
        .iter()
        .map(|dispatch| {
            dispatch
                .frame()
                .timing()
                .clock()
                .monotonic_ns()
                .expect("monotonic")
        })
        .collect()
}

#[test]
fn live_only_and_dvr_only_fire_independently() {
    let live_period = OUTPUT_5FPS_PERIOD_NS;
    let long = live_period * 1_000;
    let mut live_path = LiveDvrCadenceSelector::new(live_period, long, 8).expect("live path");
    live_path.push(frame(1, 0, 0)).expect("origin");
    let due = live_path.push(frame(2, 1, live_period)).expect("live tick");
    assert_eq!(due.len(), 1);
    assert!(matches!(due[0], CadenceDispatch::Live(_)));
    assert_eq!(
        due[0].destinations(),
        a2a_lab_dev_kit::videos::CadenceDestinations::live_only()
    );

    let mut dvr_path = LiveDvrCadenceSelector::new(long, live_period * 2, 8).expect("dvr path");
    dvr_path.push(frame(10, 0, 0)).expect("origin");
    let saved = dvr_path
        .push(frame(11, 2, live_period * 2))
        .expect("dvr tick");
    assert_eq!(saved.len(), 1);
    assert!(matches!(saved[0], CadenceDispatch::Dvr(_)));
    assert_eq!(
        saved[0].destinations(),
        a2a_lab_dev_kit::videos::CadenceDestinations::dvr_only()
    );
}

#[test]
fn neither_destination_requests_conversion_for_superseded_frames() {
    let live_period = OUTPUT_5FPS_PERIOD_NS;
    let mut selector =
        LiveDvrCadenceSelector::new(live_period, live_period * 5, 8).expect("selector");
    selector.push(frame(1, 0, 0)).expect("first");
    let mut conversions = 0_usize;
    for (sequence, offset) in [(2, 40), (3, 80), (4, 120)] {
        let dispatches = selector
            .push(frame(sequence, 0, offset))
            .expect("supersede");
        conversions += conversion_count(&dispatches);
        assert!(
            dispatches.is_empty(),
            "superseded frames must not request conversion"
        );
    }
    assert_eq!(conversions, 0);
    let due = selector.push(frame(5, 0, live_period)).expect("due");
    assert_eq!(conversion_count(&due), 1);
}

#[test]
fn shared_encode_preserves_capture_timestamp() {
    let period = OUTPUT_5FPS_PERIOD_NS;
    let mut selector = LiveDvrCadenceSelector::new(period, period, 8).expect("selector");
    let dispatches = selector.push(frame(1, 7, 0)).expect("origin");
    assert_eq!(dispatches.len(), 1);
    let dispatch = &dispatches[0];
    assert!(matches!(dispatch, CadenceDispatch::Shared(_)));
    assert_eq!(
        dispatch.destinations(),
        a2a_lab_dev_kit::videos::CadenceDestinations::shared()
    );
    assert_eq!(dispatch.frame().timing().captured_at(), timestamp(7));
    assert_eq!(dispatch.frame().timing().clock().monotonic_ns(), Some(0));

    let later = selector.push(frame(2, 8, period)).expect("aligned tick");
    assert_eq!(destinations(&later), vec![(true, true)]);
    assert_eq!(later[0].frame().timing().captured_at(), timestamp(8));
}

#[test]
fn boundaries_gaps_and_cadence_changes_stay_deterministic() {
    let live_period = 200_u64;
    let dvr_period = 600_u64;
    let mut selector = LiveDvrCadenceSelector::new(live_period, dvr_period, 16).expect("selector");
    let times: Vec<u64> = (0..20).map(|index| index * 70).collect();
    let mut all = Vec::new();
    for (sequence, time) in times.iter().copied().enumerate() {
        all.extend(
            selector
                .push(frame(sequence as u64 + 1, 0, time))
                .expect("push"),
        );
    }
    let mono = monotonic_times(&all);
    assert!(mono.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(*mono.first().expect("first"), 0);

    let gap = 10_000_000_000_u64;
    let mut gap_selector =
        LiveDvrCadenceSelector::new(OUTPUT_5FPS_PERIOD_NS, OUTPUT_5FPS_PERIOD_NS * 5, 4)
            .expect("gap selector");
    gap_selector.push(frame(1, 0, 0)).expect("start");
    let gap_dispatches = gap_selector.push(frame(2, 1, gap)).expect("gap");
    assert!(monotonic_times(&gap_dispatches).contains(&gap));

    let mut changed =
        LiveDvrCadenceSelector::new(live_period, dvr_period, 8).expect("cadence change");
    changed.push(frame(1, 0, 0)).expect("start");
    changed
        .set_dvr_period_ns(live_period)
        .expect("tighter dvr cadence");
    let aligned = changed
        .push(frame(2, 2, live_period))
        .expect("after change");
    assert_eq!(destinations(&aligned), vec![(true, true)]);

    let error = LiveDvrCadenceSelector::new(0, dvr_period, 1).expect_err("live period");
    assert!(matches!(
        error,
        A2aLabError::Invalid {
            field: "period",
            ..
        }
    ));
}
