use std::{collections::HashSet, hint::black_box, ops::ControlFlow};

use criterion::{BenchmarkId, Criterion, Throughput};
use ondas::{Time, TimeRange};

use super::{BACKEND, candidate_count, first_change, fixtures, scan_count};

pub(super) fn history(c: &mut Criterion, fixture: &str, end: u64) {
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).unwrap();
    let clock = wave.hierarchy().signal("top.clock").unwrap();
    let words = (0..64)
        .map(|i| {
            wave.hierarchy()
                .signal(&format!("top.word_{i:02}"))
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(words.iter().collect::<HashSet<_>>().len(), 64);
    let name = format!("fsdb/{}/{fixture}/file", fixtures::PROVIDER);
    let mut group = c.benchmark_group(&name);
    group.sample_size(10);
    group.bench_function(BenchmarkId::new("open", BACKEND), |b| {
        b.iter(|| {
            drop(black_box(
                ondas::open_with(black_box(&path), BACKEND).unwrap(),
            ))
        });
    });
    {
        let mut selection = wave.select(&[clock]).unwrap();
        for tick in [16, end - 16] {
            let time = Time::from_ticks(tick);
            group.bench_function(
                BenchmarkId::new(format!("sample/top.clock/t{tick}"), BACKEND),
                |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
            );
        }
        // Identical early output across short/long files exposes unused-suffix load cost.
        for (start, stop) in [(17, 49), (end - 31, end)] {
            let range = TimeRange::closed(Time::from_ticks(start), Time::from_ticks(stop));
            assert!(scan_count(&mut selection, range) > 1);
            group.bench_function(
                BenchmarkId::new(format!("scan/top.clock/{start}..={stop}"), BACKEND),
                |b| b.iter(|| black_box(scan_count(&mut selection, black_box(range)))),
            );
        }
        for stop in [49, end] {
            let range = TimeRange::closed(Time::from_ticks(17), Time::from_ticks(stop));
            assert!(selection.scan(range, first_change).unwrap().is_break());
            group.bench_function(
                BenchmarkId::new(format!("first-change/top.clock/17..={stop}"), BACKEND),
                |b| b.iter(|| black_box(selection.scan(black_box(range), first_change).unwrap())),
            );
        }
    }
    let time = Time::from_ticks(end);
    let range = TimeRange::closed(Time::from_ticks(end - 31), time);
    for count in [1, 4, 64] {
        let signals = &words[..count];
        group.bench_function(
            BenchmarkId::new(
                format!("samples-batch/top.word_00/prefix{count}/t{end}"),
                BACKEND,
            ),
            |b| b.iter(|| black_box(wave.samples(black_box(signals), black_box(time)).unwrap())),
        );
        let mut selection = wave.select(signals).unwrap();
        group.bench_function(
            BenchmarkId::new(
                format!("samples-prepared/top.word_00/prefix{count}/t{end}"),
                BACKEND,
            ),
            |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
        );
        assert!(scan_count(&mut selection, range) > count as u64);
        group.bench_function(
            BenchmarkId::new(
                format!("scan/top.word_00/prefix{count}/{}..={end}", end - 31),
                BACKEND,
            ),
            |b| b.iter(|| black_box(scan_count(&mut selection, black_box(range)))),
        );
        assert!(candidate_count(&mut selection, range) > 0);
        group.bench_function(
            BenchmarkId::new(
                format!(
                    "candidate-times/top.word_00/prefix{count}/{}..={end}",
                    end - 31
                ),
                BACKEND,
            ),
            |b| b.iter(|| black_box(candidate_count(&mut selection, black_box(range)))),
        );
    }
    // Match four independent one-shot queries against the batch/prepared cases above.
    group.bench_function(
        BenchmarkId::new(
            format!("samples-individual/top.word_00/prefix4/t{end}"),
            BACKEND,
        ),
        |b| {
            b.iter(|| {
                for &signal in black_box(&words[..4]) {
                    black_box(wave.sample(signal, black_box(time)).unwrap());
                }
            })
        },
    );
    // Full traversal is streamed; owned results remain bounded to a 32-tick window.
    let mut selection = wave.select(&[clock]).unwrap();
    let full = TimeRange::closed(Time::from_ticks(17), time);
    assert!(scan_count(&mut selection, full) > 1);
    group.bench_function(
        BenchmarkId::new(format!("scan/top.clock/17..={end}"), BACKEND),
        |b| b.iter(|| black_box(scan_count(&mut selection, black_box(full)))),
    );
    group.bench_function(
        BenchmarkId::new(format!("trace/top.clock/{}..={end}", end - 31), BACKEND),
        |b| b.iter(|| black_box(selection.traces(black_box(range)).unwrap())),
    );
    group.finish();

    let mut group = c.benchmark_group(format!("{name}/sample-series"));
    group.sample_size(10);
    // A fixed eight-query block keeps timestamp distribution equal across counts.
    for count in [8_u64, 32] {
        let times = (0..8)
            .map(|i| Time::from_ticks(end - (7 - i) * 16))
            .cycle()
            .take(count as usize)
            .collect::<Vec<_>>();
        assert_eq!(times.len(), count as usize);
        assert!(times.chunks(8).all(|block| block == &times[..8]));
        group.throughput(Throughput::Elements(count));
        group.bench_function(
            BenchmarkId::new(
                format!("top.clock/n{count}/block8/end{end}/step16"),
                BACKEND,
            ),
            |b| {
                b.iter(|| {
                    for &time in black_box(&times) {
                        black_box(selection.samples(time).unwrap());
                    }
                })
            },
        );
    }
    group.finish();
}

pub(super) fn topology(c: &mut Criterion, fixture: &str, histories: usize) {
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).unwrap();
    assert_eq!(wave.hierarchy().signals().count(), histories);
    let sparse = wave.hierarchy().signal("top.probe_sparse").unwrap();
    let constant = wave.hierarchy().signal("top.probe_constant").unwrap();
    let mut group = c.benchmark_group(format!("fsdb/{}/{fixture}/file", fixtures::PROVIDER));
    group.sample_size(20);
    group.bench_function(BenchmarkId::new("open", BACKEND), |b| {
        b.iter(|| {
            drop(black_box(
                ondas::open_with(black_box(&path), BACKEND).unwrap(),
            ))
        });
    });
    group.bench_function(BenchmarkId::new("hierarchy/variable-names", BACKEND), |b| {
        b.iter(|| {
            for v in wave.hierarchy().variables() {
                black_box(v.name());
            }
        });
    });
    group.bench_function(
        BenchmarkId::new("lookup/top.probe_constant", BACKEND),
        |b| {
            b.iter(|| {
                black_box(
                    wave.hierarchy()
                        .signal(black_box("top.probe_constant"))
                        .unwrap(),
                )
            });
        },
    );
    let time = Time::from_ticks(524_288);
    let quiet = TimeRange::closed(time, Time::from_ticks(524_320));
    for (name, signal) in [
        ("top.probe_sparse", sparse),
        ("top.probe_constant", constant),
    ] {
        let mut selection = wave.select(&[signal]).unwrap();
        group.bench_function(
            BenchmarkId::new(format!("sample/{name}/t524288"), BACKEND),
            |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
        );
        assert_eq!(scan_count(&mut selection, quiet), 1);
        group.bench_function(
            BenchmarkId::new(format!("scan/{name}/524288..=524320"), BACKEND),
            |b| b.iter(|| black_box(scan_count(&mut selection, black_box(quiet)))),
        );
        group.bench_function(
            BenchmarkId::new(format!("candidate-times/{name}/524288..=524320"), BACKEND),
            |b| b.iter(|| black_box(candidate_count(&mut selection, black_box(quiet)))),
        );
    }
    let mut selection = wave.select(&[sparse]).unwrap();
    let full = TimeRange::closed(Time::from_ticks(1), Time::from_ticks(1_048_576));
    assert_eq!(scan_count(&mut selection, full), 3);
    group.bench_function(
        BenchmarkId::new("scan/top.probe_sparse/1..=1048576", BACKEND),
        |b| {
            b.iter(|| black_box(scan_count(&mut selection, black_box(full))));
        },
    );
    assert!(selection.scan(full, first_change).unwrap().is_break());
    group.bench_function(
        BenchmarkId::new("first-change/top.probe_sparse/1..=1048576", BACKEND),
        |b| {
            b.iter(|| black_box(selection.scan(black_box(full), first_change).unwrap()));
        },
    );
    group.finish();
}

