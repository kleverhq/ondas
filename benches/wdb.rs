use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ondas::{HierarchyPath, SampleRef, ScanRef, Selection, Time, TimeRange, ValueRef, Waveform};
use sha2::{Digest, Sha256};
use std::{hint::black_box, ops::ControlFlow};

#[path = "../tests/support/fixtures.rs"]
mod fixtures;
#[path = "../tests/support/fsdb_compatibility.rs"]
mod fsdb_compatibility;
#[allow(dead_code, unused_imports)]
#[path = "../tests/support/normalized_oracle.rs"]
mod normalized_oracle;

const IDS: [&str; 4] = ["regular64/0", "wide1024/0", "payload/0", "constant64/0"];
// Independent full-history model: SHA-256 of each tick (little-endian u64)
// followed by its exact-width MSB-first ASCII bits, in chronological order.
const EXPECTED: [[(u64, &str); 4]; 2] = [
    [
        (
            101,
            "e410259500881d633cdd0647f99cedcf0ca23ea92a0355564f90e234f1458a44",
        ),
        (
            101,
            "61c50b80398dee0923dafb53b9bd6fca438f4cc82ac63a50f5f459f3ddb1f762",
        ),
        (
            10001,
            "6fae42906e003730464a97ac5e5f57f627a1e80aae1187dedc8ffc4de0c6122a",
        ),
        (
            1,
            "1854428042b64e41ec263ad7289d837abb510b5874c3ecad1fc5c094230e392a",
        ),
    ],
    [
        (
            11,
            "45a4dd22297572cc51948ee3da0f60c8aba47001f428314e7ef7fd5d145c953d",
        ),
        (
            1001,
            "b605b6a3d2726aab3e90a01307aeab65b0aea14b0cf159272d971369b7f3ab60",
        ),
        (
            100001,
            "0673416c827f6bb6b222b832a73a8385eec1aef1003778209ec43dd39370575d",
        ),
        (
            1,
            "1854428042b64e41ec263ad7289d837abb510b5874c3ecad1fc5c094230e392a",
        ),
    ],
];

fn scan_count(selection: &mut Selection<'_>, range: TimeRange) -> u64 {
    let mut count = 0;
    let _ = selection
        .scan(range, |record| {
            black_box(record);
            count += 1;
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    count
}

fn compare(c: &mut Criterion) {
    for (profile, suffix) in ["s", "m"].into_iter().enumerate() {
        let mut group = c.benchmark_group(format!("matched-opb/{suffix}/file"));
        group.sample_size(20);
        for (format, number, backend) in [
            ("wdb", 300 + profile, "wdb-native"),
            ("fsdb", 60 + profile, "fsdb-lib"),
        ] {
            if format == "fsdb" && !cfg!(feature = "fsdb-lib") {
                continue;
            }
            let name = format!("{format}/{format}{number:04}-opb-{suffix}");
            let (path, sidecar) = fixtures::load_artifact(&fixtures::root(), &name);
            let oracle = &sidecar["oracle"];
            let end: u64 = oracle["metadata"]["time_span"]["last"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap();
            let range = TimeRange::closed(Time::ZERO, Time::from_ticks(end));
            let paths: Vec<_> = IDS
                .iter()
                .map(|id| {
                    let variable = oracle["hierarchy"]["variables"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|v| v["signal"] == *id)
                        .unwrap();
                    HierarchyPath::from_components(
                        variable["path"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_str().unwrap()),
                    )
                })
                .collect();
            let resolve = |wave: &Waveform| {
                paths
                    .iter()
                    .map(|p| wave.hierarchy().signal_path(p).unwrap())
                    .collect::<Vec<_>>()
            };
            let Some(mut wave) = fsdb_compatibility::open(ondas::open_with(&path, backend), &path)
            else {
                continue;
            };
            let signals = resolve(&wave);
            let mut hashes = [Sha256::new(), Sha256::new(), Sha256::new(), Sha256::new()];
            let mut counts = [0_u64; 4];
            let _ = wave
                .select(&signals)
                .unwrap()
                .scan_each(range, |index, record| {
                    let ScanRef::Change {
                        time,
                        value: ValueRef::Bits(bits),
                        ..
                    } = record
                    else {
                        panic!("expected bits change from zero")
                    };
                    hashes[index].update(time.ticks().to_le_bytes());
                    hashes[index].update(bits.to_string().as_bytes());
                    counts[index] += 1;
                    ControlFlow::<()>::Continue(())
                })
                .unwrap();
            for (index, hash) in hashes.into_iter().enumerate() {
                assert_eq!(
                    counts[index], EXPECTED[profile][index].0,
                    "{name} {}",
                    IDS[index]
                );
                assert_eq!(
                    format!("{:x}", hash.finalize()),
                    EXPECTED[profile][index].1,
                    "{name} {}",
                    IDS[index]
                );
                for expected in oracle["signals"][IDS[index]]["samples"].as_array().unwrap() {
                    let tick: u64 = expected["time"].as_str().unwrap().parse().unwrap();
                    let expected = normalized_oracle::sample_assertions(
                        expected,
                        oracle["signals"][IDS[index]]["windows"]
                            .as_array()
                            .map_or(&[][..], Vec::as_slice),
                        tick,
                    );
                    let actual = wave.sample(signals[index], Time::from_ticks(tick)).unwrap();
                    let SampleRef::Value {
                        value: ValueRef::Bits(bits),
                        changed_at,
                        ..
                    } = actual.as_ref()
                    else {
                        panic!("expected bits point")
                    };
                    assert_eq!(
                        bits.to_string(),
                        expected["value"]["bits"].as_str().unwrap(),
                        "{name} {} t{tick}",
                        IDS[index]
                    );
                    if let Some(changed) = expected["changed_at"].as_str() {
                        assert_eq!(changed_at.map(Time::ticks), Some(changed.parse().unwrap()));
                    }
                    assert!(changed_at.is_none_or(|changed| changed.ticks() <= tick));
                }
            }
            // Fresh readers include setup and destruction at both source bounds.
            for (label, tick) in [("t0", Time::ZERO), ("end", Time::from_ticks(end))] {
                group.bench_function(
                    BenchmarkId::new(format!("open+first-four/{label}"), backend),
                    |b| {
                        b.iter(|| {
                            let mut fresh = ondas::open_with(black_box(&path), backend).unwrap();
                            let signals = resolve(&fresh);
                            black_box(fresh.samples(&signals, tick).unwrap());
                        })
                    },
                );
            }
            group.bench_function(
                BenchmarkId::new("point/fresh-selection/end", backend),
                |b| {
                    b.iter(|| {
                        black_box(
                            wave.select(black_box(&signals))
                                .unwrap()
                                .samples(Time::from_ticks(end))
                                .unwrap(),
                        )
                    })
                },
            );
            let mut selected = wave.select(&signals).unwrap();
            selected.samples(Time::from_ticks(end)).unwrap();
            group.bench_function(BenchmarkId::new("point/primed-repeat/end", backend), |b| {
                b.iter(|| black_box(selected.samples(Time::from_ticks(end)).unwrap()))
            });
            group.bench_function(BenchmarkId::new("scan/full-four", backend), |b| {
                b.iter(|| black_box(scan_count(&mut selected, range)))
            });
        }
        group.finish();
    }
}

criterion_group!(benches, compare);
criterion_main!(benches);
