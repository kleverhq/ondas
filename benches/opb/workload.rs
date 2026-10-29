use std::{
    collections::{HashMap, HashSet},
    hint::black_box,
    ops::ControlFlow,
    time::{Duration, Instant},
};

use ondas::{
    Item, Logic, SampleRef, ScanRef, Selection, Signal, Time, TimeRange, ValueRef, Waveform,
};

use super::{
    consumer::{Consumer, Timing},
    input::{Job, Prepared, compose},
};

pub fn open<C: Consumer>(job: &Job, out: &mut C) {
    let wave = job.open();
    out.open(&wave);
    drop(wave);
}

fn timed(f: impl FnOnce()) -> Duration {
    let start = Instant::now();
    f();
    start.elapsed()
}

fn walk<C: Consumer>(wave: &Waveform, out: &mut C) {
    let mut stack = wave.hierarchy().roots().collect::<Vec<_>>();
    while let Some(item) = stack.pop() {
        out.item(&item);
        if let Item::Scope(scope) = item {
            stack.extend(scope.children());
        }
    }
    out.hierarchy_end();
}

fn samples<C: Consumer>(selection: &mut Selection<'_>, time: Time, out: &mut C) {
    let rows = selection.samples(black_box(time)).unwrap();
    for (slot, sample) in rows.iter().enumerate() {
        out.sample(slot, time, sample.as_ref());
    }
    drop(rows);
}

fn batch<C: Consumer>(selection: &mut Selection<'_>, times: &[Time], out: &mut C) {
    let rows = selection.samples_at(black_box(times)).unwrap();
    for (time, row) in &rows {
        for (slot, sample) in row.iter().enumerate() {
            out.sample(slot, *time, sample.as_ref());
        }
    }
    drop(rows);
}

fn scan<C: Consumer>(selection: &mut Selection<'_>, range: TimeRange, limit: usize, out: &mut C) {
    let mut changes = 0;
    let result = selection
        .scan_each(black_box(range), |slot, record| {
            out.scan(slot, record);
            if matches!(record, ScanRef::Change { .. }) {
                changes += 1;
                if limit > 0 && changes == limit {
                    return ControlFlow::Break(());
                }
            }
            ControlFlow::Continue(())
        })
        .unwrap();
    assert_eq!(result.is_break(), limit > 0);
}

fn wave_point<C: Consumer>(
    wave: &mut Waveform,
    signals: &[Signal],
    time: Time,
    singular: bool,
    out: &mut C,
) {
    if singular {
        let sample = wave.sample(black_box(signals[0]), black_box(time)).unwrap();
        out.sample(0, time, sample.as_ref());
        drop(sample);
    } else {
        let rows = wave.samples(black_box(signals), black_box(time)).unwrap();
        for (slot, sample) in rows.iter().enumerate() {
            out.sample(slot, time, sample.as_ref());
        }
        drop(rows);
    }
}

fn retained<C: Consumer>(job: &Job, selection: &mut Selection<'_>, out: &mut C) {
    match job.workload.as_str() {
        "after-prior-prefix" => scan(selection, job.range, job.limit, out),
        "same-wave-repeat-batch" => batch(selection, &job.times, out),
        "after-prior-point" => samples(selection, job.point, out),
        _ => unreachable!("retained operation"),
    }
}

fn retained_prior(job: &Job, selection: &mut Selection<'_>) {
    match job.history.as_str() {
        "identical" | "identical-batch-retain-selection" => retained(job, selection, &mut Timing),
        "neighbor-same" if job.workload.ends_with("prefix") => scan(
            selection,
            TimeRange::closed(job.prior_start, job.range.end().unwrap()),
            job.limit,
            &mut Timing,
        ),
        "neighbor-same" => samples(selection, job.prior_point, &mut Timing),
        _ => unreachable!("retained prior"),
    }
}

