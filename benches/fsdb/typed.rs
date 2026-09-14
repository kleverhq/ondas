use std::{hint::black_box, ops::ControlFlow};

use criterion::{BenchmarkId, Criterion};
use ondas::{Time, TimeRange};

use super::{BACKEND, candidate_count, fixtures, scan_count};

pub(super) fn records(c: &mut Criterion, fixture: &str, names: &[&str]) {
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).unwrap();
    let time = Time::from_ticks(2049);
    let range = TimeRange::closed(Time::from_ticks(2047), time);
    let mut group = c.benchmark_group(format!("fsdb/{}/{fixture}/file", fixtures::PROVIDER));
    group.sample_size(10);
    for &name in names {
        let signal = wave.hierarchy().signal(name).unwrap();
        let mut selection = wave.select(&[signal]).unwrap();
        group.bench_function(
            BenchmarkId::new(format!("sample/{name}/t2049"), BACKEND),
            |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
        );
        group.bench_function(
            BenchmarkId::new(format!("visit-samples/{name}/t2049"), BACKEND),
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
        assert!(scan_count(&mut selection, range) > 1);
        group.bench_function(
            BenchmarkId::new(format!("scan/{name}/2047..=2049"), BACKEND),
            |b| b.iter(|| black_box(scan_count(&mut selection, black_box(range)))),
        );
        group.bench_function(
            BenchmarkId::new(format!("trace/{name}/2047..=2049"), BACKEND),
            |b| b.iter(|| black_box(selection.traces(black_box(range)).unwrap())),
        );
        if name == "top.trigger" {
            assert!(candidate_count(&mut selection, range) > 0);
            group.bench_function(
                BenchmarkId::new("candidate-times/top.trigger/2047..=2049", BACKEND),
                |b| b.iter(|| black_box(candidate_count(&mut selection, black_box(range)))),
            );
        }
    }
    group.finish();
}
