use std::ops::ControlFlow;

use ondas::{Encoding, Logic, SampleRef, ScanRef, Time, TimeRange, ValueRef, Waveform};

use super::{fixture_support, root};

fn check(mut wave: Waveform, fixture: &str) {
    let signal = wave.hierarchy().signal("top.probe_constant").unwrap();
    assert_eq!(signal.encoding(), Encoding::Bits { width: 1 });
    let end = Time::from_ticks(1_048_576);
    let mut count = 0;
    let outcome = wave
        .select(&[signal])
        .unwrap()
        .scan(TimeRange::closed(Time::ZERO, end), |record| {
            let ScanRef::Change {
                signal: actual,
                time,
                value: ValueRef::Bits(bits),
            } = record
            else {
                panic!("constant prefix must contain only its initialization");
            };
            assert_eq!(count, 0);
            assert_eq!(actual, signal);
            assert_eq!(time, Time::ZERO);
            assert_eq!(bits.width(), 1);
            assert_eq!(bits.bit(0), Some(Logic::Zero));
            count += 1;
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert!(outcome.is_continue());
    assert_eq!(count, 1);

    let sample = wave.sample(signal, end).unwrap();
    let SampleRef::Value {
        signal: actual,
        value: ValueRef::Bits(bits),
        changed_at,
    } = sample.as_ref()
    else {
        panic!("constant point must have bits");
    };
    assert_eq!(actual, signal);
    assert_eq!(bits.width(), 1);
    assert_eq!(bits.bit(0), Some(Logic::Zero));
    assert_eq!(changed_at, None);
    println!(
        "{fixture}: scopes={} variables={} signals={}",
        wave.hierarchy().scopes().count(),
        wave.hierarchy().variables().count(),
        wave.hierarchy().signals().count(),
    );
}

#[test]
#[ignore = "requires installed fixtures"]
fn fst_matched_constant_observations() {
    let fixture = "fst/fst0084-topology-small";
    let (path, _) = fixture_support::load_artifact(&root(), fixture);
    check(ondas::open_with(path, "fst-lib").unwrap(), fixture);
}

#[cfg(feature = "shm-lib")]
#[test]
#[ignore = "requires real SHM runtime and installed fixtures"]
fn shm_matched_constant_observations() {
    let fixture = "shm/shm0081-history-long";
    let (path, _) = fixture_support::load_artifact(&root(), fixture);
    check(ondas::open_with(path, "shm-lib").unwrap(), fixture);
}

#[cfg(feature = "fsdb-lib")]
#[test]
#[ignore = "requires real FSDB runtime and installed fixtures"]
fn fsdb_matched_constant_observations() {
    let fixture = "fsdb/fsdb0012-topology-small";
    let (path, _) = fixture_support::load_artifact(&root(), fixture);
    let Some(wave) = super::fsdb_compatibility::open(ondas::open_with(&path, "fsdb-lib"), &path)
    else {
        return;
    };
    check(wave, fixture);
}
