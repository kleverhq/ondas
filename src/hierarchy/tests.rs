use super::*;

fn fixture() -> Hierarchy {
    let scopes = vec![
        ScopeData {
            name: "tb".into(),
            parent: None,
            kind: "module".into(),
            definition_name: Some("testbench".into()),
            packing: None,
        },
        ScopeData {
            name: "dut".into(),
            parent: Some(0),
            kind: "struct".into(),
            definition_name: None,
            packing: Some(Packing::Packed),
        },
    ];
    let variables = [
        ("data", Some(0)),
        ("alias", Some(0)),
        ("data[7:0]", Some(1)),
        ("mem[0]", Some(0)),
        ("no_history", None),
        ("duplicate", Some(0)),
        ("duplicate", Some(0)),
        ("with space", Some(0)),
    ]
    .into_iter()
    .map(|(name, signal)| VariableData {
        name: name.into(),
        parent: Some(1),
        kind: "wire".into(),
        direction: Direction::Input,
        range: Some(BitRange::new(-16, 15)),
        is_constant: true,
        type_name: Some("state_t".into()),
        enumeration: Some(EnumerationData {
            name: Some("state_t".into()),
            variants: vec![("00".into(), "IDLE".into()), ("01".into(), "RUN".into())],
        }),
        signal,
    })
    .collect();
    Hierarchy::new(
        scopes,
        variables,
        vec![
            Encoding::Bits { width: 32 },
            Encoding::Bits { width: 8 },
            Encoding::Real,
        ],
    )
}

#[test]
fn hierarchy_views_aliases_and_metadata() {
    let hierarchy = fixture();
    assert_eq!(hierarchy.roots().count(), 1);
    assert_eq!(hierarchy.scopes().count(), 2);
    assert_eq!(hierarchy.variables().count(), 8);
    assert_eq!(hierarchy.signals().count(), 3);
    let tb = hierarchy.scope("tb").unwrap();
    assert_eq!(tb.name(), "tb");
    assert!(tb.parent().is_none());
    assert_eq!(tb.kind(), "module");
    assert_eq!(tb.definition_name(), Some("testbench"));
    assert!(matches!(tb.children().next(), Some(Item::Scope(_))));
    let dut = hierarchy.scope("tb.dut").unwrap();
    assert_eq!(dut.parent().unwrap().path(), tb.path());
    assert_eq!(dut.packing(), Some(Packing::Packed));
    assert_eq!(dut.children().count(), 8);
    assert!(dut.children().all(|item| matches!(item, Item::Variable(_))));
    let data = hierarchy.variable("tb.dut.data").unwrap();
    assert_eq!(data.name(), "data");
    assert_eq!(data.parent().unwrap().path(), dut.path());
    assert_eq!(data.kind(), "wire");
    assert_eq!(data.direction(), Direction::Input);
    assert!(data.is_constant());
    assert_eq!(data.type_name(), Some("state_t"));
    assert_eq!(data.range(), Some(BitRange::new(-16, 15)));
    assert_eq!(data.range().unwrap().width(), 32);
    let enumeration = data.enumeration().unwrap();
    assert_eq!(enumeration.name(), Some("state_t"));
    assert_eq!(
        enumeration
            .variants()
            .map(|v| (v.encoded, v.label))
            .collect::<Vec<_>>(),
        [("00", "IDLE"), ("01", "RUN")]
    );
    let whole = data.signal().unwrap();
    assert_eq!(hierarchy.signal("tb.dut.alias").unwrap(), whole);
    let names = |signal| {
        hierarchy
            .aliases(signal)
            .unwrap()
            .map(|v| v.name())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(whole),
        names(whole.slice(23, 8).unwrap().slice(7, 0).unwrap())
    );
    assert_eq!(names(whole).len(), 6);
    let clone = hierarchy.clone();
    assert_eq!(clone.validate(whole).unwrap(), 0);
    drop(hierarchy);
    assert_eq!(clone.variable("tb.dut.data").unwrap().name(), "data");
}

