use std::{collections::HashSet, hint::black_box, ops::ControlFlow};

use criterion::{BatchSize, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ondas::{Signal, Time, Waveform};

#[path = "shm/cold.rs"]
mod cold;
#[path = "../tests/support/fixtures.rs"]
mod fixtures;
#[path = "support/matched_active4.rs"]
mod matched_active4;
#[path = "support/matched_clock.rs"]
mod matched_clock;
#[path = "support/matched_constant.rs"]
mod matched_constant;
#[path = "support/matched_wide.rs"]
mod matched_wide;
#[path = "../tests/support/shm_workloads.rs"]
mod workloads;
use workloads::{BACKEND, candidate_count, first_change, range, scan_count};

fn reader_identity(_: &mut Criterion) {
    println!(
        "SHM reader: {}; aliases: {}",
        env!("ONDAS_SHM_READER"),
        env!("ONDAS_SHM_ALIAS_PATH")
    );
}

fn basic(c: &mut Criterion, fixture: &str, name: &str) {
    let (path, _) = fixtures::load_artifact(&fixtures::root(), fixture);
    let wave = ondas::open_with(&path, BACKEND).unwrap();
    assert!(wave.hierarchy().signal(name).is_ok());
    let mut group = c.benchmark_group(format!("{fixture}/directory/setup"));
    group.sample_size(10);
    group.bench_function(BenchmarkId::new("open-drop", BACKEND), |b| {
        b.iter(|| {
            drop(black_box(
                ondas::open_with(black_box(&path), BACKEND).unwrap(),
            ))
        })
    });
    group.bench_function(BenchmarkId::new("metadata-only", BACKEND), |b| {
        b.iter(|| black_box(ondas::read_metadata(black_box(&path)).unwrap()))
    });
    group.bench_function(BenchmarkId::new("hierarchy/names", BACKEND), |b| {
        b.iter(|| {
            for variable in wave.hierarchy().variables() {
                black_box(variable.name());
            }
        })
    });
    for second in [false, true] {
        group.bench_function(
            BenchmarkId::new(
                if second {
                    "lookup/second"
                } else {
                    "lookup/first"
                },
                BACKEND,
            ),
            |b| {
                b.iter_batched_ref(
                    || {
                        let wave = ondas::open_with(&path, BACKEND).unwrap();
                        if second {
                            black_box(wave.hierarchy().signal(name).unwrap());
                        }
                        wave
                    },
                    |wave| black_box(wave.hierarchy().signal(black_box(name)).unwrap()),
                    BatchSize::PerIteration,
                )
            },
        );
    }
    group.finish();
}

// The same selection and bounded window compare callback delivery, timestamps,
// and owned traces. Hash checks and handle resolution precede registration.
fn window(
    c: &mut Criterion,
    fixture: &str,
    wave: &mut Waveform,
    signals: &[Signal],
    label: &str,
    start: u64,
    end: u64,
) {
    let window = range(start, end);
    let mut selection = wave.select(signals).unwrap();
    let mut group = c.benchmark_group(format!(
        "{fixture}/directory/prepared/{label}/{start}..={end}"
    ));
    group.sample_size(10);
    scan_count(&mut selection, window).unwrap();
    candidate_count(&mut selection, window).unwrap();
    group.bench_function(BenchmarkId::new("scan", BACKEND), |b| {
        b.iter(|| black_box(scan_count(&mut selection, black_box(window)).unwrap()))
    });
    group.bench_function(BenchmarkId::new("candidate-times", BACKEND), |b| {
        b.iter(|| black_box(candidate_count(&mut selection, black_box(window)).unwrap()))
    });
    group.bench_function(BenchmarkId::new("owned-traces", BACKEND), |b| {
        b.iter(|| black_box(selection.traces(black_box(window)).unwrap()))
    });
    group.bench_function(BenchmarkId::new("first-change", BACKEND), |b| {
        b.iter(|| black_box(first_change(&mut selection, black_box(window)).unwrap()))
    });
    group.finish();
}

fn histories(c: &mut Criterion) {
    for (fixture, end) in workloads::HISTORY {
        basic(c, fixture, "top.phase_0");
        let (path, _) = fixtures::load_artifact(&fixtures::root(), fixture);
        let mut wave = ondas::open_with(&path, BACKEND).unwrap();
        let signals = ["top.phase_0", "top.phase_1", "top.phase_2", "top.phase_3"]
            .map(|name| wave.hierarchy().signal(name).unwrap());
        assert_eq!(signals.iter().collect::<HashSet<_>>().len(), 4);
        for start in [0, end - 63] {
            window(
                c,
                fixture,
                &mut wave,
                &signals,
                "phases/bases4/slots4",
                start,
                start + 63,
            );
        }
        let time = Time::from_ticks(end);
        let mut group = c.benchmark_group(format!("{fixture}/directory/points/t{end}"));
        group.sample_size(10);
        group.bench_function(BenchmarkId::new("select/bases4", BACKEND), |b| {
            b.iter(|| drop(black_box(wave.select(black_box(&signals)).unwrap())))
        });
        group.bench_function(
            BenchmarkId::new("samples-individual/bases4", BACKEND),
            |b| {
                b.iter(|| {
                    for signal in signals {
                        black_box(wave.sample(black_box(signal), black_box(time)).unwrap());
                    }
                })
            },
        );
        group.bench_function(
            BenchmarkId::new("samples-batch-fresh-selection/bases4", BACKEND),
            |b| b.iter(|| black_box(wave.samples(black_box(&signals), black_box(time)).unwrap())),
        );
        {
            let mut selection = wave.select(&signals).unwrap();
            selection.samples(time).unwrap();
            group.bench_function(
                BenchmarkId::new("samples-prepared-warm-repeat/bases4", BACKEND),
                |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
            );
            let times = [
                end - 3,
                end - 2,
                end - 2,
                end - 1,
                end - 2,
                end - 3,
                end,
                end,
            ]
            .map(Time::from_ticks);
            group.throughput(Throughput::Elements(8));
            group.bench_function(
                BenchmarkId::new("sample-series/mixed8/bases4", BACKEND),
                |b| {
                    b.iter(|| {
                        black_box(
                            workloads::series(
                                &mut selection,
                                black_box(&times),
                                |time, index, sample| {
                                    black_box((time, index, sample));
                                },
                            )
                            .unwrap(),
                        )
                    })
                },
            );
            let times = (end - 7..=end).map(Time::from_ticks).collect::<Vec<_>>();
            workloads::series(&mut selection, &times, |_, _, _| {}).unwrap();
            group.bench_function(
                BenchmarkId::new("sample-series/forward8/bases4", BACKEND),
                |b| {
                    b.iter(|| {
                        black_box(
                            workloads::series(
                                &mut selection,
                                black_box(&times),
                                |time, index, sample| {
                                    black_box((time, index, sample));
                                },
                            )
                            .unwrap(),
                        )
                    })
                },
            );
        }
        // Opening and signal resolution are outside this first-query region.
        group.throughput(Throughput::Elements(1));
        for tick in [63, end] {
            group.bench_function(
                BenchmarkId::new(format!("cold-reader/sample/phase0/t{tick}"), BACKEND),
                |b| {
                    b.iter_batched_ref(
                        || {
                            let wave = ondas::open_with(&path, BACKEND).unwrap();
                            let signal = wave.hierarchy().signal("top.phase_0").unwrap();
                            (wave, signal)
                        },
                        |(wave, signal)| {
                            black_box(wave.sample(*signal, Time::from_ticks(tick)).unwrap())
                        },
                        BatchSize::PerIteration,
                    )
                },
            );
        }
        group.finish();
    }
}

fn banks(c: &mut Criterion) {
    for fixture in [workloads::BANK, workloads::TOPOLOGY] {
        basic(c, fixture, "top.probe_sparse");
        let (path, _) = fixtures::load_artifact(&fixtures::root(), fixture);
        let mut wave = ondas::open_with(path, BACKEND).unwrap();
        for class in ["dense", "sparse"] {
            let signals = (0..64)
                .map(|index| {
                    wave.hierarchy()
                        .signal(&format!("top.bank[{index}].{class}"))
                        .unwrap()
                })
                .collect::<Vec<_>>();
            assert_eq!(signals.iter().collect::<HashSet<_>>().len(), 64);
            for count in [1, 16, 64] {
                window(
                    c,
                    fixture,
                    &mut wave,
                    &signals[..count],
                    &format!("bank/{class}/bases{count}/slots{count}"),
                    257,
                    320,
                );
            }
        }
        let signals = ["top.probe_sparse", "top.probe_constant"]
            .map(|name| wave.hierarchy().signal(name).unwrap());
        window(
            c,
            fixture,
            &mut wave,
            &signals,
            "quiet-probes/bases2/slots2",
            257,
            320,
        );
        let quiet = range(257, 320);
        assert_eq!(
            scan_count(&mut wave.select(&signals).unwrap(), quiet).unwrap(),
            2
        );
        let mut group = c.benchmark_group(format!(
            "{fixture}/directory/fresh-selection/quiet-probes/bases2/slots2/257..=320"
        ));
        group.sample_size(10);
        group.bench_function(BenchmarkId::new("scan", BACKEND), |b| {
            b.iter(|| {
                let mut selection = wave.select(black_box(&signals)).unwrap();
                black_box(scan_count(&mut selection, black_box(quiet)).unwrap())
            })
        });
        group.finish();
    }
}

fn wide(c: &mut Criterion) {
    let fixture = workloads::WIDE;
    basic(c, fixture, "top.wide");
    let (path, _) = fixtures::load_artifact(&fixtures::root(), fixture);
    let mut wave = ondas::open_with(path, BACKEND).unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    assert_eq!(wide.encoding(), ondas::Encoding::Bits { width: 4096 });
    let opposite = wave.hierarchy().signal("top.wide_msb").unwrap();
    let control = wave.hierarchy().signal("top.control").unwrap();
    let low = wide.slice(0, 0).unwrap();
    let stable = wide.slice(4095, 4080).unwrap();
    for (label, signals) in [
        ("scalar/bases1/slots1", vec![control]),
        ("whole-lsb/bases1/slots1", vec![wide]),
        ("whole-msb/bases1/slots1", vec![opposite]),
        ("low-bit/bases1/slots1", vec![low]),
        ("stable-high/bases1/slots1", vec![stable]),
        (
            "overlap-duplicate/bases1/slots4",
            vec![wide, low, stable, low],
        ),
    ] {
        window(c, fixture, &mut wave, &signals, label, 17, 80);
    }
    let mut group = c.benchmark_group(format!("{fixture}/directory/composed"));
    group.sample_size(10);
    for (topology, driver) in [
        ("shared-base", control),
        ("distinct-base", opposite.slice(4095, 4095).unwrap()),
    ] {
        let mut selection = wave.select(&[control, driver, wide]).unwrap();
        for eager in [false, true] {
            for stop in [false, true] {
                let label = format!(
                    "{topology}/{}/accept1in16/0..=63/{}",
                    if eager { "eager" } else { "selective" },
                    if stop { "first-result" } else { "all-results" }
                );
                group.bench_function(BenchmarkId::new(label, BACKEND), |b| {
                    b.iter(|| {
                        black_box(
                            workloads::payload(
                                &mut selection,
                                range(0, 63),
                                16,
                                eager,
                                |time, value| {
                                    black_box((time, value));
                                    if stop {
                                        ControlFlow::Break(())
                                    } else {
                                        ControlFlow::Continue(())
                                    }
                                },
                            )
                            .unwrap(),
                        )
                    })
                });
            }
        }
    }
    group.finish();
}

fn typed(c: &mut Criterion) {
    let fixture = workloads::TYPED;
    let (path, _) = fixtures::load_artifact(&fixtures::root(), fixture);
    let mut wave = ondas::open_with(path, BACKEND).unwrap();
    for name in [
        "top.logic4",
        "top.real_value",
        "top.text_short",
        "top.text_long",
        "top.pulse",
    ] {
        let signal = wave.hierarchy().signal(name).unwrap();
        window(
            c,
            fixture,
            &mut wave,
            &[signal],
            &format!("typed/{name}/bases1/slots1"),
            3,
            7,
        );
    }
}

fn late_point(c: &mut Criterion) {
    let fixture = "shm/shm0091-late-history";
    let (path, _) = fixtures::load_artifact(&fixtures::root(), fixture);
    let mut wave = ondas::open_with(path, BACKEND).unwrap();
    let signals = ["top.marker", "top.late"].map(|name| wave.hierarchy().signal(name).unwrap());
    let time = Time::from_ticks(7);
    wave.samples(&signals, time).unwrap();
    let mut group = c.benchmark_group(format!("{fixture}/directory/points/t7"));
    group.sample_size(10);
    // Marker certifies first; missing initialization of late requires fallback.
    // Selection construction and owned-result destruction are inside timing.
    group.bench_function(
        BenchmarkId::new("samples-batch-fresh-selection/marker-late/bases2", BACKEND),
        |b| b.iter(|| black_box(wave.samples(black_box(&signals), black_box(time)).unwrap())),
    );
    group.finish();
}

fn matched(c: &mut Criterion) {
    let fixture = "shm/shm0081-history-long";
    let (path, _) = fixtures::load_artifact(&fixtures::root(), fixture);
    let wave = ondas::open_with(&path, BACKEND).unwrap();
    matched_clock::register(c, fixture, &path, BACKEND, "top.phase_0", wave);
}

fn matched_widths(c: &mut Criterion) {
    let fixture = "shm/shm0084-wide-compact-toggle";
    let (path, _) = fixtures::load_artifact(&fixtures::root(), fixture);
    matched_wide::register(
        c,
        fixture,
        BACKEND,
        ondas::open_with(path, BACKEND).unwrap(),
    );
}

fn matched_active_bases(c: &mut Criterion) {
    let fixture = "shm/shm0082-sparse-dense-bank";
    let (path, _) = fixtures::load_artifact(&fixtures::root(), fixture);
    matched_active4::register(
        c,
        fixture,
        BACKEND,
        [
            "top.bank[0].dense",
            "top.bank[2].dense",
            "top.bank[4].dense",
            "top.bank[6].dense",
        ],
        ondas::open_with(path, BACKEND).unwrap(),
    );
}

fn matched_constant_history(c: &mut Criterion) {
    let fixture = "shm/shm0081-history-long";
    let (path, _) = fixtures::load_artifact(&fixtures::root(), fixture);
    matched_constant::register(
        c,
        fixture,
        BACKEND,
        ondas::open_with(path, BACKEND).unwrap(),
    );
}

fn cold_queries(c: &mut Criterion) {
    cold::register(c);
}

criterion_group!(
    benches,
    reader_identity,
    histories,
    banks,
    wide,
    typed,
    late_point,
    matched,
    matched_widths,
    matched_active_bases,
    matched_constant_history,
    cold_queries
);
criterion_main!(benches);
