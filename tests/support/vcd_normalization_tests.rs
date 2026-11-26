use std::collections::HashSet;

use ondas::{Encoding, HierarchyPath, Item, PathComponent, Waveform};

use super::fixture_support;

fn open(name: &str, bytes: bool) -> Waveform {
    let root = fixture_support::root();
    let (directory, sidecar) = fixture_support::read_sidecar(&root, name);
    let path = fixture_support::verify_artifact(&directory, &sidecar["artifact"], name);
    if bytes {
        ondas::open_bytes_with(
            "split.vcd",
            std::fs::read(path).unwrap().into(),
            "vcd-native",
        )
        .unwrap()
    } else {
        ondas::open_with(path, "vcd-native").unwrap()
    }
}

fn public_signals_match_declarations(wave: &Waveform) {
    let hierarchy = wave.hierarchy();
    let signals: HashSet<_> = hierarchy.signals().collect();
    assert_eq!(signals.len(), hierarchy.signals().count());
    assert_eq!(
        signals,
        hierarchy.variables().filter_map(|v| v.signal()).collect()
    );
}

#[test]
#[ignore = "requires installed VCD fixtures; run just conformance"]
fn vcd_split_public_hierarchy_and_aliases() {
    for bytes in [false, true] {
        let wave = open("vcd/vcd0052-test", bytes);
        let hierarchy = wave.hierarchy();
        let count = hierarchy.variable("test.count").unwrap();
        let signal = count.signal().unwrap();
        assert_eq!(signal.encoding(), Encoding::Bits { width: 3 });
        assert_eq!(count.kind(), "wire");
        assert!(count.range().is_none());
        assert_eq!(
            hierarchy
                .variables()
                .filter(|v| v.path() == count.path())
                .count(),
            1
        );
        let scope = hierarchy.scope("test").unwrap();
        assert_eq!(
            scope
                .children()
                .filter(|item| matches!(item, Item::Variable(v) if v.name() == "count"))
                .count(),
            1
        );
        assert_eq!(hierarchy.signal_path(&count.path()).unwrap(), signal);
        assert_eq!(
            hierarchy.signal("test.count[2:1]").unwrap(),
            signal.slice(2, 1).unwrap()
        );
        assert_eq!(hierarchy.aliases(signal).unwrap().count(), 1);
        assert_ne!(hierarchy.signal("test.dut.count").unwrap(), signal);
        let clone = hierarchy.clone();
        assert!(clone.variable("test.count").unwrap() == count);
        let other = open("vcd/vcd0052-test", bytes);
        assert!(other.hierarchy().variable("test.count").unwrap() != count);
        assert!(other.hierarchy().aliases(signal).is_err());
        public_signals_match_declarations(&wave);

        let wave = open("vcd/vcd0082-dump", bytes);
        let hierarchy = wave.hierarchy();
        for name in ["read1data", "read2data"] {
            let variable = hierarchy.variable(&format!("rf_bench.{name}")).unwrap();
            let signal = variable.signal().unwrap();
            assert_eq!(signal.width(), Some(16));
            assert!(variable.range().is_none());
            assert_eq!(
                hierarchy
                    .variables()
                    .filter(|v| v.path() == variable.path())
                    .count(),
                1
            );
            let alias = hierarchy.signal(&format!("rf_bench.DUT.{name}")).unwrap();
            assert_eq!(signal, alias);
            let aliases: Vec<_> = hierarchy
                .aliases(signal.slice(7, 4).unwrap())
                .unwrap()
                .collect();
            assert!(aliases.contains(&variable));
            assert!(aliases.iter().all(|v| v.signal() == Some(signal)));
        }
        public_signals_match_declarations(&wave);
    }
}

