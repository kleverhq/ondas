use std::hint::black_box;

use criterion::{BenchmarkId, Criterion};
use ondas::{Time, Waveform};

// Four distinct scalar histories alternate each tick through 4096.
// Surrounding designs and time units differ. This is a qualified FST reference,
// not an FSDB comparison. Independent conformance checks validate the outputs.
pub fn register(
    c: &mut Criterion,
    fixture: &str,
    backend: &str,
    names: [&str; 4],
    mut wave: Waveform,
) {
    let signals = names.map(|name| wave.hierarchy().signal(name).unwrap());
    let time = Time::from_ticks(4096);
    wave.samples(&signals, time).unwrap();
    let mut group = c.benchmark_group(format!("{fixture}/matched-active4/t4096"));
    group.sample_size(10);
    // Opening and resolution precede timing; selection and result destruction
    // remain inside each iteration.
    group.bench_function(
        BenchmarkId::new("point/fresh-selection/bases4/slots4", backend),
        |b| b.iter(|| black_box(wave.samples(black_box(&signals), black_box(time)).unwrap())),
    );
    group.finish();
}
