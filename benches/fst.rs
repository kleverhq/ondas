use std::{collections::HashSet, fs, hint::black_box, ops::ControlFlow, sync::Arc};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ondas::{ScanRef, Selection, Time, TimeRange};

#[path = "../tests/support/fixtures.rs"]
mod fixtures;
#[path = "fst/hotpaths.rs"]
mod hotpaths;

const BACKEND: &str = "fst-native";
const SCR1_SIGNALS: [&str; 4] = [
    "TOP.clk",
    "TOP.scr1_top_tb_ahb.i_top.timer_val",
    "TOP.scr1_top_tb_ahb.i_top.i_core_top.i_pipe_top.i_pipe_exu.i_ialu.main_sum_res",
    "TOP.scr1_top_tb_ahb.i_top.i_core_top.i_pipe_top.i_pipe_ifu.imem_addr_ff",
];

fn scan_count(selection: &mut Selection<'_>, range: TimeRange) -> u64 {
    let mut count = 0_u64;
    let _ = selection
        .scan(range, |_| {
            count += 1;
            ControlFlow::<()>::Continue(())
        })
        .expect("scan workload");
    count
}

fn first_change(record: ScanRef<'_>) -> ControlFlow<Time> {
    match record {
        ScanRef::Change { time, .. } => ControlFlow::Break(time),
        _ => ControlFlow::Continue(()),
    }
}

