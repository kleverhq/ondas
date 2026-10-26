//! Logical component names must not require reader-specific lookup syntax.
use ondas::{Hierarchy, HierarchyPath, Item, Sample, Time, Value, Variable};

#[path = "support/fsdb_compatibility.rs"]
mod fsdb_compatibility;

fn variable<'h>(hierarchy: &'h Hierarchy, components: &[&str]) -> Variable<'h> {
    let path = HierarchyPath::from_components(components.iter().copied());
    let declaration = hierarchy.variable_path(&path).unwrap();
    assert_eq!(declaration.name(), *components.last().unwrap());
    assert_eq!(declaration.path(), path);
    let parsed = HierarchyPath::parse(&path.to_string()).unwrap();
    assert_eq!(parsed, path);
    assert!(hierarchy.variable(&path.to_string()).unwrap() == declaration);
    assert!(hierarchy.variable_path(&parsed).unwrap() == declaration);
    assert_eq!(
        hierarchy.signal_path(&path).unwrap(),
        declaration.signal().unwrap()
    );
    let parent = declaration.parent().unwrap();
    assert_eq!(parent.path(), path.parent().unwrap());
    assert!(
        parent
            .children()
            .any(|child| matches!(child, Item::Variable(v) if v == declaration))
    );
    declaration
}

