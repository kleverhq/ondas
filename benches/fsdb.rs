use std::{collections::HashSet, hint::black_box, ops::ControlFlow};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ondas::{ScanRef, Selection, Time, TimeRange};

#[path = "../tests/support/fixtures.rs"]
mod fixtures;

#[path = "fsdb/controlled.rs"]
mod controlled;
#[path = "fsdb/typed.rs"]
mod typed;

const BACKEND: &str = "fsdb-lib";

fn scan_count(selection: &mut Selection<'_>, range: TimeRange) -> u64 {
    let mut count = 0;
    let _ = selection
        .scan(range, |_| {
            count += 1;
            ControlFlow::<()>::Continue(())
        })
        .expect("scan workload");
    count
}

fn candidate_count(selection: &mut Selection<'_>, range: TimeRange) -> u64 {
    let mut count = 0;
    let _ = selection
        .scan_candidate_times(range, |_| {
            count += 1;
            ControlFlow::<()>::Continue(())
        })
        .expect("candidate workload");
    count
}

fn first_change(record: ScanRef<'_>) -> ControlFlow<Time> {
    match record {
        ScanRef::Change { time, .. } => ControlFlow::Break(time),
        _ => ControlFlow::Continue(()),
    }
}

fn compare(c: &mut Criterion) {
    let fixture = "fsdb0004-compare";
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).expect("open compare workload");
    let paths = [
        "compare_tb.clk",
        "compare_tb.dut.counter",
        "compare_tb.dut.status",
        "compare_tb.dut.unit_a.data",
    ];
    let signals = paths.map(|path| wave.hierarchy().signal(path).unwrap());
    assert_eq!(signals.iter().collect::<HashSet<_>>().len(), 4);
    let mut group = c.benchmark_group(format!("fsdb/{}/{fixture}/file", fixtures::PROVIDER));
    group.sample_size(20);

    // Includes fresh Reader construction and destruction, not cold-disk I/O.
    group.bench_function(BenchmarkId::new("open", BACKEND), |b| {
        b.iter(|| {
            drop(black_box(
                ondas::open_with(black_box(&path), BACKEND).unwrap(),
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
    group.bench_function(
        BenchmarkId::new(format!("lookup/{}", paths[3]), BACKEND),
        |b| {
            b.iter(|| black_box(wave.hierarchy().signal(black_box(paths[3])).unwrap()));
        },
    );
    for count in [1, 4] {
        group.bench_function(
            BenchmarkId::new(
                format!("select/clk+counter+status+data/prefix{count}"),
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
    }

    let time = Time::from_ticks(195);
    // Both one-shot alternatives include selection creation and result destruction.
    group.bench_function(
        BenchmarkId::new("samples-individual/clk+counter+status+data/t195", BACKEND),
        |b| {
            b.iter(|| {
                for &signal in black_box(&signals) {
                    black_box(wave.sample(signal, black_box(time)).unwrap());
                }
            })
        },
    );
    group.bench_function(
        BenchmarkId::new("samples-batch/clk+counter+status+data/t195", BACKEND),
        |b| b.iter(|| black_box(wave.samples(black_box(&signals), black_box(time)).unwrap())),
    );
    let mut selection = wave.select(&signals).unwrap();
    group.bench_function(
        BenchmarkId::new("samples-prepared/clk+counter+status+data/t195", BACKEND),
        |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
    );
    group.bench_function(
        BenchmarkId::new("visit-samples/clk+counter+status+data/t195", BACKEND),
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
            });
        },
    );
    // Owned full histories are bounded by this compact recording and fixed selection.
    let range = TimeRange::closed(Time::from_ticks(0), Time::from_ticks(205));
    assert!(scan_count(&mut selection, range) > signals.len() as u64);
    group.bench_function(
        BenchmarkId::new("scan/clk+counter+status+data/0..=205", BACKEND),
        |b| b.iter(|| black_box(scan_count(&mut selection, black_box(range)))),
    );
    group.bench_function(
        BenchmarkId::new("trace/clk+counter+status+data/0..=205", BACKEND),
        |b| b.iter(|| black_box(selection.traces(black_box(range)).unwrap())),
    );
    assert!(candidate_count(&mut selection, range) > 0);
    group.bench_function(
        BenchmarkId::new("candidate-times/clk+counter+status+data/0..=205", BACKEND),
        |b| b.iter(|| black_box(candidate_count(&mut selection, black_box(range)))),
    );
    group.finish();
}

fn mode_change(c: &mut Criterion) {
    let fixture = "fsdb0003-mode-change";
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).expect("open mode-change workload");
    let clock = wave.hierarchy().signal("doe_core_cbc.clk").unwrap();
    let ready = wave.hierarchy().signal("doe_core_cbc.ready").unwrap();
    let constant = wave
        .hierarchy()
        .signal("doe_core_cbc.enc_block.ready")
        .unwrap();
    let group_name = format!("fsdb/{}/{fixture}/file", fixtures::PROVIDER);
    let mut group = c.benchmark_group(&group_name);
    group.sample_size(20);
    group.bench_function(BenchmarkId::new("open", BACKEND), |b| {
        b.iter(|| {
            drop(black_box(
                ondas::open_with(black_box(&path), BACKEND).unwrap(),
            ))
        });
    });
    {
        let mut selection = wave.select(&[clock]).unwrap();
        // Equal-width active windows; the later query still traverses the prefix.
        for start in [20, 680] {
            let time = Time::from_ticks(start);
            group.bench_function(
                BenchmarkId::new(format!("sample/doe_core_cbc.clk/t{start}"), BACKEND),
                |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
            );
            let end = start + 20;
            let range = TimeRange::closed(time, Time::from_ticks(end));
            assert!(scan_count(&mut selection, range) > 1);
            group.bench_function(
                BenchmarkId::new(format!("scan/doe_core_cbc.clk/{start}..={end}"), BACKEND),
                |b| b.iter(|| black_box(scan_count(&mut selection, black_box(range)))),
            );
            let range = TimeRange::closed(time, Time::from_ticks(710));
            assert!(selection.scan(range, first_change).unwrap().is_break());
            assert!(scan_count(&mut selection, range) > 1);
            group.bench_function(
                BenchmarkId::new(format!("scan/doe_core_cbc.clk/{start}..=710"), BACKEND),
                |b| b.iter(|| black_box(scan_count(&mut selection, black_box(range)))),
            );
            // SDK loading happens before the callback can stop this traversal.
            group.bench_function(
                BenchmarkId::new(
                    format!("first-change/doe_core_cbc.clk/{start}..=710"),
                    BACKEND,
                ),
                |b| b.iter(|| black_box(selection.scan(black_box(range), first_change).unwrap())),
            );
        }
    }
    let quiet = TimeRange::closed(Time::from_ticks(300), Time::from_ticks(320));
    for (name, signal) in [
        ("doe_core_cbc.ready", ready),
        ("doe_core_cbc.enc_block.ready", constant),
    ] {
        let mut selection = wave.select(&[signal]).unwrap();
        // One entering-state record and no changes, not an empty selection.
        assert_eq!(scan_count(&mut selection, quiet), 1);
        group.bench_function(
            BenchmarkId::new(format!("scan/{name}/300..=320"), BACKEND),
            |b| b.iter(|| black_box(scan_count(&mut selection, black_box(quiet)))),
        );
        group.bench_function(
            BenchmarkId::new(format!("candidate-times/{name}/300..=320"), BACKEND),
            |b| b.iter(|| black_box(candidate_count(&mut selection, black_box(quiet)))),
        );
    }
    group.finish();

    let mut group = c.benchmark_group(format!("{group_name}/sample-series"));
    group.sample_size(20);
    let mut selection = wave.select(&[clock]).unwrap();
    // Repeat the same eight timestamps so throughput compares equal query work.
    // Reusing a selection retains grouping, not a decoded-history cache.
    for count in [8_u64, 32] {
        let times = (0..8)
            .map(|index| Time::from_ticks(700 - (7 - index) * 5))
            .cycle()
            .take(count as usize)
            .collect::<Vec<_>>();
        assert_eq!(times.len(), count as usize);
        assert!(times.chunks(8).all(|block| block == &times[..8]));
        group.throughput(Throughput::Elements(count));
        group.bench_function(
            BenchmarkId::new(
                format!("doe_core_cbc.clk/n{count}/block8/end700/step5"),
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

fn wide(c: &mut Criterion) {
    let fixture = "fsdb0009-wide-bus";
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).expect("open wide workload");
    let word = wave.hierarchy().signal("tb.dat1").unwrap();
    let low = word.slice(0, 0).unwrap();
    let stable = word.slice(63, 1).unwrap();
    let time = Time::from_ticks(60_000);
    let range = TimeRange::closed(Time::from_ticks(30_001), time);
    let mut group = c.benchmark_group(format!("fsdb/{}/{fixture}/file", fixtures::PROVIDER));
    group.sample_size(20);
    group.bench_function(BenchmarkId::new("open", BACKEND), |b| {
        b.iter(|| {
            drop(black_box(
                ondas::open_with(black_box(&path), BACKEND).unwrap(),
            ))
        });
    });
    // Same 1024-bit base history: whole/low change, while bits 63:1 stay fixed.
    // This fixture has only two post-initial base changes, not sustained wide activity.
    for (name, signal, active) in [
        ("tb.dat1/whole1024", word, true),
        ("tb.dat1/0:0", low, true),
        ("tb.dat1/63:1", stable, false),
    ] {
        let mut selection = wave.select(&[signal]).unwrap();
        group.bench_function(
            BenchmarkId::new(format!("sample/{name}/t60000"), BACKEND),
            |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
        );
        let count = scan_count(&mut selection, range);
        if active {
            assert!(count > 1);
        } else {
            assert_eq!(count, 1);
        }
        group.bench_function(
            BenchmarkId::new(format!("scan/{name}/30001..=60000"), BACKEND),
            |b| b.iter(|| black_box(scan_count(&mut selection, black_box(range)))),
        );
        group.bench_function(
            BenchmarkId::new(format!("trace/{name}/30001..=60000"), BACKEND),
            |b| b.iter(|| black_box(selection.traces(black_box(range)).unwrap())),
        );
        // Candidates may contain base activity even for a stable projection.
        group.bench_function(
            BenchmarkId::new(format!("candidate-times/{name}/30001..=60000"), BACKEND),
            |b| b.iter(|| black_box(candidate_count(&mut selection, black_box(range)))),
        );
    }
    for count in [1, 8, 64] {
        let signals = vec![low; count];
        let mut selection = wave.select(&signals).unwrap();
        assert_eq!(selection.samples(time).unwrap().len(), count);
        group.bench_function(
            BenchmarkId::new(
                format!("samples/tb.dat1/0:0/duplicates{count}/t60000"),
                BACKEND,
            ),
            |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
        );
    }
    group.finish();
}

fn controlled_corpus(c: &mut Criterion) {
    for (fixture, end) in [
        ("fsdb0010-history-short", 4096),
        ("fsdb0011-history-long", 1_048_576),
    ] {
        controlled::history(c, fixture, end);
    }
    for (fixture, histories) in [
        ("fsdb0012-topology-small", 16),
        ("fsdb0013-topology-many-handles", 16_384),
        ("fsdb0014-topology-many-times", 16),
        ("fsdb0016-topology-many-aliases", 16),
    ] {
        controlled::topology(c, fixture, histories);
    }
    controlled::wide(c, "fsdb0015-wide-compact-toggle");
    controlled::aliases(c, "fsdb0016-topology-many-aliases");
    typed::records(
        c,
        "fsdb0017-typed-records",
        &[
            "top.logic4",
            "top.trigger",
            "top.real64",
            "top.real32",
            "top.text_short",
            "top.text_long",
        ],
    );
    typed::records(c, "fsdb0018-native-real32", &["top.real32", "top.control"]);
}

criterion_group!(benches, compare, mode_change, wide, controlled_corpus);
criterion_main!(benches);
