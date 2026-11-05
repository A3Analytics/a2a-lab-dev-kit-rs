//! Fresh-frame selection at an output rate, with no camera and no codec.

use a2a_lab_dev_kit::videos::{
    CaptureClock, CaptureFormat, CaptureIdentity, CaptureTiming, CapturedColorFrame, ColorMatrix,
    ColorProperties, ColorRange, FrameRejection, FreshFrameGate, OUTPUT_5FPS_PERIOD_NS,
};
use a2a_lab_dev_kit::{A2aLabError, UtcTimestamp, VideoSourceId};

fn timestamp(seconds: u32) -> UtcTimestamp {
    UtcTimestamp::parse(&format!("2020-01-01T00:00:{seconds:02}Z")).expect("timestamp")
}

fn frame(
    sequence: u64,
    utc_seconds: u32,
    clock: Option<&str>,
    monotonic_ns: Option<u64>,
) -> CapturedColorFrame {
    let clock = match clock {
        Some(name) => CaptureClock::identified(name, monotonic_ns).expect("clock"),
        None => CaptureClock::monotonic(monotonic_ns.expect("monotonic")),
    };
    CapturedColorFrame::yuy2(
        CaptureIdentity::new(VideoSourceId::new("cam-1").expect("source"), sequence),
        CaptureTiming::new(timestamp(utc_seconds), clock),
        2,
        2,
        4,
        &[16, 128, 16, 128, 16, 128, 16, 128],
        ColorProperties::new(ColorMatrix::Bt709, ColorRange::Limited),
        None,
    )
    .expect("frame")
}

fn pull_times(gate: &mut FreshFrameGate) -> Vec<u64> {
    let mut times = Vec::new();
    while let Some(frame) = gate.pull() {
        times.push(frame.timing().clock().monotonic_ns().expect("monotonic"));
    }
    times
}

fn tick_choices(times: &[u64], period: u64) -> Vec<u64> {
    let origin = times[0];
    let mut used = vec![false; times.len()];
    let mut chosen = Vec::new();
    let mut tick = origin;
    let last = *times.last().expect("times");
    while tick <= last {
        let mut choice = None;
        for (index, time) in times.iter().enumerate() {
            if !used[index] && *time <= tick {
                choice = Some(index);
            }
        }
        if let Some(index) = choice {
            used[index] = true;
            chosen.push(times[index]);
        }
        let Some(next) = tick.checked_add(period) else {
            break;
        };
        tick = next;
    }
    chosen
}

