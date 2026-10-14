use std::{fs, hint::black_box, ops::ControlFlow, sync::Arc};

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use ondas::{SampleRef, Selection, Time, TimeRange, ValueRef, Waveform};

#[path = "../tests/support/ghw_workloads.rs"]
mod coordinates;
#[path = "../tests/support/fixtures.rs"]
mod fixtures;
#[path = "../tests/support/query_workloads.rs"]
mod workloads;
use coordinates::range;
const BACKEND: &str = "ghw-native";

fn scan(selection: &mut Selection<'_>, window: TimeRange) {
    let _ = selection
        .scan_each(window, |slot, record| {
            black_box((slot, record));
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
}

fn point_series(selection: &mut Selection<'_>, times: [u64; 3]) {
    for tick in times {
        let _ = selection
            .visit_samples(Time::from_ticks(tick), |sample| {
                black_box(sample);
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
    }
}

fn owned_bits(wave: &mut Waveform, tick: u64) -> String {
    let wide = wave.hierarchy().signal(coordinates::WIDE_PATH).unwrap();
    selection_bits(&mut wave.select(&[wide]).unwrap(), tick)
}

fn selection_bits(selection: &mut Selection<'_>, tick: u64) -> String {
    let mut output = None;
    let _ = selection
        .visit_samples(Time::from_ticks(tick), |sample| {
            let SampleRef::Value {
                value: ValueRef::Bits(bits),
                ..
            } = sample
            else {
                panic!("bit sample")
            };
            output = Some(bits.to_string());
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    output.unwrap()
}

fn wide(c: &mut Criterion) {
    let (path, _) = fixtures::load_artifact(&fixtures::root(), coordinates::WIDE);
    let bytes: Arc<[u8]> = fs::read(&path).unwrap().into();
    for mode in ["file", "bytes"] {
        let open = || {
            if mode == "file" {
                ondas::open_with(&path, BACKEND).unwrap()
            } else {
                ondas::open_bytes_with("waveform.ghw", Arc::clone(&bytes), BACKEND).unwrap()
            }
        };
        let mut wave = open();
        let wide = wave.hierarchy().signal(coordinates::WIDE_PATH).unwrap();
        let control = wave.hierarchy().signal(coordinates::CONTROL_PATH).unwrap();
        let low = wide.slice(0, 0).unwrap();
        let high = wide.slice(4095, 1).unwrap();
        let changing_wide = wide.slice(4094, 0).unwrap();
        let mut group = c.benchmark_group(format!("{}/{mode}", coordinates::WIDE));
        group.bench_function("open-drop", |b| b.iter(|| drop(black_box(open()))));
        group.bench_function("hierarchy/traverse", |b| {
            b.iter(|| {
                black_box(wave.hierarchy().scopes().count() + wave.hierarchy().variables().count())
            })
        });
        group.bench_function("hierarchy/first-lookup", |b| {
            b.iter_batched_ref(
                open,
                |wave| black_box(wave.hierarchy().signal(coordinates::WIDE_PATH).unwrap()),
                BatchSize::PerIteration,
            )
        });
        group.bench_function("hierarchy/repeated-lookup", |b| {
            b.iter(|| black_box(wave.hierarchy().signal(coordinates::WIDE_PATH).unwrap()))
        });
        group.bench_function("selection/duplicates-and-overlap", |b| {
            b.iter(|| {
                drop(black_box(
                    wave.select(&[wide, low, high, wide, low]).unwrap(),
                ))
            })
        });
        for (name, signals) in [
            ("whole", vec![wide]),
            ("low", vec![low]),
            ("stable-high", vec![high]),
            ("changing-wide", vec![changing_wide]),
            ("scalar", vec![control]),
            ("duplicate-projections", vec![wide, low, high, wide, low]),
        ] {
            let mut selection = wave.select(&signals).unwrap();
            for (position, window) in [("early", range(1, 129)), ("late", range(3968, 4096))] {
                group.bench_function(BenchmarkId::new(format!("scan/{name}"), position), |b| {
                    b.iter(|| scan(&mut selection, window))
                });
                group.bench_function(
                    BenchmarkId::new(format!("traces-owned/{name}"), position),
                    |b| b.iter(|| drop(black_box(selection.traces(window).unwrap()))),
                );
            }
            group.bench_function(format!("scan-first/{name}"), |b| {
                b.iter(|| {
                    black_box(
                        selection
                            .scan(range(1, 4096), |_| ControlFlow::Break(()))
                            .unwrap(),
                    )
                })
            });
            for (schedule, times) in [
                ("repeat", [64, 64, 64]),
                ("forward", [63, 64, 65]),
                ("backward", [65, 64, 63]),
                ("backstep", [64, 65, 64]),
            ] {
                point_series(&mut selection, times);
                group.bench_function(
                    BenchmarkId::new(format!("points/prepared/{name}"), schedule),
                    |b| b.iter(|| point_series(&mut selection, times)),
                );
            }
        }
        for (schedule, times) in [
            ("repeat", [64, 64, 64]),
            ("forward", [63, 64, 65]),
            ("backward", [65, 64, 63]),
            ("backstep", [64, 65, 64]),
        ] {
            group.bench_function(BenchmarkId::new("points/fresh-selection", schedule), |b| {
                b.iter(|| point_series(&mut wave.select(&[control, low]).unwrap(), times))
            });
        }
        group.bench_function("points/batched-owned", |b| {
            b.iter(|| {
                drop(black_box(
                    wave.samples(&[wide, low, high, wide], Time::from_ticks(129))
                        .unwrap(),
                ))
            })
        });
        group.bench_function("points/individual-owned", |b| {
            b.iter(|| {
                for signal in [wide, low, high, wide] {
                    drop(black_box(
                        wave.sample(signal, Time::from_ticks(129)).unwrap(),
                    ));
                }
            })
        });
        for (driver_name, driver) in [("distinct-scalar", control), ("shared-base", low)] {
            let mut selection = wave.select(&[driver, low, wide]).unwrap();
            for (case, window, limit) in [
                ("stride64", range(1, 4096), usize::MAX),
                ("no-result", range(2, 64), usize::MAX),
                ("after-rejection", range(2, 129), 1),
            ] {
                for eager in [false, true] {
                    group.bench_function(
                        format!(
                            "conditional/{driver_name}/{case}/{}",
                            if eager { "eager-owned" } else { "borrowed" }
                        ),
                        |b| {
                            b.iter(|| {
                                black_box(
                                    workloads::conditional_query(
                                        &mut selection,
                                        window,
                                        64,
                                        limit,
                                        eager,
                                        |time, value| {
                                            black_box((time, value));
                                        },
                                    )
                                    .unwrap(),
                                )
                            })
                        },
                    );
                }
                group.bench_function(
                    format!("conditional/{driver_name}/{case}/sequential"),
                    |b| {
                        b.iter(|| {
                            black_box(
                                workloads::conditional_scan(
                                    &mut selection,
                                    window,
                                    64,
                                    limit,
                                    |time, value| {
                                        black_box((time, value));
                                    },
                                )
                                .unwrap(),
                            )
                        })
                    },
                );
            }
        }
        let mut selection = wave.select(&[control, low]).unwrap();
        group.bench_function("query/current-predecessor-backstep", |b| {
            b.iter(|| {
                black_box(
                    workloads::adjacent_query(&mut selection, range(63, 129), |time, sample| {
                        black_box((time, sample));
                    })
                    .unwrap(),
                )
            })
        });
        group.bench_function("query/first-high-after-rejection", |b| {
            b.iter(|| black_box(workloads::first_high(&mut selection, range(2, 4096)).unwrap()))
        });
        group.bench_function("candidates/duplicates", |b| {
            b.iter(|| {
                black_box(
                    selection
                        .scan_candidate_times(range(1, 129), |time| {
                            black_box(time);
                            ControlFlow::<()>::Continue(())
                        })
                        .unwrap(),
                )
            })
        });
        group.finish();
    }
}

fn topology(c: &mut Criterion) {
    for fixture in coordinates::TOPOLOGIES {
        let (path, _) = fixtures::load_artifact(&fixtures::root(), fixture);
        let mut wave = ondas::open_with(&path, BACKEND).unwrap();
        let probes = (0..64)
            .map(|i| wave.hierarchy().signal(&coordinates::probe(i)).unwrap())
            .collect::<Vec<_>>();
        let constant = wave.hierarchy().signal(coordinates::CONSTANT_PATH).unwrap();
        let noise = wave.hierarchy().signal(coordinates::NOISE_PATH).unwrap();
        let mut group = c.benchmark_group(format!("{fixture}/file"));
        group.bench_function("open-drop", |b| {
            b.iter(|| drop(black_box(ondas::open_with(&path, BACKEND).unwrap())))
        });
        for count in [1, 16, 64] {
            let mut selection = wave.select(&probes[..count]).unwrap();
            group.bench_function(BenchmarkId::new("sparse-unique/scan", count), |b| {
                b.iter(|| scan(&mut selection, range(0, 256)))
            });
            group.bench_function(BenchmarkId::new("sparse-unique/samples", count), |b| {
                b.iter(|| drop(black_box(selection.samples(Time::from_ticks(193)).unwrap())))
            });
            group.bench_function(BenchmarkId::new("sparse-unique/early-stop", count), |b| {
                b.iter(|| {
                    black_box(
                        selection
                            .scan_candidate_times(range(1, 256), ControlFlow::Break)
                            .unwrap(),
                    )
                })
            });
        }
        for (name, signal) in [("constant", constant), ("noise", noise)] {
            let mut selection = wave.select(&[signal]).unwrap();
            for (position, window) in [("early", range(1, 65)), ("late", range(192, 256))] {
                group.bench_function(BenchmarkId::new(format!("{name}/scan"), position), |b| {
                    b.iter(|| scan(&mut selection, window))
                });
            }
        }
        group.finish();
    }
}

fn realistic(c: &mut Criterion) {
    for fixture in [
        "ghw/ghw0090-ws2815b-driver",
        "ghw/ghw0001-wellen-ghw-bug",
        "ghw/ghw0094-surfer-issue-547",
    ] {
        let (path, sidecar) = fixtures::load_artifact(&fixtures::root(), fixture);
        let var = sidecar["oracle"]["hierarchy"]["variables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| {
                v.get("signal")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
            })
            .unwrap();
        let exact = ondas::HierarchyPath::from_components(
            var["path"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p.as_str().unwrap()),
        );
        let mut wave = ondas::open_with(&path, BACKEND).unwrap();
        let signal = wave.hierarchy().signal_path(&exact).unwrap();
        let mut group = c.benchmark_group(format!("{fixture}/file"));
        group.bench_function("open-drop", |b| {
            b.iter(|| drop(black_box(ondas::open_with(&path, BACKEND).unwrap())))
        });
        let mut selection = wave.select(&[signal, signal]).unwrap();
        group.bench_function("scan-prefix/duplicate-history", |b| {
            b.iter(|| scan(&mut selection, range(0, 4_000_000)))
        });
        group.bench_function("points/backstep", |b| {
            b.iter(|| point_series(&mut selection, [4_000_000, 0, 4_000_000]))
        });
        group.finish();
    }
}

fn wellen_reference(c: &mut Criterion) {
    let (path, _) = fixtures::load_artifact(&fixtures::root(), coordinates::WIDE);
    let load = || {
        let mut wave = wellen::simple::read(&path).unwrap();
        let var = wave.hierarchy().lookup_var(&["top"], "wide").unwrap();
        let signal = wave.hierarchy()[var].signal_ref();
        wave.load_signals(&[signal]);
        (wave, signal)
    };
    let read = |wave: &wellen::simple::Waveform, signal: wellen::SignalRef| {
        let at = wave.time_table().partition_point(|&tick| tick <= 129) - 1;
        let offset = wave
            .get_signal(signal)
            .unwrap()
            .get_offset(at as wellen::TimeTableIdx)
            .unwrap();
        wave.get_signal(signal)
            .unwrap()
            .get_value_at(&offset, offset.elements - 1)
            .to_bit_string()
            .unwrap()
    };
    let (reference, signal) = load();
    let mut native = ondas::open_with(&path, BACKEND).unwrap();
    assert_eq!(owned_bits(&mut native, 129), read(&reference, signal));
    let mut group = c.benchmark_group(format!("{}/file/reference", coordinates::WIDE));
    group.bench_function("open-first-owned/ghw-native", |b| {
        b.iter(|| {
            let mut wave = ondas::open_with(&path, BACKEND).unwrap();
            black_box(owned_bits(&mut wave, 129));
        })
    });
    group.bench_function("open-first-owned/wellen-0.25.6", |b| {
        b.iter(|| {
            let (wave, signal) = load();
            black_box(read(&wave, signal));
        })
    });
    group.bench_function("warm-owned/ghw-native", |b| {
        let wide = native.hierarchy().signal(coordinates::WIDE_PATH).unwrap();
        let mut selection = native.select(&[wide]).unwrap();
        black_box(selection_bits(&mut selection, 129));
        b.iter(|| black_box(selection_bits(&mut selection, 129)))
    });
    group.bench_function("warm-owned/wellen-0.25.6-retained-history", |b| {
        b.iter(|| black_box(read(&reference, signal)))
    });
    group.finish();
}

criterion_group!(benches, wide, topology, realistic, wellen_reference);
criterion_main!(benches);
