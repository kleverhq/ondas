use ondas::{
    Error, Logic, QueryContext, Result, SampleRef, Selection, Signal, Time, TimeRange, Value,
    ValueRef, Waveform,
};
use std::{fmt::Write, ops::ControlFlow};

const HEADER: &str = "$var wire 1 a activity $end $var wire 1 b other $end
$var wire 1 g gate $end $var event 1 e events $end
$var wire 4 p payload $end $var wire 4 p alias $end
$var wire 4 n never $end $enddefinitions $end\n";

fn open(records: &[(u64, u8, String)], reverse: bool) -> (Waveform, Vec<Signal>) {
    let mut records = records.iter().collect::<Vec<_>>();
    // Preserve each source's order; permute only distinct sources at a tick.
    records
        .sort_by_key(|(time, source, _)| (*time, if reverse { u8::MAX - source } else { *source }));
    let mut input = HEADER.to_owned();
    for (time, _, record) in records {
        writeln!(input, "#{time} {record}").unwrap();
    }
    let wave =
        ondas::open_bytes_with("composition.vcd", input.into_bytes().into(), "vcd-native").unwrap();
    let signal = |name| wave.hierarchy().signal(name).unwrap();
    let activity = signal("activity");
    let payload = signal("payload");
    let selected = vec![
        activity,
        signal("gate"),
        signal("events"),
        payload,
        signal("alias"),
        payload.slice(2, 1).unwrap(),
        activity,
        signal("other"),
        signal("never"),
    ];
    assert_eq!(selected[0], selected[6]);
    assert_eq!(selected[3], selected[4]);
    (wave, selected)
}

fn previous(time: Time) -> Option<Time> {
    time.ticks().checked_sub(1).map(Time::from_ticks)
}

fn levels(context: &QueryContext<'_>, time: Time, slots: &[usize]) -> Result<Vec<Option<Logic>>> {
    let mut values = Vec::new();
    let _ = context.visit_samples(time, slots, |slot, sample| {
        assert_eq!(slot, slots[values.len()]);
        values.push(match sample {
            SampleRef::Value {
                value: ValueRef::Bits(bits),
                ..
            } if bits.width() == 1 => bits.bit(0),
            _ => None,
        });
        Ok(ControlFlow::<()>::Continue(()))
    })?;
    Ok(values)
}

#[derive(Default)]
struct Output {
    rows: Vec<(u64, Vec<Value>)>,
    candidates: usize,
    payload_visits: usize,
}

// Ordinary caller policy, not a predicate/sampling-mode API or evaluator.
fn collect_first(
    selection: &mut Selection<'_>,
    range: TimeRange,
    observe: fn(Time) -> Option<Time>,
    payload: &[usize],
    output: &mut Output,
) -> Result<ControlFlow<()>> {
    // Independent activity may schedule extra candidates; the condition confirms
    // only the primary activity (including its shared slot).
    let drivers = [0, 6, 7];
    selection.query(range, &drivers, |context| {
        output.candidates += 1;
        let current = context.time();
        let Some(before) = previous(current) else {
            return Ok(ControlFlow::Continue(()));
        };
        let prior = levels(context, before, &drivers[..2])?;
        let now = levels(context, current, &drivers[..2])?;
        assert_eq!(levels(context, before, &drivers[..2])?, prior); // t-1 → t → t-1
        let confirmed = prior.iter().zip(now).any(|(before, after)| {
            matches!(
                (*before, after),
                (Some(Logic::Zero), Some(Logic::One)) | (Some(Logic::One), Some(Logic::Zero))
            )
        });
        if !confirmed || levels(context, current, &[1])? != [Some(Logic::One)] {
            return Ok(ControlFlow::Continue(()));
        }
        let Some(at) = observe(current) else {
            return Ok(ControlFlow::Continue(()));
        };
        let mut occurrences = 0;
        let _ = context.visit_samples(at, &[2], |_, sample| {
            let SampleRef::Event {
                occurrences: count, ..
            } = sample
            else {
                return Err(Error::Backend {
                    backend: "test consumer".into(),
                    operation: "condition",
                    message: "expected event observation".into(),
                });
            };
            occurrences = count;
            Ok(ControlFlow::<()>::Continue(()))
        })?;
        if occurrences == 0 {
            return Ok(ControlFlow::Continue(()));
        }
        let mut pending = Vec::new();
        let _ = context.visit_samples(at, payload, |slot, sample| {
            output.payload_visits += 1;
            assert_eq!(slot, payload[pending.len()]);
            let SampleRef::Value { value, .. } = sample else {
                return Err(Error::Backend {
                    backend: "test consumer".into(),
                    operation: "payload",
                    message: "required payload unavailable".into(),
                });
            };
            pending.push(value.to_owned());
            Ok(ControlFlow::<()>::Continue(()))
        })?;
        output.rows.push((current.ticks(), pending)); // Commit only after every read succeeds.
        Ok(ControlFlow::Break(()))
    })
}