pub(super) fn wide(c: &mut Criterion, fixture: &str) {
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).unwrap();
    let lsb = wave.hierarchy().signal("top.wide").unwrap();
    let msb = wave.hierarchy().signal("top.wide_msb").unwrap();
    let control = wave.hierarchy().signal("top.control").unwrap();
    let time = Time::from_ticks(2056);
    let range = TimeRange::closed(Time::from_ticks(2048), time);
    let mut group = c.benchmark_group(format!("fsdb/{}/{fixture}/file", fixtures::PROVIDER));
    group.sample_size(10);
    for (name, signal, active) in [
        ("top.wide/whole4096", lsb, true),
        ("top.wide_msb/whole4096", msb, true),
        ("top.control", control, true),
        ("top.wide/0:0", lsb.slice(0, 0).unwrap(), true),
        ("top.wide/4095:1", lsb.slice(4095, 1).unwrap(), false),
    ] {
        let mut selection = wave.select(&[signal]).unwrap();
        group.bench_function(
            BenchmarkId::new(format!("sample/{name}/t2056"), BACKEND),
            |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
        );
        group.bench_function(
            BenchmarkId::new(format!("visit-samples/{name}/t2056"), BACKEND),
            |b| {
                b.iter(|| {
                    black_box(
                        selection
                            .visit_samples(black_box(time), |sample| {
                                black_box(sample);
                                ControlFlow::<()>::Continue(())
                            })
                            .unwrap(),
                    )
                })
            },
        );
        let records = scan_count(&mut selection, range);
        if active {
            assert!(records > 1);
        } else {
            assert_eq!(records, 1);
        }
        group.bench_function(
            BenchmarkId::new(format!("scan/{name}/2048..=2056"), BACKEND),
            |b| b.iter(|| black_box(scan_count(&mut selection, black_box(range)))),
        );
        group.bench_function(
            BenchmarkId::new(format!("candidate-times/{name}/2048..=2056"), BACKEND),
            |b| b.iter(|| black_box(candidate_count(&mut selection, black_box(range)))),
        );
        group.bench_function(
            BenchmarkId::new(format!("trace/{name}/2048..=2056"), BACKEND),
            |b| b.iter(|| black_box(selection.traces(black_box(range)).unwrap())),
        );
    }
    // Single-entry queries are already covered by the sample cases above.
    for (name, signal) in [("whole4096", lsb), ("low-bit", lsb.slice(0, 0).unwrap())] {
        for count in [8, 64] {
            let signals = vec![signal; count];
            let mut selection = wave.select(&signals).unwrap();
            assert_eq!(selection.samples(time).unwrap().len(), count);
            group.bench_function(
                BenchmarkId::new(
                    format!("samples/top.wide/{name}/duplicates{count}/t2056"),
                    BACKEND,
                ),
                |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
            );
        }
    }
    group.finish();
}

