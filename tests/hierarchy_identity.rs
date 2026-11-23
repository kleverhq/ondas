use ondas::{
    HierarchyPath, Item, LookupError, PathStyle, Scope, SignalSelector, Time, Value, Variable,
};

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
    assert!(matches!(
        hierarchy.signal(r#"top."duplicate[7:0]""#),
        Err(LookupError::Ambiguous { matches: 2, .. })
    ));
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
    for (selection, (logical, sv, expected)) in selected.iter().zip([
        ("top.alias", "top.alias", "10100101"),
        ("top.alias[7:0]", "top.alias[7:0]", "10100101"),
        (r#"top."literal[7:0]""#, r"top.\literal[7:0] ", "10100101"),
        ("top.literal[7:0]", r"top.\literal [7:0]", "11110000"),
        ("top.literal[3:0]", r"top.\literal [3:0]", "0000"),
        ("top.alias", "top.alias", "10100101"),
    ]) {
        for (style, spelling) in [(PathStyle::Logical, logical), (PathStyle::Sv, sv)] {
            let emitted = selection.selector().format(style).unwrap();
            assert_eq!(emitted, spelling);
            let again = h.resolve_signal(&emitted, None).unwrap();
            assert!(again.declaration() == selection.declaration());
            assert_eq!(again.projection(), selection.projection());
            assert_eq!(again.signal(), selection.signal());
        }
        let ondas::Sample::Value {
            value: Value::Bits(bits),
            ..
        } = wave.sample(selection.signal(), Time::ZERO).unwrap()
        else {
            panic!("expected bits");
        };
        assert_eq!(bits.as_ref().to_string(), expected);
    }
    let retained = {
        let scope = HierarchyPath::parse(r#"top.\a.b "#).unwrap();
        let selector = SignalSelector::parse(r#""c"[0:0]"#).unwrap();
        assert_eq!(selector.path().len(), 1);
        h.resolve_selector(&selector, Some(&scope)).unwrap()
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
        h.resolve_signal(r#""duplicate[7:0]""#, Some(&scope)),
        Err(LookupError::Ambiguous { path, matches: 2 })
            if path == HierarchyPath::from_components(["top", "duplicate[7:0]"])
    ));
    let forced = h.resolve_signal("duplicate[7:0]", Some(&scope)).unwrap();
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

#[test]
fn selector_spelling_and_lookup_follow_the_declared_path() {
    let source = br##"$scope module top $end
$var wire 8 ! data $end
$var wire 4 " \data[3:0] $end
$var wire 4 # \absent[3:0] $end
$var wire 8 $ mem[-1] $end
$var wire 8 % \mem[-1] $end
$scope module logic[-1] $end
$var wire 1 & q $end
$upscope $end
$scope module \logic $end
$var wire 1 ' \q $end
$upscope $end
$upscope $end
$enddefinitions $end
#0 b10100101 ! b1111 " b1110 # b1001 $ b0110 % 0& 1'
"##;
    let mut wave = ondas::open_bytes("selectors.vcd", source.as_slice().into()).unwrap();
    let h = wave.hierarchy().clone();
    let scope = HierarchyPath::parse("top").unwrap();
    for (inputs, logical, sv, expected_bits) in [
        (
            &["data[3:0]", r#""data"[3:0]"#, r"\data [3:0]"][..],
            "top.data[3:0]",
            "top.data[3:0]",
            "0101",
        ),
        (
            &[r#""data[3:0]""#, r"\data[3:0] "][..],
            r#"top."data[3:0]""#,
            r"top.\data[3:0] ",
            "1111",
        ),
        (
            &["mem[-1][3:0]"][..],
            "top.mem[-1][3:0]",
            "top.mem[-1][3:0]",
            "1001",
        ),
        (
            &[r#""mem[-1]"[3:0]"#][..],
            r#"top."mem[-1]"[3:0]"#,
            r"top.\mem[-1] [3:0]",
            "0110",
        ),
        (
            &["logic[-1].q"][..],
            "top.logic[-1].q",
            "top.logic[-1].q",
            "0",
        ),
        (
            &["logic.q[0:0]"][..],
            "top.logic.q[0:0]",
            r"top.\logic .\q [0:0]",
            "1",
        ),
    ] {
        for input in inputs {
            let selector = SignalSelector::parse(input).unwrap();
            let resolved = h.resolve_selector(&selector, Some(&scope)).unwrap();
            let from_text = h.resolve_signal(input, Some(&scope)).unwrap();
            assert!(resolved.declaration() == from_text.declaration());
            assert_eq!(resolved.projection(), from_text.projection());
            assert_eq!(resolved.signal(), from_text.signal());
            for (style, spelling) in [(PathStyle::Logical, logical), (PathStyle::Sv, sv)] {
                let emitted = resolved.selector().format(style).unwrap();
                assert_eq!(emitted, spelling);
                let again = h.resolve_signal(&emitted, None).unwrap();
                assert!(again.declaration() == resolved.declaration());
                assert_eq!(again.projection(), resolved.projection());
            }
            let ondas::Sample::Value {
                value: Value::Bits(bits),
                ..
            } = wave.sample(resolved.signal(), Time::ZERO).unwrap()
            else {
                panic!("expected bits for {input}");
            };
            assert_eq!(bits.as_ref().to_string(), expected_bits);
        }
    }
    assert!(matches!(
        h.resolve_signal("absent[3:0]", Some(&scope)),
        Err(LookupError::NotFound { path }) if path == scope.join("absent")
    ));
    assert!(matches!(
        h.variable("top.data[3:0]"),
        Err(LookupError::InvalidPath(_))
    ));
    assert!(matches!(
        h.scope("top.data[3:0]"),
        Err(LookupError::InvalidPath(_))
    ));
    let declaration = h.variable("top.logic.q").unwrap();
    let sv = r"top.\logic .\q ";
    assert_eq!(declaration.path().format(PathStyle::Sv).unwrap(), sv);
    assert!(declaration == h.variable(sv).unwrap());
    assert_eq!(
        declaration
            .parent()
            .unwrap()
            .path()
            .format(PathStyle::Sv)
            .unwrap(),
        r"top.\logic "
    );
}