fn scr1(c: &mut Criterion) {
    let fixture = "fst0015-scr1-max-ahb-coremark";
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    // Loading and allocating the shared bytes is not part of bytes opening.
    let bytes: Arc<[u8]> = fs::read(&path).unwrap().into();
    let late = Time::from_ticks(6_244_000);
    let late_range = TimeRange::closed(late, Time::from_ticks(6_244_044));

    for mode in ["file", "bytes"] {
        let open = || match mode {
            "file" => ondas::open_with(black_box(&path), BACKEND).unwrap(),
            "bytes" => {
                ondas::open_bytes_with("waveform.fst", Arc::clone(black_box(&bytes)), BACKEND)
                    .unwrap()
            }
            _ => unreachable!(),
        };
        let mut wave = open();
        let signals = SCR1_SIGNALS.map(|path| wave.hierarchy().signal(path).unwrap());
        assert_eq!(signals.iter().collect::<HashSet<_>>().len(), 4);
        let group_name = format!("fst/{}/{fixture}/{mode}", fixtures::PROVIDER);
        let mut group = c.benchmark_group(&group_name);
        group.sample_size(10);

        // Fresh waveform and destruction, with a potentially warm filesystem cache.
        group.bench_function(BenchmarkId::new("open", BACKEND), |b| {
            b.iter(|| drop(black_box(open())));
        });
        {
            let mut selection = wave.select(&signals[..1]).unwrap();
            selection.samples(late).expect("late sample preflight");
            group.bench_function(BenchmarkId::new("sample/TOP.clk/t6244000", BACKEND), |b| {
                b.iter(|| black_box(selection.samples(black_box(late)).unwrap()));
            });
            assert!(scan_count(&mut selection, late_range) > 0);
            group.bench_function(
                BenchmarkId::new("scan/TOP.clk/6244000..=6244044", BACKEND),
                |b| b.iter(|| black_box(scan_count(&mut selection, black_box(late_range)))),
            );
        }
        // Only these three operations have paired file/bytes cases.
        if mode == "bytes" {
            group.finish();
            continue;
        }

        group.bench_function(
            BenchmarkId::new(format!("lookup/{}", SCR1_SIGNALS[3]), BACKEND),
            |b| {
                b.iter(|| black_box(wave.hierarchy().signal(black_box(SCR1_SIGNALS[3])).unwrap()));
            },
        );
        for count in [1, 4] {
            wave.select(&signals[..count]).expect("selection preflight");
            group.bench_function(
                BenchmarkId::new(format!("select/scr1-fixed4/prefix{count}"), BACKEND),
                |b| {
                    b.iter(|| {
                        drop(black_box(
                            wave.select(black_box(&signals[..count])).unwrap(),
                        ))
                    });
                },
            );
        }
        {
            let mut selection = wave.select(&signals[..1]).unwrap();
            let early = Time::from_ticks(62_400);
            selection.samples(early).expect("early sample preflight");
            group.bench_function(BenchmarkId::new("sample/TOP.clk/t62400", BACKEND), |b| {
                b.iter(|| black_box(selection.samples(black_box(early)).unwrap()));
            });
            // Equal-width early/late windows distinguish prefix work from output size.
            // The full scan remains callback-based rather than owning millions of values.
            for (start, end) in [(62_400, 62_444), (1, 6_244_302)] {
                let range = TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end));
                assert!(scan_count(&mut selection, range) > 0);
                group.bench_function(
                    BenchmarkId::new(format!("scan/TOP.clk/{start}..={end}"), BACKEND),
                    |b| b.iter(|| black_box(scan_count(&mut selection, black_box(range)))),
                );
            }
            for start in [62_400, 6_244_000] {
                let range = TimeRange::closed(Time::from_ticks(start), Time::from_ticks(6_244_302));
                assert!(selection.scan(range, first_change).unwrap().is_break());
                group.bench_function(
                    BenchmarkId::new(
                        format!("scan-first-change/TOP.clk/{start}..=6244302"),
                        BACKEND,
                    ),
                    |b| {
                        b.iter(|| {
                            black_box(selection.scan(black_box(range), first_change).unwrap())
                        })
                    },
                );
            }
            selection.traces(late_range).expect("trace preflight");
            group.bench_function(
                BenchmarkId::new("trace/TOP.clk/6244000..=6244044", BACKEND),
                |b| b.iter(|| black_box(selection.traces(black_box(late_range)).unwrap())),
            );
            let mut candidates = || {
                let mut count = 0_u64;
                let _ = selection
                    .scan_candidate_times(black_box(late_range), |_| {
                        count += 1;
                        ControlFlow::<()>::Continue(())
                    })
                    .unwrap();
                black_box(count)
            };
            assert!(candidates() > 0);
            group.bench_function(
                BenchmarkId::new("candidate-times/TOP.clk/6244000..=6244044", BACKEND),
                |b| b.iter(&mut candidates),
            );
        }
        // Both alternatives include selection construction and owned result destruction.
        for &signal in &signals {
            wave.sample(signal, late)
                .expect("individual sample preflight");
        }
        wave.samples(&signals, late)
            .expect("batch sample preflight");
        group.bench_function(
            BenchmarkId::new("samples-individual/scr1-fixed4/t6244000", BACKEND),
            |b| {
                b.iter(|| {
                    for &signal in black_box(&signals) {
                        black_box(wave.sample(signal, black_box(late)).unwrap());
                    }
                })
            },
        );
        group.bench_function(
            BenchmarkId::new("samples-batch/scr1-fixed4/t6244000", BACKEND),
            |b| b.iter(|| black_box(wave.samples(black_box(&signals), black_box(late)).unwrap())),
        );
        {
            let mut selection = wave.select(&signals).unwrap();
            selection.samples(late).expect("prepared batch preflight");
            group.bench_function(
                BenchmarkId::new("samples-prepared/scr1-fixed4/t6244000", BACKEND),
                |b| b.iter(|| black_box(selection.samples(black_box(late)).unwrap())),
            );
        }
        // Whole and projected histories read the same active 33-bit base signal.
        let word = signals[2];
        for (name, signal) in [("whole", word), ("7:0", word.slice(7, 0).unwrap())] {
            let mut selection = wave.select(&[signal]).unwrap();
            selection
                .traces(late_range)
                .expect("projected trace preflight");
            group.bench_function(
                BenchmarkId::new(
                    format!("trace/{}/{name}/6244000..=6244044", SCR1_SIGNALS[2]),
                    BACKEND,
                ),
                |b| b.iter(|| black_box(selection.traces(black_box(late_range)).unwrap())),
            );
        }
        {
            let upper = word.slice(32, 16).unwrap();
            let overlap = word.slice(23, 8).unwrap();
            let mut selection = wave.select(&[word, upper, overlap, upper]).unwrap();
            selection
                .samples(late)
                .expect("repeated projection preflight");
            group.bench_function(
                BenchmarkId::new(
                    format!(
                        "samples-projections/{}/whole+32:16+23:8+32:16/t6244000",
                        SCR1_SIGNALS[2]
                    ),
                    BACKEND,
                ),
                |b| b.iter(|| black_box(selection.samples(black_box(late)).unwrap())),
            );
        }
        group.finish();

        let mut group = c.benchmark_group(format!("{group_name}/sample-series"));
        group.sample_size(10);
        let mut selection = wave.select(&signals[..1]).unwrap();
        for count in [8_u64, 32] {
            let times = (0..count)
                .map(|index| Time::from_ticks(6_244_000 - (count - 1 - index) * 32))
                .collect::<Vec<_>>();
            for &time in &times {
                selection.samples(time).expect("sample-series preflight");
            }
            group.throughput(Throughput::Elements(count));
            group.bench_function(
                BenchmarkId::new(format!("TOP.clk/n{count}/end6244000/step32"), BACKEND),
                |b| {
                    b.iter(|| {
                        for &time in black_box(&times) {
                            black_box(selection.samples(time).unwrap());
                        }
                    })
                },
            );
        }
        group.finish();
    }
}