#[test]
fn exact_lookup_precedes_selectors() {
    let hierarchy = fixture();
    let data = hierarchy.signal("tb.dut.data").unwrap();
    assert_eq!(
        hierarchy.signal("tb.dut.data[7:0]").unwrap(),
        hierarchy.signal_at(1)
    );
    assert_eq!(
        hierarchy.signal("tb.dut.data[15:8]").unwrap(),
        data.slice(15, 8).unwrap()
    );
    assert_eq!(
        hierarchy.signal("tb.dut.mem[0][7:0]").unwrap(),
        data.slice(7, 0).unwrap()
    );
    assert_eq!(
        hierarchy.signal(r#"tb.dut."with space"[7:0]"#).unwrap(),
        data.slice(7, 0).unwrap()
    );
    assert_eq!(
        hierarchy
            .signal("tb.dut.\\with+punct [7:0]")
            .err()
            .map(|e| matches!(e, LookupError::NotFound { .. })),
        Some(true)
    );
    assert_eq!(
        hierarchy.signal(r#"tb.dut."data[7:0]""#).unwrap(),
        hierarchy.signal_at(1)
    );
    assert!(matches!(
        hierarchy.signal("tb.dut.data[0]"),
        Err(LookupError::NotFound { .. })
    ));
    assert!(matches!(
        hierarchy.signal("tb.dut.duplicate"),
        Err(LookupError::Ambiguous { matches: 2, .. })
    ));
    assert!(matches!(
        hierarchy.signal("tb.dut.duplicate[7:0]"),
        Err(LookupError::Ambiguous { matches: 2, .. })
    ));
    assert!(matches!(
        hierarchy.signal("tb.dut.no_history"),
        Err(LookupError::NoSignal { .. })
    ));
    assert!(matches!(
        hierarchy.signal("tb.dut.no_history[7:0]"),
        Err(LookupError::NoSignal { .. })
    ));
    assert!(matches!(
        hierarchy.signal("tb.dut.data[32:0]"),
        Err(LookupError::InvalidSlice(SliceError::OutOfBounds { .. }))
    ));
    assert!(matches!(
        hierarchy.signal("tb.dut.data[0:1]"),
        Err(LookupError::InvalidSlice(SliceError::InvalidRange { .. }))
    ));
    for text in [
        "tb.dut.data[x:0]",
        "tb.dut.data[-1:0]",
        "tb.dut.data[4294967296:0]",
    ] {
        assert!(
            matches!(hierarchy.signal(text), Err(LookupError::InvalidPath(_))),
            "{text}"
        );
    }
}

#[test]
fn signals_compose_normalize_and_validate() {
    let hierarchy = fixture();
    let data = hierarchy.signal_at(0);
    assert!(!data.is_slice());
    assert_eq!(data.slice(31, 0).unwrap(), data);
    let slice = data.slice(31, 16).unwrap().slice(7, 0).unwrap();
    assert_eq!(slice, data.slice(23, 16).unwrap());
    assert_eq!(slice.slice(7, 0).unwrap(), slice);
    assert_eq!(slice.base(), data);
    assert!(slice.is_slice());
    assert_eq!(slice.index(), 0);
    assert_eq!(slice.lsb(), 16);
    assert_eq!(slice.encoding(), Encoding::Bits { width: 8 });
    assert_eq!(slice.width(), Some(8));
    assert!(matches!(
        slice.slice(8, 0),
        Err(SliceError::OutOfBounds { width: 8, .. })
    ));
    assert_eq!(hierarchy.signal_at(2).width(), None);
    assert!(matches!(
        hierarchy.signal_at(2).slice(0, 0),
        Err(SliceError::NotBits)
    ));
    assert!(matches!(
        fixture().aliases(slice),
        Err(crate::Error::InvalidSignal { .. })
    ));
    assert!(matches!(
        fixture().validate(data),
        Err(crate::Error::InvalidSignal { .. })
    ));
    assert_eq!(hierarchy.validate(slice).unwrap(), 0);
    for invalid in [
        Signal {
            index: usize::MAX,
            ..data
        },
        Signal {
            base_encoding: Encoding::Real,
            ..data
        },
        Signal {
            encoding: Encoding::Real,
            ..data
        },
        Signal {
            encoding: Encoding::Bits { width: 0 },
            ..data
        },
        Signal {
            lsb: u32::MAX,
            ..slice
        },
        Signal { lsb: 30, ..slice },
        Signal {
            lsb: 1,
            ..hierarchy.signal_at(2)
        },
    ] {
        assert!(matches!(
            hierarchy.validate(invalid),
            Err(crate::Error::InvalidSignal { .. })
        ));
    }
}

#[test]
fn root_declarations_duplicate_scopes_and_extreme_ranges() {
    let hierarchy = Hierarchy::new(
        vec![
            ScopeData {
                name: "same".into(),
                parent: None,
                kind: "module".into(),
                definition_name: None,
                packing: None,
            },
            ScopeData {
                name: "same".into(),
                parent: None,
                kind: "module".into(),
                definition_name: None,
                packing: None,
            },
        ],
        vec![VariableData {
            name: "root".into(),
            parent: None,
            kind: "parameter".into(),
            direction: Direction::Unknown,
            range: None,
            is_constant: true,
            type_name: None,
            enumeration: None,
            signal: None,
        }],
        vec![],
    );
    assert_eq!(hierarchy.roots().count(), 3);
    assert!(matches!(
        hierarchy.scope("same"),
        Err(LookupError::Ambiguous { matches: 2, .. })
    ));
    assert!(matches!(
        hierarchy.scope(""),
        Err(LookupError::NotFound { .. })
    ));
    let root = hierarchy.variable("root").unwrap();
    assert!(root.parent().is_none());
    assert_eq!(root.path().to_string(), "root");
    assert!(root.signal().is_none());
    assert!(root.enumeration().is_none());
    assert_eq!(BitRange::new(i64::MIN, i64::MAX - 1).width(), u64::MAX);
}

#[test]
#[should_panic(expected = "range width exceeds u64")]
fn unrepresentable_range_width_panics() {
    BitRange::new(i64::MIN, i64::MAX).width();
}

#[test]
fn paths_round_trip_and_keep_exact_components() {
    let root = HierarchyPath::parse("").unwrap();
    assert!(root.is_empty());
    assert_eq!(root.len(), 0);
    assert_eq!(root.name(), None);
    assert_eq!(root.parent(), None);
    assert_eq!(root.to_string(), "");
    assert_eq!(root.to_verilog().unwrap(), "");
    let path = HierarchyPath::parse(r#"tb.\gen.blk[0] ."имя сигнала"."""#).unwrap();
    assert_eq!(
        path.components().collect::<Vec<_>>(),
        ["tb", "gen.blk[0]", "имя сигнала", ""]
    );
    assert_eq!(path.to_string(), r#"tb."gen.blk[0]"."имя сигнала"."""#);
    assert_eq!(HierarchyPath::parse(&path.to_string()).unwrap(), path);
    assert_eq!(path.parent().unwrap().join(""), path);
    assert!(path.to_verilog().is_err());
    let sv = HierarchyPath::from_components(["tb", "gen.blk[0]", "module", "a\\b", "a\"b"]);
    assert_eq!(HierarchyPath::parse(&sv.to_verilog().unwrap()).unwrap(), sv);
    for name in [
        "", ".", "..", "a.b", "[7:0]", "a b", "\\", "\"", "a/b", "é", "\0\u{1f}", "\u{7f}",
    ] {
        let path = HierarchyPath::from_components([name]);
        assert_eq!(HierarchyPath::parse(&path.to_string()).unwrap(), path);
    }
}

#[test]
fn quoted_json_escapes_and_surrogates() {
    let path = HierarchyPath::parse(r#""\"\\\/\b\f\n\r\t\u0000\u001f\uD83D\uDE00""#).unwrap();
    assert_eq!(path.name(), Some("\"\\/\u{8}\u{c}\n\r\t\0\u{1f}😀"));
    assert_eq!(path.to_string(), r#""\"\\/\b\f\n\r\t\u0000\u001f😀""#);
    assert_eq!(
        HierarchyPath::parse(r#"tb."a\u000Ab""#)
            .unwrap()
            .to_string(),
        r#"tb."a\nb""#
    );
}

#[test]
fn path_errors_have_byte_offsets() {
    for (text, offset) in [
        (".", 0),
        ("a..b", 2),
        ("a.", 2),
        ("é. bad", 3),
        ("é.a b", 4),
        ("a\\b", 1),
        ("\\", 1),
        ("\\abc", 4),
        ("\"abc", 4),
        ("\"a\"b", 3),
        ("\"é\\q\"", 4),
        ("\"a\nb\"", 2),
    ] {
        let error = HierarchyPath::parse(text).unwrap_err();
        assert_eq!(error.offset, offset, "{text:?}: {error}");
    }
    for text in [
        r#""\uD800""#,
        r#""\uDC00""#,
        r#""\uD800\u0041""#,
        r#""\u12""#,
        r#""\uXXXX""#,
        r#""\q""#,
        "\"a\\",
        "a. ",
        "a.\\ .b",
    ] {
        let error = HierarchyPath::parse(text).unwrap_err();
        assert!(error.offset <= text.len(), "{text:?}: {error}");
    }
}
