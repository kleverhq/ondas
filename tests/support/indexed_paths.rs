use ondas::{HierarchyPath, PathComponent, Sample, Time, Value, Waveform};

pub fn check_q(wave: &mut Waveform, parent: &str, literal: bool, escaped: bool, value: &str) {
    let parent = HierarchyPath::parse(parent).unwrap();
    let component = if literal {
        PathComponent::new("u_reg[0]")
    } else {
        PathComponent::with_indices("u_reg", &[0])
    };
    let scope_path = parent.join(&component);
    let variable_path = scope_path.join("q");
    let hierarchy = wave.hierarchy().clone();
    let scope = hierarchy.scope_path(&scope_path).unwrap();
    assert_eq!(scope.name(), component.name());
    assert_eq!(scope.name_was_escaped(), escaped);
    assert_eq!(scope.path().components().last(), Some(&component));
    assert_eq!(scope.parent().unwrap().path(), parent);
    assert_eq!(
        HierarchyPath::from_components(scope.path().components()),
        scope_path
    );
    assert_eq!(
        hierarchy
            .scopes()
            .filter(|s| s.path() == scope_path)
            .count(),
        1
    );

    let variable = hierarchy.variable_path(&variable_path).unwrap();
    assert_eq!(variable.name(), "q");
    assert!(!variable.name_was_escaped());
    assert_eq!(
        HierarchyPath::from_components(variable.path().components()),
        variable_path
    );
    assert_eq!(variable.path().parent(), Some(scope_path.clone()));
    assert!(variable.parent().unwrap() == scope);
    assert!(
        scope
            .children()
            .any(|item| matches!(item, ondas::Item::Variable(v) if v == variable))
    );
    let signal = variable.signal().unwrap();
    assert_eq!(signal.width(), Some(1));
    assert_eq!(hierarchy.signal_path(&variable_path).unwrap(), signal);

    let (logical, sv) = if literal {
        (
            format!("{parent}.\"u_reg[0]\""),
            format!(r"{parent}.\u_reg[0] "),
        )
    } else {
        let indexed = format!("{parent}.u_reg[0]");
        (indexed.clone(), indexed)
    };
    assert_eq!(variable_path.to_string(), format!("{logical}.q"));
    assert_eq!(variable_path.to_verilog().unwrap(), format!("{sv}.q"));
    for scope_text in [logical, sv] {
        assert!(hierarchy.scope(&scope_text).unwrap() == scope);
        let text = format!("{scope_text}.q");
        assert!(hierarchy.variable(&text).unwrap() == variable);
        assert_eq!(hierarchy.signal(&text).unwrap(), signal);
        assert!(hierarchy.resolve_signal(&text, None).unwrap().declaration() == variable);
    }
    assert!(
        hierarchy
            .resolve_signal("q", Some(&scope_path))
            .unwrap()
            .declaration()
            == variable
    );
    for tick in [0, 4, 9] {
        let Sample::Value {
            value: Value::Bits(bits),
            ..
        } = wave.sample(signal, Time::from_ticks(tick)).unwrap()
        else {
            panic!("expected q to be recorded at {tick}");
        };
        assert_eq!(bits.as_ref().to_string(), value);
    }
}
