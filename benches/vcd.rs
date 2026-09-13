use std::{collections::HashSet, hint::black_box, ops::ControlFlow};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ondas::{ScanRef, Time, TimeRange};

#[path = "../tests/support/fixtures.rs"]
mod fixtures;

const FIXTURE: &str = "vcd0071-swerv1";
const BACKEND: &str = "vcd-native";
const SIGNALS: [&str; 4] = [
    "TOP.core_clk",
    "TOP.tb_top.cycleCnt",
    "TOP.tb_top.commit_count",
    "TOP.tb_top.WriteData",
];

// Fixed relative paths under TOP.tb_top, not an order-dependent hierarchy prefix.
const WIDE_SIGNALS: [&str; 64] = [
    "core_clk",
    "MAX_CYCLES",
    "WriteData",
    "commit_count",
    "cycleCnt",
    "debug_brkpt_status",
    "dec_tlu_perfcnt0",
    "dec_tlu_perfcnt1",
    "dec_tlu_perfcnt2",
    "dec_tlu_perfcnt3",
    "dma_axi_araddr",
    "dma_axi_arburst",
    "dma_axi_arid",
    "dma_axi_arlen",
    "dma_axi_arprot",
    "dma_axi_arready",
    "dma_axi_arsize",
    "dma_axi_arvalid",
    "dma_axi_awaddr",
    "dma_axi_awburst",
    "dma_axi_awid",
    "dma_axi_awlen",
    "dma_axi_awprot",
    "dma_axi_awready",
    "dma_axi_awsize",
    "dma_axi_awvalid",
    "dma_axi_bid",
    "dma_axi_bready",
    "dma_axi_bresp",
    "dma_axi_bvalid",
    "dma_axi_rdata",
    "dma_axi_rlast",
    "dma_axi_rready",
    "dma_axi_rvalid",
    "dma_axi_wdata",
    "dma_axi_wlast",
    "dma_axi_wready",
    "dma_axi_wstrb",
    "dma_axi_wvalid",
    "dma_hrdata",
    "dma_hready",
    "dma_hready_out",
    "dma_hresp",
    "dma_hwdata",
    "el",
    "fd",
    "ic_haddr",
    "ic_hburst",
    "ic_hmastlock",
    "ic_hprot",
    "ic_hrdata",
    "ic_hready",
    "ic_hresp",
    "ic_hsize",
    "ic_htrans",
    "ic_hwrite",
    "ifu_axi_araddr",
    "ifu_axi_arburst",
    "ifu_axi_arcache",
    "ifu_axi_arid",
    "ifu_axi_arlen",
    "ifu_axi_arlock",
    "ifu_axi_arprot",
    "ifu_axi_arqos",
];

