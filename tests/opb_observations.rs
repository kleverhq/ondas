#[allow(dead_code)]
#[path = "../benches/opb/consumer.rs"]
mod consumer;

use consumer::{Consumer, Verify};
use ondas::{HierarchyPath, Waveform};
use serde_json::Value;

fn waveform() -> Waveform {
    let text = br#"$scope module top $end
$var wire 64 ! wanted $end
$var wire 64 ! alias $end
$var wire 64 " other $end
$upscope $end
$enddefinitions $end
#0
b0 !
b1 "
"#;
    ondas::open_bytes_with("identity.vcd", text.to_vec().into(), "vcd-native").unwrap()
}

fn rows(bytes: Vec<u8>) -> Vec<Value> {
    String::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line.strip_prefix("OPB:").unwrap()).unwrap())
        .collect()
}

#[test]
fn lookup_observation_requires_the_returned_history_and_accepts_aliases() {
    let wave = waveform();
    let path = HierarchyPath::from_components(["top", "alias"]);
    let wanted = wave.hierarchy().signal("top.wanted").unwrap();
    let other = wave.hierarchy().signal("top.other").unwrap();
    let mut bytes = Vec::new();
    let mut out = Verify::new(&mut bytes);
    out.lookup(&wave, &path, wanted);
    out.lookup(&wave, &path, other);
    out.finish();
    let rows = rows(bytes);
    assert_eq!(rows[0]["path"], serde_json::json!(["top", "alias"]));
    assert!(rows[1]["path"].is_null());
    assert_eq!(rows[0]["width"], rows[1]["width"]);
}

#[test]
fn selection_observation_checks_order_with_equal_counts_and_widths() {
    let wave = waveform();
    let wanted = wave.hierarchy().signal("top.wanted").unwrap();
    let other = wave.hierarchy().signal("top.other").unwrap();
    let requested = [wanted, other, wanted];
    let reordered = [other, wanted, wanted];
    let mut bytes = Vec::new();
    let mut out = Verify::new(&mut bytes);
    out.selection(&requested, &requested);
    out.selection(&requested, &reordered);
    out.finish();
    let rows = rows(bytes);
    assert_eq!(rows[0]["same_order"], true);
    assert_eq!(rows[1]["same_order"], false);
    for key in ["slots", "bases", "widths"] {
        assert_eq!(rows[0][key], rows[1][key]);
    }
}

#[test]
fn inventory_groups_aliases_and_source_mapping_preserves_repeated_positions() {
    let wave = waveform();
    let wanted = wave.hierarchy().signal("top.wanted").unwrap();
    let other = wave.hierarchy().signal("top.other").unwrap();
    let signals = wave.hierarchy().signals().collect::<Vec<_>>();
    let mut bytes = Vec::new();
    let mut out = Verify::new(&mut bytes);
    out.sources(&wave, &[wanted, other, wanted]);
    out.inventory(&wave, signals.into_iter());
    out.finish();
    let rows = rows(bytes);
    assert_eq!(
        rows[0]["paths"],
        serde_json::json!([["top", "wanted"], ["top", "other"], ["top", "wanted"]])
    );
    assert_eq!(rows.iter().filter(|r| r["kind"] == "signal").count(), 3);
    let summary = rows.iter().find(|r| r["kind"] == "signals").unwrap();
    assert_eq!(summary["count"], 2);
    assert_eq!(summary["unique"], true);
    assert_eq!(summary["same_set"], true);
    let mut declarations = rows
        .iter()
        .filter(|r| r["kind"] == "signal")
        .map(|r| r["path"].clone())
        .collect::<Vec<_>>();
    declarations.sort_by_key(|p| p.to_string());
    assert_eq!(
        declarations,
        vec![
            serde_json::json!(["top", "alias"]),
            serde_json::json!(["top", "other"]),
            serde_json::json!(["top", "wanted"])
        ]
    );
}
