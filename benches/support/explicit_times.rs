use std::{hint::black_box, path::Path};

use criterion::{BatchSize, Criterion};
use ondas::Time;

pub fn register(
    c: &mut Criterion,
    fixture: &str,
    path: &Path,
    backend: &str,
    name: &str,
    end: u64,
) {
    let open = || ondas::open_with(path, backend).unwrap();
    let mut wave = open();
    let signal = wave.hierarchy().signal(name).unwrap();
    let mut group = c.benchmark_group(format!("{fixture}/explicit-times/{backend}"));
    group.bench_function("metadata-only", |b| {
        b.iter(|| black_box(ondas::read_metadata(black_box(path)).unwrap()))
    });
    for count in [4, 32] {
        let times = [end, 0, end / 2, end / 2]
            .into_iter()
            .cycle()
            .take(count)
            .map(Time::from_ticks)
            .collect::<Vec<_>>();
        let expected = wave.samples_at(&[signal], &times).unwrap();
        assert_eq!(expected.len(), times.len());
        for ((time, samples), requested) in expected.iter().zip(&times) {
            assert_eq!(time, requested);
            assert_eq!(
                format!("{samples:?}"),
                format!("{:?}", [wave.sample(signal, *requested).unwrap()])
            );
        }
        group.bench_function(format!("fresh-waveform/slots1/points{count}"), |b| {
            b.iter_batched_ref(
                || {
                    let wave = open();
                    let signal = wave.hierarchy().signal(name).unwrap();
                    (wave, signal)
                },
                |(wave, signal)| {
                    drop(black_box(
                        wave.samples_at(&[*signal], black_box(&times)).unwrap(),
                    ));
                },
                BatchSize::PerIteration,
            )
        });
        group.bench_function(format!("fresh-selection/slots1/points{count}"), |b| {
            b.iter(|| black_box(wave.samples_at(&[signal], black_box(&times)).unwrap()))
        });
        let mut selection = wave.select(&[signal]).unwrap();
        selection.samples_at(&times).unwrap();
        group.bench_function(format!("repeat/slots1/points{count}"), |b| {
            b.iter(|| black_box(selection.samples_at(black_box(&times)).unwrap()))
        });
    }
    group.finish();
}