fn prior(job: &Job, p: &mut Prepared) {
    match job.history.as_str() {
        "distant-disjoint" | "background-10000" => {
            let mut selection = p.wave.select(&p.prior).unwrap();
            if job.history == "background-10000" {
                scan(
                    &mut selection,
                    TimeRange::closed(Time::from_ticks(0), job.range.end().unwrap()),
                    10000,
                    &mut Timing,
                );
            } else if job.workload.ends_with("prefix") {
                scan(
                    &mut selection,
                    TimeRange::closed(job.prior_point, job.range.end().unwrap()),
                    job.limit,
                    &mut Timing,
                );
            } else {
                samples(&mut selection, job.prior_point, &mut Timing);
            }
            drop(selection);
        }
        "identical-point-drop-selection" => {
            wave_point(&mut p.wave, &p.signals, job.point, false, &mut Timing)
        }
        "identical-batch-drop-selection" => {
            let rows = p.wave.samples_at(&p.signals, &job.times).unwrap();
            black_box(&rows);
            drop(rows);
        }
        "one-complete-walk" => walk(&p.wave, &mut Timing),
        "1-variable-lookups" | "2-variable-lookups" => {
            for _ in 0..if job.history.starts_with('1') { 1 } else { 2 } {
                black_box(p.wave.hierarchy().signal_path(&job.lookup).unwrap());
            }
        }
        _ => {}
    }
}

// Borrowed targets and retained selections stay local to their Waveform owner.
pub fn operation<C: Consumer>(job: &Job, p: &mut Prepared, out: &mut C) -> Duration {
    prior(job, p);
    if job.boundary == "S" {
        let mut selection = p.wave.select(&p.signals).unwrap();
        retained_prior(job, &mut selection);
        let duration = timed(|| retained(job, &mut selection, out));
        drop(selection);
        return duration;
    }
    if matches!(job.workload.as_str(), "local-items" | "children-ready") {
        let scope = p
            .wave
            .hierarchy()
            .scopes()
            .find(|s| s.path() == job.scope)
            .unwrap();
        if job.workload == "children-ready" {
            for item in scope.children() {
                Timing.item(&item);
            }
        }
        return timed(|| {
            for item in scope.children() {
                out.item(&item);
            }
            out.hierarchy_end();
        });
    }
    if job.workload.starts_with("metadata-") {
        let objects = if job.workload == "metadata-scopes" {
            p.wave
                .hierarchy()
                .scopes()
                .map(|s| (s.path(), Item::Scope(s)))
                .collect::<HashMap<_, _>>()
        } else {
            p.wave
                .hierarchy()
                .variables()
                .map(|v| (v.path(), Item::Variable(v)))
                .collect::<HashMap<_, _>>()
        };
        let objects = job
            .metadata
            .iter()
            .map(|path| objects.get(path).unwrap())
            .collect::<Vec<_>>();
        return timed(|| {
            for item in &objects {
                out.metadata(item);
            }
        });
    }
    timed(|| execute(job, p, out))
}

