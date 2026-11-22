use ondas::{HierarchyPath, Item, LookupError, Scope, Time, Value, Variable};

const SOURCE: &[u8] = br##"$scope module top $end
$var wire 8 ! \literal[7:0] $end
$var wire 8 " \literal $end
$var wire 8 ! alias $end
$var wire 1 # \[0] $end
$var wire 1 $ \name/with/slash $end
$var wire 1 % \\leading $end
$var wire 8 & \duplicate[7:0] $end
$var wire 8 ' \duplicate[7:0] $end
$var wire 8 ( duplicate $end
$scope module \a.b $end
$var wire 1 ) c $end
$upscope $end
$var wire 1 * \a.b.c $end
$upscope $end
$enddefinitions $end
#0
b10100101 !
b11110000 "
1# 0$ 1%
b00000000 &
b11111111 '
b01010101 (
0) 1*
"##;

#[test]
fn views_keep_identity_across_clones_and_independent_opens() {
    fn requires_eq<T: Eq>() {}
    requires_eq::<Scope<'_>>();
    requires_eq::<Variable<'_>>();

    let wave = ondas::open_bytes("identity.vcd", SOURCE.into()).unwrap();
    let hierarchy = wave.hierarchy().clone();
    let clone = hierarchy.clone();
    let independent = ondas::open_bytes("identity.vcd", SOURCE.into()).unwrap();
    assert_eq!(hierarchy.scopes().count(), 2);
    assert_eq!(hierarchy.variables().count(), 11);
    assert_eq!(clone.scopes().count(), hierarchy.scopes().count());
    assert_eq!(clone.variables().count(), hierarchy.variables().count());
    assert_eq!(
        independent.hierarchy().scopes().count(),
        hierarchy.scopes().count()
    );
    assert_eq!(
        independent.hierarchy().variables().count(),
        hierarchy.variables().count()
    );
    for ((original, copied), foreign) in hierarchy
        .scopes()
        .zip(clone.scopes())
        .zip(independent.hierarchy().scopes())
    {
        assert_eq!(original.path(), foreign.path());
        assert!(original == copied);
        assert!(original != foreign);
        assert!(original.parent() == copied.parent());
    }
    for ((original, copied), foreign) in hierarchy
        .variables()
        .zip(clone.variables())
        .zip(independent.hierarchy().variables())
    {
        assert_eq!(original.path(), foreign.path());
        assert!(original == copied);
        assert!(original != foreign);
        assert!(original.parent() == copied.parent());
    }

    let literal = hierarchy.variable(r#"top."literal[7:0]""#).unwrap();
    let alias = clone.variable("top.alias").unwrap();
    assert!(literal != alias);
    assert_eq!(literal.signal(), alias.signal());
    assert!(
        hierarchy
            .aliases(alias.signal().unwrap())
            .unwrap()
            .any(|v| v == literal)
    );
    drop(wave);
    assert!(literal == clone.variable_path(&literal.path()).unwrap());
}

#[test]
fn exact_names_keep_declarations_and_component_boundaries() {
    let mut wave = ondas::open_bytes("names.vcd", SOURCE.into()).unwrap();
    let hierarchy = wave.hierarchy().clone();
    for (name, expected) in [
        ("literal[7:0]", "10100101"),
        ("literal", "11110000"),
        ("[0]", "1"),
        ("name/with/slash", "0"),
        (r"\leading", "1"),
        ("a.b.c", "1"),
    ] {
        let path = HierarchyPath::from_components(["top", name]);
        let variable = hierarchy.variable_path(&path).unwrap();
        assert_eq!(variable.name(), name);
        assert_eq!(variable.range(), None);
        assert!(variable == hierarchy.variable(&path.to_string()).unwrap());
        assert_eq!(HierarchyPath::parse(&path.to_string()).unwrap(), path);
        let signal = hierarchy.signal(&path.to_string()).unwrap();
        assert_eq!(Some(signal), variable.signal());
        let sample = wave.sample(signal, Time::ZERO).unwrap();
        let ondas::Sample::Value {
            value: Value::Bits(bits),
            ..
        } = sample
        else {
            panic!("expected bits at {path}");
        };
        assert_eq!(bits.as_ref().to_string(), expected);
    }

    let nested = hierarchy
        .variable_path(&HierarchyPath::from_components(["top", "a.b", "c"]))
        .unwrap();
    let literal = hierarchy
        .variable_path(&HierarchyPath::from_components(["top", "a.b.c"]))
        .unwrap();
    assert!(nested != literal);
    assert_ne!(nested.path(), literal.path());
    assert!(nested.parent() != literal.parent());
    for _ in 0..2 {
        assert!(matches!(
            hierarchy.signal("top.duplicate[7:0]"),
            Err(LookupError::Ambiguous { matches: 2, .. })
        ));
    }
    let top = hierarchy.scope("top").unwrap();
    let children: Vec<_> = top.children().collect();
    assert_eq!(children.len(), 11);
    for item in children {
        let parent = match item {
            Item::Scope(scope) => scope.parent(),
            Item::Variable(variable) => variable.parent(),
        };
        assert!(parent.as_ref() == Some(&top));
    }
}

#[test]
fn selector_results_preserve_aliases_projection_and_output_roundtrip() {
    let mut wave = ondas::open_bytes("identity.vcd", SOURCE.into()).unwrap();
    let h = wave.hierarchy().clone();
    let selectors = [
        "top.alias",
        "top.alias[7:0]",
        r#"top."literal[7:0]""#,
        r#"top."literal"[7:0]"#,
        r#"top."literal"[3:0]"#,
        "top.alias",
    ];
    let selected: Vec<ondas::ResolvedSelector<'_>> = selectors
        .iter()
        .map(|selector| h.resolve_signal(selector, None).unwrap())
        .collect();
    assert_eq!(
        selected
            .iter()
            .map(|s| s.declaration().name())
            .collect::<Vec<_>>(),
        [
            "alias",
            "alias",
            "literal[7:0]",
            "literal",
            "literal",
            "alias"
        ]
    );
    assert_eq!(
        selected.iter().map(|s| s.projection()).collect::<Vec<_>>(),
        [None, Some((7, 0)), None, Some((7, 0)), Some((3, 0)), None]
    );
    assert_eq!(selected[0].signal(), selected[1].signal());
    assert_eq!(selected[0].signal(), selected[2].signal());
    assert!(selected[0].declaration() != selected[2].declaration());
    for (selection, expected) in selected.iter().zip([
        "10100101", "10100101", "10100101", "11110000", "0000", "10100101",
    ]) {
        // Components and explicit bounds suffice; no input selector parser is needed.
        let path = selection.declaration().path();
        let emitted = match selection.projection() {
            None => path.to_string(),
            Some((msb, lsb)) => {
                let base = path
                    .components()
                    .map(|c| serde_json::to_string(c.name()).unwrap())
                    .collect::<Vec<_>>()
                    .join(".");
                format!("{base}[{msb}:{lsb}]")
            }
        };
        let again = h.resolve_signal(&emitted, None).unwrap();
        assert!(again.declaration() == selection.declaration());
        assert_eq!(again.projection(), selection.projection());
        assert_eq!(again.signal(), selection.signal());
        let ondas::Sample::Value {
            value: Value::Bits(bits),
            ..
        } = wave.sample(again.signal(), Time::ZERO).unwrap()
        else {
            panic!("expected bits");
        };
        assert_eq!(bits.as_ref().to_string(), expected);
    }
    let retained = {
        let scope = HierarchyPath::parse(r#"top.\a.b "#).unwrap();
        let selector = String::from(r#""c"[0:0]"#);
        h.resolve_signal(&selector, Some(&scope)).unwrap()
    };
    assert_eq!(
        retained.declaration().path(),
        HierarchyPath::from_components(["top", "a.b", "c"])
    );
    assert_eq!(retained.projection(), Some((0, 0)));
}

#[test]
fn public_selector_failures_keep_typed_errors_and_qualified_paths() {
    let wave = ondas::open_bytes("identity.vcd", SOURCE.into()).unwrap();
    let h = wave.hierarchy();
    let scope = HierarchyPath::parse("top").unwrap();
    for (selector, projection) in [("[0]", None), ("[0][0:0]", Some((0, 0)))] {
        let selected = h.resolve_signal(selector, Some(&scope)).unwrap();
        assert_eq!(selected.declaration().name(), "[0]");
        assert_eq!(selected.projection(), projection);
    }
    assert!(matches!(
        h.resolve_signal("duplicate[7:0]", Some(&scope)),
        Err(LookupError::Ambiguous { path, matches: 2 })
            if path == HierarchyPath::from_components(["top", "duplicate[7:0]"])
    ));
    let forced = h
        .resolve_signal(r#""duplicate"[7:0]"#, Some(&scope))
        .unwrap();
    assert_eq!(forced.declaration().name(), "duplicate");
    assert_eq!(forced.projection(), Some((7, 0)));
    assert!(matches!(
        h.resolve_signal("absent[3:0]", Some(&scope)),
        Err(LookupError::NotFound { path })
            if path == HierarchyPath::from_components(["top", "absent"])
    ));
    assert!(matches!(
        h.resolve_signal(r#""literal"[x:0]"#, Some(&scope)),
        Err(LookupError::InvalidPath(_))
    ));
    assert!(matches!(
        h.resolve_signal(r#""literal"[8:0]"#, Some(&scope)),
        Err(LookupError::InvalidSlice(ondas::SliceError::OutOfBounds {
            width: 8,
            msb: 8,
            lsb: 0
        }))
    ));
    assert!(matches!(
        h.resolve_signal(r#""literal"[0:1]"#, Some(&scope)),
        Err(LookupError::InvalidSlice(ondas::SliceError::InvalidRange {
            msb: 0,
            lsb: 1
        }))
    ));
}