fn rendered(output: &Output) -> Vec<(u64, Vec<String>)> {
    output
        .rows
        .iter()
        .map(|(time, values)| {
            (
                *time,
                values
                    .iter()
                    .map(|value| {
                        let ValueRef::Bits(bits) = value.as_ref() else {
                            panic!("expected bit payload")
                        };
                        bits.to_string()
                    })
                    .collect(),
            )
        })
        .collect()
}

fn sparse_records(extra: bool) -> Vec<(u64, u8, String)> {
    let mut records = [
        (0, 0, "0a"),
        (0, 1, "0b"),
        (0, 2, "0g"),
        (0, 4, "b0000 p"),
        (4, 2, "1g"),
        (5, 3, "1e"),
        (7, 4, "b0011 p"),
        (10, 0, "1a"),
        (10, 1, "1b"),
        (11, 2, "0g"),
        (13, 4, "b1100 p"),
        (15, 0, "0a"),
        (15, 0, "1a"), // Net excursion: no confirmed transition.
        (15, 1, "0b"), // Forces a candidate even if redundant primary ticks are pruned.
        (18, 2, "1g"),
        (19, 3, "1e"),
        (19, 4, "b1010 p"),
        (20, 0, "0a"),
        (20, 1, "1b"),
        (20, 2, "0g"),
        (20, 2, "1g"),
        (20, 4, "b0110 p"),
    ]
    .into_iter()
    .map(|(t, s, v)| (t, s, v.to_owned()))
    .collect::<Vec<_>>();
    if extra {
        // Independent activity forces 6 into even an exact candidate stream.
        // Control and the preceding event pass; primary confirmation rejects it.
        records.extend([
            (6, 0, "0a".into()),
            (6, 1, "1b".into()),
            (17, 0, "1a".into()),
        ]);
    }
    records
}

#[test]
fn adjacent_event_conditions_ignore_order_excursions_and_extra_candidates() {
    for reverse in [false, true] {
        for extra in [false, true] {
            for start in [0, 6] {
                let (mut wave, signals) = open(&sparse_records(extra), reverse);
                let mut selection = wave.select(&signals).unwrap();
                let mut output = Output::default();
                let result = collect_first(
                    &mut selection,
                    TimeRange::from(Time::from_ticks(start)),
                    previous,
                    &[3, 4, 5],
                    &mut output,
                )
                .unwrap();
                assert_eq!(result, ControlFlow::Break(()));
                assert_eq!(
                    rendered(&output),
                    [(20, vec!["1010".into(), "1010".into(), "01".into()])]
                );
                assert_eq!(output.payload_visits, 3);
                // Events at5 and19 are not sticky at9/20. Choosing current ticks changes policy.
                let mut current = Output::default();
                let result = collect_first(
                    &mut selection,
                    TimeRange::from(Time::from_ticks(start)),
                    Some,
                    &[3, 4, 5],
                    &mut current,
                )
                .unwrap();
                assert_eq!(result, ControlFlow::Continue(()));
                assert!(current.rows.is_empty());
                assert_eq!(current.payload_visits, 0);
            }
        }
    }
}

