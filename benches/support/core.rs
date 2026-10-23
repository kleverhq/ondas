use std::{fmt::Write, hint::black_box, sync::Arc};

use criterion::{BatchSize, BenchmarkId, Criterion};
use ondas::{Hierarchy, HierarchyPath, Item};

fn input(branches: usize) -> Arc<[u8]> {
    let mut text = String::from("$scope module top $end\n$var wire 1 root root $end\n");
    for branch in 0..branches {
        writeln!(text, "$scope module b{branch} $end").unwrap();
        for _ in 0..8 {
            text.push_str("$scope module deep $end\n");
        }
        for variable in 0..8 {
            writeln!(text, "$var wire 1 v{branch}_{variable} p{variable} $end").unwrap();
        }
        for _ in 0..9 {
            text.push_str("$upscope $end\n");
        }
    }
    text.push_str("$upscope $end\n$enddefinitions $end\n#0\n");
    text.into_bytes().into()
}

fn walk(hierarchy: &Hierarchy) -> usize {
    let mut pending = Vec::new();
    let mut count = 0;
    for item in hierarchy.roots() {
        count += 1;
        if let Item::Scope(scope) = item {
            pending.push(scope);
        }
    }
    while let Some(scope) = pending.pop() {
        for item in scope.children() {
            count += 1;
            if let Item::Scope(child) = item {
                pending.push(child);
            }
        }
    }
    count
}

pub fn register(c: &mut Criterion) {
    for (size, branches) in [("small", 8), ("medium", 64), ("large", 512)] {
        let bytes = input(branches);
        let open = || ondas::open_bytes_with("core.vcd", bytes.clone(), "vcd-native").unwrap();
        let wave = open();
        let hierarchy = wave.hierarchy();
        assert_eq!(hierarchy.scopes().count(), 1 + 9 * branches);
        assert_eq!(hierarchy.variables().count(), 8 * branches + 1);
        assert_eq!(walk(hierarchy), 2 + 17 * branches);
        let top = hierarchy.scope("top").unwrap();
        let deep_path = format!("top.b{}.{}", branches - 1, ["deep"; 8].join("."));
        let deep = hierarchy.scope(&deep_path).unwrap();
        assert_eq!(top.children().count(), branches + 1);
        assert_eq!(deep.children().count(), 8);
        let path = HierarchyPath::parse(&format!("{deep_path}.p7")).unwrap();
        let mut group = c.benchmark_group(format!("core/{size}"));
        group.bench_function("flat-names", |b| {
            b.iter(|| {
                for variable in hierarchy.variables() {
                    black_box(variable.name());
                }
            })
        });
        group.bench_function("roots/repeat", |b| {
            b.iter(|| black_box(hierarchy.roots().count()))
        });
        for (name, scope) in [("top", top), ("deep", deep)] {
            group.bench_function(BenchmarkId::new("children/repeat", name), |b| {
                b.iter(|| black_box(scope.children().count()))
            });
        }
        group.bench_function("walk/repeat", |b| b.iter(|| black_box(walk(hierarchy))));
        // Opening, parsing, and input destruction are outside the timed routine.
        // A fresh hierarchy includes any first traversal or lookup index work.
        group.bench_function("walk/first", |b| {
            b.iter_batched_ref(
                open,
                |wave| black_box(walk(wave.hierarchy())),
                BatchSize::PerIteration,
            )
        });
        for (position, path) in [
            ("top", HierarchyPath::parse("top.root").unwrap()),
            ("deep", path),
        ] {
            for (name, prior) in [("first", 0), ("build-index", 1)] {
                group.bench_function(format!("variable-path/{position}/{name}"), |b| {
                    b.iter_batched_ref(
                        || {
                            let wave = open();
                            for _ in 0..prior {
                                black_box(wave.hierarchy().variable_path(&path).unwrap());
                            }
                            wave
                        },
                        |wave| {
                            black_box(wave.hierarchy().variable_path(black_box(&path)).unwrap());
                        },
                        BatchSize::PerIteration,
                    )
                });
            }
            hierarchy.variable_path(&path).unwrap();
            hierarchy.variable_path(&path).unwrap();
            group.bench_function(format!("variable-path/{position}/ready"), |b| {
                b.iter(|| black_box(hierarchy.variable_path(black_box(&path)).unwrap()))
            });
        }
        group.bench_function("roots/first", |b| {
            b.iter_batched_ref(
                open,
                |wave| black_box(wave.hierarchy().roots().count()),
                BatchSize::PerIteration,
            )
        });
        for (name, path) in [("top", "top"), ("deep", deep_path.as_str())] {
            let path = HierarchyPath::parse(path).unwrap();
            group.bench_function(BenchmarkId::new("scope-path/repeat", name), |b| {
                b.iter(|| black_box(hierarchy.scope_path(black_box(&path)).unwrap()))
            });
        }
        group.finish();
    }
}
