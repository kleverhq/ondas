use std::hint::black_box;

use criterion::{BenchmarkId, Criterion};
use ondas::{Time, Waveform};

// The selected scalar stays zero through 1048576. Independent tests check the
// full prefix. Time units and surrounding designs differ across the fixtures.
pub fn register(c: &mut Criterion, fixture: &str, backend: &str, mut wave: Waveform) {
    let signal = wave.hierarchy().signal("top.probe_constant").unwrap();
    let time = Time::from_ticks(1_048_576);
    wave.sample(signal, time).unwrap();
    let mut group = c.benchmark_group(format!("{fixture}/matched-constant/t1048576"));
    group.sample_size(10);
    // Opening, resolution and preflight precede timing. Each iteration includes
    // selection creation and owned-result destruction.
    group.bench_function(
        BenchmarkId::new("point/fresh-selection/scalar1/bases1/slots1", backend),
        |b| b.iter(|| black_box(wave.sample(black_box(signal), black_box(time)).unwrap())),
    );
    group.finish();
}
