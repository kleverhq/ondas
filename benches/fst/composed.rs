use std::{hint::black_box, ops::ControlFlow, time::Duration};

use criterion::{BenchmarkId, Criterion};
use ondas::{Time, TimeRange, ValueRef};

use super::{BACKEND, fixtures, workloads};

fn range(start: u64, end: u64) -> TimeRange {
    TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end))
}

fn consume(time: Time, value: ValueRef<'_>) {
    black_box((time, value));
}

pub(super) fn wide(c: &mut Criterion) {
    let fixture = "fst0083-wide-compact-toggle";
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).unwrap();
    let scalar = wave.hierarchy().signal("top.control").unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    let low = wide.slice(0, 0).unwrap();
    let mut group = c.benchmark_group(format!(
        "fst/{}/{fixture}/file/normal/composed",
        fixtures::PROVIDER
    ));
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(3));

    // W1: one continuing owner versus independent point queries. Identical
    // explicit timestamps in the point comparisons, including the backstep.
    let mut selection = wave.select(&[scalar, low]).unwrap();
    for (name, ticks) in [
        ("repeat", [4094, 4094, 4094]),
        ("forward", [4093, 4094, 4095]),
        ("backstep", [4094, 4095, 4094]),
    ] {
        group.bench_function(
            BenchmarkId::new(format!("w1/points/{name}/prepared"), BACKEND),
            |b| {
                b.iter(|| {
                    for tick in black_box(ticks) {
                        black_box(selection.samples(Time::from_ticks(tick)).unwrap());
                    }
                });
            },
        );
    }
    for (start, end) in [(1, 3), (4093, 4095)] {
        group.bench_function(
            BenchmarkId::new(format!("w1/adjacent/{start}..={end}/prepared"), BACKEND),
            |b| {
                b.iter(|| {
                    black_box(
                        workloads::adjacent_query(
                            &mut selection,
                            black_box(range(start, end)),
                            |at, value| {
                                black_box((at, value));
                            },
                        )
                        .unwrap(),
                    );
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(
                format!("w1/adjacent-points/{start}..={end}/prepared"),
                BACKEND,
            ),
            |b| {
                b.iter(|| {
                    for tick in black_box(start)..=black_box(end) {
                        for at in [tick - 1, tick, tick - 1] {
                            let _ = selection
                                .visit_samples(Time::from_ticks(at), |value| {
                                    black_box(value);
                                    ControlFlow::<()>::Continue(())
                                })
                                .unwrap();
                        }
                    }
                });
            },
        );
    }
    // W2: distinct scalar driver versus payload projection sharing its base.
    // The control slot is the payload low bit in both cases. Acceptance is 64
    // of 4096 ticks; the driver and payload remain active on rejected ticks.
    for (name, driver) in [("distinct", scalar), ("shared-base", low)] {
        let mut selection = wave.select(&[driver, low, wide]).unwrap();
        for (mode, eager) in [("lazy", false), ("eager-owned", true)] {
            group.bench_function(
                BenchmarkId::new(
                    format!("w2/{name}/{mode}/stride64/1..=4096/prepared"),
                    BACKEND,
                ),
                |b| {
                    b.iter(|| {
                        black_box(
                            workloads::conditional_query(
                                &mut selection,
                                black_box(range(1, 4096)),
                                64,
                                usize::MAX,
                                eager,
                                consume,
                            )
                            .unwrap(),
                        )
                    })
                },
            );
        }
        group.bench_function(
            BenchmarkId::new(
                format!("w2/{name}/scan/stride64/1..=4096/prepared"),
                BACKEND,
            ),
            |b| {
                b.iter(|| {
                    black_box(
                        workloads::conditional_scan(
                            &mut selection,
                            black_box(range(1, 4096)),
                            64,
                            usize::MAX,
                            consume,
                        )
                        .unwrap(),
                    )
                })
            },
        );
    }
    // Zero caller payload reads versus all accepted reads, with the SAME selected
    // bases and range: these do not claim avoided internal payload decoding.
    for (mode, stride, window) in [
        ("no-match", 8192, range(2, 4096)),
        ("dense", 2, range(2, 4096)),
    ] {
        let mut selection = wave.select(&[scalar, low, wide]).unwrap();
        group.bench_function(
            BenchmarkId::new(format!("w2/distinct/{mode}/2..=4096/prepared"), BACKEND),
            |b| {
                b.iter(|| {
                    black_box(
                        workloads::conditional_query(
                            &mut selection,
                            black_box(window),
                            stride,
                            usize::MAX,
                            false,
                            consume,
                        )
                        .unwrap(),
                    )
                })
            },
        );
    }
    // W3: accept at 2049 after 2047 rejected ticks, not the first callback.
    // Full and stopped traversal have identical query bounds and predicates.
    for (name, limit) in [("full", usize::MAX), ("first", 1)] {
        let mut selection = wave.select(&[scalar, low, wide]).unwrap();
        for scan in [false, true] {
            let api = if scan { "scan" } else { "query" };
            group.bench_function(
                BenchmarkId::new(
                    format!("w3/{api}/{name}/stride2048/2..=4096/prepared"),
                    BACKEND,
                ),
                |b| {
                    b.iter(|| {
                        let result = if scan {
                            workloads::conditional_scan(
                                &mut selection,
                                black_box(range(2, 4096)),
                                2048,
                                limit,
                                consume,
                            )
                        } else {
                            workloads::conditional_query(
                                &mut selection,
                                black_box(range(2, 4096)),
                                2048,
                                limit,
                                false,
                                consume,
                            )
                        };
                        black_box(result.unwrap());
                    })
                },
            );
        }
    }
    {
        let mut selection = wave.select(&[scalar, low, wide]).unwrap();
        group.bench_function(
            BenchmarkId::new("w3/query/first/stride64/4000..=4096/prepared", BACKEND),
            |b| {
                b.iter(|| {
                    black_box(
                        workloads::conditional_query(
                            &mut selection,
                            black_box(range(4000, 4096)),
                            64,
                            1,
                            false,
                            consume,
                        )
                        .unwrap(),
                    )
                })
            },
        );
    }
    group.bench_function(
        BenchmarkId::new(
            "w3/query/first/stride2048/2..=4096/fresh-open-select-drop",
            BACKEND,
        ),
        |b| {
            b.iter(|| {
                let mut wave = ondas::open_with(black_box(&path), BACKEND).unwrap();
                let scalar = wave.hierarchy().signal("top.control").unwrap();
                let wide = wave.hierarchy().signal("top.wide").unwrap();
                let low = wide.slice(0, 0).unwrap();
                black_box(
                    workloads::conditional_query(
                        &mut wave.select(&[scalar, low, wide]).unwrap(),
                        range(2, 4096),
                        2048,
                        1,
                        false,
                        consume,
                    )
                    .unwrap(),
                );
            })
        },
    );
    group.finish();
}

pub(super) fn sections(c: &mut Criterion) {
    let fixture = "fst0015-scr1-max-ahb-coremark";
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).unwrap();
    let clock = wave.hierarchy().signal("TOP.clk").unwrap();
    let constant = wave
        .hierarchy()
        .signal("TOP.$unit.SCR1_ARCH_RST_VECTOR")
        .unwrap();
    let mut selection = wave.select(&[clock, constant]).unwrap();
    let mut group = c.benchmark_group(format!(
        "fst/{}/{fixture}/file/normal/composed",
        fixtures::PROVIDER
    ));
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(3));
    // Equal 45-tick windows; two cross actual value-section starts.
    for start in [1000, 745320, 5812370] {
        group.bench_function(
            BenchmarkId::new(
                format!("w1/adjacent/{start}..={}/prepared", start + 44),
                BACKEND,
            ),
            |b| {
                b.iter(|| {
                    black_box(
                        workloads::adjacent_query(
                            &mut selection,
                            black_box(range(start, start + 44)),
                            |at, sample| {
                                black_box((at, sample));
                            },
                        )
                        .unwrap(),
                    )
                })
            },
        );
    }
    group.finish();
}

