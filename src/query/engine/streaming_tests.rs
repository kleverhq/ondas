use super::*;
use crate::backends::{
    Reader,
    generated::{self, Counts, PAYLOAD, Probe},
};
use std::cell::Cell;
use std::collections::BTreeSet;

thread_local! {
    static TICK_WORK: Cell<(usize, usize, usize)> = const { Cell::new((0, 0, 0)) };
}

pub(super) fn setup_visits(n: usize) {
    TICK_WORK.with(|work| work.update(|(setup, compare, commit)| (setup + n, compare, commit)));
}

pub(super) fn comparison_visit() {
    TICK_WORK.with(|work| work.update(|(setup, compare, commit)| (setup, compare + 1, commit)));
}

pub(super) fn commit_visit() {
    TICK_WORK.with(|work| work.update(|(setup, compare, commit)| (setup, compare, commit + 1)));
}

fn tick_work() -> (usize, usize, usize) {
    TICK_WORK.with(Cell::get)
}

pub(super) fn probe(reader: &Reader) -> Option<Probe> {
    if let Reader::Generated(reader) = reader {
        Some(reader.probe.clone())
    } else {
        None
    }
}

// Observe real engine slots after every raw record, not an assumed bound.
pub(super) fn observe_pending(probe: &Option<Probe>, slots: &[Slot]) {
    let Some(probe) = probe else { return };
    let mut records = 0;
    let mut bytes = 0;
    for slot in slots {
        for value in slot
            .state
            .as_ref()
            .map(|state| &state.value)
            .into_iter()
            .chain(slot.pending.as_ref())
        {
            records += 1;
            bytes += match value {
                Value::Real(_) => 8,
                Value::String(value) => value.len(),
                _ => panic!("unexpected generated persistent class"),
            };
        }
        records += usize::from(slot.events > 0);
    }
    let mut counts = probe.lock().unwrap();
    counts.max_pending_records = counts.max_pending_records.max(records);
    counts.max_pending_bytes = counts.max_pending_bytes.max(bytes);
    counts.max_slots = counts.max_slots.max(slots.len());
}

fn fixture(ticks: u64, fail_at: Option<(u64, usize)>) -> (Waveform, Vec<Signal>, Probe) {
    let mut wave = Waveform::memory(
        vec![
            Encoding::Real,
            Encoding::Event,
            Encoding::String,
            Encoding::String,
        ],
        vec![],
        None,
    );
    let signals = wave.hierarchy().signals().collect::<Vec<_>>();
    let probe = Probe::default();
    wave.reader = Reader::Generated(generated::Reader {
        ticks,
        fail_at,
        probe: probe.clone(),
    });
    (wave, signals, probe)
}

fn counts(probe: &Probe) -> Counts {
    probe.lock().unwrap().clone()
}

