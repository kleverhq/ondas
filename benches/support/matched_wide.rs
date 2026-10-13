use std::{hint::black_box, ops::ControlFlow};

use criterion::{BenchmarkId, Criterion};
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

// Selected histories alternate their low bit at each tick through 4096.
// Time units and surrounding designs differ. Independent tests check values,
// normalized change times and exact counts outside the benchmark consumer.
pub fn register(c: &mut Criterion, fixture: &str, backend: &str, mut wave: Waveform) {
    let control = wave.hierarchy().signal("top.control").unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    let end = Time::from_ticks(4096);
    let range = TimeRange::closed(Time::from_ticks(4088), end);
    let mut group = c.benchmark_group(format!("{fixture}/matched-wide/t4096"));
    group.sample_size(10);
    for (name, signal) in [
        ("scalar1", control),
        ("whole4096", wide),
        ("stable-high4095", wide.slice(4095, 1).unwrap()),
    ] {
        group.bench_function(
            BenchmarkId::new(
                format!("point/fresh-selection/{name}/bases1/slots1"),
                backend,
            ),
            |b| b.iter(|| black_box(wave.sample(black_box(signal), black_box(end)).unwrap())),
        );
        let mut selection = wave.select(&[signal]).unwrap();
        scan_count(&mut selection, range);
        group.bench_function(
            BenchmarkId::new(
                format!("scan/warm-repeat/{name}/bases1/slots1/4088..=4096"),
                backend,
            ),
            |b| b.iter(|| black_box(scan_count(&mut selection, black_box(range)))),
        );
    }
    group.finish();
}