pub(super) fn wrapped(c: &mut Criterion) {
    // This locked input has an FST whole-file gzip wrapper, not ordinary
    // per-section compression. Its tiny size does not model large wrapper RSS.
    let fixture = "fst0050-wellen-32";
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let open = || ondas::open_with(black_box(&path), BACKEND).unwrap();
    let mut wave = open();
    let signal = wave.hierarchy().signal("wellen_32.spisub_s.cs_n").unwrap();
    let window = range(1, 2_000_000_000);
    let mut group = c.benchmark_group(format!(
        "fst/{}/{fixture}/file/whole-file-gzip/composed",
        fixtures::PROVIDER
    ));
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(3));
    group.bench_function(BenchmarkId::new("open-drop", BACKEND), |b| {
        b.iter(|| drop(black_box(open())))
    });
    let mut selection = wave.select(&[signal]).unwrap();
    group.bench_function(
        BenchmarkId::new("w3/first-high/1..=2000000000/prepared", BACKEND),
        |b| {
            b.iter(|| {
                let _ =
                    black_box(workloads::first_high(&mut selection, black_box(window)).unwrap());
            })
        },
    );
    group.bench_function(
        BenchmarkId::new(
            "w3/first-high/1..=2000000000/fresh-open-select-drop",
            BACKEND,
        ),
        |b| {
            b.iter(|| {
                let mut wave = open();
                let signal = wave.hierarchy().signal("wellen_32.spisub_s.cs_n").unwrap();
                let _ = black_box(
                    workloads::first_high(&mut wave.select(&[signal]).unwrap(), window).unwrap(),
                );
            })
        },
    );
    group.finish();
}