#[test]
#[ignore = "requires installed VCD fixtures; run just conformance"]
fn vcd_multidimensional_array_elements_stay_separate() {
    for bytes in [false, true] {
        let wave = open("vcd/vcd0017-nonzero-ranges", bytes);
        let hierarchy = wave.hierarchy();
        let signals: Vec<_> = [0, 1]
            .map(|index| {
                let path = HierarchyPath::from_components(["TOP", "tb"])
                    .join(PathComponent::with_indices("arr_elem", &[10, index]));
                let variable = hierarchy.variable_path(&path).unwrap();
                assert_eq!(variable.name(), "arr_elem");
                assert_eq!(variable.path(), path);
                assert!(variable.range().is_none());
                let signal = variable.signal().unwrap();
                assert_eq!(signal.width(), Some(1));
                signal
            })
            .into();
        assert_ne!(signals[0], signals[1]);
        assert!(
            hierarchy
                .variable_path(&HierarchyPath::from_components([
                    "TOP",
                    "tb",
                    "arr_elem[10]"
                ]))
                .is_err()
        );
        public_signals_match_declarations(&wave);
    }
}

#[test]
#[ignore = "requires installed VCD fixtures; run just conformance"]
fn vcd_normalized_selector_retains_declaration_and_projection() {
    for bytes in [false, true] {
        let mut wave = open("vcd/vcd0109-split-layouts", bytes);
        let h = wave.hierarchy().clone();
        let declaration = h.variable("desc").unwrap();
        let whole = h.resolve_signal("desc", None).unwrap();
        assert!(whole.declaration() == declaration);
        assert!(whole.declaration().range().is_none());
        assert_eq!(whole.projection(), None);

        let full = h.resolve_signal(r#""desc"[3:0]"#, None).unwrap();
        assert_eq!(full.signal(), whole.signal());
        assert_eq!(full.projection(), Some((3, 0)));
        assert!(full.declaration() == declaration);

        let low = h.resolve_signal(r#""desc"[1:0]"#, None).unwrap();
        assert_eq!(low.signal(), whole.signal().slice(1, 0).unwrap());
        assert_eq!(low.projection(), Some((1, 0)));
        assert!(low.declaration() == declaration);
        assert_sample(
            wave.sample(low.signal(), ondas::Time::from_ticks(5))
                .unwrap()
                .as_ref(),
            5,
            LOW,
        );

        let scope = HierarchyPath::from_components(["aliases"]);
        let alias = h.resolve_signal(r#""bus"[1:0]"#, Some(&scope)).unwrap();
        assert_eq!(alias.signal(), low.signal());
        assert_eq!(alias.projection(), Some((1, 0)));
        assert!(alias.declaration() == h.variable("aliases.bus").unwrap());
        assert!(alias.declaration() != declaration);
        assert!(alias.declaration().range().is_none());

        let literal = h
            .resolve_signal(r#"negative_cases."literal[3:2]""#, None)
            .unwrap();
        assert_eq!(literal.declaration().name(), "literal[3:2]");
        assert_eq!(literal.projection(), None);
        assert_eq!(literal.signal(), h.signal("high_alias").unwrap());
        assert!(matches!(
            h.resolve_signal("negative_cases.gap", None),
            Err(ondas::LookupError::Ambiguous { matches: 2, .. })
        ));
    }
}

// These completed-tick histories come from the authored source record table.
const BUS: &[(u64, &str)] = &[
    (5, "10xx"),
    (10, "1001"),
    (15, "1101"),
    (20, "11xz"),
    (25, "00xz"),
    (35, "0000"),
];
const LOW: &[(u64, &str)] = &[(5, "xx"), (10, "01"), (20, "xz"), (35, "00")];
const HIGH: &[(u64, &str)] = &[(5, "10"), (15, "11"), (25, "00")];
const MIXED: &[(u64, &str)] = &[
    (0, "xx1xx"),
    (5, "101xx"),
    (10, "10101"),
    (15, "11001"),
    (20, "110xz"),
    (25, "000xz"),
    (35, "00000"),
];
const BITS: &[(u64, &str)] = &[(0, "101"), (15, "001"), (20, "011"), (35, "010")];

fn assert_sample(sample: ondas::SampleRef<'_>, tick: u64, history: &[(u64, &str)]) {
    let expected = history
        .iter()
        .enumerate()
        .rev()
        .find(|(_, (at, _))| *at <= tick);
    match (sample, expected) {
        (ondas::SampleRef::Missing { .. }, None) => {}
        (
            ondas::SampleRef::Value {
                value: ondas::ValueRef::Bits(bits),
                changed_at,
                ..
            },
            Some((index, (at, text))),
        ) => {
            assert_eq!(bits.to_string(), *text, "tick {tick}");
            assert_eq!(
                changed_at,
                (index != 0).then_some(ondas::Time::from_ticks(*at)),
                "tick {tick}"
            );
        }
        (actual, expected) => panic!("tick {tick}: {actual:?} versus {expected:?}"),
    }
}

#[test]
#[ignore = "requires installed VCD fixtures; run just conformance"]
fn vcd_split_layouts_metadata_identity_and_rejections() {
    use ondas::LookupError;
    for bytes in [false, true] {
        let mut wave = open("vcd/vcd0109-split-layouts", bytes);
        let h = wave.hierarchy().clone();
        let bus = h.signal("desc").unwrap();
        // The negative-index scalar group declares its lowest index first.
        assert_eq!(
            h.signal("bits").unwrap(),
            h.signal("negative_bits").unwrap()
        );
        let alias_paths = ["desc", "desc_ordered", "asc", "negative", "aliases.bus"];
        for path in alias_paths {
            let variable = h.variable(path).unwrap();
            assert_eq!(variable.signal(), Some(bus));
            assert_eq!(variable.kind(), "wire");
            assert!(variable.range().is_none());
            assert!(!variable.is_constant());
            assert!(variable.signedness().is_none());
            assert!(variable.logic_domain().is_none());
            assert!(variable.type_name().is_none());
            assert!(variable.enumeration().is_none());
            assert_eq!(
                h.variables()
                    .filter(|v| v.path() == variable.path())
                    .count(),
                1
            );
        }
        let mut aliases: Vec<_> = h
            .aliases(bus.slice(2, 1).unwrap())
            .unwrap()
            .map(|v| v.path().to_string())
            .collect();
        aliases.sort();
        let mut expected = alias_paths.to_vec();
        expected.sort();
        assert_eq!(aliases, expected);
        let hi = h.signal("high_alias").unwrap();
        let lo = h.signal("low_alias").unwrap();
        assert_eq!(
            h.variable("high_alias").unwrap().range(),
            Some(ondas::BitRange::new(7, 6))
        );
        assert_eq!(
            h.variable("low_alias").unwrap().range(),
            Some(ondas::BitRange::new(5, 4))
        );
        let declarations = h.variables().count();
        let signals: Vec<_> = h.signals().collect();
        assert_eq!(wave.concat(&[hi, lo]).unwrap(), bus);
        assert_eq!(wave.concat(&[bus]).unwrap(), bus);
        let nested = wave.concat(&[bus.slice(3, 2).unwrap(), bus]).unwrap();
        assert_ne!(nested, bus);
        assert_eq!(h.aliases(nested).unwrap().count(), 0);
        assert_eq!(h.variables().count(), declarations);
        assert_eq!(h.signals().collect::<Vec<_>>(), signals);
        assert_eq!(h.aliases(bus).unwrap().count(), 5);
        assert_ne!(h.signal("reversed").unwrap(), bus);
        assert_ne!(h.signal("repeated").unwrap(), bus);
        assert_ne!(h.signal("reversed").unwrap(), h.signal("repeated").unwrap());
        assert_sample(
            wave.sample(nested, ondas::Time::from_ticks(5))
                .unwrap()
                .as_ref(),
            5,
            &[(5, "1010xx")],
        );
        assert_sample(
            wave.sample(h.signal("reversed").unwrap(), ondas::Time::from_ticks(10))
                .unwrap()
                .as_ref(),
            10,
            &[(5, "xx10"), (10, "0110")],
        );
        assert_sample(
            wave.sample(h.signal("repeated").unwrap(), ondas::Time::from_ticks(5))
                .unwrap()
                .as_ref(),
            5,
            &[(5, "1010")],
        );
        let roots: Vec<_> = h
            .roots()
            .filter_map(|item| match item {
                Item::Variable(v) => Some(v.name().to_owned()),
                _ => None,
            })
            .collect();
        assert_eq!(&roots[..3], ["bits", "desc", "asc"]);
        for (name, count) in [
            ("gap", 2),
            ("overlap", 2),
            ("direction", 2),
            ("competing", 3),
            ("kind", 2),
            ("constant", 2),
            ("insufficient", 2),
            ("mixed_encoding", 2),
            ("escaped", 2),
        ] {
            let path = format!("negative_cases.{name}");
            assert!(
                matches!(h.variable(&path), Err(LookupError::Ambiguous { matches, .. }) if matches == count),
                "{path}"
            );
            assert!(
                matches!(h.signal(&path), Err(LookupError::Ambiguous { matches, .. }) if matches == count),
                "{path}"
            );
        }
        for name in [
            "literal[3:2]",
            "literal[1:0]",
            "array[10][0]",
            "array[10][1]",
            "attached[7:0]",
        ] {
            let variable = h
                .variable_path(&HierarchyPath::from_components(["negative_cases", name]))
                .unwrap();
            assert_eq!(variable.name_was_escaped(), name != "attached[7:0]");
            assert!(variable.range().is_none());
        }
        let same: Vec<_> = h.scopes().filter(|s| s.name() == "same").collect();
        assert_eq!(same.len(), 2);
        assert!(same[0] != same[1]);
        for scope in same {
            let children: Vec<_> = scope.children().collect();
            assert_eq!(children.len(), 1);
            let Item::Variable(v) = &children[0] else {
                panic!("variable")
            };
            assert_eq!(v.signal().unwrap().width(), Some(1));
            assert!(v.parent().unwrap() == scope);
        }
        public_signals_match_declarations(&wave);
    }
}

#[test]
#[ignore = "requires installed VCD fixtures; run just conformance"]
fn vcd_split_values_use_common_operations() {
    use ondas::{ScanRef, Time, TimeRange, ValueRef};
    use std::ops::ControlFlow;
    for bytes in [false, true] {
        let mut wave = open("vcd/vcd0109-split-layouts", bytes);
        let h = wave.hierarchy().clone();
        let bus = h.signal("desc").unwrap();
        let low = bus.slice(1, 0).unwrap();
        let high = bus.slice(3, 1).unwrap().slice(2, 1).unwrap();
        assert_eq!(high, bus.slice(3, 2).unwrap());
        let selected = [
            bus,
            low,
            high,
            h.signal("mixed").unwrap(),
            h.signal("never").unwrap(),
            bus,
            h.signal("bits").unwrap(),
        ];
        let histories = [BUS, LOW, HIGH, MIXED, &[], BUS, BITS];
        let times = [35, 0, 5, 30, 25, 15, 20, 10, 40, 4, 5].map(Time::from_ticks);
        let batches = wave.samples_at(&selected, &times).unwrap();
        for ((at, samples), expected_at) in batches.iter().zip(times) {
            assert_eq!(*at, expected_at);
            for (slot, sample) in samples.iter().enumerate() {
                assert_eq!(sample.signal(), selected[slot]);
                assert_sample(sample.as_ref(), at.ticks(), histories[slot]);
                assert_sample(
                    wave.sample(selected[slot], *at).unwrap().as_ref(),
                    at.ticks(),
                    histories[slot],
                );
            }
        }
        assert!(matches!(
            wave.sample(h.signal("low_alias").unwrap(), Time::from_ticks(5))
                .unwrap(),
            ondas::Sample::Missing { .. }
        ));
        assert_sample(
            wave.sample(low, Time::from_ticks(5)).unwrap().as_ref(),
            5,
            LOW,
        );
        assert!(matches!(
            wave.sample(selected[4].slice(3, 2).unwrap(), Time::from_ticks(40))
                .unwrap(),
            ondas::Sample::Missing { .. }
        ));
        for range in [
            TimeRange::all(),
            TimeRange::closed(Time::from_ticks(12), Time::from_ticks(30)),
            TimeRange::point(Time::from_ticks(30)),
        ] {
            let traces = wave.traces(&selected, range).unwrap();
            let mut rows = vec![Vec::new(); selected.len()];
            let mut initials = vec![None; selected.len()];
            let _ = wave
                .select(&selected)
                .unwrap()
                .scan_each(range, |slot, record| {
                    match record {
                        ScanRef::Change {
                            signal,
                            time,
                            value: ValueRef::Bits(bits),
                            ..
                        } => {
                            assert_eq!(signal, selected[slot]);
                            rows[slot].push((time.ticks(), bits.to_string()));
                        }
                        ScanRef::Initial {
                            signal,
                            value: ValueRef::Bits(bits),
                            changed_at,
                            ..
                        } => {
                            assert_eq!(signal, selected[slot]);
                            initials[slot] = Some((changed_at, bits.to_string()));
                        }
                        _ => panic!("bits"),
                    }
                    ControlFlow::<()>::Continue(())
                })
                .unwrap();
            for (slot, history) in histories.iter().enumerate() {
                let expected: Vec<_> = history
                    .iter()
                    .filter(|(at, _)| {
                        *at >= range.start().ticks()
                            && range.end().is_none_or(|end| *at <= end.ticks())
                    })
                    .map(|(at, text)| (*at, text.to_string()))
                    .collect();
                let initial = history
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, (at, _))| *at < range.start().ticks())
                    .map(|(index, (at, text))| {
                        (
                            (index != 0).then_some(Time::from_ticks(*at)),
                            text.to_string(),
                        )
                    });
                assert_eq!(rows[slot], expected);
                assert_eq!(initials[slot], initial);
                let trace = &traces[slot];
                assert_eq!(
                    trace
                        .changes()
                        .iter()
                        .map(|change| {
                            let ValueRef::Bits(bits) = change.value() else {
                                panic!()
                            };
                            (change.time().ticks(), bits.to_string())
                        })
                        .collect::<Vec<_>>(),
                    expected
                );
                assert_eq!(
                    trace.initial().map(|value| {
                        let ValueRef::Bits(bits) = value.value() else {
                            panic!()
                        };
                        (value.changed_at(), bits.to_string())
                    }),
                    initial
                );
            }
        }
        let mut candidates = Vec::new();
        let _ = wave
            .select(&selected)
            .unwrap()
            .query(TimeRange::all(), &[0, 0, 6], |ctx| {
                let time = ctx.time();
                candidates.push(time.ticks());
                for at in [time.ticks(), time.ticks().saturating_sub(1), time.ticks()] {
                    let mut slots = Vec::new();
                    let _ =
                        ctx.visit_samples(Time::from_ticks(at), &[2, 4, 0, 2], |slot, sample| {
                            slots.push(slot);
                            assert_sample(sample, at, histories[slot]);
                            Ok(ControlFlow::<()>::Continue(()))
                        })?;
                    assert_eq!(slots, [2, 4, 0, 2]);
                }
                Ok(ControlFlow::<()>::Continue(()))
            })
            .unwrap();
        assert!(candidates.windows(2).all(|pair| pair[0] < pair[1]));
        for at in [0, 5, 10, 15, 20, 25, 35] {
            assert!(candidates.contains(&at));
        }
        let mut calls = 0;
        assert_eq!(
            wave.scan(&selected, TimeRange::all(), |_| {
                calls += 1;
                ControlFlow::Break(9)
            })
            .unwrap(),
            ControlFlow::Break(9)
        );
        assert_eq!(calls, 1);
        let mut calls = 0;
        assert_eq!(
            wave.select(&selected)
                .unwrap()
                .query(TimeRange::all(), &[0], |_| {
                    calls += 1;
                    Ok(ControlFlow::Break(8))
                })
                .unwrap(),
            ControlFlow::Break(8)
        );
        assert_eq!(calls, 1);
    }
}
