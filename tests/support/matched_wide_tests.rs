use std::ops::ControlFlow;

use ondas::{BitsRef, Logic, SampleRef, ScanRef, Time, TimeRange, ValueRef, Waveform};

use super::{fixture_support, root};

fn bits_equal(bits: BitsRef<'_>, width: u32, low: Logic) {
    assert_eq!(bits.width(), width);
    for bit in 0..width {
        assert_eq!(
            bits.bit(bit),
            Some(if bit == 0 { low } else { Logic::Zero })
        );
    }
}

fn check(mut wave: Waveform) {
    let control = wave.hierarchy().signal("top.control").unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    let end = Time::from_ticks(4096);
    let start = Time::from_ticks(4088);
    for (signal, width, active) in [
        (control, 1, true),
        (wide, 4096, true),
        (wide.slice(4095, 1).unwrap(), 4095, false),
    ] {
        let sample = wave.sample(signal, end).unwrap();
        let SampleRef::Value {
            signal: sampled,
            value: ValueRef::Bits(bits),
            changed_at,
        } = sample.as_ref()
        else {
            panic!("matched wide point must have bits");
        };
        assert_eq!(sampled, signal);
        bits_equal(bits, width, Logic::Zero);
        assert_eq!(changed_at, active.then_some(end));
        let mut selection = wave.select(&[signal]).unwrap();
        // Both fresh and repeated scans have identical independent expectations.
        for _ in 0..2 {
            let mut count = 0;
            let outcome = selection
                .scan(TimeRange::closed(start, end), |record| {
                    match record {
                        ScanRef::Initial {
                            signal: scanned,
                            value: ValueRef::Bits(bits),
                            changed_at,
                        } => {
                            assert_eq!(scanned, signal);
                            assert_eq!(count, 0);
                            bits_equal(bits, width, if active { Logic::One } else { Logic::Zero });
                            assert_eq!(changed_at, active.then_some(Time::from_ticks(4087)));
                        }
                        ScanRef::Change {
                            signal: scanned,
                            time,
                            value: ValueRef::Bits(bits),
                        } => {
                            assert!(active && count > 0);
                            assert_eq!(scanned, signal);
                            assert_eq!(time.ticks(), 4088 + count - 1);
                            let low = if time.ticks() % 2 == 0 {
                                Logic::Zero
                            } else {
                                Logic::One
                            };
                            bits_equal(bits, width, low);
                        }
                        _ => panic!("matched wide scan must have bits"),
                    }
                    count += 1;
                    ControlFlow::<()>::Continue(())
                })
                .unwrap();
            assert!(outcome.is_continue());
            assert_eq!(count, if active { 10 } else { 1 });
        }
    }
}

#[test]
#[ignore = "requires installed fixtures"]
fn fst_matched_wide_observations() {
    let (path, _) = fixture_support::load_artifact(&root(), "fst/fst0083-wide-compact-toggle");
    check(ondas::open_with(path, "fst-lib").unwrap());
}

#[cfg(feature = "shm-lib")]
#[test]
#[ignore = "requires real SHM runtime and installed fixtures"]
fn shm_matched_wide_observations() {
    let (path, _) = fixture_support::load_artifact(&root(), "shm/shm0084-wide-compact-toggle");
    check(ondas::open_with(path, "shm-lib").unwrap());
}

#[cfg(feature = "fsdb-lib")]
#[test]
#[ignore = "requires real FSDB runtime and installed fixtures"]
fn fsdb_matched_wide_observations() {
    let (path, _) = fixture_support::load_artifact(&root(), "fsdb/fsdb0015-wide-compact-toggle");
    let Some(wave) = super::fsdb_compatibility::open(ondas::open_with(&path, "fsdb-lib"), &path)
    else {
        return;
    };
    check(wave);
}