pub(super) fn aliases(c: &mut Criterion, fixture: &str) {
    // The matching 16-history topology cases are registered by the caller too.
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).unwrap();
    let sparse = wave.hierarchy().signal("top.probe_sparse").unwrap();
    let constant = wave.hierarchy().signal("top.probe_constant").unwrap();
    assert_eq!(wave.hierarchy().signals().count(), 16);
    assert_eq!(wave.hierarchy().variables().count(), 4112);
    let signals = std::iter::once(sparse)
        .chain((0..63).map(|i| {
            let path = format!("top.alias_group_{:03}.alias_{:02}", i / 16, i % 16);
            let signal = wave.hierarchy().signal(&path).unwrap();
            assert_eq!(signal, if i % 2 == 0 { sparse } else { constant });
            signal
        }))
        .collect::<Vec<_>>();
    let time = Time::from_ticks(524_288);
    let mut group = c.benchmark_group(format!(
        "fsdb/{}/{fixture}/file/aliases",
        fixtures::PROVIDER
    ));
    group.sample_size(20);
    for count in [1, 8, 64] {
        group.bench_function(
            BenchmarkId::new(
                format!("select/base+alias-group-000..003/prefix{count}"),
                BACKEND,
            ),
            |b| {
                b.iter(|| {
                    drop(black_box(
                        wave.select(black_box(&signals[..count])).unwrap(),
                    ))
                })
            },
        );
        let mut selection = wave.select(&signals[..count]).unwrap();
        assert_eq!(selection.samples(time).unwrap().len(), count);
        group.bench_function(
            BenchmarkId::new(
                format!("samples/base+alias-group-000..003/prefix{count}/t524288"),
                BACKEND,
            ),
            |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
        );
    }
    group.finish();
}
