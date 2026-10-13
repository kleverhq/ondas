use std::{collections::HashSet, hint::black_box, path::Path};

use criterion::{BenchmarkId, Criterion, Throughput};
use ondas::Time;

use super::{BACKEND, candidate_count, first_change, range, scan_count};

pub(super) fn history(c: &mut Criterion, fixture: &str, path: &Path, end: u64) {
    let mut wave = ondas::open_with(path, BACKEND).unwrap();
    let clock = wave.hierarchy().signal("top.clock").unwrap();
    let words = (0..64)
        .map(|i| {
            wave.hierarchy()
                .signal(&format!("top.word_{i:02}"))
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(words.iter().collect::<HashSet<_>>().len(), 64);
    assert!(words.iter().all(|s| s.width() == Some(32)));
    let time = Time::from_ticks(end);
    let mut group = c.benchmark_group(format!("{fixture}/file/history"));
    group.sample_size(10);
    for count in [1, 16, 64] {
        let signals = &words[..count];
        group.bench_function(
            BenchmarkId::new(format!("select/words/prefix{count}"), BACKEND),
            |b| {
                b.iter(|| drop(black_box(wave.select(black_box(signals)).unwrap())));
            },
        );
        group.bench_function(
            BenchmarkId::new(format!("samples-batch/words/prefix{count}/t{end}"), BACKEND),
            |b| {
                b.iter(|| black_box(wave.samples(black_box(signals), black_box(time)).unwrap()));
            },
        );
        let mut selected = wave.select(signals).unwrap();
        group.bench_function(
            BenchmarkId::new(
                format!("samples-prepared/words/prefix{count}/t{end}"),
                BACKEND,
            ),
            |b| {
                b.iter(|| black_box(selected.samples(black_box(time)).unwrap()));
            },
        );
        let window = range(end - 31, end);
        group.bench_function(
            BenchmarkId::new(
                format!("scan/words/prefix{count}/{}..={end}", end - 31),
                BACKEND,
            ),
            |b| {
                b.iter(|| black_box(scan_count(&mut selected, black_box(window))));
            },
        );
    }
    // Matched four-base one-shot cases include selection and result destruction.
    for batch in [false, true] {
        group.bench_function(
            BenchmarkId::new(
                format!(
                    "samples-{}/words/prefix4/t{end}",
                    if batch { "batch" } else { "individual" }
                ),
                BACKEND,
            ),
            |b| {
                b.iter(|| {
                    if batch {
                        black_box(
                            wave.samples(black_box(&words[..4]), black_box(time))
                                .unwrap(),
                        );
                    } else {
                        for &signal in black_box(&words[..4]) {
                            black_box(wave.sample(signal, black_box(time)).unwrap());
                        }
                    }
                });
            },
        );
    }
    for count in [1, 4] {
        let high = words[..count]
            .iter()
            .map(|s| s.slice(31, 16).unwrap())
            .collect::<Vec<_>>();
        group.bench_function(
            BenchmarkId::new(
                format!("samples-batch/stable-high/words/prefix{count}/t{end}"),
                BACKEND,
            ),
            |b| {
                b.iter(|| black_box(wave.samples(black_box(&high), black_box(time)).unwrap()));
            },
        );
    }
    let mut selected = wave.select(&[clock]).unwrap();
    for (start, stop) in [(17, 48), (end - 31, end)] {
        let window = range(start, stop);
        let time = Time::from_ticks(start);
        group.bench_function(
            BenchmarkId::new(format!("samples-prepared/clock/t{start}"), BACKEND),
            |b| {
                b.iter(|| black_box(selected.samples(black_box(time)).unwrap()));
            },
        );
        group.bench_function(
            BenchmarkId::new(format!("scan/clock/{start}..={stop}"), BACKEND),
            |b| {
                b.iter(|| black_box(scan_count(&mut selected, black_box(window))));
            },
        );
        group.bench_function(
            BenchmarkId::new(format!("trace/clock/{start}..={stop}"), BACKEND),
            |b| {
                b.iter(|| black_box(selected.traces(black_box(window)).unwrap()));
            },
        );
        group.bench_function(
            BenchmarkId::new(format!("candidates/clock/{start}..={stop}"), BACKEND),
            |b| {
                b.iter(|| black_box(candidate_count(&mut selected, black_box(window))));
            },
        );
        group.bench_function(
            BenchmarkId::new(format!("first-change/clock/{start}..={stop}"), BACKEND),
            |b| {
                b.iter(|| black_box(selected.scan(black_box(window), first_change).unwrap()));
            },
        );
    }
    let window = range(17, end);
    group.bench_function(
        BenchmarkId::new(format!("scan-full/clock/17..={end}"), BACKEND),
        |b| {
            b.iter(|| black_box(scan_count(&mut selected, black_box(window))));
        },
    );
    group.bench_function(
        BenchmarkId::new(format!("first-change/clock/17..={end}"), BACKEND),
        |b| {
            b.iter(|| black_box(selected.scan(black_box(window), first_change).unwrap()));
        },
    );
    group.finish();
    let mut group = c.benchmark_group(format!("{fixture}/file/sample-series"));
    group.sample_size(10);
    for count in [8, 32] {
        let times = (0..8)
            .map(|i| Time::from_ticks(end - (7 - i) * 16))
            .cycle()
            .take(count)
            .collect::<Vec<_>>();
        group.throughput(Throughput::Elements(count as u64));
        group.bench_function(
            BenchmarkId::new(format!("clock/block8/n{count}/end{end}/step16"), BACKEND),
            |b| {
                b.iter(|| {
                    for &time in black_box(&times) {
                        black_box(selected.samples(time).unwrap());
                    }
                });
            },
        );
    }
    group.finish();
    drop(selected);

    // Fresh selections measure cold entering-state work, independently of reuse.
    let mut group = c.benchmark_group(format!("{fixture}/file/history/regression"));
    group.sample_size(10);
    for (label, start, stop) in [
        ("early", 17, 48),
        ("late", end - 31, end),
        ("short-nonzero", 17, 18),
    ] {
        let window = range(start, stop);
        group.bench_function(
            BenchmarkId::new(
                format!("scan/fresh-selection/clock/{label}/{start}..={stop}"),
                BACKEND,
            ),
            |b| {
                b.iter(|| {
                    let mut selection = wave.select(&[clock]).unwrap();
                    black_box(scan_count(&mut selection, black_box(window)));
                });
            },
        );
    }
    group.finish();
}

pub(super) fn topology(c: &mut Criterion, fixture: &str, path: &Path, aliases: bool) {
    let mut wave = ondas::open_with(path, BACKEND).unwrap();
    let sparse = wave.hierarchy().signal("top.probe_sparse").unwrap();
    let constant = wave.hierarchy().signal("top.probe_constant").unwrap();
    if aliases {
        assert_eq!(wave.hierarchy().signal("top.alias_255").unwrap(), sparse);
        assert_eq!(wave.hierarchy().aliases(sparse).unwrap().count(), 257);
    }
    let mut group = c.benchmark_group(format!("{fixture}/file/topology"));
    group.sample_size(10);
    for (name, signal) in [("probe_sparse", sparse), ("probe_constant", constant)] {
        let mut selected = wave.select(&[signal]).unwrap();
        for (start, end) in [(17, 49), (4031, 4063)] {
            let window = range(start, end);
            group.bench_function(
                BenchmarkId::new(format!("scan/{name}/{start}..={end}"), BACKEND),
                |b| {
                    b.iter(|| black_box(scan_count(&mut selected, black_box(window))));
                },
            );
            group.bench_function(
                BenchmarkId::new(format!("samples-prepared/{name}/t{end}"), BACKEND),
                |b| {
                    b.iter(|| {
                        black_box(selected.samples(black_box(Time::from_ticks(end))).unwrap())
                    });
                },
            );
        }
        let window = range(17, 129);
        group.bench_function(
            BenchmarkId::new(format!("first-change/{name}/17..=129"), BACKEND),
            |b| {
                b.iter(|| black_box(selected.scan(black_box(window), first_change).unwrap()));
            },
        );
    }
    if aliases {
        let inputs = (0..256)
            .map(|i| {
                wave.hierarchy()
                    .signal(&format!("top.alias_{i:03}"))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        for count in [1, 16, 256] {
            let mut selected = wave.select(&inputs[..count]).unwrap();
            group.bench_function(
                BenchmarkId::new(
                    format!("samples-prepared/aliases/entries{count}/bases1/t4096"),
                    BACKEND,
                ),
                |b| {
                    b.iter(|| {
                        black_box(selected.samples(black_box(Time::from_ticks(4096))).unwrap())
                    });
                },
            );
        }
    }
    group.finish();
}

pub(super) fn wide(c: &mut Criterion, fixture: &str, path: &Path) {
    let mut wave = ondas::open_with(path, BACKEND).unwrap();
    let bus = wave.hierarchy().signal("top.wide").unwrap();
    let control = wave.hierarchy().signal("top.control").unwrap();
    assert_eq!(bus.width(), Some(4096));
    assert_eq!(control.width(), Some(1));
    let low = bus.slice(0, 0).unwrap();
    let high = bus.slice(4095, 1).unwrap();
    let inputs = [
        ("whole", vec![bus]),
        ("low", vec![low]),
        ("stable-high", vec![high]),
        ("scalar", vec![control]),
        ("low-duplicate16", vec![low; 16]),
        ("whole-duplicate4", vec![bus; 4]),
        ("overlap", vec![bus, low, high]),
    ];
    let mut group = c.benchmark_group(format!("{fixture}/file/wide"));
    group.sample_size(10);
    for (label, signals) in inputs {
        let mut selected = wave.select(&signals).unwrap();
        let window = range(2017, 2048);
        let time = Time::from_ticks(2047);
        group.bench_function(
            BenchmarkId::new(format!("samples-prepared/{label}/t2047"), BACKEND),
            |b| {
                b.iter(|| black_box(selected.samples(black_box(time)).unwrap()));
            },
        );
        group.bench_function(
            BenchmarkId::new(format!("visit-samples/{label}/t2047"), BACKEND),
            |b| {
                b.iter(|| {
                    black_box(
                        selected
                            .visit_samples(black_box(time), |sample| {
                                black_box(sample);
                                std::ops::ControlFlow::<()>::Continue(())
                            })
                            .unwrap(),
                    )
                });
            },
        );
        group.bench_function(
            BenchmarkId::new(format!("scan/{label}/2017..=2048"), BACKEND),
            |b| {
                b.iter(|| black_box(scan_count(&mut selected, black_box(window))));
            },
        );
        group.bench_function(
            BenchmarkId::new(format!("trace/{label}/2017..=2048"), BACKEND),
            |b| {
                b.iter(|| black_box(selected.traces(black_box(window)).unwrap()));
            },
        );
        group.bench_function(
            BenchmarkId::new(format!("candidates/{label}/2017..=2048"), BACKEND),
            |b| {
                b.iter(|| black_box(candidate_count(&mut selected, black_box(window))));
            },
        );
    }
    group.finish();
}