fn chipyard(c: &mut Criterion) {
    let fixture = "fst0000-chipyard-clusteredrocketconfig-dhrystone";
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).expect("open Chipyard workload");
    let paths = [
        "TOP.TestDriver.clock",
        "TOP.TestDriver.testHarness.chiptop0.axi4_mem_0_bits_aw_valid",
        "TOP.TestDriver.testHarness.chiptop0.axi4_mem_0_bits_aw_bits_addr",
        "TOP.TestDriver.testHarness.chiptop0.axi4_mem_0_bits_w_bits_data",
    ];
    let signals = paths.map(|path| wave.hierarchy().signal(path).unwrap());
    assert_eq!(signals.iter().collect::<HashSet<_>>().len(), 4);
    let mut group = c.benchmark_group(format!("fst/{}/{fixture}/file", fixtures::PROVIDER));
    group.sample_size(10);
    group.bench_function(BenchmarkId::new("open", BACKEND), |b| {
        b.iter(|| {
            drop(black_box(
                ondas::open_with(black_box(&path), BACKEND).unwrap(),
            ))
        });
    });
    {
        let mut selection = wave.select(&signals[..1]).unwrap();
        for start in [16_000_000, 1_600_000_000] {
            let time = Time::from_ticks(start);
            selection.samples(time).expect("Chipyard sample preflight");
            group.bench_function(
                BenchmarkId::new(format!("sample/TOP.TestDriver.clock/t{start}"), BACKEND),
                |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
            );
            let end = start + 10_000;
            let range = TimeRange::closed(time, Time::from_ticks(end));
            assert!(scan_count(&mut selection, range) > 0);
            group.bench_function(
                BenchmarkId::new(
                    format!("scan/TOP.TestDriver.clock/{start}..={end}"),
                    BACKEND,
                ),
                |b| b.iter(|| black_box(scan_count(&mut selection, black_box(range)))),
            );
        }
    }
    let time = Time::from_ticks(1_600_000_000);
    let mut selection = wave.select(&signals).unwrap();
    selection.samples(time).expect("Chipyard batch preflight");
    group.bench_function(
        BenchmarkId::new(
            "samples-prepared/clock+aw_valid+aw_addr+w_data/t1600000000",
            BACKEND,
        ),
        |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
    );
    group.finish();
}

fn picorv32(c: &mut Criterion) {
    let fixture = "fst0012-picorv32-test-ez-vcd";
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).expect("open PicoRV32 workload");
    // Every unique history, not an order-dependent prefix of declarations or aliases.
    let signals = wave.hierarchy().signals().collect::<Vec<_>>();
    assert!(!signals.is_empty());
    let all = format!("all-unique/n{}", signals.len());
    let mut group = c.benchmark_group(format!("fst/{}/{fixture}/file", fixtures::PROVIDER));
    group.bench_function(BenchmarkId::new("open", BACKEND), |b| {
        b.iter(|| {
            drop(black_box(
                ondas::open_with(black_box(&path), BACKEND).unwrap(),
            ))
        });
    });
    group.bench_function(BenchmarkId::new("hierarchy/variable-names", BACKEND), |b| {
        b.iter(|| {
            for variable in wave.hierarchy().variables() {
                black_box(variable.name());
            }
        });
    });
    wave.select(&signals)
        .expect("all-signal selection preflight");
    group.bench_function(BenchmarkId::new(format!("select/{all}"), BACKEND), |b| {
        b.iter(|| drop(black_box(wave.select(black_box(&signals)).unwrap())));
    });
    let mut selection = wave.select(&signals).unwrap();
    let time = Time::from_ticks(11_000_000);
    selection
        .samples(time)
        .expect("all-signal sample preflight");
    group.bench_function(
        BenchmarkId::new(format!("samples-prepared/{all}/t11000000"), BACKEND),
        |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
    );
    let range = TimeRange::closed(Time::from_ticks(0), time);
    assert!(scan_count(&mut selection, range) > 0);
    group.bench_function(
        BenchmarkId::new(format!("scan/{all}/0..=11000000"), BACKEND),
        |b| b.iter(|| black_box(scan_count(&mut selection, black_box(range)))),
    );
    // This compact recording makes full owned materialization a bounded workload.
    selection.traces(range).expect("all-signal trace preflight");
    group.bench_function(
        BenchmarkId::new(format!("trace/{all}/0..=11000000"), BACKEND),
        |b| b.iter(|| black_box(selection.traces(black_box(range)).unwrap())),
    );
    group.finish();
}

criterion_group!(
    benches,
    scr1,
    chipyard,
    picorv32,
    hotpaths::topology,
    hotpaths::wide,
    hotpaths::boundaries
);
criterion_main!(benches);
