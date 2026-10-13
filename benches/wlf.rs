use std::{hint::black_box, ops::ControlFlow, path::Path};

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use ondas::{ScanRef, Selection, Time, TimeRange};

#[path = "wlf/controlled.rs"]
mod controlled;
#[path = "../tests/support/fixtures.rs"]
mod fixtures;

const BACKEND: &str = "wlf-lib";

fn range(start: u64, end: u64) -> TimeRange {
    TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end))
}

fn scan_count(selection: &mut Selection<'_>, window: TimeRange) -> u64 {
    let mut count = 0;
    let _ = selection
        .scan(window, |_| {
            count += 1;
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    count
}

fn candidate_count(selection: &mut Selection<'_>, window: TimeRange) -> u64 {
    let mut count = 0;
    let _ = selection
        .scan_candidate_times(window, |_| {
            count += 1;
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    count
}

fn first_change(row: ScanRef<'_>) -> ControlFlow<Time> {
    match row {
        ScanRef::Change { time, .. } => ControlFlow::Break(time),
        _ => ControlFlow::Continue(()),
    }
}

fn opening(c: &mut Criterion, fixture: &str, path: &Path, lookup: &str) {
    let wave = ondas::open_with(path, BACKEND).unwrap();
    wave.hierarchy().signal(lookup).unwrap();
    let mut group = c.benchmark_group(format!("{fixture}/file/opening"));
    group.sample_size(10);
    group.bench_function(BenchmarkId::new("metadata-only", BACKEND), |b| {
        b.iter(|| black_box(ondas::read_metadata(black_box(path)).unwrap()));
    });
    // Open/drop includes SDK setup, owned declarations and cleanup, not cold disk.
    group.bench_function(BenchmarkId::new("open-drop", BACKEND), |b| {
        b.iter(|| {
            drop(black_box(
                ondas::open_with(black_box(path), BACKEND).unwrap(),
            ))
        });
    });
    group.bench_function(BenchmarkId::new("hierarchy/variable-names", BACKEND), |b| {
        b.iter(|| {
            for variable in wave.hierarchy().variables() {
                black_box(variable.name());
            }
        });
    });
    for prior in [0, 1] {
        group.bench_function(
            BenchmarkId::new(format!("lookup/fresh/prior{prior}/{lookup}"), BACKEND),
            |b| {
                b.iter_batched_ref(
                    || {
                        let wave = ondas::open_with(path, BACKEND).unwrap();
                        if prior == 1 {
                            black_box(wave.hierarchy().signal(lookup).unwrap());
                        }
                        wave
                    },
                    |wave| black_box(wave.hierarchy().signal(black_box(lookup)).unwrap()),
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.bench_function(
        BenchmarkId::new(format!("lookup/repeated/{lookup}"), BACKEND),
        |b| {
            b.iter(|| black_box(wave.hierarchy().signal(black_box(lookup)).unwrap()));
        },
    );
    group.finish();
}

fn typed(c: &mut Criterion, fixture: &str, path: &Path, name: &str, tick: u64, end: u64) {
    let mut wave = ondas::open_with(path, BACKEND).unwrap();
    let signal = wave.hierarchy().signal(name).unwrap();
    let mut selected = wave.select(&[signal]).unwrap();
    let time = Time::from_ticks(tick);
    let window = range(0, end);
    let mut group = c.benchmark_group(format!("{fixture}/file/typed"));
    group.sample_size(10);
    group.bench_function(
        BenchmarkId::new(format!("samples-prepared/{name}/t{tick}"), BACKEND),
        |b| {
            b.iter(|| black_box(selected.samples(black_box(time)).unwrap()));
        },
    );
    group.bench_function(
        BenchmarkId::new(format!("visit-samples/{name}/t{tick}"), BACKEND),
        |b| {
            b.iter(|| {
                let _ = black_box(
                    selected
                        .visit_samples(black_box(time), |sample| {
                            black_box(sample);
                            ControlFlow::<()>::Continue(())
                        })
                        .unwrap(),
                );
            });
        },
    );
    group.bench_function(
        BenchmarkId::new(format!("scan/{name}/0..={end}"), BACKEND),
        |b| {
            b.iter(|| black_box(scan_count(&mut selected, black_box(window))));
        },
    );
    group.bench_function(
        BenchmarkId::new(format!("trace/{name}/0..={end}"), BACKEND),
        |b| {
            b.iter(|| black_box(selected.traces(black_box(window)).unwrap()));
        },
    );
    group.bench_function(
        BenchmarkId::new(format!("candidates/{name}/0..={end}"), BACKEND),
        |b| {
            b.iter(|| black_box(candidate_count(&mut selected, black_box(window))));
        },
    );
    group.finish();
}

fn workloads(c: &mut Criterion) {
    let root = fixtures::root();
    for (fixture, end) in [
        ("wlf/wlf0074-history-short", 256),
        ("wlf/wlf0075-history-long", 4096),
    ] {
        let (path, _) = fixtures::load_artifact(&root, fixture);
        opening(c, fixture, &path, "top.word_63");
        controlled::history(c, fixture, &path, end);
    }
    for (fixture, aliases) in [
        ("wlf/wlf0076-topology-small", false),
        ("wlf/wlf0077-topology-many-handles", false),
        ("wlf/wlf0078-topology-many-times", false),
        ("wlf/wlf0080-topology-many-aliases", true),
    ] {
        let (path, _) = fixtures::load_artifact(&root, fixture);
        opening(
            c,
            fixture,
            &path,
            if aliases {
                "top.alias_255"
            } else {
                "top.probe_sparse"
            },
        );
        controlled::topology(c, fixture, &path, aliases);
    }
    let fixture = "wlf/wlf0079-wide-compact-toggle";
    let (path, _) = fixtures::load_artifact(&root, fixture);
    opening(c, fixture, &path, "top.wide");
    controlled::wide(c, fixture, &path);
    for (fixture, name, tick, end) in [
        ("wlf/wlf0070-typed-deltas-aliases", "top.real_value", 14, 14),
        ("wlf/wlf0071-event-occurrences", "top.pulse", 3, 6),
        ("wlf/wlf0072-strings-and-names", "top.message", 5, 7),
    ] {
        let (path, _) = fixtures::load_artifact(&root, fixture);
        typed(c, fixture, &path, name, tick, end);
    }
    // An imported ModelSim recording supplements the controlled native Writer files.
    let fixture = "wlf/wlf0038-count4b-3-2-vsim";
    let (path, _) = fixtures::load_artifact(&root, fixture);
    opening(c, fixture, &path, "count4b.clk");
    typed(c, fixture, &path, "count4b.clk", 100, 200);
}

criterion_group!(benches, workloads);
criterion_main!(benches);
