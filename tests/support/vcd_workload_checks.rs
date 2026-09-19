//! Observation checks for the exact consumers timed by benches/vcd.rs.
use std::ops::ControlFlow;

use ondas::{SampleRef, Time, TimeRange, ValueRef};

use super::{fixture_catalog as fixtures, vcd_workloads as workloads};

fn bits(value: ValueRef<'_>) -> String {
    let ValueRef::Bits(bits) = value else {
        panic!("expected bit workload")
    };
    bits.to_string()
}

fn observation(sample: SampleRef<'_>) -> Option<(String, Option<Time>)> {
    match sample {
        SampleRef::Missing { .. } => None,
        SampleRef::Value {
            value, changed_at, ..
        } => Some((bits(value), changed_at)),
        _ => panic!("unexpected sample in bit workload"),
    }
}

#[test]
#[ignore = "requires ONDAS_FIXTURES; run just conformance"]
fn composed_baseline_matches_point_and_sequential_observations() {
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), workloads::FIXTURE);
    let mut wave = ondas::open_with(path, workloads::BACKEND).unwrap();
    let signals = workloads::signals(&wave);
    assert_eq!(signals[2].width(), Some(64));
    assert_ne!(signals[0], signals[1]);
    assert_ne!(signals[1], signals[2]);
    assert_ne!(signals[0], signals[3]);

    // Compare the exact W1 read order, including zero, both window boundaries,
    // and repeated predecessor reads, against independent point queries.
    for range in [workloads::range(0, 100), workloads::range(13000, 13100)] {
        let mut observed = Vec::new();
        workloads::temporal(
            &mut wave.select(&signals).unwrap(),
            range,
            |at, slot, sample| {
                observed.push((at, slot, observation(sample)));
            },
        )
        .unwrap();
        assert!(!observed.is_empty());
        for (at, slot, expected) in observed {
            assert_eq!(
                observation(wave.sample(signals[slot], at).unwrap().as_ref()),
                expected
            );
        }
    }
    for ticks in [
        [13000, 13000, 13000, 13000],
        [13000, 13001, 13002, 13003],
        [13003, 13002, 13001, 13000],
    ] {
        let expected = ticks.map(|t| {
            wave.samples(&signals, Time::from_ticks(t))
                .unwrap()
                .iter()
                .map(|s| observation(s.as_ref()))
                .collect::<Vec<_>>()
        });
        let mut selection = wave.select(&signals).unwrap();
        for (tick, expected) in ticks.into_iter().zip(expected) {
            let mut actual = Vec::new();
            let _ = selection
                .visit_samples(Time::from_ticks(tick), |sample| {
                    actual.push(observation(sample));
                    ControlFlow::<()>::Continue(())
                })
                .unwrap();
            assert_eq!(actual, expected);
        }
    }

    let mut selection = wave.select(&signals).unwrap();
    for (range, mask, count, first) in [
        (workloads::range(0, 13675), 0, 1368, Some(5)),
        (workloads::range(0, 13675), 63, 21, Some(635)),
        (workloads::range(13000, 13675), 63, 1, Some(13435)),
        (TimeRange::all(), u32::MAX, 0, None),
    ] {
        let mut expected = Vec::new();
        let scanned = workloads::conditional_scan(&mut selection, range, mask, |time, value| {
            expected.push((time, bits(value)));
            ControlFlow::Continue(())
        })
        .unwrap();
        assert_eq!(scanned, count);
        assert_eq!(expected.first().map(|(t, _)| t.ticks()), first);
        if count > 1 {
            assert!(
                expected.windows(2).any(|rows| rows[0].1 != rows[1].1),
                "payload must be active at accepted samples"
            );
        }
        for drivers in [&[0][..], &[0, 0][..], &[0, 3][..]] {
            let mut actual = Vec::new();
            let (candidates, accepted) =
                workloads::conditional(&mut selection, range, drivers, mask, |time, value| {
                    actual.push((time, bits(value)));
                    ControlFlow::Continue(())
                })
                .unwrap();
            assert_eq!(actual, expected);
            assert_eq!(accepted, count);
            eprintln!(
                "drivers={drivers:?}, mask={mask}: candidates={candidates}, accepted={accepted}"
            );
            if mask == 63 {
                assert!(candidates > accepted * 20);
            }
            // Early stop then reuse the same selection; compare the complete
            // first accepted observation, not only its timestamp.
            let mut first_row = None;
            let (_, stopped) =
                workloads::conditional(&mut selection, range, drivers, mask, |time, value| {
                    first_row = Some((time, bits(value)));
                    ControlFlow::Break(())
                })
                .unwrap();
            assert_eq!(first_row.as_ref(), expected.first());
            assert_eq!(stopped, u64::from(count > 0));
        }
        eprintln!("range={range:?}, mask={mask}: accepted={scanned}, first={first:?}");
    }
}
