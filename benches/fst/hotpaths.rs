use std::{hint::black_box, ops::ControlFlow};

use criterion::{BenchmarkId, Criterion};
use ondas::{Encoding, Selection, Time, TimeRange};

use super::{BACKEND, first_change, fixtures, scan_count};

fn candidates(selection: &mut Selection<'_>, range: TimeRange) -> u64 {
    let mut count = 0;
    let _ = selection
        .scan_candidate_times(range, |_| {
            count += 1;
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    count
}

pub(super) fn strings(c: &mut Criterion) {
    for fixture in [
        "fst0044-overlay-tb-issue-21",
        "fst0060-manytypes2",
        "fst0061-shortstring",
    ] {
        let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
        let mut wave = ondas::open_with(&path, BACKEND).unwrap();
        let signals = wave
            .hierarchy()
            .signals()
            .filter(|signal| signal.encoding() == Encoding::String)
            .collect::<Vec<_>>();
        assert!(!signals.is_empty());
        let window = TimeRange::all();
        let mut selection = wave.select(&signals).unwrap();
        assert!(candidates(&mut selection, window) > 0);
        assert!(scan_count(&mut selection, window) > 0);
        let mut group =
            c.benchmark_group(format!("fst/{}/{fixture}/file/strings", fixtures::PROVIDER));
        group.sample_size(30);
        group.bench_function(BenchmarkId::new("candidate-times/all", BACKEND), |b| {
            b.iter(|| black_box(candidates(&mut selection, black_box(window))));
        });
        group.bench_function(BenchmarkId::new("scan/all", BACKEND), |b| {
            b.iter(|| black_box(scan_count(&mut selection, black_box(window))));
        });
        group.finish();
    }
}

pub(super) fn topology(c: &mut Criterion) {
    // Identical selected histories: only unselected handles or global times vary.
    for (fixture, handles) in [
        ("fst0084-topology-small", 16),
        ("fst0085-topology-many-handles", 16384),
        ("fst0086-topology-many-times", 16),
    ] {
        let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
        let mut wave = ondas::open_with(&path, BACKEND).unwrap();
        assert_eq!(wave.hierarchy().signals().count(), handles);
        let sparse = wave.hierarchy().signal("top.probe_sparse").unwrap();
        let constant = wave.hierarchy().signal("top.probe_constant").unwrap();
        let quiet = Time::from_ticks(524288);
        let late = Time::from_ticks(1048576);
        let window = TimeRange::closed(quiet, Time::from_ticks(524320));
        let full = TimeRange::closed(Time::from_ticks(1), late);
        let mut group = c.benchmark_group(format!("fst/{}/{fixture}/file", fixtures::PROVIDER));
        group.sample_size(20);
        for (name, signal) in [("sparse", sparse), ("constant", constant)] {
            let mut selection = wave.select(&[signal]).unwrap();
            selection.samples(quiet).unwrap();
            group.bench_function(
                BenchmarkId::new(format!("sample/{name}/t524288"), BACKEND),
                |b| {
                    b.iter(|| black_box(selection.samples(black_box(quiet)).unwrap()));
                },
            );
        }
        let mut selection = wave.select(&[sparse]).unwrap();
        selection.samples(late).unwrap();
        group.bench_function(BenchmarkId::new("sample/sparse/t1048576", BACKEND), |b| {
            b.iter(|| black_box(selection.samples(black_box(late)).unwrap()));
        });
        assert_eq!(scan_count(&mut selection, window), 1);
        group.bench_function(
            BenchmarkId::new("scan/sparse/524288..=524320", BACKEND),
            |b| {
                b.iter(|| black_box(scan_count(&mut selection, black_box(window))));
            },
        );
        // Candidate times may legally include a superset of effective changes.
        candidates(&mut selection, window);
        group.bench_function(
            BenchmarkId::new("candidate-times/sparse/524288..=524320", BACKEND),
            |b| {
                b.iter(|| black_box(candidates(&mut selection, black_box(window))));
            },
        );
        // Initial records are not the first actual change.
        assert_eq!(
            selection.scan(full, first_change).unwrap(),
            ControlFlow::Break(Time::from_ticks(4096))
        );
        group.bench_function(
            BenchmarkId::new("first-change/sparse/1..=1048576", BACKEND),
            |b| {
                b.iter(|| black_box(selection.scan(black_box(full), first_change).unwrap()));
            },
        );
        assert_eq!(scan_count(&mut selection, full), 3);
        group.bench_function(BenchmarkId::new("scan/sparse/1..=1048576", BACKEND), |b| {
            b.iter(|| black_box(scan_count(&mut selection, black_box(full))));
        });
        group.finish();
    }
}

pub(super) fn wide(c: &mut Criterion) {
    let fixture = "fst0083-wide-compact-toggle";
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    let scalar = wave.hierarchy().signal("top.control").unwrap();
    let low = wide.slice(0, 0).unwrap();
    let high = wide.slice(4095, 1).unwrap();
    let late = Time::from_ticks(4096);
    let window = TimeRange::closed(Time::from_ticks(2048), late);
    let mut group = c.benchmark_group(format!("fst/{}/{fixture}/file", fixtures::PROVIDER));
    group.sample_size(20);
    // The scalar and low bit toggle together; the high slice is stable.
    for (name, signal, expected) in [
        ("scalar", scalar, 2050),
        ("whole4096", wide, 2050),
        ("low-bit", low, 2050),
        ("stable-high4095", high, 1),
    ] {
        let mut selection = wave.select(&[signal]).unwrap();
        selection.samples(late).unwrap();
        assert_eq!(scan_count(&mut selection, window), expected);
        group.bench_function(
            BenchmarkId::new(format!("sample/{name}/t4096"), BACKEND),
            |b| {
                b.iter(|| black_box(selection.samples(black_box(late)).unwrap()));
            },
        );
        group.bench_function(
            BenchmarkId::new(format!("scan/{name}/2048..=4096"), BACKEND),
            |b| {
                b.iter(|| black_box(scan_count(&mut selection, black_box(window))));
            },
        );
        if signal == wide || signal == high {
            assert!(candidates(&mut selection, window) >= expected - 1);
            group.bench_function(
                BenchmarkId::new(format!("candidate-times/{name}/2048..=4096"), BACKEND),
                |b| {
                    b.iter(|| black_box(candidates(&mut selection, black_box(window))));
                },
            );
        }
        if signal == wide {
            selection.traces(window).unwrap();
            group.bench_function(
                BenchmarkId::new("trace/whole4096/2048..=4096", BACKEND),
                |b| {
                    b.iter(|| black_box(selection.traces(black_box(window)).unwrap()));
                },
            );
        }
    }
    // Duplicate entries share the base read but retain separate result entries.
    for count in [1, 8, 64] {
        let signals = vec![low; count];
        let mut selection = wave.select(&signals).unwrap();
        assert_eq!(selection.samples(late).unwrap().len(), count);
        group.bench_function(
            BenchmarkId::new(format!("samples/low-bit/duplicates{count}/t4096"), BACKEND),
            |b| {
                b.iter(|| black_box(selection.samples(black_box(late)).unwrap()));
            },
        );
    }
    group.finish();
}

pub(super) fn boundaries(c: &mut Criterion) {
    let fixture = "fst0015-scr1-max-ahb-coremark";
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).unwrap();
    let constant = wave
        .hierarchy()
        .signal("TOP.$unit.SCR1_ARCH_RST_VECTOR")
        .unwrap();
    let clock = wave.hierarchy().signal("TOP.clk").unwrap();
    let mut selection = wave.select(&[constant]).unwrap();
    let mut group = c.benchmark_group(format!("fst/{}/{fixture}/file", fixtures::PROVIDER));
    group.sample_size(20);
    // Actual section starts; the selected reset vector does not change.
    for boundary in [745342_u64, 3248312, 5812392] {
        for tick in [boundary - 1, boundary, boundary + 1] {
            let time = Time::from_ticks(tick);
            selection.samples(time).unwrap();
            group.bench_function(
                BenchmarkId::new(format!("sample/reset-vector/t{tick}"), BACKEND),
                |b| {
                    b.iter(|| black_box(selection.samples(black_box(time)).unwrap()));
                },
            );
        }
    }
    let mut selection = wave.select(&[clock]).unwrap();
    for boundary in [745342_u64, 3248312, 5812392] {
        let window = TimeRange::closed(
            Time::from_ticks(boundary - 20),
            Time::from_ticks(boundary + 20),
        );
        assert!(candidates(&mut selection, window) > 0);
        group.bench_function(
            BenchmarkId::new(format!("candidate-times/clock/section{boundary}"), BACKEND),
            |b| b.iter(|| black_box(candidates(&mut selection, black_box(window)))),
        );
    }
    group.finish();
}