pub fn execute<C: Consumer>(job: &Job, p: &mut Prepared, out: &mut C) {
    match job.workload.as_str() {
        "walk-dfs" | "walk-ready" => walk(&p.wave, out),
        "flat-scopes" => {
            for scope in p.wave.hierarchy().scopes() {
                out.item(&Item::Scope(scope));
            }
            out.hierarchy_end();
        }
        "flat-variables" => {
            for variable in p.wave.hierarchy().variables() {
                out.item(&Item::Variable(variable));
            }
            out.hierarchy_end();
        }
        "flat-signals" => {
            out.inventory(&p.wave, p.wave.hierarchy().signals());
        }
        "lookup-signal"
        | "constant-answer-lookup"
        | "signal-lookup-after-prior"
        | "text-lookup-signal" => {
            let signal = if job.workload == "text-lookup-signal" {
                p.wave
                    .hierarchy()
                    .signal(black_box(&job.lookup_text))
                    .unwrap()
            } else {
                p.wave
                    .hierarchy()
                    .signal_path(black_box(&job.lookup))
                    .unwrap()
            };
            out.lookup(&p.wave, &job.lookup, signal);
        }
        "lookup-scope" | "text-lookup-scope" => {
            let scope = if job.workload == "text-lookup-scope" {
                p.wave
                    .hierarchy()
                    .scope(black_box(&job.lookup_text))
                    .unwrap()
            } else {
                p.wave
                    .hierarchy()
                    .scope_path(black_box(&job.lookup))
                    .unwrap()
            };
            out.scope(scope);
        }
        "missing-last-signal" => out.missing(
            p.wave
                .hierarchy()
                .signal_path(black_box(&job.lookup))
                .is_err(),
        ),
        "missing-last-scope" => out.missing(
            p.wave
                .hierarchy()
                .scope_path(black_box(&job.lookup))
                .is_err(),
        ),
        "lookup-many-signal" => {
            for path in &job.lookup_many {
                let signal = p.wave.hierarchy().signal_path(black_box(path)).unwrap();
                out.lookup(&p.wave, path, signal);
            }
        }
        "lookup-many-scope" => {
            for path in &job.lookup_many {
                out.scope(p.wave.hierarchy().scope_path(black_box(path)).unwrap());
            }
        }
        "browse-deep" => browse(job, &p.wave, out),
        "computed-point"
        | "whole-base-reference-point"
        | "point-owned"
        | "repeated-handle-point"
        | "overlapping-slices-point"
        | "four-state-point"
        | "value-shape-point"
        | "constant-answer-point"
        | "same-wave-new-selection-point"
        | "after-prior-point" => {
            wave_point(
                &mut p.wave,
                &p.signals,
                job.point,
                job.api == "Waveform::sample",
                out,
            );
        }
        "point-borrowed" => {
            let mut selection = p.wave.select(&p.signals).unwrap();
            let mut slot = 0;
            let result = selection
                .visit_samples(black_box(job.point), |sample| {
                    out.sample(slot, job.point, sample);
                    slot += 1;
                    ControlFlow::<()>::Continue(())
                })
                .unwrap();
            assert!(result.is_continue());
            drop(selection);
        }
        "explicit-times"
        | "explicit-times-order"
        | "explicit-single-time"
        | "constant-answer-batch"
        | "same-wave-repeat-batch" => {
            let rows = p
                .wave
                .samples_at(black_box(&p.signals), black_box(&job.times))
                .unwrap();
            for (time, samples) in &rows {
                for (slot, sample) in samples.iter().enumerate() {
                    out.sample(slot, *time, sample.as_ref());
                }
            }
            drop(rows);
        }
        "same-grid-repeated-points" => {
            for &time in &job.times {
                wave_point(&mut p.wave, &p.signals, time, false, out);
            }
        }
        "point-session" => {
            if !p.session.is_empty() {
                for (j, &time) in job.times.iter().enumerate() {
                    let signals = &p.session[job.choices[j]];
                    if job.api == "Waveform::samples per request" {
                        wave_point(&mut p.wave, signals, time, false, out);
                    } else {
                        let mut selection = p.wave.select(signals).unwrap();
                        samples(&mut selection, time, out);
                        drop(selection);
                    }
                }
            } else {
                let mut selection = p.wave.select(&p.signals).unwrap();
                for &time in &job.times {
                    samples(&mut selection, time, out);
                }
                drop(selection);
            }
        }
        "select-drop" => {
            let selection = p.wave.select(black_box(&p.signals)).unwrap();
            out.selection(&p.signals, selection.signals());
            drop(selection);
        }
        "full-window"
        | "first-changes"
        | "first-change-end-bound"
        | "constant-answer-prefix"
        | "after-prior-prefix"
        | "value-shape-scan"
        | "computed-scan"
        | "whole-base-reference-scan"
        | "repeated-handle-scan"
        | "overlapping-slices-scan"
        | "four-state-scan" => {
            let mut selection = p.wave.select(&p.signals).unwrap();
            scan(&mut selection, job.range, job.limit, out);
            drop(selection);
        }
        "full-corpus" => {
            let core = p.signals.iter().copied().collect::<HashSet<_>>();
            let signals = p
                .wave
                .hierarchy()
                .signals()
                .filter(|s| core.contains(s))
                .collect::<Vec<_>>();
            out.sources(&p.wave, &signals);
            let mut selection = p.wave.select(&signals).unwrap();
            scan(&mut selection, job.range, 0, out);
            drop(selection);
        }
        "materialize-traces" => {
            let traces = if job.api == "Waveform::trace" {
                vec![
                    p.wave
                        .trace(black_box(p.signals[0]), black_box(job.range))
                        .unwrap(),
                ]
            } else {
                p.wave
                    .traces(black_box(&p.signals), black_box(job.range))
                    .unwrap()
            };
            for (slot, trace) in traces.iter().enumerate() {
                if let Some(initial) = trace.initial() {
                    out.scan(
                        slot,
                        ScanRef::Initial {
                            signal: trace.signal(),
                            value: initial.value(),
                            changed_at: initial.changed_at(),
                        },
                    );
                }
            }
            for (slot, trace) in traces.iter().enumerate() {
                for change in trace.changes() {
                    out.scan(
                        slot,
                        ScanRef::Change {
                            signal: trace.signal(),
                            time: change.time(),
                            value: change.value(),
                        },
                    );
                }
            }
            drop(traces);
        }
        "saved-trace-samples_at" | "saved-trace-sample" => {
            for (slot, trace) in p.saved.iter().enumerate() {
                if job.workload == "saved-trace-sample" {
                    for &time in &job.times {
                        out.sample(slot, time, trace.sample(black_box(time)).unwrap());
                    }
                } else {
                    let rows = trace.samples_at(black_box(&job.times)).unwrap();
                    for (time, sample) in &rows {
                        out.sample(slot, *time, *sample);
                    }
                    drop(rows);
                }
            }
        }
        "candidate-union" | "candidates-stable-slice" => {
            let mut selection = p.wave.select(&p.signals).unwrap();
            let result = selection
                .scan_candidate_times(black_box(job.range), |time| {
                    out.candidate(time);
                    ControlFlow::<()>::Continue(())
                })
                .unwrap();
            assert!(result.is_continue());
            drop(selection);
        }
        "inspect-window" => {
            let mut selection = p.wave.select(&p.signals).unwrap();
            samples(&mut selection, job.point, out);
            scan(&mut selection, job.range, 100, out);
            samples(&mut selection, job.late_point, out);
            drop(selection);
        }
        "break-then-independent-point" => {
            let mut selection = p.wave.select(&p.signals).unwrap();
            scan(&mut selection, job.range, 100, out);
            drop(selection);
            wave_point(&mut p.wave, &p.independent, job.late_point, false, out);
        }
        "create-composition" => {
            let signal = compose(&mut p.wave, &p.signals, job.shape == "nested16-first");
            out.definition(&mut p.wave, signal, job.point);
        }
        "reject-all"
        | "accept-all-edges"
        | "accept-every16-edge"
        | "first-accepted-after-rejects"
        | "many-drivers"
        | "current-previous-repeat"
        | "eager-every16-edge"
        | "driver-payload-base-sharing" => query(job, p, out),
        other => panic!("not implemented: {other}"),
    }
}

