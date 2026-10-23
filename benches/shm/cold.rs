use std::{collections::HashSet, hint::black_box};

use criterion::{BenchmarkId, Criterion};
use ondas::{Encoding, Signal, Time, Waveform};

use super::{fixtures, workloads};
use workloads::{BACKEND, candidate_count, first_change, range, scan_count};

fn window(
    c: &mut Criterion,
    fixture: &str,
    wave: &mut Waveform,
    signals: &[Signal],
    label: &str,
    start: u64,
    end: u64,
) {
    let window = range(start, end);
    let mut group = c.benchmark_group(format!(
        "{fixture}/directory/fresh-selection/{label}/{start}..={end}"
    ));
    group.sample_size(10);
    // Each iteration constructs its own selection. Registration and earlier
    // benchmark cases cannot seed a query checkpoint for these operations.
    group.bench_function(BenchmarkId::new("scan", BACKEND), |b| {
        b.iter(|| {
            let mut selection = wave.select(black_box(signals)).unwrap();
            black_box(scan_count(&mut selection, black_box(window)).unwrap())
        })
    });
    group.bench_function(BenchmarkId::new("candidate-times", BACKEND), |b| {
        b.iter(|| {
            let mut selection = wave.select(black_box(signals)).unwrap();
            black_box(candidate_count(&mut selection, black_box(window)).unwrap())
        })
    });
    group.bench_function(BenchmarkId::new("owned-traces", BACKEND), |b| {
        b.iter(|| black_box(wave.traces(black_box(signals), black_box(window)).unwrap()))
    });
    group.bench_function(BenchmarkId::new("first-change", BACKEND), |b| {
        b.iter(|| {
            let mut selection = wave.select(black_box(signals)).unwrap();
            black_box(first_change(&mut selection, black_box(window)).unwrap())
        })
    });
    group.finish();
}

fn history(c: &mut Criterion, fixture: &str, end: u64) {
    let (path, _) = fixtures::load_artifact(&fixtures::root(), fixture);
    let mut wave = ondas::open_with(path, BACKEND).unwrap();
    let signals = [
        "top.phase_0",
        "top.phase_1",
        "top.phase_2",
        "top.phase_3",
        "top.step",
    ]
    .map(|name| wave.hierarchy().signal(name).unwrap());
    let mut group = c.benchmark_group(format!("{fixture}/directory/cold-points/t{end}"));
    group.sample_size(10);
    for count in [1, 4, 5] {
        group.bench_function(
            BenchmarkId::new(
                format!("fresh-selection/bases{count}/slots{count}"),
                BACKEND,
            ),
            |b| {
                b.iter(|| {
                    black_box(
                        wave.samples(black_box(&signals[..count]), Time::from_ticks(end))
                            .unwrap(),
                    )
                })
            },
        );
    }
    let mut selection = wave.select(&signals).unwrap();
    selection.samples(Time::from_ticks(end)).unwrap();
    group.bench_function(
        BenchmarkId::new("warm-repeat/bases5/slots5", BACKEND),
        |b| b.iter(|| black_box(selection.samples(Time::from_ticks(black_box(end))).unwrap())),
    );
    group.finish();
    drop(selection);
    for (label, selected) in [
        ("phases/bases4/slots4", &signals[..4]),
        ("phases-integer/bases5/slots5", &signals[..]),
    ] {
        for start in [1, end - 63] {
            window(c, fixture, &mut wave, selected, label, start, start + 63);
        }
    }
}

fn histories(c: &mut Criterion) {
    for (fixture, end) in workloads::HISTORY {
        history(c, fixture, end);
    }
}

fn wires(c: &mut Criterion) {
    let fixture = "shm/shm0001-cadence-rtl-to-gdsii-flow-gate-level-simulation-waves";
    let (path, _) = fixtures::load_artifact(&fixtures::root(), fixture);
    let mut wave = ondas::open_with(path, BACKEND).unwrap();
    let wire = wave
        .hierarchy()
        .signal("counter_test.counter1.g705.A")
        .unwrap();
    let mut group = c.benchmark_group(format!("{fixture}/directory/cold-wire/t90000"));
    group.sample_size(10);
    group.bench_function(
        BenchmarkId::new("point/fresh-selection/bases1", BACKEND),
        |b| {
            b.iter(|| {
                black_box(
                    wave.sample(black_box(wire), Time::from_ticks(90_000))
                        .unwrap(),
                )
            })
        },
    );
    group.finish();
    for start in [1, 90_000] {
        window(
            c,
            fixture,
            &mut wave,
            &[wire],
            "wire/bases1/slots1",
            start,
            start + 99,
        );
    }
}

fn disjoint_calls(c: &mut Criterion) {
    let fixture = workloads::TOPOLOGY;
    let (path, _) = fixtures::load_artifact(&fixtures::root(), fixture);
    let mut wave = ondas::open_with(path, BACKEND).unwrap();
    let signals = ["top.probe_sparse", "top.probe_constant"]
        .map(|name| wave.hierarchy().signal(name).unwrap());
    let mut group = c.benchmark_group(format!("{fixture}/directory/disjoint-calls/257..=320"));
    group.sample_size(10);
    let window = range(257, 320);
    group.bench_function(BenchmarkId::new("union/bases2/slots2", BACKEND), |b| {
        b.iter(|| black_box(wave.traces(black_box(&signals), black_box(window)).unwrap()))
    });
    group.bench_function(BenchmarkId::new("two-reads/bases2/slots2", BACKEND), |b| {
        b.iter(|| {
            let first = wave
                .trace(black_box(signals[0]), black_box(window))
                .unwrap();
            let second = wave
                .trace(black_box(signals[1]), black_box(window))
                .unwrap();
            black_box((first, second))
        })
    });
    group.finish();
}

fn imported_batch(c: &mut Criterion) {
    let fixture = "shm/shm0017-1d-vector-based-npu-singlecore-withoutsfp-sim-gls-waves";
    let (path, _) = fixtures::load_artifact(&fixtures::root(), fixture);
    let mut wave = ondas::open_with(path, BACKEND).unwrap();
    let mut seen = HashSet::new();
    let signals = wave
        .hierarchy()
        .variables()
        .filter_map(|variable| variable.signal())
        .filter(|signal| matches!(signal.encoding(), Encoding::Bits { .. }))
        .filter(|signal| seen.insert(*signal))
        .take(100)
        .collect::<Vec<_>>();
    assert_eq!(signals.len(), 100, "requires 100 distinct bit-history IDs");
    let mut group = c.benchmark_group(format!("{fixture}/directory/cold-wire-batch/t90000"));
    group.sample_size(10);
    group.bench_function(
        BenchmarkId::new("fresh-selection/bases100/slots100", BACKEND),
        |b| {
            b.iter(|| {
                black_box(
                    wave.samples(black_box(&signals), Time::from_ticks(90_000))
                        .unwrap(),
                )
            })
        },
    );
    group.finish();
}

pub(super) fn register(c: &mut Criterion) {
    histories(c);
    wires(c);
    imported_batch(c);
    disjoint_calls(c);
}
