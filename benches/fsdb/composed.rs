use std::{hint::black_box, ops::ControlFlow};

use criterion::{BenchmarkId, Criterion};
use ondas::{Time, TimeRange};

use super::{BACKEND, candidate_count, fixtures, scan_count};
#[path = "../../tests/support/fsdb_workloads.rs"]
pub(crate) mod workloads;

fn range(start: u64, end: u64) -> TimeRange {
    TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end))
}

pub(super) fn temporal(c: &mut Criterion) {
    for (fixture, end) in [
        ("fsdb0010-history-short", 4096),
        ("fsdb0011-history-long", 1_048_576),
    ] {
        let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
        let mut wave = ondas::open_with(path, BACKEND).unwrap();
        let signals = ["top.clock", "top.word_00", "top.word_01"]
            .map(|name| wave.hierarchy().signal(name).unwrap());
        let mut selection = wave.select(&signals).unwrap();
        let mut group =
            c.benchmark_group(format!("fsdb/{}/{fixture}/W1/prepared", fixtures::PROVIDER));
        group.sample_size(10);
        for start in [0, end - 31] {
            let window = range(start, start + 31);
            group.bench_function(
                BenchmarkId::new(
                    format!("adjacent/clock+word00+word01/{start}..={}", start + 31),
                    BACKEND,
                ),
                |b| {
                    b.iter(|| {
                        black_box(
                            workloads::adjacent(
                                &mut selection,
                                black_box(window),
                                &[0],
                                &[0, 1, 2],
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
        // A bounded repeated/forward/backward series. Unlike QueryContext,
        // point reads allow arbitrary backward jumps.
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
        group.bench_function(
            BenchmarkId::new(
                format!("points/clock+word00+word01/mixed8/end{end}"),
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
        group.finish();
    }
}

pub(super) fn payload(c: &mut Criterion) {
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), workloads::WIDE);
    let mut wave = ondas::open_with(&path, BACKEND).unwrap();
    let mut group = c.benchmark_group(format!(
        "fsdb/{}/{}/W2/prepared",
        fixtures::PROVIDER,
        workloads::WIDE
    ));
    group.sample_size(10);
    for shared in [false, true] {
        let signals = workloads::wide_signals(&wave, shared);
        let mut selection = wave.select(&signals).unwrap();
        let topology = if shared {
            "shared-driver"
        } else {
            "independent-drivers"
        };
        // Both drivers tick simultaneously; duplicate/base activity is deduped.
        for period in [2, 64] {
            for sequential in [false, true] {
                let operation = if sequential {
                    "scan-consumer"
                } else {
                    "selective-query"
                };
                group.bench_function(
                    BenchmarkId::new(
                        format!("{operation}/{topology}/accept1in{period}/0..=4096"),
                        BACKEND,
                    ),
                    |b| {
                        b.iter(|| {
                            let accept = |time: Time, high| high && time.ticks() % period == 1;
                            let observe = |time, value: ondas::ValueRef<'_>| {
                                black_box((time, value));
                                ControlFlow::Continue(())
                            };
                            let counts = if sequential {
                                workloads::sequential(
                                    &mut selection,
                                    range(0, 4096),
                                    &[0, 1],
                                    accept,
                                    observe,
                                )
                            } else {
                                workloads::conditional(
                                    &mut selection,
                                    range(0, 4096),
                                    &[0, 1],
                                    accept,
                                    observe,
                                )
                            };
                            black_box(counts.unwrap());
                        })
                    },
                );
            }
        }
    }
    group.finish();

    let signals = workloads::wide_signals(&wave, false);
    let mut selection = wave.select(&signals).unwrap();
    let mut group = c.benchmark_group(format!(
        "fsdb/{}/{}/W3",
        fixtures::PROVIDER,
        workloads::WIDE
    ));
    group.sample_size(10);
    for start in [0, 2048] {
        let window = range(start, 4096);
        group.bench_function(
            BenchmarkId::new(
                format!("prepared/scan/driver+control+payload/{start}..=4096"),
                BACKEND,
            ),
            |b| b.iter(|| black_box(scan_count(&mut selection, black_box(window)))),
        );
        group.bench_function(
            BenchmarkId::new(
                format!("prepared/candidates/driver+control+payload/{start}..=4096"),
                BACKEND,
            ),
            |b| b.iter(|| black_box(candidate_count(&mut selection, black_box(window)))),
        );
        for (outcome, after) in [
            ("first-accepted", start + 128),
            ("rejected-prefix", 3968),
            ("no-match", 4097),
        ] {
            for fresh in [false, true] {
                let boundary = if fresh {
                    "fresh-open-select-query-drop"
                } else {
                    "prepared"
                };
                group.bench_function(
                    BenchmarkId::new(
                        format!("{boundary}/{outcome}/after{after}/accept1in64/{start}..=4096"),
                        BACKEND,
                    ),
                    |b| {
                        b.iter(|| {
                            let accept = |time: Time, high| {
                                high && time.ticks() >= after && time.ticks() % 64 == 1
                            };
                            let observe = |time, value: ondas::ValueRef<'_>| {
                                black_box((time, value));
                                ControlFlow::Break(())
                            };
                            // Include hierarchy resolution, selection, query cleanup and
                            // waveform destruction for the fresh end-to-end boundary.
                            let counts = if fresh {
                                let mut wave = ondas::open_with(black_box(&path), BACKEND).unwrap();
                                let signals = workloads::wide_signals(&wave, false);
                                workloads::conditional(
                                    &mut wave.select(&signals).unwrap(),
                                    window,
                                    &[0],
                                    accept,
                                    observe,
                                )
                            } else {
                                workloads::conditional(
                                    &mut selection,
                                    window,
                                    &[0],
                                    accept,
                                    observe,
                                )
                            };
                            black_box(counts.unwrap());
                        })
                    },
                );
            }
        }
    }
    group.finish();
}

pub(super) fn typed(c: &mut Criterion) {
    let fixture = "fsdb0017-typed-records";
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(path, workloads::BACKEND).unwrap();
    let signals = [
        "top.trigger",
        "top.logic4",
        "top.real64",
        "top.real32",
        "top.text_short",
        "top.text_long",
    ]
    .map(|name| wave.hierarchy().signal(name).unwrap());
    let mut selection = wave.select(&signals).unwrap();
    let mut group = c.benchmark_group(format!("fsdb/{}/{fixture}/W1/prepared", fixtures::PROVIDER));
    group.sample_size(10);
    group.bench_function(
        BenchmarkId::new("adjacent/event+logic+reals+strings/2048..=2080", BACKEND),
        |b| {
            b.iter(|| {
                black_box(
                    workloads::adjacent(
                        &mut selection,
                        range(2048, 2080),
                        &[0],
                        &[0, 1, 2, 3, 4, 5],
                        |time, index, sample| {
                            black_box((time, index, sample));
                        },
                    )
                    .unwrap(),
                )
            })
        },
    );
    group.finish();
}