fn swerv(c: &mut Criterion) {
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), FIXTURE);
    let mut wave = ondas::open_with(&path, BACKEND).expect("open VCD workload");
    let signals = SIGNALS.map(|path| wave.hierarchy().signal(path).expect("workload signal"));
    let early = Time::from_ticks(1000);
    let late = Time::from_ticks(13000);
    let group_name = format!("vcd/{}/{FIXTURE}/file", fixtures::PROVIDER);
    let mut group = c.benchmark_group(&group_name);

    // Opening includes destruction, but does not claim a cold filesystem cache.
    group.bench_function(BenchmarkId::new("open", BACKEND), |b| {
        b.iter(|| {
            drop(black_box(
                ondas::open_with(black_box(&path), BACKEND).unwrap(),
            ))
        });
    });

    {
        let mut selection = wave.select(&signals[..1]).unwrap();
        for time in [early, late] {
            selection.samples(time).expect("sample preflight");
            group.bench_function(
                BenchmarkId::new(format!("sample/{}/t{}", SIGNALS[0], time.ticks()), BACKEND),
                |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
            );
        }
        for (start, end) in [(0, 100), (13000, 13100), (12000, 13000)] {
            let range = TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end));
            let _ = selection
                .scan(range, |_| ControlFlow::<()>::Continue(()))
                .expect("scan preflight");
            group.bench_function(
                BenchmarkId::new(format!("scan/{}/{start}..={end}", SIGNALS[0]), BACKEND),
                |b| {
                    b.iter(|| {
                        let mut count = 0_u64;
                        let _ = selection
                            .scan(black_box(range), |_| {
                                count += 1;
                                ControlFlow::<()>::Continue(())
                            })
                            .unwrap();
                        black_box(count)
                    });
                },
            );
        }
        let range = TimeRange::closed(Time::from_ticks(12000), late);
        let first_change = |record: ScanRef<'_>| match record {
            ScanRef::Change { time, .. } => ControlFlow::Break(time),
            _ => ControlFlow::Continue(()),
        };
        for start in [1, 12000] {
            let range = TimeRange::closed(Time::from_ticks(start), late);
            assert!(
                selection
                    .scan(range, first_change)
                    .expect("first-change preflight")
                    .is_break(),
                "first-change workload must find a change"
            );
            group.bench_function(
                BenchmarkId::new(
                    format!("scan-first-change/{}/{start}..=13000", SIGNALS[0]),
                    BACKEND,
                ),
                |b| b.iter(|| black_box(selection.scan(black_box(range), first_change).unwrap())),
            );
        }
        let mut candidates = 0_u64;
        let _ = selection
            .scan_candidate_times(range, |_| {
                candidates += 1;
                ControlFlow::<()>::Continue(())
            })
            .expect("candidate-times preflight");
        assert!(
            candidates > 0,
            "candidate-times workload must find activity"
        );
        group.bench_function(
            BenchmarkId::new(
                format!("candidate-times/{}/12000..=13000", SIGNALS[0]),
                BACKEND,
            ),
            |b| {
                b.iter(|| {
                    let mut count = 0_u64;
                    let _ = selection
                        .scan_candidate_times(black_box(range), |_| {
                            count += 1;
                            ControlFlow::<()>::Continue(())
                        })
                        .unwrap();
                    black_box(count)
                });
            },
        );
        selection.traces(range).expect("trace preflight");
        group.bench_function(
            BenchmarkId::new(format!("trace/{}/12000..=13000", SIGNALS[0]), BACKEND),
            |b| b.iter(|| black_box(selection.traces(black_box(range)).unwrap())),
        );
    }

    // Both one-shot alternatives include selection creation and owned result drops.
    for &signal in &signals {
        wave.sample(signal, late)
            .expect("individual sample preflight");
    }
    wave.samples(&signals, late)
        .expect("batch sample preflight");
    let batch = format!("{}/t{}", SIGNALS.join("+"), late.ticks());
    group.bench_function(
        BenchmarkId::new(format!("samples-individual/{batch}"), BACKEND),
        |b| {
            b.iter(|| {
                for &signal in black_box(&signals) {
                    black_box(wave.sample(signal, black_box(late)).unwrap());
                }
            });
        },
    );
    group.bench_function(
        BenchmarkId::new(format!("samples-batch/{batch}"), BACKEND),
        |b| {
            b.iter(|| black_box(wave.samples(black_box(&signals), black_box(late)).unwrap()));
        },
    );
    {
        let wide = WIDE_SIGNALS.map(|name| {
            wave.hierarchy()
                .signal(&format!("TOP.tb_top.{name}"))
                .expect("wide workload signal")
        });
        assert_eq!(
            wide.iter().collect::<HashSet<_>>().len(),
            64,
            "wide workload must select 64 unique histories"
        );
        let mut selection = wave.select(&wide).unwrap();
        selection.samples(late).expect("wide sample preflight");
        group.bench_function(
            BenchmarkId::new("samples-wide/TOP.tb_top/fixed64/t13000", BACKEND),
            |b| b.iter(|| black_box(selection.samples(black_box(late)).unwrap())),
        );
    }
    {
        let word = signals[3];
        let upper = word.slice(31, 16).unwrap();
        let overlap = word.slice(23, 8).unwrap();
        let mut selection = wave.select(&[word, upper, overlap, upper]).unwrap();
        selection
            .samples(late)
            .expect("projection sample preflight");
        group.bench_function(
            BenchmarkId::new(
                "samples-projections/TOP.tb_top.WriteData/whole+31:16+23:8+31:16/t13000",
                BACKEND,
            ),
            |b| b.iter(|| black_box(selection.samples(black_box(late)).unwrap())),
        );
    }
    group.finish();

    // A 32-query iteration replays almost the whole file 32 times. Keep this
    // group's sample count small rather than changing the other workloads.
    let mut group = c.benchmark_group(format!("{group_name}/sample-series"));
    group.sample_size(10);
    let mut selection = wave.select(&signals[..1]).unwrap();
    for count in [8_u64, 32] {
        let times = (0..count)
            .map(|index| Time::from_ticks(13000 - (count - 1 - index) * 32))
            .collect::<Vec<_>>();
        for &time in &times {
            selection.samples(time).expect("sample-series preflight");
        }
        group.throughput(Throughput::Elements(count));
        group.bench_function(
            BenchmarkId::new(format!("{}/n{count}/end13000/step32", SIGNALS[0]), BACKEND),
            |b| {
                b.iter(|| {
                    for &time in black_box(&times) {
                        black_box(selection.samples(time).unwrap());
                    }
                });
            },
        );
    }
    group.finish();
}

