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

    // Quiet ticks and a quiet window still have state. Read all operations
    // through the same selection, without choosing a strategy by format.
    let range = TimeRange::closed(Time::from_ticks(1), end);
    let mut selected = wave.select(&[signal, signal]).unwrap();
    let saved = selected.traces(range).unwrap();
    assert_eq!(saved.len(), 2);
    for trace in &saved {
        assert!(trace.initial().is_some());
        assert!(trace.changes().is_empty());
    }
    let times = [end, Time::from_ticks(1), end, Time::from_ticks(17)];
    let rows = selected.samples_at(&times).unwrap();
    assert_eq!(rows.len(), times.len());
    for ((time, samples), expected_time) in rows.iter().zip(times) {
        assert_eq!(*time, expected_time);
        assert_eq!(samples.len(), 2);
        for (sample, trace) in samples.iter().zip(&saved) {
            for sample in [sample.as_ref(), trace.sample(*time).unwrap()] {
                assert_constant(sample, signal);
            }
        }
    }
    let mut records = 0;
    let outcome = selected
        .scan_each(range, |slot, record| {
            assert_eq!(slot, records);
            assert!(matches!(record, ScanRef::Initial { .. }));
            records += 1;
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert_eq!(outcome, ControlFlow::Continue(()));
    assert_eq!(records, 2);
    let _ = selected
        .query(range, &[0], |ctx| {
            for time in [ctx.time(), Time::from_ticks(ctx.time().ticks() - 1)] {
                let _ = ctx.visit_samples(time, &[1, 0, 1], |_, sample| {
                    assert_constant(sample, signal);
                    Ok(ControlFlow::<()>::Continue(()))
                })?;
            }
            Ok(ControlFlow::<()>::Continue(()))
        })
        .unwrap();
    println!(
        "{fixture}: scopes={} variables={} signals={}",
        wave.hierarchy().scopes().count(),
        wave.hierarchy().variables().count(),
        wave.hierarchy().signals().count(),
    );
}

fn assert_constant(sample: SampleRef<'_>, signal: ondas::Signal) {
    let SampleRef::Value {
        signal: actual,
        value: ValueRef::Bits(bits),
        changed_at,
    } = sample
    else {
        panic!("constant observation must have bits");
    };
    assert_eq!(actual, signal);
    assert_eq!(bits.width(), 1);
    assert_eq!(bits.bit(0), Some(Logic::Zero));
    assert_eq!(changed_at, None);
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
