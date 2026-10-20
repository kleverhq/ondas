use super::*;

#[test]
#[ignore = "requires Xcelium and installed fixtures"]
fn shm_enum_nine_state_keys_match_values() {
    let fixture = load_fixture(&root(), "shm/shm0086-nine-state");
    let mut wave = ondas::open_with(&fixture.path, "shm-lib").unwrap();
    let hierarchy = wave.hierarchy().clone();
    let variable = hierarchy
        .variable_path(&HierarchyPath::from_components(["top(test)", "value"]))
        .unwrap();
    let table = variable.enumeration().unwrap();
    assert_eq!(
        table
            .variants()
            .map(|v| (v.encoded.to_string(), v.label))
            .collect::<Vec<_>>(),
        [
            ("u", "U"),
            ("x", "X"),
            ("0", "0"),
            ("1", "1"),
            ("z", "Z"),
            ("w", "W"),
            ("l", "L"),
            ("h", "H"),
            ("-", "-")
        ]
        .into_iter()
        .map(|(key, label)| (key.to_owned(), label))
        .collect::<Vec<_>>()
    );
    let history = &fixture.oracle["signals"]["signal-1"];
    for (variant, change) in table
        .variants()
        .zip(list(&history["windows"][0], "changes"))
    {
        let ondas::Sample::Value {
            value: ondas::Value::Bits(bits),
            ..
        } = wave
            .sample(
                variable.signal().unwrap(),
                Time::from_ticks(tick(&change["time"])),
            )
            .unwrap()
        else {
            panic!("expected nine-state bits")
        };
        assert_eq!(
            bits.as_ref().to_string(),
            change["value"]["bits"].as_str().unwrap()
        );
        assert!(variant.encoded.iter_msb().eq(bits.as_ref().iter_msb()));
    }
}

#[test]
#[ignore = "requires Xcelium and installed fixtures"]
fn shm_enum_tables_match_whole_values() {
    let (mut tables, mut variants, mut unlabelled) = (0, 0, 0);
    for id in ["shm0093-enum-core", "shm0094-enum-xz"] {
        let name = format!("shm/{id}");
        let fixture = load_fixture(&root(), &name);
        // Independent native tables and complete recorded windows are in the
        // pinned oracle. Check the ordinary reader operations as well as keys.
        assert!(pool_queries(&fixture, false, &name));
        let mut wave = ondas::open_with(&fixture.path, "shm-lib").unwrap();
        let hierarchy = wave.hierarchy().clone();
        for declaration in list(&fixture.oracle["hierarchy"], "variables") {
            let Some(expected) = declaration.get("enumeration").filter(|v| !v.is_null()) else {
                continue;
            };
            let variable = hierarchy
                .variable_path(&path(&declaration["path"]))
                .unwrap();
            let signal = variable.signal().unwrap();
            let table = variable.enumeration().unwrap();
            tables += 1;
            variants += table.variants().count();
            for variant in table.variants() {
                assert_eq!(variant.encoded.width(), signal.width().unwrap());
            }
            let history = &fixture.oracle["signals"][declaration["signal"].as_str().unwrap()];
            for change in list(&history["windows"][0], "changes") {
                let ondas::Sample::Value {
                    value: ondas::Value::Bits(bits),
                    ..
                } = wave
                    .sample(signal, Time::from_ticks(tick(&change["time"])))
                    .unwrap()
                else {
                    panic!("expected enum bits")
                };
                let expected_bits = change["value"]["bits"].as_str().unwrap();
                assert_eq!(bits.as_ref().to_string(), expected_bits);
                let labels = table
                    .variants()
                    .filter(|v| v.encoded.iter_msb().eq(bits.as_ref().iter_msb()))
                    .map(|v| v.label)
                    .collect::<Vec<_>>();
                let expected_labels = list(expected, "variants")
                    .iter()
                    .filter(|v| v["encoded"].as_str().unwrap() == expected_bits)
                    .map(|v| v["label"].as_str().unwrap())
                    .collect::<Vec<_>>();
                unlabelled += usize::from(expected_labels.is_empty());
                assert_eq!(labels, expected_labels, "{name}: {}", variable.path());
            }
        }
        let control = if id == "shm0093-enum-core" {
            "top.unsigned129_bits"
        } else {
            "top.logic65xz_bits"
        };
        assert!(hierarchy.variable(control).unwrap().enumeration().is_none());
        // Closing the native owner must not invalidate borrowed hierarchy keys.
        drop(wave);
        for declaration in list(&fixture.oracle["hierarchy"], "variables") {
            let Some(expected) = declaration.get("enumeration").filter(|v| !v.is_null()) else {
                continue;
            };
            let variable = hierarchy
                .variable_path(&path(&declaration["path"]))
                .unwrap();
            let table = variable.enumeration().expect("retained enum table");
            let retained = table
                .variants()
                .map(|v| {
                    assert_eq!(
                        v.encoded.width(),
                        variable.signal().unwrap().width().unwrap()
                    );
                    json!({"encoded": v.encoded.to_string(), "label": v.label})
                })
                .collect::<Vec<_>>();
            assert_eq!(
                json!(retained),
                expected["variants"],
                "{name}: retained enum"
            );
        }
    }
    assert_eq!((tables, variants), (25, 99));
    assert!(
        unlabelled > 0,
        "exercise queryable values with no enum label"
    );
}