fn compact_wide(c: &mut Criterion) {
    let fixture = "vcd0096-wide-compact-toggle";
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).expect("open compact-wide workload");
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    let control = wave.hierarchy().signal("top.control").unwrap();
    let mut group = c.benchmark_group(format!("vcd/{}/{fixture}/file", fixtures::PROVIDER));
    group.bench_function(BenchmarkId::new("open", BACKEND), |b| {
        b.iter(|| {
            drop(black_box(
                ondas::open_with(black_box(&path), BACKEND).unwrap(),
            ))
        });
    });
    let time = Time::from_ticks(4096);
    for (name, signal) in [
        ("top.wide", wide),
        ("top.wide[0:0]", wide.slice(0, 0).unwrap()),
        ("top.control", control),
    ] {
        let mut selection = wave.select(&[signal]).unwrap();
        selection.samples(time).expect("compact sample preflight");
        group.bench_function(
            BenchmarkId::new(format!("sample/{name}/t4096"), BACKEND),
            |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
        );
    }
    let range = TimeRange::closed(Time::from_ticks(2048), time);
    for (name, signal) in [
        ("top.wide", wide),
        ("top.wide[4095:1]", wide.slice(4095, 1).unwrap()),
    ] {
        let mut selection = wave.select(&[signal]).unwrap();
        let _ = selection
            .scan(range, |_| ControlFlow::<()>::Continue(()))
            .expect("compact scan preflight");
        group.bench_function(
            BenchmarkId::new(format!("scan/{name}/2048..=4096"), BACKEND),
            |b| {
                b.iter(|| {
                    let mut count = 0_u64;
                    let _ = selection
                        .scan(black_box(range), |_| {
                            count += 1;
                            ControlFlow::<()>::Continue(())
                        })
                        .unwrap();
                    black_box(count)
                });
            },
        );
    }
    let mut selection = wave.select(&[wide]).unwrap();
    let _ = selection
        .scan_candidate_times(range, |_| ControlFlow::<()>::Continue(()))
        .expect("compact candidate-times preflight");
    group.bench_function(
        BenchmarkId::new("candidate-times/top.wide/2048..=4096", BACKEND),
        |b| {
            b.iter(|| {
                let mut count = 0_u64;
                let _ = selection
                    .scan_candidate_times(black_box(range), |_| {
                        count += 1;
                        ControlFlow::<()>::Continue(())
                    })
                    .unwrap();
                black_box(count)
            });
        },
    );
    group.finish();
}

fn scr1(c: &mut Criterion) {
    let fixture = "vcd0097-scr1-max-ahb-coremark";
    let (path, _) = fixtures::load_artifact(&fixtures::provider(), fixture);
    let mut wave = ondas::open_with(&path, BACKEND).expect("open SCR1 workload");
    let paths = [
        "TOP.clk",
        "TOP.scr1_top_tb_ahb.i_top.timer_val",
        "TOP.scr1_top_tb_ahb.i_top.i_core_top.i_pipe_top.i_pipe_exu.i_ialu.main_sum_res",
        "TOP.scr1_top_tb_ahb.i_top.i_core_top.i_pipe_top.i_pipe_ifu.imem_addr_ff",
    ];
    let signals = paths.map(|path| wave.hierarchy().signal(path).expect("SCR1 workload signal"));
    let mut group = c.benchmark_group(format!("vcd/{}/{fixture}/file", fixtures::PROVIDER));
    // Late queries parse almost a gigabyte per iteration.
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
        for ticks in [62444, 6244000] {
            let time = Time::from_ticks(ticks);
            selection.samples(time).expect("SCR1 sample preflight");
            group.bench_function(
                BenchmarkId::new(format!("sample/TOP.clk/t{ticks}"), BACKEND),
                |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
            );
        }
        for start in [62400, 6244000] {
            let end = start + 44;
            let range = TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end));
            let _ = selection
                .scan(range, |_| ControlFlow::<()>::Continue(()))
                .expect("SCR1 scan preflight");
            group.bench_function(
                BenchmarkId::new(format!("scan/TOP.clk/{start}..={end}"), BACKEND),
                |b| {
                    b.iter(|| {
                        let mut count = 0_u64;
                        let _ = selection
                            .scan(black_box(range), |_| {
                                count += 1;
                                ControlFlow::<()>::Continue(())
                            })
                            .unwrap();
                        black_box(count)
                    });
                },
            );
        }
    }
    let time = Time::from_ticks(6244000);
    let mut selection = wave.select(&signals).unwrap();
    selection.samples(time).expect("SCR1 batch preflight");
    group.bench_function(
        BenchmarkId::new(
            "samples-batch/clk+timer_val+main_sum_res+imem_addr_ff/t6244000",
            BACKEND,
        ),
        |b| b.iter(|| black_box(selection.samples(black_box(time)).unwrap())),
    );
    group.finish();
}

criterion_group!(benches, swerv, compact_wide, scr1);
criterion_main!(benches);
