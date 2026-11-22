use super::*;

#[test]
fn child_links_preserve_identity_order_and_share_lazy_storage() {
    let mut data = Arc::try_unwrap(fixture().data).ok().unwrap();
    data.scopes = [Some(2), None, None, Some(2), Some(1)]
        .into_iter()
        .map(|parent| ScopeData {
            component: "same".into(),
            name_was_escaped: false,
            parent,
            kind: "module".into(),
            definition_name: None,
            packing: None,
            is_hidden: false,
        })
        .collect();
    for (variable, parent) in data.variables.iter_mut().zip([
        None,
        Some(2),
        Some(0),
        Some(2),
        None,
        Some(1),
        Some(3),
        Some(4),
    ]) {
        variable.parent = parent;
    }
    let hierarchy = Hierarchy::new(data.scopes, data.variables, data.encodings);
    let cloned = hierarchy.clone();
    assert_eq!(hierarchy.scopes().count(), 5);
    assert_eq!(hierarchy.variables().count(), 8);
    assert!(hierarchy.data.children.get().is_none());
    let identify = |item| match item {
        Item::Scope(scope) => ("scope", scope.index),
        Item::Variable(variable) => ("variable", variable.index),
    };
    assert_eq!(
        cloned.roots().map(identify).collect::<Vec<_>>(),
        [("scope", 1), ("scope", 2), ("variable", 0), ("variable", 4)]
    );
    assert!(hierarchy.data.children.get().is_none());
    assert_eq!(cloned.scopes().next().unwrap().children().count(), 1);
    assert!(std::ptr::eq(
        hierarchy.data.children.get().unwrap(),
        cloned.data.children.get().unwrap()
    ));
    let expected = [
        vec![("variable", 2)],
        vec![("scope", 4), ("variable", 5)],
        vec![("scope", 0), ("scope", 3), ("variable", 1), ("variable", 3)],
        vec![("variable", 6)],
        vec![("variable", 7)],
    ];
    for (scope, expected) in hierarchy.scopes().zip(expected) {
        assert_eq!(scope.children().map(identify).collect::<Vec<_>>(), expected);
        assert_eq!(scope.children().map(identify).collect::<Vec<_>>(), expected);
    }
    drop(hierarchy);
    assert_eq!(cloned.roots().count(), 4);
    let empty = Hierarchy::new(vec![], vec![], vec![]);
    assert_eq!(empty.roots().count(), 0);
    assert!(empty.data.children.get().is_none());
}

#[test]
fn enum_keys_match_whole_bits_without_guessing_or_restricting_labels() {
    let data = EnumerationData::for_encoding(
        Some("state".into()),
        vec![
            ("01XZHUWL-".into(), "état".into()),
            ("01xzhuwl-".into(), "other label".into()),
        ],
        Encoding::Bits { width: 9 },
    )
    .unwrap();
    let table = Enumeration { data: &data };
    let bits = crate::BitsRef::from_ascii(b"01XZHUWL-").unwrap();
    let labels: Vec<_> = table
        .variants()
        .filter(|v| v.encoded == bits)
        .map(|v| v.label)
        .collect();
    assert_eq!(labels, ["état", "other label"]);
    let key: crate::BitsRef<'_> = Enumeration { data: &data }
        .variants()
        .next()
        .unwrap()
        .encoded;
    assert_eq!(key.width(), 9);
    assert_eq!(key.bit(0), Some(crate::Logic::DontCare));
    assert_eq!(key.bit(8), Some(crate::Logic::Zero));
    let owned = key.to_owned();
    drop(data);
    assert_eq!(owned, bits);
    assert_eq!(owned.as_ref().to_string(), "01xzhuwl-");
    for key in ["1", "001", "0?", "é"] {
        assert!(
            EnumerationData::for_encoding(
                None,
                vec![(key.into(), "label".into())],
                Encoding::Bits { width: 2 }
            )
            .is_none()
        );
    }
    assert!(
        EnumerationData::for_encoding(
            None,
            vec![("01".into(), "label".into())],
            Encoding::Unsupported
        )
        .is_none()
    );
}