#[test]
fn vcd_logical_names_preserve_literal_characters() {
    let text = br#"$scope module top $end
$var wire 8 ! \literal.dot $end
$var wire 8 " \literal[7:0] $end
$var wire 8 # \literal $end
$var wire 8 $ \\leading $end
$var wire 8 % \a\b $end
$upscope $end
$enddefinitions $end
#0
b00000001 !
b00000010 "
b00000011 #
b00000100 $
b00000101 %
"#;
    let wave = ondas::open_bytes_with("names.vcd", text.to_vec().into(), "vcd-native").unwrap();
    for name in [
        "literal.dot",
        "literal[7:0]",
        "literal",
        r"\leading",
        r"a\b",
    ] {
        assert!(variable(wave.hierarchy(), &["top", name]).name_was_escaped());
        assert!(
            wave.hierarchy()
                .variable_path(&HierarchyPath::from_components(["top", name]))
                .unwrap()
                .range()
                .is_none()
        );
    }
    assert_ne!(
        wave.hierarchy().signal(r#"top."literal[7:0]""#).unwrap(),
        wave.hierarchy().signal("top.literal").unwrap()
    );
}

#[test]
fn vcd_equal_logical_scope_paths_do_not_merge_source_declarations() {
    let text = br#"$scope module top $end
$scope module \same $end
$var wire 1 ! escaped_child $end
$upscope $end
$scope module same $end
$var wire 1 " plain_child $end
$upscope $end
$upscope $end
$enddefinitions $end
#0
0!
1"
"#;
    let wave = ondas::open_bytes_with("scopes.vcd", text.to_vec().into(), "vcd-native").unwrap();
    same_path_scopes(wave.hierarchy());
}

fn same_path_scopes(hierarchy: &Hierarchy) {
    let path = HierarchyPath::from_components(["top", "same"]);
    let scopes: Vec<_> = hierarchy.scopes().filter(|s| s.path() == path).collect();
    assert_eq!(scopes.len(), 2);
    assert!(scopes[0] != scopes[1]);
    assert!(matches!(
        hierarchy.scope_path(&path),
        Err(ondas::LookupError::Ambiguous { matches: 2, .. })
    ));
    for (scope, child) in scopes.iter().zip(["escaped_child", "plain_child"]) {
        let children: Vec<_> = scope.children().collect();
        assert_eq!(children.len(), 1);
        let Item::Variable(v) = &children[0] else {
            panic!("expected child declaration")
        };
        assert_eq!(v.name(), child);
        assert!(v.parent().unwrap() == *scope);
        assert_eq!(scope.name_was_escaped(), child == "escaped_child");
    }
}

fn fixture(format: &str, name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(format)
        .join(name)
        .join(format!("waveform.{format}"))
}

#[test]
#[ignore = "requires locked FST fixtures; run just conformance"]
fn fst_logical_names() {
    let wave = ondas::open_with(fixture("fst", "fst0087-sparse-dense-active"), "fst-lib").unwrap();
    for name in ["plain", "foo.bar", "foo/bar"] {
        assert!(variable(wave.hierarchy(), &["top", name]).name_was_escaped());
    }
}

#[cfg(feature = "wlf-lib")]
#[test]
#[ignore = "requires Questa and installed WLF fixtures"]
fn wlf_logical_names() {
    let wave = ondas::open_with(fixture("wlf", "wlf0072-strings-and-names"), "wlf-lib").unwrap();
    assert!(variable(wave.hierarchy(), &["top", "name.with.dot"]).name_was_escaped());
    let declaration = wave.hierarchy().variable(r#"top."name.with.dot""#).unwrap();
    assert_eq!(declaration.reader_name(), Some("\\name.with.dot "));
    assert_eq!(
        declaration.range().map(|r| (r.msb(), r.lsb())),
        Some((7, 0))
    );
}

#[cfg(feature = "shm-lib")]
#[test]
#[ignore = "requires Xcelium and installed SHM fixtures"]
fn shm_logical_names() {
    let wave = ondas::open_with(fixture("shm", "shm0085-typed-records"), "shm-lib").unwrap();
    assert!(!variable(wave.hierarchy(), &["top", "literal.dot"]).name_was_escaped());
}

#[cfg(feature = "fsdb-lib")]
#[test]
#[ignore = "requires Verdi and installed FSDB fixtures"]
fn fsdb_logical_names() {
    let wave = ondas::open_with(fixture("fsdb", "fsdb0033-sti-dac"), "fsdb-lib").unwrap();
    let components = ["testfixture2", "u_rtl", "data_in_reg[7]"];
    let path = HierarchyPath::from_components(components);
    let scope = wave.hierarchy().scope_path(&path).unwrap();
    assert_eq!(scope.name(), "data_in_reg[7]");
    assert!(scope.name_was_escaped());
    assert!(wave.hierarchy().scope(&path.to_string()).unwrap() == scope);
    assert!(
        scope
            .parent()
            .unwrap()
            .children()
            .any(|child| matches!(child, Item::Scope(s) if s == scope))
    );
    assert!(scope.children().count() > 0);
}

// All producers record the same HDL declarations. These expected components
// and values come from the authored source, independently of SDK spelling.
fn common_logical_names(path: std::path::PathBuf, backend: &str) {
    let Some(mut wave) = fsdb_compatibility::open(ondas::open_with(&path, backend), &path) else {
        return;
    };
    let hierarchy = wave.hierarchy().clone();
    let mut cases = Vec::new();
    for (index, name) in [
        "regular",
        "plain",
        "with.dot",
        "with/slash",
        "literal[7:0]",
        "literal",
        r"\leading",
        r"a\b",
        "bus.dot",
        "flag[0:0]",
    ]
    .into_iter()
    .enumerate()
    {
        cases.push((
            vec!["top", name],
            [0x10 + index, 0x50 + index, 0xa0 + index],
            8,
        ));
    }
    for scope in [
        "scope.with.dot",
        r"\scope.leading",
        r"scope\internal",
        "plain_scope",
    ] {
        cases.push((vec!["top", scope, "leaf[0]"], [0x1a, 0x5a, 0xaa], 8));
    }
    cases.push((vec!["top", "scalar[0:0]"], [0, 1, 0], 1));
    cases.push((vec!["top", "scalar"], [1, 0, 1], 1));
    for (components, values, width) in cases {
        let declaration = variable(&hierarchy, &components);
        let range = declaration.range().map(|r| (r.msb(), r.lsb()));
        if width == 8 {
            assert_eq!(range, Some((7, 0)));
        } else if declaration.name() == "scalar[0:0]" {
            assert_eq!(range, None, "literal suffix is not a declared range");
        } else {
            // Some native APIs omit the explicit single-bit declaration bounds.
            assert!(range.is_none() || range == Some((0, 0)));
        }
        let signal = declaration.signal().unwrap();
        assert_eq!(signal.width(), Some(width as u32));
        for (tick, expected) in [0, 4, 9].into_iter().zip(values) {
            let Sample::Value {
                value: Value::Bits(bits),
                ..
            } = wave.sample(signal, Time::from_ticks(tick)).unwrap()
            else {
                panic!("expected initialized bits for {components:?}");
            };
            assert_eq!(
                bits.as_ref().to_string(),
                format!("{expected:0width$b}"),
                "{components:?} at {tick}"
            );
        }
    }
}

#[test]
#[ignore = "requires installed logical-name fixtures; run just conformance"]
fn vcd_common_logical_names() {
    common_logical_names(fixture("vcd", "vcd0099-logical-names"), "vcd-native");
}

#[test]
#[ignore = "requires installed logical-name fixtures; run just conformance"]
fn fst_common_logical_names() {
    common_logical_names(fixture("fst", "fst0088-logical-names"), "fst-lib");
}

#[cfg(feature = "fsdb-lib")]
#[test]
#[ignore = "requires Verdi and installed logical-name fixtures"]
fn fsdb_common_logical_names() {
    common_logical_names(fixture("fsdb", "fsdb0056-logical-names"), "fsdb-lib");
}

#[cfg(feature = "shm-lib")]
#[test]
#[ignore = "requires Xcelium and installed logical-name fixtures"]
fn shm_common_logical_names() {
    common_logical_names(fixture("shm", "shm0092-logical-names"), "shm-lib");
}

#[cfg(feature = "wlf-lib")]
#[test]
#[ignore = "requires Questa and installed logical-name fixtures"]
fn wlf_common_logical_names() {
    common_logical_names(fixture("wlf", "wlf0082-logical-names"), "wlf-lib");
}

fn same_path_aliases(path: std::path::PathBuf, backend: &str) {
    let mut wave = ondas::open_with(path, backend).unwrap();
    let hierarchy = wave.hierarchy().clone();
    let path = HierarchyPath::from_components(["top", "same"]);
    let declarations: Vec<_> = hierarchy.variables().filter(|v| v.path() == path).collect();
    assert_eq!(declarations.len(), 2);
    assert!(declarations[0] != declarations[1]);
    assert_eq!(declarations[0].signal(), declarations[1].signal());
    assert!(declarations[0].parent() == declarations[1].parent());
    assert!(declarations.iter().all(|v| v.name() == "same"));
    assert_eq!(
        declarations.iter().filter(|v| v.name_was_escaped()).count(),
        1
    );
    assert!(matches!(
        hierarchy.variable_path(&path),
        Err(ondas::LookupError::Ambiguous { matches: 2, .. })
    ));
    assert!(matches!(
        hierarchy.signal("top.same"),
        Err(ondas::LookupError::Ambiguous { matches: 2, .. })
    ));
    let parent = declarations[0].parent().unwrap();
    let children: Vec<_> = parent
        .children()
        .filter_map(|child| match child {
            Item::Variable(v) if v.path() == path => Some(v),
            _ => None,
        })
        .collect();
    assert_eq!(children.len(), 2);
    assert!(declarations.iter().all(|v| children.contains(v)));
    let signal = declarations[0].signal().unwrap();
    for (tick, expected) in [(0, "00000001"), (4, "00000010"), (9, "00000011")] {
        let Sample::Value {
            value: Value::Bits(bits),
            ..
        } = wave.sample(signal, Time::from_ticks(tick)).unwrap()
        else {
            panic!("expected shared history");
        };
        assert_eq!(bits.as_ref().to_string(), expected);
    }
}

#[test]
#[ignore = "requires installed same-path alias fixtures; run just conformance"]
fn vcd_same_path_aliases_remain_distinct_declarations() {
    same_path_aliases(fixture("vcd", "vcd0100-same-path-aliases"), "vcd-native");
}

#[cfg(feature = "fsdb-lib")]
#[test]
#[ignore = "requires Verdi and installed same-path alias fixtures"]
fn fsdb_same_path_aliases_remain_distinct_declarations() {
    let path = fixture("fsdb", "fsdb0057-same-path-aliases");
    let Some(wave) = fsdb_compatibility::open(ondas::open_with(&path, "fsdb-lib"), &path) else {
        return;
    };
    same_path_aliases(path, "fsdb-lib");
    let mut spellings: Vec<_> = wave
        .hierarchy()
        .variables()
        .map(|v| {
            assert_eq!(v.name_was_escaped(), v.reader_name() == Some(r"\same"));
            v.reader_name().unwrap()
        })
        .collect();
    spellings.sort_unstable();
    assert_eq!(spellings, [r"\same", "same"]);
}

#[test]
#[ignore = "requires installed scope-collision fixtures; run just conformance"]
fn vcd_same_path_scopes_keep_direct_children() {
    let wave = ondas::open_with(fixture("vcd", "vcd0101-scope-collisions"), "vcd-native").unwrap();
    same_path_scopes(wave.hierarchy());
}

#[test]
#[ignore = "requires installed scope-collision fixtures; run just conformance"]
fn fst_same_path_scopes_keep_direct_children() {
    let wave = ondas::open_with(fixture("fst", "fst0089-scope-collisions"), "fst-lib").unwrap();
    same_path_scopes(wave.hierarchy());
}
