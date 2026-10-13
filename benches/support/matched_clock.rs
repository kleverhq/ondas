use std::{hint::black_box, ops::ControlFlow, path::Path};

use criterion::{BatchSize, BenchmarkId, Criterion};
use ondas::{Selection, Time, TimeRange, Waveform};

fn scan_count(selection: &mut Selection<'_>, range: TimeRange) -> u64 {
    let mut count = 0;
    let _ = selection
        .scan(range, |_| {
            count += 1;
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    count
}

// Both fixtures have a selected one-bit history alternating at every tick.
// Their time units and surrounding designs differ. Compare tick counts, not
// physical durations. Independent conformance tests check the selected history.
pub fn register(
    c: &mut Criterion,
    fixture: &str,
    path: &Path,
    backend: &str,
    name: &str,
    mut wave: Waveform,
) {
    let clock = wave.hierarchy().signal(name).unwrap();
    let time = Time::from_ticks(1_048_576);
    let range = TimeRange::closed(Time::from_ticks(1_048_513), time);
    let mut group = c.benchmark_group(format!("{fixture}/matched-clock/t1048576"));
    group.sample_size(10);

    // Fresh reader, lookup, and input/output destruction are outside timing.
    group.bench_function(BenchmarkId::new("point/cold-reader/slots1", backend), |b| {
        b.iter_batched_ref(
            || {
                let wave = ondas::open_with(path, backend).unwrap();
                let signal = wave.hierarchy().signal(name).unwrap();
                (wave, signal)
            },
            |(wave, signal)| black_box(wave.sample(*signal, black_box(time)).unwrap()),
            BatchSize::PerIteration,
        )
    });
    // Selection creation and result destruction are inside timing.
    group.bench_function(
        BenchmarkId::new("point/fresh-selection/slots1", backend),
        |b| b.iter(|| black_box(wave.sample(black_box(clock), black_box(time)).unwrap())),
    );
    {
        // One base history produces four duplicate output slots.
        let mut selection = wave.select(&[clock; 4]).unwrap();
        selection.samples(time).unwrap();
        group.bench_function(
            BenchmarkId::new("point/warm-repeat/bases1/slots4", backend),
            |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
        );
    }
    group.bench_function(
        BenchmarkId::new("scan/fresh-selection/slots1/1048513..=1048576", backend),
        |b| {
            b.iter(|| {
                let mut selection = wave.select(&[clock]).unwrap();
                black_box(scan_count(&mut selection, black_box(range)))
            })
        },
    );
    let mut selection = wave.select(&[clock]).unwrap();
    scan_count(&mut selection, range);
    group.bench_function(
        BenchmarkId::new("scan/warm-repeat/slots1/1048513..=1048576", backend),
        |b| b.iter(|| black_box(scan_count(&mut selection, black_box(range)))),
    );
    group.finish();
}
