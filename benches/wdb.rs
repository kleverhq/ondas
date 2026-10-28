use std::{collections::HashSet, hint::black_box, ops::ControlFlow, time::Duration};

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use ondas::{SampleRef, ScanRef, Time, TimeRange, ValueRef};
use serde_json::{Value as Json, json};

#[path = "support/explicit_times.rs"]
mod explicit_times;
#[path = "../tests/support/fixtures.rs"]
mod fixtures;
#[allow(dead_code, unused_imports)]
#[path = "../tests/support/normalized_oracle.rs"]
mod normalized_oracle;

const BACKEND: &str = "wdb-lib";

fn value(value: ValueRef<'_>) -> Json {
    match value {
        ValueRef::Bits(bits) => json!({"bits": bits.to_string()}),
        _ => panic!("workload requires independently covered bit values"),
    }
}

fn sample(actual: SampleRef<'_>, expected: &Json) {
    match actual {
        SampleRef::Value {
            value: bits,
            changed_at,
            ..
        } => {
            assert_eq!(value(bits), expected["value"]);
            if let Some(time) = expected["changed_at"].as_str() {
                assert_eq!(changed_at.map(Time::ticks), Some(time.parse().unwrap()));
            }
        }
        SampleRef::Missing { .. } => assert_eq!(expected["missing"], true),
        _ => panic!("unexpected workload sample"),
    }
}

