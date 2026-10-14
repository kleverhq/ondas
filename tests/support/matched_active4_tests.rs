use std::{collections::HashSet, ops::ControlFlow};

use ondas::{Encoding, Logic, SampleRef, ScanRef, Time, TimeRange, ValueRef, Waveform};

use super::{fixture_support, root};

fn check(mut wave: Waveform, names: [&str; 4]) {
    let signals = names.map(|name| wave.hierarchy().signal(name).unwrap());
    assert_eq!(
        signals
            .map(|signal| signal.base())
            .into_iter()
            .collect::<HashSet<_>>()
            .len(),
        4
    );
    for signal in signals {
        assert_eq!(signal.encoding(), Encoding::Bits { width: 1 });
    }
    let time = Time::from_ticks(4096);
    let samples = wave.samples(&signals, time).unwrap();
    assert_eq!(samples.len(), 4);
    for (sample, signal) in samples.iter().zip(signals) {
        let SampleRef::Value {
            signal: sampled,
            value: ValueRef::Bits(bits),
            changed_at,
        } = sample.as_ref()
        else {
            panic!("active scalar point must have bits");
        };
        assert_eq!(sampled, signal);
        assert_eq!(bits.width(), 1);
        assert_eq!(bits.bit(0), Some(Logic::Zero));
        assert_eq!(changed_at, Some(time));
    }
    let mut counts = [0_u64; 4];
    let outcome = wave
        .select(&signals)
        .unwrap()
        .scan(TimeRange::closed(Time::ZERO, time), |record| {
            let ScanRef::Change {
                signal,
                time,
                value: ValueRef::Bits(bits),
            } = record
            else {
                panic!("active scalar prefix must contain changes");
            };
            let index = signals
                .iter()
                .position(|selected| *selected == signal)
                .unwrap();
            assert_eq!(time.ticks(), counts[index]);
            assert_eq!(bits.width(), 1);
            assert_eq!(
                bits.bit(0),
                Some(if time.ticks() % 2 == 0 {
                    Logic::Zero
                } else {
                    Logic::One
                })
            );
            counts[index] += 1;
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert!(outcome.is_continue());
    assert_eq!(counts, [4097; 4]);
}

#[test]
#[ignore = "requires installed fixtures"]
fn fst_matched_active4_observations() {
    let (path, _) = fixture_support::load_artifact(&root(), "fst/fst0087-sparse-dense-active");
    check(
        ondas::open_with(path, "fst-lib").unwrap(),
        [
            "top.dense.bit_00",
            "top.dense.bit_01",
            "top.dense.bit_63",
            "top.sparse.bit_00",
        ],
    );
}

#[cfg(feature = "shm-lib")]
#[test]
#[ignore = "requires real SHM runtime and installed fixtures"]
fn shm_matched_active4_observations() {
    let (path, _) = fixture_support::load_artifact(&root(), "shm/shm0082-sparse-dense-bank");
    check(
        ondas::open_with(path, "shm-lib").unwrap(),
        [
            "top.bank[0].dense",
            "top.bank[2].dense",
            "top.bank[4].dense",
            "top.bank[6].dense",
        ],
    );
}