#[test]
fn five_fps_keeps_every_native_frame_and_thirty_to_five_uses_real_frames() {
    let mut gate = FreshFrameGate::at_5fps(8).expect("gate");
    for (index, time) in [0, OUTPUT_5FPS_PERIOD_NS, OUTPUT_5FPS_PERIOD_NS * 2]
        .into_iter()
        .enumerate()
    {
        let rejected = gate
            .push(frame(index as u64, 0, None, Some(time)))
            .expect("push");
        assert!(rejected.is_empty(), "{rejected:?}");
    }
    assert_eq!(
        pull_times(&mut gate),
        vec![0, OUTPUT_5FPS_PERIOD_NS, OUTPUT_5FPS_PERIOD_NS * 2]
    );

    let mut gate = FreshFrameGate::at_5fps(16).expect("gate");
    let step = 1_000_000_000 / 30;
    let mut source = Vec::new();
    let mut time = 0_u64;
    for index in 0..31 {
        source.push(time);
        gate.push(frame(index, 1, None, Some(time))).expect("push");
        time = time.saturating_add(step);
    }
    let selected = pull_times(&mut gate);
    assert_eq!(selected, tick_choices(&source, OUTPUT_5FPS_PERIOD_NS));
    assert!(selected.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(selected.len(), 5);
    assert_eq!(*selected.first().expect("first"), 0);
}

#[test]
fn non_divisor_rate_matches_integer_ticks_without_drift() {
    let period = 200;
    let source: Vec<u64> = (0..200).map(|index| index * 70).collect();
    let mut gate = FreshFrameGate::new(period, 256).expect("gate");
    for (index, time) in source.iter().copied().enumerate() {
        gate.push(frame(index as u64, 0, None, Some(time)))
            .expect("push");
    }
    let selected = pull_times(&mut gate);
    let expected = tick_choices(&source, period);
    assert_eq!(selected, expected);
    assert!(selected.len() > 50);
    assert_eq!(selected.last().copied(), expected.last().copied());
}

#[test]
fn slow_consumer_keeps_the_newest_frames_and_counts_drops() {
    let mut gate = FreshFrameGate::at_5fps(2).expect("gate");
    let mut dropped = Vec::new();
    for index in 0..4 {
        let rejected = gate
            .push(frame(index, 0, None, Some(index * OUTPUT_5FPS_PERIOD_NS)))
            .expect("push");
        dropped.extend(rejected);
    }
    assert_eq!(gate.ready_len(), 2);
    assert_eq!(gate.dropped(), 2);
    assert!(
        dropped
            .iter()
            .all(|frame| frame.reason() == FrameRejection::DroppedOldest)
    );
    assert_eq!(
        dropped
            .iter()
            .map(|frame| frame.sequence())
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    let kept: Vec<u64> = std::iter::from_fn(|| gate.pull())
        .map(|frame| frame.identity().sequence())
        .collect();
    assert_eq!(kept, vec![2, 3]);
}

#[test]
fn timestamps_survive_and_resets_do_not_invent_frames() {
    let mut gate = FreshFrameGate::at_5fps(8).expect("gate");
    gate.push(frame(7, 40, Some("device"), Some(0)))
        .expect("first");
    gate.push(frame(8, 3, Some("device"), Some(OUTPUT_5FPS_PERIOD_NS)))
        .expect("second");
    let first = gate.pull().expect("selected");
    let second = gate.pull().expect("selected");
    assert_eq!(first.identity().sequence(), 7);
    assert_eq!(second.identity().sequence(), 8);
    assert_eq!(first.timing().captured_at(), timestamp(40));
    assert_eq!(second.timing().captured_at(), timestamp(3));
    assert!(gate.pull().is_none());

    let rejected = gate
        .push(frame(9, 4, Some("device"), Some(0)))
        .expect("reset");
    assert_eq!(rejected.len(), 0);
    let restarted = gate.pull().expect("restart");
    assert_eq!(restarted.identity().sequence(), 9);
    assert!(gate.pull().is_none());

    gate.push(frame(10, 5, Some("device"), Some(50)))
        .expect("hold");
    let reset = gate
        .push(frame(11, 5, Some("replug"), Some(0)))
        .expect("replug");
    assert_eq!(reset[0].sequence(), 10);
    assert_eq!(reset[0].reason(), FrameRejection::ClockReset);
    assert_eq!(gate.pull().expect("new clock").identity().sequence(), 11);
    assert!(gate.pull().is_none());

    let mut gate = FreshFrameGate::at_5fps(8).expect("gate");
    gate.push(frame(1, 0, None, Some(0))).expect("start");
    gate.push(frame(2, 0, None, Some(10_000_000_000)))
        .expect("gap");
    assert_eq!(pull_times(&mut gate), vec![0, 10_000_000_000]);
}

#[test]
fn rejections_are_visible_before_pull_and_shutdown_drops_the_open_candidate() {
    let mut gate = FreshFrameGate::at_5fps(4).expect("gate");
    let mut decoded = 0_u32;
    gate.push(frame(1, 0, None, Some(0))).expect("start");
    let rejected = gate.push(frame(2, 0, None, Some(50))).expect("early");
    assert!(rejected.is_empty());
    let rejected = gate.push(frame(3, 0, None, Some(80))).expect("fresher");
    assert_eq!(rejected[0].sequence(), 2);
    assert_eq!(rejected[0].reason(), FrameRejection::Superseded);
    assert_eq!(decoded, 0);
    assert_eq!(gate.stale(), 1);

    let closed = gate.shutdown();
    assert_eq!(closed[0].sequence(), 3);
    assert_eq!(closed[0].reason(), FrameRejection::Shutdown);
    let selected = gate.pull().expect("already selected");
    decoded = decoded.saturating_add(1);
    assert_eq!(selected.identity().sequence(), 1);
    assert_eq!(selected.format(), CaptureFormat::Yuy2);
    assert!(gate.pull().is_none());
    let after = gate
        .push(frame(4, 0, None, Some(OUTPUT_5FPS_PERIOD_NS)))
        .expect("closed");
    assert_eq!(after[0].reason(), FrameRejection::Shutdown);
    assert_eq!(decoded, 1);
}

#[test]
fn missing_monotonic_time_and_zero_period_are_rejected() {
    let mut gate = FreshFrameGate::at_5fps(1).expect("gate");
    let error = gate
        .push(frame(1, 0, Some("device"), None))
        .expect_err("clock");
    assert!(matches!(error, A2aLabError::Invalid { field: "clock", .. }));
    let error = FreshFrameGate::new(0, 1).expect_err("period");
    assert!(matches!(
        error,
        A2aLabError::Invalid {
            field: "period",
            ..
        }
    ));
}