fn fixture() -> Hierarchy {
    let scopes = vec![
        ScopeData {
            component: "tb".into(),
            name_was_escaped: false,
            parent: None,
            kind: "module".into(),
            definition_name: Some("testbench".into()),
            packing: None,
            is_hidden: false,
        },
        ScopeData {
            component: "dut".into(),
            name_was_escaped: false,
            parent: Some(0),
            kind: "struct".into(),
            definition_name: None,
            packing: Some(Packing::Packed),
            is_hidden: false,
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
    .map(|(name, signal)| {
        let width = if signal == Some(1) { 8 } else { 32 };
        VariableData {
            component: name.into(),
            reader_name: None,
            name_was_escaped: false,
            parent: Some(1),
            kind: "wire".into(),
            direction: Direction::Input,
            range: Some(BitRange::new(-16, 15)),
            is_constant: true,
            type_name: Some("state_t".into()),
            signedness: None,
            logic_domain: None,
            enumeration: EnumerationData::for_encoding(
                Some("state_t".into()),
                vec![
                    ("0".repeat(width), "IDLE".into()),
                    (format!("{:0width$b}", 1), "RUN".into()),
                ],
                Encoding::Bits {
                    width: width as u32,
                },
            ),
            signal,
        }
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
fn identity_keeps_same_path_objects_and_their_children_separate() {
    let mut data = Arc::try_unwrap(fixture().data).ok().unwrap();
    assert_eq!(
        data.variables.len(),
        8,
        "collision fixture declaration count"
    );
    data.scopes = [None, None, Some(0), Some(1)]
        .into_iter()
        .map(|parent| ScopeData {
            component: if parent.is_none() { "same" } else { "leaf" }.into(),
            name_was_escaped: false,
            parent,
            kind: "module".into(),
            definition_name: None,
            packing: None,
            is_hidden: false,
        })
        .collect();
    // Identical metadata and a shared history still describe separate objects.
    for (index, variable) in data.variables.iter_mut().enumerate() {
        variable.component = "same declaration".into();
        variable.parent = Some(index / 2);
        variable.signal = (index < 4).then_some(0);
    }
    let hierarchy = Hierarchy::new(data.scopes, data.variables, data.encodings);
    let clone = hierarchy.clone();
    let scopes: Vec<_> = hierarchy.scopes().collect();
    let variables: Vec<_> = hierarchy.variables().collect();
    assert_eq!(scopes[0].path(), scopes[1].path());
    assert_eq!(scopes[2].path(), scopes[3].path());
    assert!(scopes[0] != scopes[1]);
    assert!(scopes[2] != scopes[3]);
    assert_eq!(variables[0].signal(), variables[1].signal());
    assert!(variables[0] != variables[1]);
    assert_eq!(variables[4].signal(), None);
    assert!(variables[4] != variables[5]);

    // This small consumer assigns its own tree positions using only public
    // views and equality. Neither paths nor history handles are object IDs.
    let tree: Vec<_> = clone
        .scopes()
        .map(|scope| {
            let parent = scope
                .parent()
                .map(|parent| scopes.iter().position(|s| *s == parent).unwrap());
            let mut child_scopes = Vec::new();
            let mut child_variables = Vec::new();
            for child in scope.children() {
                match child {
                    Item::Scope(child) => {
                        assert!(child.parent().as_ref() == Some(&scope));
                        child_scopes.push(scopes.iter().position(|s| *s == child).unwrap());
                    }
                    Item::Variable(child) => {
                        assert!(child.parent().as_ref() == Some(&scope));
                        child_variables.push(variables.iter().position(|v| *v == child).unwrap());
                    }
                }
            }
            (parent, child_scopes, child_variables)
        })
        .collect();
    assert_eq!(
        tree,
        [
            (None, vec![2], vec![0, 1]),
            (None, vec![3], vec![2, 3]),
            (Some(0), vec![], vec![4, 5]),
            (Some(1), vec![], vec![6, 7]),
        ]
    );
    for (variable, original) in clone.variables().zip(&variables) {
        assert!(variable == *original);
    }
    let path = HierarchyPath::from_components(["same", "same declaration"]);
    for _ in 0..2 {
        assert!(matches!(
            hierarchy.variable_path(&path),
            Err(LookupError::Ambiguous { matches: 4, .. })
        ));
    }
}

#[test]
fn repeated_and_concurrent_lookups_preserve_exact_results() {
    let hierarchy = fixture();
    std::thread::scope(|threads| {
        for _ in 0..4 {
            let cloned = hierarchy.clone();
            threads.spawn(move || {
                for _ in 0..3 {
                    assert_eq!(cloned.variable("tb.dut.data").unwrap().name(), "data");
                    assert_eq!(cloned.variable("tb.dut.alias").unwrap().name(), "alias");
                    assert!(matches!(
                        cloned.variable("tb.dut.duplicate"),
                        Err(LookupError::Ambiguous { matches: 2, .. })
                    ));
                    assert!(matches!(
                        cloned.variable("tb.dut.absent"),
                        Err(LookupError::NotFound { .. })
                    ));
                    assert_eq!(cloned.signal("tb.dut.data[7:0]").unwrap().width(), Some(8));
                    assert_eq!(cloned.signal("tb.dut.data[1:0]").unwrap().width(), Some(2));
                }
            });
        }
    });
    assert!(
        hierarchy.variable("tb.dut.data").unwrap()
            == hierarchy.clone().variable("tb.dut.data").unwrap()
    );
}

#[test]
fn hidden_flag_is_local_and_does_not_filter_hierarchy() {
    let mut hierarchy = fixture();
    Arc::get_mut(&mut hierarchy.data).unwrap().scopes[0].is_hidden = true;
    let parent = hierarchy.scope("tb").unwrap();
    let child = hierarchy.scope("tb.dut").unwrap();
    assert!(parent.is_hidden());
    assert!(!child.is_hidden());
    assert!(child.parent().unwrap().is_hidden());
    assert!(parent.children().next().is_some());
    assert!(hierarchy.variable("tb.dut.data").is_ok());
}

#[test]
fn component_lookup_matches_linear_lookup_across_duplicate_scope_paths() {
    let mut data = Arc::try_unwrap(fixture().data).ok().unwrap();
    for parent in [None, Some(2)] {
        data.scopes.push(ScopeData {
            component: if parent.is_none() { "tb" } else { "dut" }.into(),
            name_was_escaped: false,
            parent,
            kind: "module".into(),
            definition_name: None,
            packing: None,
            is_hidden: false,
        });
    }
    let variable = data.variables.last_mut().unwrap();
    variable.component = "data".into();
    variable.parent = Some(3);
    let hierarchy = Hierarchy::new(data.scopes, data.variables, data.encodings);
    let paths = hierarchy
        .variables()
        .map(|variable| variable.path())
        .chain([
            HierarchyPath::from_components([] as [&str; 0]),
            HierarchyPath::parse("tb.missing").unwrap(),
        ]);
    for path in paths {
        let expected = hierarchy
            .variables()
            .filter(|variable| variable.path() == path)
            .map(|variable| variable.index)
            .collect::<Vec<_>>();
        match hierarchy.variable_path(&path) {
            Ok(variable) => assert_eq!(expected, [variable.index]),
            Err(LookupError::NotFound { .. }) => assert!(expected.is_empty()),
            Err(LookupError::Ambiguous { matches, .. }) => {
                assert!(expected.len() > 1);
                assert_eq!(matches, expected.len());
            }
            Err(error) => panic!("unexpected lookup error: {error}"),
        }
    }
    assert_eq!(
        hierarchy
            .scopes()
            .map(|scope| scope.name())
            .collect::<Vec<_>>(),
        ["tb", "dut", "tb", "dut"]
    );
    assert_eq!(
        hierarchy
            .variables()
            .map(|variable| variable.name())
            .collect::<Vec<_>>(),
        [
            "data",
            "alias",
            "data[7:0]",
            "mem[0]",
            "no_history",
            "duplicate",
            "duplicate",
            "data"
        ]
    );
}

#[test]
fn unique_lookup_and_branching_duplicates_preserve_outcomes() {
    let hierarchy = fixture();
    let data = HierarchyPath::from_components(["tb", "dut", "data"]);
    let variable = hierarchy.variable_path(&data).unwrap();
    assert_eq!(variable.index, 0);
    assert_eq!(
        hierarchy.clone().variable_path(&data).unwrap().index,
        variable.index
    );
    assert_eq!(
        hierarchy
            .variable_path(&HierarchyPath::from_components(["tb", "dut", "data[7:0]"]))
            .unwrap()
            .index,
        2
    );
    assert_eq!(
        hierarchy
            .variable_path(&HierarchyPath::from_components(["tb", "dut", "with space"]))
            .unwrap()
            .index,
        7
    );
    assert!(matches!(
        hierarchy.variable_path(&HierarchyPath::from_components(["tb", "dut", "absent"])),
        Err(LookupError::NotFound { .. })
    ));
    assert!(matches!(
        hierarchy.variable_path(&HierarchyPath::from_components([] as [&str; 0])),
        Err(LookupError::NotFound { .. })
    ));
    assert!(matches!(
        hierarchy.variable_path(&HierarchyPath::from_components(["tb", "dut", "duplicate"])),
        Err(LookupError::Ambiguous { matches: 2, .. })
    ));
    assert!(matches!(
        hierarchy.signal_path(&HierarchyPath::from_components(["tb", "dut", "no_history"])),
        Err(LookupError::NoSignal { .. })
    ));

    // A single root variable uses the same unique path without a frontier.
    let root_variable = VariableData {
        component: "root".into(),
        reader_name: None,
        name_was_escaped: false,
        parent: None,
        kind: "wire".into(),
        direction: Direction::Input,
        range: Some(BitRange::new(0, 0)),
        is_constant: false,
        type_name: None,
        signedness: None,
        logic_domain: None,
        enumeration: None,
        signal: Some(0),
    };
    let root = Hierarchy::new(
        vec![],
        vec![root_variable],
        vec![Encoding::Bits { width: 1 }],
    );
    assert_eq!(
        root.variable_path(&HierarchyPath::from_components(["root"]))
            .unwrap()
            .index,
        0
    );

    // Two root scopes share the first component, but the target exists under
    // only one branch: the full frontier must find it instead of reporting an
    // early ambiguity.
    let mut data = Arc::try_unwrap(fixture().data).ok().unwrap();
    let first = data.scopes.len();
    for _ in 0..2 {
        data.scopes.push(ScopeData {
            component: "branch".into(),
            name_was_escaped: false,
            parent: None,
            kind: "module".into(),
            definition_name: None,
            packing: None,
            is_hidden: false,
        });
    }
    data.variables.push(VariableData {
        component: "target".into(),
        reader_name: None,
        name_was_escaped: false,
        parent: Some(first),
        kind: "wire".into(),
        direction: Direction::Input,
        range: Some(BitRange::new(0, 0)),
        is_constant: false,
        type_name: None,
        signedness: None,
        logic_domain: None,
        enumeration: None,
        signal: Some(0),
    });
    let one_branch = Hierarchy::new(data.scopes, data.variables, data.encodings);
    let path = HierarchyPath::from_components(["branch", "target"]);
    let found = one_branch.variable_path(&path).unwrap();
    assert_eq!(found.parent().unwrap().name(), "branch");
    let scope = HierarchyPath::from_components(["branch"]);
    assert!(matches!(
        one_branch.scope_path(&scope),
        Err(LookupError::Ambiguous { .. })
    ));
    assert!(
        one_branch
            .resolve_signal("target", Some(&scope))
            .unwrap()
            .declaration()
            == found
    );

    // The same path with the target under both branches is ambiguous.
    let mut data = Arc::try_unwrap(fixture().data).ok().unwrap();
    let first = data.scopes.len();
    for _ in 0..2 {
        data.scopes.push(ScopeData {
            component: "branch".into(),
            name_was_escaped: false,
            parent: None,
            kind: "module".into(),
            definition_name: None,
            packing: None,
            is_hidden: false,
        });
    }
    for parent in [first, first + 1] {
        data.variables.push(VariableData {
            component: "target".into(),
            reader_name: None,
            name_was_escaped: false,
            parent: Some(parent),
            kind: "wire".into(),
            direction: Direction::Input,
            range: Some(BitRange::new(0, 0)),
            is_constant: false,
            type_name: None,
            signedness: None,
            logic_domain: None,
            enumeration: None,
            signal: Some(0),
        });
    }
    let two_branches = Hierarchy::new(data.scopes, data.variables, data.encodings);
    assert!(matches!(
        two_branches.variable_path(&path),
        Err(LookupError::Ambiguous { matches: 2, .. })
    ));
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
    assert_eq!(data.range().unwrap().msb(), -16);
    assert_eq!(data.range().unwrap().lsb(), 15);
    let enumeration = data.enumeration().unwrap();
    assert_eq!(enumeration.name(), Some("state_t"));
    assert_eq!(
        enumeration
            .variants()
            .map(|v| (v.encoded.to_string(), v.label))
            .collect::<Vec<_>>(),
        [("0".repeat(32), "IDLE"), (format!("{:032b}", 1), "RUN")]
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
fn interpretation_belongs_to_declarations_not_shared_histories() {
    let cases = [
        (
            "signed",
            Some(0),
            Some(Signedness::Signed),
            Some(LogicDomain::NineState),
        ),
        (
            "unsigned_alias",
            Some(0),
            Some(Signedness::Unsigned),
            Some(LogicDomain::TwoState),
        ),
        (
            "four_state_alias",
            Some(0),
            None,
            Some(LogicDomain::FourState),
        ),
        ("unknown_alias", Some(0), None, None),
        ("real", Some(1), None, None),
        ("string", Some(2), None, None),
        ("event", Some(3), None, None),
        ("no_history", None, None, None),
    ];
    // Construct metadata alone: no reader exists that could load value histories.
    let hierarchy = Hierarchy::new(
        vec![],
        cases
            .iter()
            .map(|&(name, signal, signedness, logic_domain)| VariableData {
                component: name.into(),
                reader_name: None,
                name_was_escaped: false,
                parent: None,
                kind: "variable".into(),
                direction: Direction::Unknown,
                range: None,
                is_constant: false,
                type_name: None,
                signedness,
                logic_domain,
                enumeration: (signal == Some(0)).then(|| {
                    EnumerationData::for_encoding(
                        Some(name.into()),
                        vec![("00000000".into(), name.into())],
                        Encoding::Bits { width: 8 },
                    )
                    .unwrap()
                }),
                signal,
            })
            .collect(),
        vec![
            Encoding::Bits { width: 8 },
            Encoding::Real,
            Encoding::String,
            Encoding::Event,
        ],
    );
    for (name, signal, signedness, logic_domain) in cases {
        let variable = hierarchy.variable(name).unwrap();
        assert_eq!(variable.signedness(), signedness);
        assert_eq!(variable.logic_domain(), logic_domain);
        assert_eq!(
            variable.signal(),
            signal.map(|index| hierarchy.signal_at(index))
        );
        if let Some(signal) = variable.signal() {
            let whole = hierarchy.resolve_signal(name, None).unwrap();
            assert!(whole.declaration() == variable);
            assert_eq!(whole.signal(), signal);
            assert_eq!(whole.projection(), None);
            if signal.width() == Some(8) {
                let full = hierarchy
                    .resolve_signal(&format!("{name}[7:0]"), None)
                    .unwrap();
                assert!(full.declaration() == variable);
                assert_eq!(full.signal(), whole.signal());
                assert_eq!(full.projection(), Some((7, 0)));
                assert_eq!(full.declaration().signedness(), signedness);
                assert_eq!(full.declaration().logic_domain(), logic_domain);
                assert_eq!(
                    full.declaration()
                        .enumeration()
                        .unwrap()
                        .variants()
                        .next()
                        .unwrap()
                        .label,
                    name
                );
            } else {
                assert!(matches!(
                    hierarchy.resolve_signal(&format!("{name}[0:0]"), None),
                    Err(LookupError::InvalidSlice(SliceError::NotBits))
                ));
            }
        }
    }
    let signal = hierarchy.signal("signed").unwrap();
    assert_eq!(signal, hierarchy.signal("unsigned_alias").unwrap());
    assert_eq!(signal.encoding(), Encoding::Bits { width: 8 });
    assert_eq!(signal.slice(3, 0).unwrap().width(), Some(4));
    assert_eq!(hierarchy.aliases(signal).unwrap().count(), 4);
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
        hierarchy.signal(r#"tb.dut."mem[0]"[7:0]"#).unwrap(),
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
fn resolved_selectors_keep_paths_and_explicit_normalized_projections() {
    let h = fixture();
    for (selector, name, projection, width) in [
        ("tb.dut.data", "data", None, 32),
        ("tb.dut.data[31:0]", "data", Some((31, 0)), 32),
        ("tb.dut.alias[31:0]", "alias", Some((31, 0)), 32),
        ("tb.dut.data[7:0]", "data[7:0]", None, 8),
        (r#"tb.dut."data[7:0]""#, "data[7:0]", None, 8),
        (r#"tb.dut."data"[7:0]"#, "data", Some((7, 0)), 8),
        (r#"tb.dut."mem[0]""#, "mem[0]", None, 32),
        (r#"tb.dut."mem[0]"[7:0]"#, "mem[0]", Some((7, 0)), 8),
        (r#"tb.dut."with space"[0:0]"#, "with space", Some((0, 0)), 1),
    ] {
        let resolved = h.resolve_signal(selector, None).unwrap();
        assert_eq!(resolved.declaration().name(), name, "{selector}");
        assert!(
            resolved.declaration()
                == h.variable_path(&HierarchyPath::from_components(["tb", "dut", name]))
                    .unwrap()
        );
        assert_eq!(resolved.projection(), projection, "{selector}");
        assert_eq!(resolved.signal().width(), Some(width));
        assert_eq!(resolved.signal(), h.signal(selector).unwrap());
        // The HDL range is unrelated to the normalized selector bounds.
        assert_eq!(resolved.declaration().range(), Some(BitRange::new(-16, 15)));
    }
    let selections: Vec<_> = ["tb.dut.data", "tb.dut.alias[31:0]", "tb.dut.data"]
        .into_iter()
        .map(|text| h.resolve_signal(text, None).unwrap())
        .collect();
    assert_eq!(
        selections
            .iter()
            .map(|r| r.declaration().name())
            .collect::<Vec<_>>(),
        ["data", "alias", "data"]
    );
    assert!(
        selections
            .iter()
            .all(|r| r.signal() == selections[0].signal())
    );
    assert_eq!(selections[1].projection(), Some((31, 0)));
}

#[test]
fn scoped_selectors_preserve_syntax_and_component_prefixes() {
    let h = fixture();
    let scope = HierarchyPath::parse(r#""tb".\dut "#).unwrap();
    for (relative, absolute) in [
        ("data", "tb.dut.data"),
        ("data[7:0]", "tb.dut.data[7:0]"),
        (r#""data"[7:0]"#, r#"tb.dut."data"[7:0]"#),
        ("data[15:8]", "tb.dut.data[15:8]"),
        (r#""mem[0]"[7:0]"#, r#"tb.dut."mem[0]"[7:0]"#),
        (r#"\data [15:8]"#, "tb.dut.data[15:8]"),
    ] {
        let expected = h.resolve_signal(absolute, None).unwrap();
        for text in [relative, absolute] {
            let actual = h.resolve_signal(text, Some(&scope)).unwrap();
            assert!(actual.declaration() == expected.declaration(), "{text}");
            assert_eq!(actual.signal(), expected.signal(), "{text}");
            assert_eq!(actual.projection(), expected.projection(), "{text}");
        }
    }
    let empty = HierarchyPath::from_components([] as [&str; 0]);
    assert_eq!(
        h.resolve_signal("tb.dut.data", Some(&empty))
            .unwrap()
            .signal(),
        h.signal("tb.dut.data").unwrap()
    );
    for text in ["tb2.data", "dut.data", "tb.dut", r#""tb.dut".data"#] {
        let Err(LookupError::NotFound { path }) = h.resolve_signal(text, Some(&scope)) else {
            panic!("{text} must remain relative to the selected prefix");
        };
        let wanted = HierarchyPath::from_components(
            scope
                .components()
                .chain(HierarchyPath::parse(text).unwrap().components()),
        );
        assert_eq!(path, wanted);
    }
    let invalid = r#""data"[x:0]"#;
    for scope in [None, Some(&scope)] {
        assert!(matches!(
            h.resolve_signal(invalid, scope),
            Err(LookupError::InvalidPath(PathError { offset: 7, .. }))
        ));
    }
}

#[test]
fn scoped_empty_paths_cannot_select_the_prefix_declaration() {
    let mut data = Arc::try_unwrap(fixture().data).ok().unwrap();
    data.variables[0].component = "tb".into();
    data.variables[0].parent = None;
    let h = Hierarchy::new(data.scopes, data.variables, data.encodings);
    let scope = HierarchyPath::from_components(["tb"]);
    assert!(h.variable("tb").is_ok());
    assert!(h.scope("tb").is_ok());
    for text in ["", "[3:0]"] {
        assert!(
            matches!(h.resolve_signal(text, Some(&scope)), Err(LookupError::NotFound { path }) if path.is_empty())
        );
    }
    let mut data = Arc::try_unwrap(h.data).ok().unwrap();
    data.variables[1].component = "[3:0]".into();
    data.variables[1].parent = Some(0);
    let h = Hierarchy::new(data.scopes, data.variables, data.encodings);
    let literal = h.resolve_signal("[3:0]", Some(&scope)).unwrap();
    assert_eq!(
        literal.declaration().path(),
        HierarchyPath::from_components(["tb", "[3:0]"])
    );
    assert_eq!(literal.projection(), None);
}

#[test]
fn resolved_selector_errors_keep_exact_first_precedence() {
    for ambiguous in [false, true] {
        let mut data = Arc::try_unwrap(fixture().data).ok().unwrap();
        if ambiguous {
            data.variables[5].component = "data[7:0]".into();
        } else {
            data.variables[2].signal = None;
        }
        let h = Hierarchy::new(data.scopes, data.variables, data.encodings);
        let scope = HierarchyPath::parse("tb.dut").unwrap();
        let error = h.resolve_signal("data[7:0]", Some(&scope)).err().unwrap();
        if ambiguous {
            assert!(matches!(error, LookupError::Ambiguous { matches: 2, .. }));
        } else {
            assert!(matches!(error, LookupError::NoSignal { .. }));
        }
        assert_eq!(
            h.signal("tb.dut.data[7:0]").unwrap_err().to_string(),
            error.to_string()
        );
        assert_eq!(
            h.resolve_signal(r#""data"[7:0]"#, Some(&scope))
                .unwrap()
                .projection(),
            Some((7, 0))
        );
    }
    let h = fixture();
    for (text, expected) in [
        (
            "tb.dut.data[32:0]",
            SliceError::OutOfBounds {
                width: 32,
                msb: 32,
                lsb: 0,
            },
        ),
        (
            "tb.dut.data[0:1]",
            SliceError::InvalidRange { msb: 0, lsb: 1 },
        ),
    ] {
        let Err(LookupError::InvalidSlice(error)) = h.resolve_signal(text, None) else {
            panic!("{text}");
        };
        assert_eq!(error.to_string(), expected.to_string());
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
                component: "same".into(),
                parent: None,
                kind: "module".into(),
                definition_name: None,
                packing: None,
                is_hidden: false,
                name_was_escaped: false,
            },
            ScopeData {
                component: "same".into(),
                parent: None,
                kind: "module".into(),
                definition_name: None,
                packing: None,
                is_hidden: false,
                name_was_escaped: false,
            },
        ],
        vec![VariableData {
            component: "root".into(),
            reader_name: None,
            name_was_escaped: false,
            parent: None,
            kind: "parameter".into(),
            direction: Direction::Unknown,
            range: None,
            is_constant: true,
            type_name: None,
            signedness: None,
            logic_domain: None,
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
        path.components()
            .map(PathComponent::name)
            .collect::<Vec<_>>(),
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
fn indexed_components_keep_literal_names_and_numeric_indices_distinct() {
    let root = HierarchyPath::parse("top").unwrap();
    let literal = root.join("u_reg[0]");
    assert_eq!(root.join(std::borrow::Cow::Borrowed("u_reg[0]")), literal);
    assert_eq!(
        HierarchyPath::from_components([Box::<str>::from("top"), "u_reg[0]".into()]),
        literal
    );
    let indexed = root.join(PathComponent::indexed("u_reg", 0));
    assert_ne!(literal, indexed);
    assert_eq!(
        std::collections::HashSet::from([&literal, &indexed]).len(),
        2
    );
    assert_eq!(literal.name(), Some("u_reg[0]"));
    assert_eq!(indexed.name(), Some("u_reg"));
    assert_eq!(indexed.parent().as_ref(), Some(&root));
    assert_eq!(literal, HierarchyPath::parse(r#"top."u_reg[0]""#).unwrap());
    assert_eq!(literal, HierarchyPath::parse(r"top.\u_reg[0] ").unwrap());
    assert_eq!(indexed, "top.u_reg[0]".parse().unwrap());
    for component in [
        PathComponent::indexed("a.b", i64::MIN),
        PathComponent::indexed("a[0]", i64::MAX),
        PathComponent::indexed("a", -1),
        PathComponent::indexed("module", 0),
    ] {
        let path = root.join(component);
        assert_eq!(HierarchyPath::from_components(path.components()), path);
        assert_eq!(HierarchyPath::parse(&path.to_string()).unwrap(), path);
        assert_eq!(
            HierarchyPath::parse(&path.to_verilog().unwrap()).unwrap(),
            path
        );
    }
    assert_eq!(root.join("module").to_verilog().unwrap(), r"top.\module ");
    assert_eq!(HierarchyPath::parse("[0]").unwrap().name(), Some("[0]"));
    assert_eq!(
        root.join("arr[10][0]"),
        HierarchyPath::parse("top.arr[10][0]").unwrap()
    );
}

#[test]
fn path_styles_round_trip_contract_examples() {
    let bar = HierarchyPath::parse("bar").unwrap();
    let cases = [
        (
            &["bar.foo", r#"bar."foo""#, r"bar.\foo "][..],
            "bar.foo",
            "bar.foo",
            bar.join("foo"),
        ),
        (
            &[r#"bar."foo.baz""#, r"bar.\foo.baz "][..],
            r#"bar."foo.baz""#,
            r"bar.\foo.baz ",
            bar.join("foo.baz"),
        ),
        (
            &["bar.foo[0].baz"][..],
            "bar.foo[0].baz",
            "bar.foo[0].baz",
            bar.join(PathComponent::indexed("foo", 0)).join("baz"),
        ),
        (
            &[r#"bar."foo[0]".baz"#, r"bar.\foo[0] .baz"][..],
            r#"bar."foo[0]".baz"#,
            r"bar.\foo[0] .baz",
            bar.join("foo[0]").join("baz"),
        ),
    ];

    for (inputs, logical, sv, expected) in cases {
        for input in inputs {
            assert_eq!(HierarchyPath::parse(input).unwrap(), expected, "{input:?}");
            assert_eq!(
                input.parse::<HierarchyPath>().unwrap(),
                expected,
                "{input:?}"
            );
        }
        for (style, spelling) in [(PathStyle::Logical, logical), (PathStyle::Sv, sv)] {
            let output = expected.format(style).unwrap();
            assert_eq!(output, spelling, "{style:?}");
            assert_eq!(HierarchyPath::parse(&output).unwrap(), expected);
        }
        assert_eq!(expected.to_string(), logical);
        assert_eq!(expected.to_verilog().unwrap(), sv);
    }
}

#[test]
fn path_styles_handle_sv_escape_boundaries_and_errors() {
    let top = HierarchyPath::parse("top").unwrap();
    let keyword = top.join("module");
    assert_eq!(keyword.format(PathStyle::Sv).unwrap(), r"top.\module ");
    let indexed_keyword = top.join(PathComponent::indexed("module", 0));
    assert_eq!(
        indexed_keyword.format(PathStyle::Sv).unwrap(),
        r"top.\module [0]"
    );

    let unterminated = r"top.\u_reg[0].q";
    assert_eq!(
        HierarchyPath::parse(unterminated).unwrap_err().offset,
        unterminated.len()
    );

    let spaced = top.join("a b");
    assert_eq!(spaced.format(PathStyle::Logical).unwrap(), r#"top."a b""#);
    assert!(matches!(
        spaced.format(PathStyle::Sv),
        Err(PathFormatError::NotRepresentable { component }) if component == "a b"
    ));
}

#[test]
fn exact_unicode_paths_do_not_normalize_names() {
    let names = ["é", "e\u{301}"];
    let hierarchy = Hierarchy::new(
        vec![],
        names
            .iter()
            .enumerate()
            .map(|(index, name)| VariableData {
                component: (*name).into(),
                reader_name: None,
                name_was_escaped: false,
                parent: None,
                kind: "wire".into(),
                direction: Direction::Unknown,
                range: None,
                is_constant: false,
                type_name: None,
                signedness: None,
                logic_domain: None,
                enumeration: None,
                signal: Some(index),
            })
            .collect(),
        vec![Encoding::Bits { width: 1 }; 2],
    );
    let root = HierarchyPath::from_components(Vec::<String>::new());
    let mut resolved = Vec::new();
    for name in names {
        let path = root.join(name);
        assert!(root.is_empty(), "join must not mutate its source");
        assert_eq!(path.len(), 1);
        assert_eq!(path.components().len(), 1);
        assert_eq!(path.parent(), Some(root.clone()));
        let parsed: HierarchyPath = path.to_string().parse().unwrap();
        assert_eq!(parsed, path);
        let declaration = hierarchy.variable_path(&parsed).unwrap();
        assert_eq!(declaration.name(), name);
        assert_eq!(declaration.path(), path);
        resolved.push(hierarchy.signal_path(&parsed).unwrap());
        assert!(
            matches!(path.to_verilog(), Err(PathFormatError::NotRepresentable { component }) if component == name)
        );
    }
    assert_ne!(resolved[0], resolved[1]);
    assert_eq!(hierarchy.signal(r#""\u00e9""#).unwrap(), resolved[0]);
    let bad: std::result::Result<HierarchyPath, _> = "é..x".parse();
    assert_eq!(bad.unwrap_err().offset, 3);
    assert!(
        matches!(hierarchy.variable("é..x"), Err(LookupError::InvalidPath(error)) if error.offset == 3)
    );
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
