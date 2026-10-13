use std::ops::ControlFlow;

use ondas::{Logic, SampleRef, ScanRef, Time, TimeRange, ValueRef, Waveform};

use super::{fixture_support, root};

fn check(mut wave: Waveform, name: &str) {
    let clock = wave.hierarchy().signal(name).unwrap();
    let end = Time::from_ticks(1_048_576);
    let start = Time::from_ticks(1_048_513);
    let expected = |tick: u64| {
        if tick.is_multiple_of(2) {
            Logic::Zero
        } else {
            Logic::One
        }
    };
    // The first point and duplicate prepared points have the same observation.
    let sample = wave.sample(clock, end).unwrap();
    let mut selection = wave.select(&[clock; 4]).unwrap();
    for (samples, expected_count) in [
        vec![sample],
        selection.samples(end).unwrap(),
        selection.samples(end).unwrap(),
    ]
    .into_iter()
    .zip([1, 4, 4])
    {
        assert_eq!(samples.len(), expected_count);
        for sample in samples {
            let SampleRef::Value {
                value: ValueRef::Bits(bits),
                changed_at,
                ..
            } = sample.as_ref()
            else {
                panic!("clock must have a persistent bit");
            };
            assert_eq!(bits.width(), 1);
            assert_eq!(bits.bit(0), Some(expected(end.ticks())));
            assert_eq!(changed_at, Some(end));
        }
    }
    drop(selection);

    let range = TimeRange::closed(start, end);
    let mut selection = wave.select(&[clock]).unwrap();
    // Check fresh and prepared scans independently of the benchmark consumer.
    for _ in 0..2 {
        let mut count = 0;
        let result = selection
            .scan(range, |record| {
                match record {
                    ScanRef::Initial {
                        value: ValueRef::Bits(bits),
                        changed_at,
                        ..
                    } => {
                        assert_eq!(count, 0);
                        assert_eq!(bits.bit(0), Some(expected(start.ticks() - 1)));
                        assert_eq!(changed_at, Some(Time::from_ticks(start.ticks() - 1)));
                    }
                    ScanRef::Change {
                        time,
                        value: ValueRef::Bits(bits),
                        ..
                    } => {
                        assert_eq!(time.ticks(), start.ticks() + count - 1);
                        assert_eq!(bits.bit(0), Some(expected(time.ticks())));
                    }
                    _ => panic!("clock scan must have bit values"),
                }
                count += 1;
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
        assert!(result.is_continue());
        assert_eq!(count, 65);
    }
}

#[cfg(feature = "shm-lib")]
#[test]
#[ignore = "requires real SHM runtime and installed fixtures"]
fn shm_matched_clock_observations() {
    let (path, _) = fixture_support::load_artifact(&root(), "shm/shm0081-history-long");
    check(ondas::open_with(path, "shm-lib").unwrap(), "top.phase_0");
}

#[cfg(feature = "fsdb-lib")]
#[test]
#[ignore = "requires real FSDB runtime and installed fixtures"]
fn fsdb_matched_clock_observations() {
    let (path, _) = fixture_support::load_artifact(&root(), "fsdb/fsdb0011-history-long");
    let Some(wave) = super::fsdb_compatibility::open(ondas::open_with(&path, "fsdb-lib"), &path)
    else {
        return;
    };
    check(wave, "top.clock");
}