#[test]
fn sparse_active_finalization_visits_only_touched_slots() {
    let mut text = String::new();
    for i in 0..64 {
        text.push_str(&format!(
            "$var wire {} v{i} n{i} $end ",
            if i == 0 { 2 } else { 1 }
        ));
    }
    text.push_str("$var event 1 e trigger $end $enddefinitions $end #0 ");
    for i in 0..64 {
        text.push_str(&format!("{}v{i} ", if i == 0 { "b00 " } else { "0" }));
    }
    text.push_str("#1 b01 v0 #2 b00 v0 b01 v0 1e 1e #3 1v63 ");
    let mut wave =
        crate::open_bytes_with("sparse.vcd", text.into_bytes().into(), "vcd-native").unwrap();
    let signals = (0..64)
        .map(|i| wave.hierarchy().signal(&format!("n{i}")).unwrap())
        .collect::<Vec<_>>();
    let event = wave.hierarchy().signal("trigger").unwrap();
    let mut selected = vec![signals[0], signals[0].slice(0, 0).unwrap(), signals[0]];
    selected.extend_from_slice(&signals[1..]);
    selected.push(event);
    let event_index = selected.len() - 1;
    let quiet_index = event_index - 1;
    let mut selection = wave.select(&selected).unwrap();
    TICK_WORK.with(|work| work.set((0, 0, 0)));
    let mut changes = Vec::new();
    let _ = selection
        .scan_each(TimeRange::all(), |index, record| {
            if let ScanRef::Change { time, value, .. } = record {
                changes.push((
                    index,
                    time.ticks(),
                    matches!(value, ValueRef::Event { occurrences: 2 }),
                ));
            }
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert_eq!(
        tick_work(),
        (66, 71, 71),
        "setup, compare and commit visits"
    );
    assert!(changes.contains(&(event_index, 2, true)));
    assert!(changes.contains(&(quiet_index, 3, false)));
    assert!(
        !changes
            .iter()
            .any(|&(index, tick, _)| tick == 2 && index < 3)
    );
    let samples = selection.samples(Time::from_ticks(3)).unwrap();
    assert!(matches!(
        samples[event_index],
        Sample::Event { occurrences: 0, .. }
    ));
    assert!(
        matches!(samples[quiet_index], Sample::Value { changed_at: Some(t), .. } if t == Time::from_ticks(3))
    );
    assert_eq!(format!("{:?}", samples[0]), format!("{:?}", samples[2]));
    let mut previous_events = 0;
    let _ = selection
        .query(
            TimeRange::point(Time::from_ticks(3)),
            &[quiet_index],
            |ctx| {
                ctx.visit_samples(Time::from_ticks(2), &[event_index], |_, sample| {
                    let SampleRef::Event { occurrences, .. } = sample else {
                        panic!("previous event")
                    };
                    previous_events = occurrences;
                    Ok(ControlFlow::<()>::Continue(()))
                })
            },
        )
        .unwrap();
    assert_eq!(previous_events, 2);
}

#[test]
fn generated_stream_bounds_and_deferred_caller_work_do_not_grow_with_history() {
    let mut footprints = Vec::new();
    for ticks in [4, 64, 4096] {
        let (mut wave, signals, probe) = fixture(ticks, None);
        let selected = [signals[0], signals[1], signals[2], signals[2]];
        let mut selection = wave.select(&selected).unwrap();
        let mut candidates = 0;
        let mut subset_requests = 0;
        let mut deliveries = 0;
        let mut owned_payload = Vec::new();
        let _ = selection
            .query(TimeRange::from(Time::from_ticks(1)), &[0], |ctx| {
                candidates += 1;
                let tick = ctx.time().ticks();
                assert_eq!(tick, candidates);
                assert_eq!(ctx.previous_events.len(), 3);
                let before_counts = counts(&probe);
                assert_eq!(before_counts.starts, 1);
                let before = Time::from_ticks(tick.checked_sub(1).unwrap());
                for time in [before, ctx.time(), before] {
                    subset_requests += 1;
                    let mut delivered = Vec::new();
                    let _ = ctx.visit_samples(time, &[0, 1], |index, sample| {
                        delivered.push(index);
                        deliveries += 1;
                        match (index, sample) {
                            (
                                0,
                                SampleRef::Value {
                                    value: ValueRef::Real(value),
                                    ..
                                },
                            ) => assert_eq!(value, time.ticks() as f64),
                            (1, SampleRef::Event { occurrences: 2, .. }) => (),
                            _ => panic!("unexpected or unrequested sample"),
                        }
                        Ok(ControlFlow::<()>::Continue(()))
                    })?;
                    assert_eq!(delivered, [0, 1]);
                }
                if tick == ticks - 1 {
                    let mut delivered = Vec::new();
                    let _ = ctx.visit_samples(ctx.time(), &[3, 2], |index, sample| {
                        delivered.push(index);
                        let SampleRef::Value {
                            value: ValueRef::String(value),
                            ..
                        } = sample
                        else {
                            panic!("payload class")
                        };
                        // This is caller-owned output, distinct from fallback state copies.
                        owned_payload.push(value.to_owned());
                        Ok(ControlFlow::<()>::Continue(()))
                    })?;
                    assert_eq!(delivered, [3, 2]);
                } else {
                    assert!(owned_payload.is_empty());
                }
                let after_counts = counts(&probe);
                assert_eq!(after_counts.starts, before_counts.starts);
                assert_eq!(after_counts.advances, before_counts.advances);
                Ok(ControlFlow::<()>::Continue(()))
            })
            .unwrap();
        assert_eq!(candidates, ticks - 1);
        assert_eq!(subset_requests, 3 * (ticks - 1));
        assert_eq!(deliveries, 6 * (ticks - 1));
        let observed = counts(&probe);
        assert_eq!(observed.requested_bases, [0, 1, 2]);
        assert_eq!(observed.advances, 4 * ticks);
        assert_eq!(observed.payload_decodes, ticks); // sequential fallback is permitted
        assert_eq!(observed.unused_decodes, 0);
        assert_eq!(
            (observed.starts, observed.releases, observed.active),
            (1, 1, 0)
        );
        assert_eq!(observed.max_slots, 3);
        assert_eq!(observed.max_pending_records, 5);
        assert_eq!(observed.max_pending_bytes, 2 * (8 + PAYLOAD.len()));
        footprints.push((observed.max_pending_records, observed.max_pending_bytes));
        drop(wave);
        assert_eq!(counts(&probe).reader_drops, 1);
        assert_eq!(owned_payload, [PAYLOAD, PAYLOAD]);
    }
    assert!(footprints.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn early_break_has_a_logical_advance_budget_and_releases_the_read_lease() {
    let (mut wave, signals, probe) = fixture(u64::MAX, None);
    probe.lock().unwrap().advance_budget = Some(5); // four records + one lookahead
    let mut selection = wave.select(&signals[..3]).unwrap();
    let token = String::from("owned break token").into_boxed_str();
    let address = token.as_ptr();
    let mut token = Some(token);
    let mut delivered = Vec::new();
    let result = selection
        .query(TimeRange::all(), &[0], |ctx| {
            assert_eq!(ctx.time(), Time::ZERO);
            assert_eq!(counts(&probe).advances, 5);
            ctx.visit_samples(ctx.time(), &[2, 0], |index, _| {
                delivered.push(index);
                Ok(ControlFlow::Break(token.take().unwrap()))
            })
        })
        .unwrap();
    let ControlFlow::Break(token) = result else {
        panic!("lost break")
    };
    assert_eq!(token.as_ptr(), address);
    assert_eq!(&*token, "owned break token");
    assert_eq!(delivered, [2]);
    let observed = counts(&probe);
    assert_eq!(
        (observed.starts, observed.releases, observed.active),
        (1, 1, 0)
    );
    probe.lock().unwrap().advance_budget = None;
    let mut fresh = 0;
    let _ = selection
        .query(TimeRange::point(Time::ZERO), &[0], |ctx| {
            fresh += 1;
            assert_eq!(ctx.time(), Time::ZERO);
            Ok(ControlFlow::<()>::Continue(()))
        })
        .unwrap();
    assert_eq!(fresh, 1);
    assert_eq!((counts(&probe).starts, counts(&probe).releases), (2, 2));
    drop(wave);
    assert_eq!(counts(&probe).reader_drops, 1);
}

#[test]
fn generated_source_failures_never_publish_an_unfinished_tick() {
    for fail_at in [(0, 0), (0, 2), (2, 2)] {
        for indexed in [false, true] {
            let (mut wave, signals, probe) = fixture(5, Some(fail_at));
            let mut selection = wave.select(&signals[..3]).unwrap();
            let mut published = BTreeSet::new();
            let result = if indexed {
                selection.scan_each(TimeRange::all(), |index, record| {
                    let ScanRef::Change { time, value, .. } = record else {
                        panic!("no entering state at zero")
                    };
                    published.insert(time.ticks());
                    if index == 1 {
                        assert!(matches!(value, ValueRef::Event { occurrences: 2 }));
                    }
                    ControlFlow::<()>::Continue(())
                })
            } else {
                selection.query(TimeRange::all(), &[0, 1], |ctx| {
                    published.insert(ctx.time().ticks());
                    ctx.visit_samples(ctx.time(), &[1], |_, sample| {
                        assert!(matches!(sample, SampleRef::Event { occurrences: 2, .. }));
                        Ok(ControlFlow::<()>::Continue(()))
                    })
                })
            };
            assert!(
                matches!(result,Err(Error::Backend {backend,operation:"read",message}) if backend=="generated" && message=="injected source failure")
            );
            assert_eq!(published, (0..fail_at.0).collect());
            let observed = counts(&probe);
            assert_eq!(
                (observed.starts, observed.releases, observed.active),
                (1, 1, 0)
            );
            let Reader::Generated(reader) = &mut selection.waveform.reader else {
                unreachable!()
            };
            reader.fail_at = None;
            let mut fresh = 0;
            let _ = selection
                .query(TimeRange::point(Time::ZERO), &[0], |ctx| {
                    fresh += 1;
                    ctx.visit_samples(ctx.time(), &[0], |_, sample| {
                        assert!(matches!(
                            sample,
                            SampleRef::Value {
                                value: ValueRef::Real(0.0),
                                changed_at: None,
                                ..
                            }
                        ));
                        Ok(ControlFlow::<()>::Continue(()))
                    })
                })
                .unwrap();
            assert_eq!(fresh, 1);
            assert_eq!((counts(&probe).starts, counts(&probe).releases), (2, 2));
            drop(wave);
            assert_eq!(counts(&probe).reader_drops, 1);
        }
    }
}

#[test]
fn selective_consumer_errors_preserve_prior_delivery_and_unused_channels_stay_unvalidated() {
    let (mut wave, signals, probe) = fixture(5, Some((0, 4)));
    let mut selection = wave.select(&signals[..3]).unwrap();
    let mut earlier = Vec::new();
    let mut subset = Vec::new();
    let mut successful_operand = None;
    let result = selection.query(TimeRange::all(), &[0], |ctx| {
        if ctx.time().ticks() < 2 {
            earlier.push(ctx.time().ticks());
            return Ok(ControlFlow::<()>::Continue(()));
        }
        ctx.visit_samples(ctx.time(), &[0, 2, 1], |index, sample| {
            subset.push(index);
            if index == 2 {
                return Err(Error::Backend {
                    backend: "consumer".into(),
                    operation: "selective read",
                    message: "sentinel".into(),
                });
            }
            let SampleRef::Value {
                value: ValueRef::Real(value),
                ..
            } = sample
            else {
                panic!("operand")
            };
            successful_operand = Some(value);
            Ok(ControlFlow::<()>::Continue(()))
        })
    });
    assert!(
        matches!(result,Err(Error::Backend {backend,operation:"selective read",message}) if backend=="consumer" && message=="sentinel")
    );
    assert_eq!(earlier, [0, 1]);
    assert_eq!(subset, [0, 2]);
    assert_eq!(successful_operand, Some(2.0));
    assert_eq!((counts(&probe).releases, counts(&probe).active), (1, 0));
    let mut fresh = 0;
    let _ = selection
        .query(TimeRange::all(), &[0], |_| {
            fresh += 1;
            Ok(ControlFlow::<()>::Continue(()))
        })
        .unwrap();
    assert_eq!(fresh, 5);
    assert_eq!(counts(&probe).unused_decodes, 0);
    let mut selection = wave.select(&[signals[3]]).unwrap();
    let result = selection.query(TimeRange::all(), &[0], |_| -> Result<ControlFlow<()>> {
        panic!("selected invalid channel must fail")
    });
    assert!(
        matches!(result,Err(Error::Backend {backend,operation:"read",..}) if backend=="generated")
    );
    let observed = counts(&probe);
    assert_eq!(observed.requested_bases, [3]);
    assert_eq!(
        (observed.starts, observed.releases, observed.active),
        (3, 3, 0)
    );
    drop(wave);
    assert_eq!(counts(&probe).reader_drops, 1);
}