fn browse<C: Consumer>(job: &Job, wave: &Waveform, out: &mut C) {
    let mut scope = wave
        .hierarchy()
        .roots()
        .find_map(|i| match i {
            Item::Scope(s) if s.name() == "tb_opb" => Some(s),
            _ => None,
        })
        .unwrap();
    for part in job.scope.components().skip(1) {
        let mut next = None;
        for item in scope.children() {
            out.item(&item);
            if let Item::Scope(s) = item
                && s.name() == part
            {
                next = Some(s);
            }
        }
        scope = next.unwrap();
    }
    for item in scope.children() {
        out.item(&item);
    }
    out.hierarchy_end();
}

fn query<C: Consumer>(job: &Job, p: &mut Prepared, out: &mut C) {
    let mut selection = p.wave.select(&p.signals).unwrap();
    if job.workload == "driver-payload-base-sharing" {
        out.selection(&p.signals, selection.signals());
    }
    let drivers = (0..job.drivers).collect::<Vec<_>>();
    let controls = (0..=job.drivers).collect::<Vec<_>>();
    let payload = (job.drivers + 1..p.signals.len()).collect::<Vec<_>>();
    let result = selection
        .query(black_box(job.range), &drivers, |ctx| {
            let time = ctx.time();
            out.candidate(time);
            let previous = Time::from_ticks(time.ticks().checked_sub(1).unwrap());
            let mut old = vec![None; job.drivers];
            let mut new = vec![None; job.drivers];
            let _ = ctx.visit_samples(previous, &drivers, |slot, s| {
                old[slot] = scalar(s);
                if job.workload == "current-previous-repeat" {
                    out.sample(slot, previous, s);
                }
                Ok(ControlFlow::<()>::Continue(()))
            })?;
            let _ = ctx.visit_samples(time, &controls, |slot, s| {
                if slot < job.drivers {
                    new[slot] = scalar(s);
                }
                if job.workload == "current-previous-repeat" {
                    out.sample(slot, time, s);
                } else {
                    black_box(s);
                }
                Ok(ControlFlow::<()>::Continue(()))
            })?;
            if job.workload == "current-previous-repeat" {
                let _ = ctx.visit_samples(previous, &drivers, |slot, s| {
                    out.sample(slot, previous, s);
                    Ok(ControlFlow::<()>::Continue(()))
                })?;
            }
            let predicate = || {
                let step = time.ticks() / job.step_ticks;
                (0..job.drivers).any(|i| {
                    let period = 16 + i as u64 * 2;
                    let rising = old[i] == Some(Logic::Zero)
                        && new[i] == Some(Logic::One)
                        && time.ticks().is_multiple_of(job.step_ticks)
                        && step >= period
                        && (step - period).is_multiple_of(period * 2);
                    rising
                        && job.workload != "reject-all"
                        && (matches!(
                            job.workload.as_str(),
                            "accept-all-edges" | "current-previous-repeat" | "many-drivers"
                        ) || ((step - period) / (period * 2) + 1).is_multiple_of(16))
                })
            };
            let mut accepted = false;
            if job.workload == "eager-every16-edge" {
                let mut decided = false;
                let _ = ctx.visit_samples(time, &payload, |slot, sample| {
                    if !decided {
                        accepted = predicate();
                        decided = true;
                        if accepted {
                            out.accepted(time);
                        }
                    }
                    if accepted {
                        out.sample(slot, time, sample);
                    } else {
                        black_box(sample);
                    }
                    Ok(ControlFlow::<()>::Continue(()))
                })?;
            } else if predicate() {
                accepted = true;
                out.accepted(time);
                let _ = ctx.visit_samples(time, &payload, |slot, sample| {
                    out.sample(slot, time, sample);
                    Ok(ControlFlow::<()>::Continue(()))
                })?;
            }
            Ok(
                if accepted && job.workload == "first-accepted-after-rejects" {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                },
            )
        })
        .unwrap();
    out.query_end(result.is_break());
    drop(selection);
}

fn scalar(sample: SampleRef<'_>) -> Option<Logic> {
    if let SampleRef::Value {
        value: ValueRef::Bits(bits),
        ..
    } = sample
    {
        bits.bit(0)
    } else {
        None
    }
}