#[test]
fn coarse_candidates_exercise_confirmation_and_completed_excursion_states() {
    for reverse in [false, true] {
        let (mut wave, signals) = open(&sparse_records(true), reverse);
        let mut selection = wave.select(&signals).unwrap();
        let mut checked = [false; 2];
        let _ = selection
            .query(
                TimeRange::from(Time::from_ticks(6)),
                &[0, 6, 7],
                |context| {
                    let time = context.time();
                    if time.ticks() == 6 {
                        checked[0] = true;
                        assert_eq!(
                            levels(context, previous(time).unwrap(), &[0, 6])?,
                            [Some(Logic::Zero); 2]
                        );
                        assert_eq!(
                            levels(context, time, &[0, 6, 1])?,
                            [Some(Logic::Zero), Some(Logic::Zero), Some(Logic::One)]
                        );
                        let _ =
                            context.visit_samples(previous(time).unwrap(), &[2], |_, sample| {
                                assert!(matches!(sample, SampleRef::Event { occurrences: 1, .. }));
                                Ok(ControlFlow::<()>::Continue(()))
                            })?;
                    } else if time.ticks() == 15 {
                        checked[1] = true;
                        let before = levels(context, previous(time).unwrap(), &[0, 6])?;
                        assert_eq!(before, [Some(Logic::One); 2]);
                        assert_eq!(levels(context, time, &[0, 6])?, before);
                        assert_eq!(levels(context, previous(time).unwrap(), &[0, 6])?, before);
                    }
                    Ok(ControlFlow::<()>::Continue(()))
                },
            )
            .unwrap();
        // Both are required by real independent-driver changes, not optional
        // redundant callbacks. Future exact candidate pruning remains valid.
        assert_eq!(checked, [true, true]);
    }
}

#[test]
fn current_control_rejects_an_otherwise_passing_previous_tick_condition() {
    let mut records = sparse_records(false);
    // Control is high at 19 but low at candidate 20; the event condition uses 19.
    records.retain(|(time, source, record)| !(*time == 20 && *source == 2 && record == "1g"));
    for reverse in [false, true] {
        let (mut wave, signals) = open(&records, reverse);
        let mut selection = wave.select(&signals).unwrap();
        let mut output = Output::default();
        let result = collect_first(
            &mut selection,
            TimeRange::from(Time::from_ticks(6)),
            previous,
            &[3, 4, 5],
            &mut output,
        )
        .unwrap();
        assert_eq!(result, ControlFlow::Continue(()));
        assert!(output.rows.is_empty());
        assert_eq!(output.payload_visits, 0);
    }
}

#[test]
fn long_rejected_prefix_stops_at_first_acceptance_and_no_match_reads_no_payload() {
    let accepted = 1024;
    for matching in [false, true] {
        let mut records = vec![(0, 2, "1g".into()), (0, 4, "b0011 p".into())];
        for time in 0..=accepted + 1 {
            records.push((time, 0, format!("{}a", time % 2)));
            records.push((time, 1, format!("{}b", time % 2)));
        }
        records.extend([
            (accepted - 1, 4, "b1010 p".into()),
            (accepted, 4, "b0110 p".into()),
        ]);
        if matching {
            records.extend([(accepted - 1, 3, "1e".into()), (accepted, 3, "1e".into())]);
        }
        let (mut wave, signals) = open(&records, false);
        let mut selection = wave.select(&signals).unwrap();
        let mut output = Output::default();
        let result = collect_first(
            &mut selection,
            TimeRange::all(),
            previous,
            &[3, 4, 5],
            &mut output,
        )
        .unwrap();
        if matching {
            assert_eq!(result, ControlFlow::Break(()));
            assert_eq!(
                rendered(&output),
                [(accepted, vec!["1010".into(), "1010".into(), "01".into()])]
            );
            assert_eq!(output.candidates, accepted as usize + 1); // Includes skipped zero, not the later match.
            assert_eq!(output.payload_visits, 3);
        } else {
            assert_eq!(result, ControlFlow::Continue(()));
            assert!(output.rows.is_empty());
            assert_eq!(output.candidates, accepted as usize + 2);
            assert_eq!(output.payload_visits, 0);
        }
    }
}

#[test]
fn failing_last_payload_does_not_commit_a_partially_constructed_row() {
    let (mut wave, signals) = open(&sparse_records(true), true);
    let mut selection = wave.select(&signals).unwrap();
    let mut output = Output::default();
    let result = collect_first(
        &mut selection,
        TimeRange::from(Time::from_ticks(6)),
        previous,
        &[3, 4, 5, 8],
        &mut output,
    );
    assert!(
        matches!(result,Err(Error::Backend {operation:"payload",message,..}) if message=="required payload unavailable")
    );
    assert_eq!(output.payload_visits, 4); // Three successful staged copies preceded Missing.
    assert!(output.rows.is_empty());
    let result = collect_first(
        &mut selection,
        TimeRange::from(Time::from_ticks(6)),
        previous,
        &[3, 4, 5],
        &mut output,
    )
    .unwrap();
    assert_eq!(result, ControlFlow::Break(()));
    assert_eq!(
        rendered(&output),
        [(20, vec!["1010".into(), "1010".into(), "01".into()])]
    );
}
