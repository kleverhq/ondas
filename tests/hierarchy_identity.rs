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
    for ((original, copied), foreign) in hierarchy
        .scopes()
        .zip(clone.scopes())
        .zip(independent.hierarchy().scopes())
    {
        assert!(original == copied);
        assert!(original != foreign);
        assert!(original.parent() == copied.parent());
    }
    for ((original, copied), foreign) in hierarchy
        .variables()
        .zip(clone.variables())
        .zip(independent.hierarchy().variables())
    {
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
    for item in top.children() {
        let parent = match item {
            Item::Scope(scope) => scope.parent(),
            Item::Variable(variable) => variable.parent(),
        };
        assert!(parent.as_ref() == Some(&top));
    }
}