fn scan_count(selection: &mut ondas::Selection<'_>, range: TimeRange) -> u64 {
    let mut count = 0;
    let _ = selection
        .scan(range, |_| {
            count += 1;
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    count
}

fn first_change(record: ScanRef<'_>) -> ControlFlow<Time> {
    match record {
        ScanRef::Change { time, .. } => ControlFlow::Break(time),
        _ => ControlFlow::Continue(()),
    }
}

fn workloads(c: &mut Criterion) {
    for (fixture, paths) in [
        (
            "wdb/wdb0022-alu-sim-behav",
            ["ALU_Sim.clk", "ALU_Sim.reset", "ALU_Sim.OP1", "ALU_Sim.OP2"],
        ),
        (
            "wdb/wdb0153-canny",
            [
                "apatb_canny_top.AESL_clock",
                "apatb_canny_top.ce",
                "apatb_canny_top.ap_clk",
                "apatb_canny_top.DEP_address_1_from",
            ],
        ),
        (
            "wdb/wdb0114-treehash-tb-behav",
            [
                "treehash_tb.CLK",
                "treehash_tb.sk_seed",
                "treehash_tb.auth_path",
                "treehash_tb.treehash_tb.auth_path",
            ],
        ),
    ] {
        let (path, sidecar) = fixtures::load_artifact(&fixtures::root(), fixture);
        let mut wave = ondas::open_with(&path, BACKEND).unwrap();
        let signals = paths.map(|name| wave.hierarchy().signal(name).unwrap());
        let histories = paths.map(|name| {
            let components = name.split('.').collect::<Vec<_>>();
            let declaration = sidecar["oracle"]["hierarchy"]["variables"]
                .as_array()
                .unwrap()
                .iter()
                .find(|variable| variable["path"] == json!(components))
                .expect("workload path needs independent oracle evidence");
            &sidecar["oracle"]["signals"][declaration["signal"].as_str().unwrap()]
        });
        let tick = |v: &Json| v.as_str().unwrap().parse::<u64>().unwrap();
        let active = histories[0]["windows"]
            .as_array()
            .and_then(|windows| {
                windows
                    .iter()
                    .filter(|window| {
                        tick(&window["start"]) > 0
                            && tick(&window["start"]) <= tick(&window["end"])
                            && !window["changes"].as_array().unwrap().is_empty()
                    })
                    .max_by_key(|window| tick(&window["end"]))
            })
            .unwrap_or_else(|| panic!("{fixture}: workload needs its late active window"));
        let end = tick(&active["end"]);
        let start = tick(&active["start"]).max(end.saturating_sub(100_000));
        let range = TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end));

        // Independent complete windows qualify values and actual selected activity.
        // Do this before any timed group, also when Criterion filters its groups.
        let mut expected_first = None;
        let mut expected_records = 0;
        for (slot, history) in histories.iter().enumerate() {
            let windows = history["windows"].as_array().unwrap();
            let window = windows
                .iter()
                .find(|w| tick(&w["start"]) <= start && end <= tick(&w["end"]))
                .unwrap_or_else(|| {
                    panic!("{fixture}: {} needs complete range evidence", paths[slot])
                });
            for time in [0, end / 2, end] {
                let window = windows
                    .iter()
                    .find(|w| tick(&w["start"]) <= time && time <= tick(&w["end"]))
                    .expect("workload point needs complete oracle coverage");
                let expected =
                    normalized_oracle::WindowEvidence::new(window, windows, false).sample(time);
                sample(
                    wave.sample(signals[slot], Time::from_ticks(time))
                        .unwrap()
                        .as_ref(),
                    &expected,
                );
            }
            let expected = normalized_oracle::changes(window)
                .into_iter()
                .filter(|(time, _)| start <= *time && *time <= end)
                .collect::<Vec<_>>();
            if let Some((time, _)) = expected.first() {
                expected_first = Some(expected_first.map_or(*time, |old: u64| old.min(*time)));
            }
            let trace = wave.trace(signals[slot], range).unwrap();
            let before = start.checked_sub(1).expect("late window starts after zero");
            let entering = windows
                .iter()
                .find(|w| tick(&w["start"]) <= before && before <= tick(&w["end"]))
                .expect("workload needs complete entering-state evidence");
            let entering =
                normalized_oracle::WindowEvidence::new(entering, windows, false).sample(before);
            match trace.initial() {
                Some(initial) => {
                    expected_records += 1;
                    assert_eq!(value(initial.value()), entering["value"]);
                    if let Some(time) = entering["changed_at"].as_str() {
                        assert_eq!(
                            initial.changed_at().map(Time::ticks),
                            Some(time.parse().unwrap())
                        );
                    }
                }
                None => assert_eq!(entering["missing"], true),
            }
            expected_records += expected.len() as u64;
            let actual = trace
                .changes()
                .iter()
                .map(|change| (change.time().ticks(), value(change.value())))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{fixture}: {}", paths[slot]);
        }
        let expected_first = expected_first.expect("workload must contain selected activity");
        assert_eq!(
            scan_count(&mut wave.select(&signals).unwrap(), range),
            expected_records
        );
        assert_eq!(
            wave.select(&signals)
                .unwrap()
                .scan(range, first_change)
                .unwrap(),
            ControlFlow::Break(Time::from_ticks(expected_first))
        );

        let unique = signals.iter().collect::<HashSet<_>>().len();
        let mut group = c.benchmark_group(format!("{fixture}/file/bases{unique}/slots4"));
        group.sample_size(10);
        // Open/drop includes reader setup and cleanup, with a possibly warm OS cache.
        group.bench_function(BenchmarkId::new("open-drop", BACKEND), |b| {
            b.iter(|| {
                drop(black_box(
                    ondas::open_with(black_box(&path), BACKEND).unwrap(),
                ))
            })
        });
        group.bench_function(BenchmarkId::new("open-point-drop", BACKEND), |b| {
            b.iter(|| {
                let mut fresh = ondas::open_with(black_box(&path), BACKEND).unwrap();
                let signals = paths.map(|name| fresh.hierarchy().signal(name).unwrap());
                drop(black_box(
                    fresh.samples(&signals, Time::from_ticks(end)).unwrap(),
                ));
                drop(fresh);
            })
        });
        group.bench_function(BenchmarkId::new("metadata-only", BACKEND), |b| {
            b.iter(|| drop(black_box(ondas::read_metadata(black_box(&path)).unwrap())))
        });
        let saved = wave.samples(&signals, Time::from_ticks(end)).unwrap();
        group.bench_function(BenchmarkId::new("text/saved-samples", BACKEND), |b| {
            b.iter(|| {
                for sample in &saved {
                    let SampleRef::Value {
                        value: ValueRef::Bits(bits),
                        ..
                    } = sample.as_ref()
                    else {
                        panic!("workload requires independently covered bit values");
                    };
                    drop(black_box(bits.to_string()));
                }
            })
        });
        // Reader setup/lookup/drop are outside this timed routine. Read cleanup and
        // output destruction are inside. Each iteration starts with a fresh reader.
        group.bench_function(BenchmarkId::new("point/cold-reader", BACKEND), |b| {
            b.iter_batched_ref(
                || {
                    let wave = ondas::open_with(&path, BACKEND).unwrap();
                    let signals = paths.map(|name| wave.hierarchy().signal(name).unwrap());
                    (wave, signals)
                },
                |(wave, signals)| {
                    drop(black_box(
                        wave.samples(signals, Time::from_ticks(end)).unwrap(),
                    ));
                },
                BatchSize::PerIteration,
            )
        });
        for time in [0, end / 2, end] {
            group.bench_function(format!("point/fresh-selection/t{time}/{BACKEND}"), |b| {
                b.iter(|| {
                    drop(black_box(
                        wave.samples(&signals, black_box(Time::from_ticks(time)))
                            .unwrap(),
                    ));
                })
            });
        }
        group.bench_function(
            BenchmarkId::new("scan/fresh-selection/late", BACKEND),
            |b| {
                b.iter(|| {
                    let mut selected = wave.select(&signals).unwrap();
                    black_box(scan_count(&mut selected, black_box(range)));
                })
            },
        );
        group.bench_function(
            BenchmarkId::new("trace/fresh-selection/late", BACKEND),
            |b| b.iter(|| drop(black_box(wave.traces(&signals, black_box(range)).unwrap()))),
        );
        group.bench_function(
            BenchmarkId::new("scan/stop-first-change/late", BACKEND),
            |b| {
                b.iter(|| {
                    let mut selected = wave.select(&signals).unwrap();
                    let _ = black_box(selected.scan(black_box(range), first_change).unwrap());
                })
            },
        );
        let mut selection = wave.select(&signals).unwrap();
        selection.samples(Time::from_ticks(end)).unwrap();
        group.bench_function(BenchmarkId::new("point/primed-repeat", BACKEND), |b| {
            b.iter(|| {
                drop(black_box(
                    selection.samples(black_box(Time::from_ticks(end))).unwrap(),
                ))
            })
        });
        group.finish();
        explicit_times::register(c, fixture, &path, BACKEND, paths[0], end);
    }
}

fn repeated_source(c: &mut Criterion) {
    let fixture = "wdb/wdb0161-mcvga-tb-behav";
    let name = "MCVGA_TB.uut.vga.letter_change_counter";
    let (path, sidecar) = fixtures::load_artifact(&fixtures::root(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).unwrap();
    let base = wave.hierarchy().signal(name).unwrap();
    let width = base.width().unwrap();
    let bus = wave.concat(&[base, base, base]).unwrap();
    let cut = bus.slice(width - 1, 0).unwrap();
    let nested = wave.concat(&[cut, base]).unwrap();
    let signals = [bus, cut, nested, bus];
    let components = name.split('.').collect::<Vec<_>>();
    let declaration = sidecar["oracle"]["hierarchy"]["variables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|variable| variable["path"] == json!(components))
        .unwrap();
    let history = &sidecar["oracle"]["signals"][declaration["signal"].as_str().unwrap()];
    let windows = history["windows"].as_array().unwrap();
    let tick = |v: &Json| v.as_str().unwrap().parse::<u64>().unwrap();
    let window = windows
        .iter()
        .filter(|w| tick(&w["start"]) > 0 && !w["changes"].as_array().unwrap().is_empty())
        .max_by_key(|w| tick(&w["end"]))
        .unwrap();
    let end = tick(&window["end"]);
    let start = end.checked_sub(100_000).unwrap();
    assert!(tick(&window["start"]) < start);
    let range = TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end));
    let evidence = normalized_oracle::WindowEvidence::new(window, windows, false);
    let repeated = |expected: &Json, copies: usize| {
        let mut result = expected.clone();
        result["value"]["bits"] = json!(expected["value"]["bits"].as_str().unwrap().repeat(copies));
        result
    };
    let point = evidence.sample(end);
    let samples = wave.samples(&signals, Time::from_ticks(end)).unwrap();
    assert_eq!(samples.len(), signals.len());
    for ((actual, signal), copies) in samples.iter().zip(signals).zip([3, 1, 2, 3]) {
        assert_eq!(actual.signal(), signal);
        sample(actual.as_ref(), &repeated(&point, copies));
    }
    let entering = evidence.sample(start - 1);
    let changes = normalized_oracle::changes(window)
        .into_iter()
        .filter(|(time, _)| start <= *time && *time <= end)
        .collect::<Vec<_>>();
    assert!(!changes.is_empty());
    let traces = wave.traces(&signals, range).unwrap();
    assert_eq!(traces.len(), signals.len());
    for ((trace, signal), copies) in traces.iter().zip(signals).zip([3, 1, 2, 3]) {
        assert_eq!(trace.signal(), signal);
        let initial = trace.initial().unwrap();
        let expected = repeated(&entering, copies);
        assert_eq!(value(initial.value()), expected["value"]);
        if let Some(time) = expected["changed_at"].as_str() {
            assert_eq!(
                initial.changed_at().map(Time::ticks),
                Some(time.parse().unwrap())
            );
        }
        let actual = trace
            .changes()
            .iter()
            .map(|change| (change.time().ticks(), value(change.value())))
            .collect::<Vec<_>>();
        let expected = changes
            .iter()
            .map(|(time, value)| {
                (
                    *time,
                    json!({"bits": value["bits"].as_str().unwrap().repeat(copies)}),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
    let mut group = c.benchmark_group(format!("{fixture}/repeated-source/bases1/slots4"));
    group.sample_size(10);
    group.bench_function(
        BenchmarkId::new("point/fresh-selection/late", BACKEND),
        |b| {
            b.iter(|| {
                drop(black_box(
                    wave.samples(&signals, black_box(Time::from_ticks(end)))
                        .unwrap(),
                ))
            });
        },
    );
    group.bench_function(
        BenchmarkId::new("trace/fresh-selection/late", BACKEND),
        |b| {
            b.iter(|| drop(black_box(wave.traces(&signals, black_box(range)).unwrap())));
        },
    );
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(2));
    targets = workloads, repeated_source
}
criterion_main!(benches);
