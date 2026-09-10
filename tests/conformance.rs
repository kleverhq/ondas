//! Full-pool native conformance plus focused FST public-API regressions.
//! The independent oracle is sparse: only listed declarations and observations
//! are assertions. Every discovered FST/VCD runs in file and bytes modes.
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use ondas::{
    Encoding, Error, Format, HierarchyPath, SampleRef, ScanRef, Signal, Time, TimeRange, TimeUnit,
    Trace, ValueRef, Waveform,
};
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};

const PROVIDER: &str = "kleverhq.ondas-fixtures";
const CASES: [&str; 7] = [
    "fst0041-counter",
    "fst0035-tb-complex-types-icarus-fst-waves",
    "fst0061-shortstring",
    "fst0032-systemc-components-waves-ace-ace-ace-ace-example-ace-axi-test",
    "fst0033-systemc-components-waves-axi4-tlm-pin-tlm-axi4-tlm-pin-tlm-example-axi4-tlm-pin-tlm",
    "fst0046-overlay-tb-reduced-issue-21",
    "fst0048-foo",
];

struct Fixture {
    path: PathBuf,
    oracle: Json,
    format: Format,
}

impl Fixture {
    fn backend(&self) -> &'static str {
        match self.format {
            Format::Fst => "fst-native",
            Format::Vcd => "vcd-native",
            _ => unreachable!(),
        }
    }

    fn logical_name(&self) -> &'static str {
        match self.format {
            Format::Fst => "conformance.fst",
            Format::Vcd => "conformance.vcd",
            _ => unreachable!(),
        }
    }
}

fn tick(value: &Json) -> u64 {
    let text = value
        .as_str()
        .expect("oracle tick must be a decimal string");
    let number: u64 = text.parse().expect("oracle tick must fit u64");
    assert_eq!(number.to_string(), text, "noncanonical oracle tick");
    number
}

fn list<'a>(value: &'a Json, key: &str) -> &'a [Json] {
    value
        .get(key)
        .map_or(&[], |v| v.as_array().expect("oracle list"))
}

fn fields(value: &Json, allowed: &[&str]) {
    for key in value.as_object().expect("oracle object").keys() {
        assert!(
            allowed.contains(&key.as_str()),
            "unsupported oracle field {key}: {value}"
        );
    }
}

fn path(value: &Json) -> HierarchyPath {
    HierarchyPath::from_components(
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap()),
    )
}

// Normalize only equality, never numeric text or string contents. NaN payloads
// are deliberately unconstrained; signed zero and all other reals stay bitwise.
fn normalized(value: &Json) -> Json {
    if let Some(bits) = value.get("real_bits") {
        let bits = u64::from_str_radix(bits.as_str().unwrap(), 16).unwrap();
        if f64::from_bits(bits).is_nan() {
            return json!({"real_bits": "7ff8000000000000"});
        }
    }
    value.clone()
}

fn value_json(value: ValueRef<'_>) -> Json {
    match value {
        ValueRef::Bits(bits) => json!({"bits": bits.to_string()}),
        ValueRef::Real(real) => {
            normalized(&json!({"real_bits": format!("{:016x}", real.to_bits())}))
        }
        ValueRef::String(text) => json!({"string": text}),
        ValueRef::Event => json!({"event": true}),
        _ => panic!("unexpected public value {value:?}"),
    }
}

fn validate_value(value: &Json, encoding: &Json) {
    assert_eq!(value.as_object().unwrap().len(), 1, "oracle value variant");
    match encoding["kind"].as_str().unwrap() {
        "bits" => {
            let text = value["bits"].as_str().unwrap();
            assert_eq!(text.len() as u64, encoding["width"].as_u64().unwrap());
            assert!(text.bytes().all(|b| b"01xzhuwl-".contains(&b)));
        }
        "real" => {
            let text = value["real_bits"].as_str().unwrap();
            assert_eq!(text.len(), 16);
            assert!(
                text.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            );
        }
        "string" => {
            value["string"].as_str().unwrap();
        }
        "event" => assert_eq!(value["event"], true),
        kind => panic!("unexpected selected oracle encoding {kind}"),
    }
}

fn validate_state(state: &Json, encoding: &Json, bound: u64, strict: bool) {
    fields(state, &["value", "changed_at"]);
    assert_ne!(encoding["kind"], "event");
    validate_value(&state["value"], encoding);
    if let Some(changed) = state.get("changed_at").filter(|v| !v.is_null()) {
        let changed = tick(changed);
        assert!(if strict {
            changed < bound
        } else {
            changed <= bound
        });
    }
}

fn validate_oracle(oracle: &Json, positive: bool) {
    fields(
        oracle,
        &["schema", "open", "metadata", "hierarchy", "signals"],
    );
    assert_eq!(oracle["schema"], 1, "oracle schema");
    if !positive {
        assert_eq!(
            oracle["open"],
            json!({"result": "error", "kind": "malformed"})
        );
        assert_eq!(
            oracle.as_object().unwrap().len(),
            2,
            "opening error has observations"
        );
        return;
    }
    assert_eq!(oracle["open"], json!({"result": "ok"}));
    let metadata = &oracle["metadata"];
    fields(
        metadata,
        &["timescale", "time_span", "writer", "date", "comments"],
    );
    if let Some(scale) = metadata.get("timescale").filter(|v| !v.is_null()) {
        fields(scale, &["factor", "unit"]);
        assert!((1..=u32::MAX as u64).contains(&scale["factor"].as_u64().unwrap()));
        assert!(
            ["s", "ms", "us", "ns", "ps", "fs", "as", "zs"]
                .contains(&scale["unit"].as_str().unwrap())
        );
    }
    if let Some(span) = metadata.get("time_span").filter(|v| !v.is_null()) {
        fields(span, &["first", "last"]);
        assert!(tick(&span["first"]) <= tick(&span["last"]));
    }
    let hierarchy = &oracle["hierarchy"];
    fields(hierarchy, &["scopes", "variables"]);
    let signals = oracle["signals"]
        .as_object()
        .expect("positive oracle signals");
    assert!(!signals.is_empty(), "empty positive oracle");
    let mut referenced = BTreeSet::new();
    for (key, allowed) in [
        ("scopes", &["path", "kind", "definition_name"][..]),
        (
            "variables",
            &[
                "path",
                "kind",
                "direction",
                "range",
                "type_name",
                "is_constant",
                "signal",
            ][..],
        ),
    ] {
        let mut paths = HashSet::new();
        for item in list(hierarchy, key) {
            fields(item, allowed);
            let components = item["path"].as_array().unwrap();
            assert!(!components.is_empty());
            let components: Vec<_> = components.iter().map(|c| c.as_str().unwrap()).collect();
            assert!(paths.insert(components), "duplicate {key} path");
            if let Some(range) = item.get("range").filter(|v| !v.is_null()) {
                fields(range, &["msb", "lsb"]);
                range["msb"].as_i64().unwrap();
                range["lsb"].as_i64().unwrap();
            }
            if let Some(id) = item.get("signal").and_then(Json::as_str) {
                assert!(signals.contains_key(id), "unresolved oracle signal {id}");
                referenced.insert(id);
            }
        }
    }
    assert_eq!(referenced, signals.keys().map(String::as_str).collect());
    for (id, signal) in signals {
        assert!(!id.is_empty());
        fields(signal, &["encoding", "samples", "windows"]);
        let encoding = &signal["encoding"];
        fields(encoding, &["kind", "width"]);
        if encoding["kind"] == "bits" {
            assert!((1..=u32::MAX as u64).contains(&encoding["width"].as_u64().unwrap()));
        } else {
            assert_eq!(encoding.as_object().unwrap().len(), 1);
            assert!(["real", "string", "event"].contains(&encoding["kind"].as_str().unwrap()));
        }
        for sample in list(signal, "samples") {
            fields(
                sample,
                &["time", "value", "changed_at", "missing", "occurrences"],
            );
            let time = tick(&sample["time"]);
            if sample.get("value").is_some() {
                let mut state = sample.clone();
                state.as_object_mut().unwrap().remove("time");
                validate_state(&state, encoding, time, false);
            } else if sample.get("occurrences").is_some() {
                assert_eq!(encoding["kind"], "event");
                tick(&sample["occurrences"]);
            } else {
                assert_eq!(sample["missing"], true);
                assert_ne!(encoding["kind"], "event");
            }
        }
        for window in list(signal, "windows") {
            fields(window, &["start", "end", "initial", "changes"]);
            let (start, end) = (tick(&window["start"]), tick(&window["end"]));
            let initial = window.get("initial").expect("window initial required");
            if !initial.is_null() {
                validate_state(initial, encoding, start, true);
            }
            if encoding["kind"] == "event" {
                assert!(initial.is_null());
            }
            let mut previous = start;
            for change in window["changes"].as_array().unwrap() {
                fields(change, &["time", "value"]);
                let time = tick(&change["time"]);
                assert!(
                    previous <= time && time <= end,
                    "{id}: unordered/out-of-window change"
                );
                previous = time;
                validate_value(&change["value"], encoding);
            }
            if start > end {
                assert!(initial.is_null() && list(window, "changes").is_empty());
            }
        }
    }
}

fn provider() -> PathBuf {
    let lock: toml::Value =
        toml::from_str(include_str!("../fixtures.lock.toml")).expect("fixture lock TOML");
    let provider_version = lock["providers"][PROVIDER]
        .as_str()
        .expect("selected provider version must be a string");
    assert!(!provider_version.is_empty(), "empty provider version");
    let root = PathBuf::from(
        std::env::var_os("ONDAS_FIXTURES")
            .expect("ONDAS_FIXTURES is required; run just conformance"),
    );
    let provider = root.join(PROVIDER);
    assert!(provider.is_dir(), "missing provider {}", provider.display());
    let catalog: Json =
        serde_json::from_slice(&fs::read(provider.join("catalog.json")).expect("provider catalog"))
            .unwrap();
    assert_eq!(catalog["schema"], 1, "catalog schema");
    assert_eq!(catalog["provider"], PROVIDER, "provider identity");
    assert_eq!(
        catalog["version"].as_str(),
        Some(provider_version),
        "provider version mismatch"
    );
    provider.canonicalize().unwrap()
}

fn load_fixture(provider: &Path, name: &str) -> Fixture {
    let directory = provider
        .join(name)
        .canonicalize()
        .unwrap_or_else(|e| panic!("{name}: fixture directory: {e}"));
    assert!(
        directory.starts_with(provider),
        "{name}: fixture escapes provider"
    );
    let sidecar: Json = serde_json::from_slice(
        &fs::read(directory.join("fixture.json"))
            .unwrap_or_else(|e| panic!("{name}: sidecar: {e}")),
    )
    .expect("fixture JSON");
    assert_eq!(sidecar["schema"], 1, "{name}: sidecar schema");
    let artifact = &sidecar["artifact"];
    let extension = artifact["format"].as_str().unwrap();
    let format = match extension {
        "fst" => Format::Fst,
        "vcd" => Format::Vcd,
        _ => panic!("{name}: unexpected format {extension}"),
    };
    let filename = format!("waveform.{extension}");
    assert_eq!(artifact["file"], filename, "{name}: artifact filename");
    let path = directory.join(filename).canonicalize().unwrap();
    assert!(
        path.starts_with(&directory) && path.is_file(),
        "{name}: artifact containment/type"
    );
    let bytes = fs::read(&path).unwrap();
    assert_eq!(
        Some(bytes.len() as u64),
        artifact["size"].as_u64(),
        "{name}: artifact size"
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        artifact["sha256"].as_str().unwrap(),
        "{name}: artifact SHA256"
    );
    let provenance = &sidecar["provenance"];
    match provenance["kind"].as_str().unwrap() {
        "authored" => (),
        "imported" | "converted" => {
            assert!(!provenance["source"].as_str().unwrap().is_empty());
            if provenance["kind"] == "converted" {
                assert!(!provenance["transform"].as_str().unwrap().is_empty());
            }
        }
        kind => panic!("{name}: invalid provenance {kind}"),
    }
    let mut tags = HashSet::new();
    for tag in list(&sidecar, "tags") {
        let tag = tag.as_str().expect("fixture tag string");
        assert!(
            [
                "conformance",
                "metadata",
                "hierarchy",
                "paths",
                "aliases",
                "values",
                "bits",
                "real",
                "strings",
                "events",
                "unknown-states",
                "ranges",
                "projections",
                "same-time",
                "constants",
                "enumerations",
                "malformed",
                "large",
            ]
            .contains(&tag)
                && tags.insert(tag),
            "{name}: unknown/duplicate tag {tag}"
        );
    }
    let oracle = sidecar["oracle"].clone();
    eprintln!("validating {PROVIDER}/{name}");
    validate_oracle(&oracle, oracle["open"]["result"] == "ok");
    Fixture {
        path,
        oracle,
        format,
    }
}

fn fixtures() -> &'static [Fixture] {
    static FIXTURES: OnceLock<Vec<Fixture>> = OnceLock::new();
    FIXTURES.get_or_init(|| {
        let provider = provider();
        CASES
            .iter()
            .map(|name| load_fixture(&provider, name))
            .collect()
    })
}

fn open(fixture: &Fixture, bytes: bool) -> ondas::Result<Waveform> {
    if bytes {
        ondas::open_bytes_with(
            fixture.logical_name(),
            fs::read(&fixture.path).unwrap().into(),
            fixture.backend(),
        )
    } else {
        ondas::open_with(&fixture.path, fixture.backend())
    }
}

fn checked_open(fixture: &Fixture, bytes: bool, context: &str) -> Option<Waveform> {
    let result = open(fixture, bytes);
    if fixture.oracle["open"]["result"] == "error" {
        assert!(
            matches!(result, Err(Error::Malformed { format, ref backend, .. }) if format == fixture.format && backend == fixture.backend()),
            "{context}: expected Malformed opening error, got {}",
            match result {
                Ok(_) => "success".to_owned(),
                Err(e) => e.to_string(),
            }
        );
        None
    } else {
        Some(result.unwrap_or_else(|e| panic!("{context}: open: {e}")))
    }
}

fn metadata(wave: &Waveform, expected: &Json, bytes: bool, fixture: &Fixture) {
    assert_eq!(wave.format(), fixture.format);
    assert_eq!(wave.backend(), fixture.backend());
    let actual = wave.metadata();
    assert_eq!(
        actual.source_name(),
        if bytes {
            fixture.logical_name()
        } else {
            fixture.path.to_str().unwrap()
        }
    );
    if let Some(scale) = expected.get("timescale") {
        let actual = actual.timescale().map(|scale| {
            let unit = match scale.unit() {
                TimeUnit::Second => "s",
                TimeUnit::Millisecond => "ms",
                TimeUnit::Microsecond => "us",
                TimeUnit::Nanosecond => "ns",
                TimeUnit::Picosecond => "ps",
                TimeUnit::Femtosecond => "fs",
                TimeUnit::Attosecond => "as",
                TimeUnit::Zeptosecond => "zs",
                _ => panic!("unknown time unit"),
            };
            json!({"factor": scale.factor(), "unit": unit})
        });
        assert_eq!(actual.unwrap_or(Json::Null), *scale, "timescale");
    }
    if let Some(span) = expected.get("time_span") {
        assert_eq!(actual.time_span().map(|s| json!({"first": s.first().ticks().to_string(), "last": s.last().ticks().to_string()})).unwrap_or(Json::Null), *span, "time span");
    }
    for (key, actual) in [("writer", actual.writer()), ("date", actual.date())] {
        if let Some(expected) = expected.get(key) {
            assert_eq!(json!(actual), *expected, "{key}");
        }
    }
    let mut comments: Vec<_> = actual.comments().collect();
    for comment in list(expected, "comments") {
        let position = comments
            .iter()
            .position(|s| Some(*s) == comment.as_str())
            .expect("missing expected comment");
        comments.remove(position);
    }
}

fn hierarchy(wave: &Waveform, oracle: &Json, check_lookups: bool) -> BTreeMap<String, Signal> {
    let hierarchy = wave.hierarchy();
    // Index only oracle-listed declarations from public traversal. Large FSTs
    // can list thousands of aliases; repeated linear lookups would be quadratic.
    let wanted_scopes: HashSet<_> = list(&oracle["hierarchy"], "scopes")
        .iter()
        .map(|v| path(&v["path"]))
        .collect();
    let wanted_variables: HashSet<_> = list(&oracle["hierarchy"], "variables")
        .iter()
        .map(|v| path(&v["path"]))
        .collect();
    let variable_names: HashSet<_> = wanted_variables
        .iter()
        .filter_map(HierarchyPath::name)
        .collect();
    let mut scopes = HashMap::new();
    for scope in hierarchy.scopes() {
        let path = scope.path();
        if wanted_scopes.contains(&path) {
            assert!(
                scopes.insert(path.clone(), scope).is_none(),
                "ambiguous scope {path}"
            );
        }
    }
    let mut variables = HashMap::new();
    for variable in hierarchy
        .variables()
        .filter(|v| variable_names.contains(v.name()))
    {
        let path = variable.path();
        if wanted_variables.contains(&path) {
            assert!(
                variables.insert(path.clone(), variable).is_none(),
                "ambiguous variable {path}"
            );
        }
    }
    for expected in list(&oracle["hierarchy"], "scopes") {
        let path = path(&expected["path"]);
        let scope = scopes
            .get(&path)
            .unwrap_or_else(|| panic!("missing scope {path}"));
        assert_eq!(scope.path(), path);
        assert_eq!(scope.name(), path.name().unwrap());
        assert_eq!(
            scope.parent().map(|p| p.path()),
            path.parent().filter(|p| !p.is_empty())
        );
        if check_lookups {
            assert_eq!(hierarchy.scope_path(&path).unwrap().path(), path);
            assert_eq!(hierarchy.scope(&path.to_string()).unwrap().path(), path);
        }
        if let Some(kind) = expected.get("kind") {
            assert_eq!(scope.kind(), kind.as_str().unwrap(), "scope {path} kind");
        }
        if let Some(name) = expected.get("definition_name") {
            assert_eq!(
                json!(scope.definition_name()),
                *name,
                "scope {path} definition"
            );
        }
    }
    let mut signals = BTreeMap::new();
    for expected in list(&oracle["hierarchy"], "variables") {
        let path = path(&expected["path"]);
        let variable = variables
            .get(&path)
            .unwrap_or_else(|| panic!("missing variable {path}"));
        assert_eq!(variable.path(), path);
        assert_eq!(variable.name(), path.name().unwrap());
        assert_eq!(
            variable.parent().map(|p| p.path()),
            path.parent().filter(|p| !p.is_empty())
        );
        if check_lookups {
            assert_eq!(hierarchy.variable_path(&path).unwrap().path(), path);
            assert_eq!(hierarchy.variable(&path.to_string()).unwrap().path(), path);
        }
        if let Some(kind) = expected.get("kind") {
            assert_eq!(variable.kind(), kind.as_str().unwrap(), "{path} kind");
        }
        if let Some(direction) = expected.get("direction") {
            assert_eq!(
                format!("{:?}", variable.direction()).to_lowercase(),
                direction.as_str().unwrap(),
                "{path} direction"
            );
        }
        if let Some(range) = expected.get("range") {
            assert_eq!(
                variable
                    .range()
                    .map(|r| json!({"msb": r.msb(), "lsb": r.lsb()}))
                    .unwrap_or(Json::Null),
                *range,
                "{path} range"
            );
        }
        if let Some(name) = expected.get("type_name") {
            assert_eq!(json!(variable.type_name()), *name, "{path} type name");
        }
        if let Some(constant) = expected.get("is_constant") {
            assert_eq!(json!(variable.is_constant()), *constant, "{path} constancy");
        }
        if let Some(id) = expected.get("signal") {
            if id.is_null() {
                assert!(variable.signal().is_none());
                continue;
            }
            let id = id.as_str().unwrap();
            let signal = variable.signal().unwrap();
            if check_lookups {
                assert_eq!(hierarchy.signal_path(&path).unwrap(), signal);
                assert_eq!(hierarchy.signal(&path.to_string()).unwrap(), signal);
            }
            assert_eq!(
                *signals.entry(id.to_owned()).or_insert(signal),
                signal,
                "{path}: alias {id}"
            );
            if check_lookups {
                assert!(
                    hierarchy
                        .aliases(signal)
                        .unwrap()
                        .any(|alias| alias.path() == path),
                    "{path}: aliases traversal"
                );
            }
        }
    }
    assert_eq!(
        signals.values().copied().collect::<HashSet<_>>().len(),
        signals.len(),
        "distinct oracle signals merged"
    );
    let all: Vec<_> = hierarchy.signals().collect();
    assert_eq!(
        all.iter().copied().collect::<HashSet<_>>().len(),
        all.len(),
        "duplicate unique signal"
    );
    assert!(
        all.iter().all(|s| !s.is_slice() && s.base() == *s),
        "whole signal iterator"
    );
    // Membership only: a sparse oracle never asserts complete file hierarchy size.
    for (id, signal) in &signals {
        assert!(all.contains(signal), "{id}: missing from unique signals");
        let expected = &oracle["signals"][id]["encoding"];
        let actual = match signal.encoding() {
            Encoding::Bits { width } => json!({"kind": "bits", "width": width}),
            Encoding::Real => json!({"kind": "real"}),
            Encoding::String => json!({"kind": "string"}),
            Encoding::Event => json!({"kind": "event"}),
            Encoding::Unsupported => json!({"kind": "unsupported"}),
            _ => panic!("unknown encoding"),
        };
        assert_eq!(actual, *expected, "{id}: encoding");
        assert_eq!(
            signal.width().map(u64::from),
            expected.get("width").and_then(Json::as_u64)
        );
    }
    signals
}

fn changed_at(actual: Option<Time>, expected: &Json, bound: u64, strict: bool, context: &str) {
    if let Some(actual) = actual {
        let actual = actual.ticks();
        assert!(
            if strict {
                actual < bound
            } else {
                actual <= bound
            },
            "{context}: changed_at {actual}, bound {bound}, strict {strict}"
        );
        if let Some(expected) = expected.get("changed_at").filter(|v| !v.is_null()) {
            assert_eq!(actual, tick(expected), "{context}: changed_at");
        }
    }
}

fn sample(actual: SampleRef<'_>, signal: Signal, expected: &Json, time: u64, context: &str) {
    assert_eq!(actual.signal(), signal, "{context}: sampled handle");
    match actual {
        SampleRef::Missing { .. } => {
            assert_eq!(expected["missing"], true, "{context}: unexpected Missing")
        }
        SampleRef::Event { occurrences, .. } => assert_eq!(
            occurrences,
            tick(&expected["occurrences"]),
            "{context}: occurrences at {time}"
        ),
        SampleRef::Value {
            value,
            changed_at: when,
            ..
        } => {
            assert_eq!(
                value_json(value),
                normalized(&expected["value"]),
                "{context}: value at {time}"
            );
            changed_at(when, expected, time, false, context);
        }
        _ => panic!("{context}: unexpected sample variant"),
    }
}

// Oracle windows are finite complete lists. Only redundant persistent writes
// may disappear; intermediate same-tick values and every event remain ordered.
fn changes(window: &Json) -> Vec<(u64, Json)> {
    let mut previous = window["initial"].get("value").map(normalized);
    let mut result = Vec::new();
    for change in list(window, "changes") {
        let value = normalized(&change["value"]);
        if value.get("event").is_some() || previous.as_ref() != Some(&value) {
            result.push((tick(&change["time"]), value.clone()));
        }
        previous = Some(value);
    }
    result
}

fn trace(actual: &Trace, signal: Signal, window: &Json, context: &str) {
    let (start, end) = (tick(&window["start"]), tick(&window["end"]));
    assert_eq!(actual.signal(), signal, "{context}: trace handle");
    assert_eq!(
        actual.range(),
        TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end))
    );
    match actual.initial() {
        None => assert!(window["initial"].is_null(), "{context}: missing initial"),
        Some(initial) => {
            assert_eq!(
                value_json(initial.value()),
                normalized(&window["initial"]["value"]),
                "{context}: initial value"
            );
            changed_at(
                initial.changed_at(),
                &window["initial"],
                start,
                true,
                context,
            );
        }
    }
    let mut previous = actual.initial().map(|i| value_json(i.value()));
    let mut observed = Vec::new();
    let mut previous_time = start;
    for change in actual.changes() {
        let time = change.time().ticks();
        assert!(
            previous_time <= time && time <= end,
            "{context}: change outside closed range/order: {time}"
        );
        previous_time = time;
        let value = value_json(change.value());
        if value.get("event").is_some() || previous.as_ref() != Some(&value) {
            observed.push((time, value.clone()));
        }
        previous = Some(value);
    }
    let expected = changes(window);
    assert_eq!(observed.len(), expected.len(), "{context}: change count");
    for (index, (actual, expected)) in observed.iter().zip(&expected).enumerate() {
        assert_eq!(actual, expected, "{context}: change {index}");
    }
}

fn sample_queries(wave: &mut Waveform, signal: Signal, expected: &Json, time: u64, context: &str) {
    let time_value = Time::from_ticks(time);
    sample(
        wave.sample(signal, time_value)
            .unwrap_or_else(|e| panic!("{context}: sample {time}: {e}"))
            .as_ref(),
        signal,
        expected,
        time,
        context,
    );
    let handles = [signal, signal];
    let samples = wave.samples(&handles, time_value).unwrap();
    assert_eq!(samples.len(), handles.len());
    for actual in &samples {
        sample(actual.as_ref(), signal, expected, time, context);
    }
    let mut selection = wave.select(&handles).unwrap();
    assert_eq!(selection.signals(), handles);
    let samples = selection.samples(time_value).unwrap();
    assert_eq!(samples.len(), handles.len());
    for actual in &samples {
        sample(actual.as_ref(), signal, expected, time, context);
    }
    let mut calls = 0;
    let result = selection
        .visit_samples(time_value, |actual| {
            calls += 1;
            sample(actual, signal, expected, time, context);
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert_eq!(result, ControlFlow::Continue(()));
    assert_eq!(calls, 2, "{context}: visit_samples callbacks");
    calls = 0;
    let result = selection
        .visit_samples(time_value, |_| {
            calls += 1;
            ControlFlow::Break("sample-stop")
        })
        .unwrap();
    assert_eq!(result, ControlFlow::Break("sample-stop"));
    assert_eq!(calls, 1, "{context}: visit_samples Break");
}

// Each row retains the callback's value past its lifetime before comparison.
// Initial rows carry changed_at; change rows carry their actual timestamp.
type Row = (Signal, bool, Option<Time>, ondas::Value);
fn row(record: ScanRef<'_>) -> Row {
    match record {
        ScanRef::Initial {
            signal,
            value,
            changed_at,
        } => (signal, true, changed_at, value.to_owned()),
        ScanRef::Change {
            signal,
            time,
            value,
        } => (signal, false, Some(time), value.to_owned()),
        _ => panic!("unknown scan record"),
    }
}

fn scan_queries(wave: &mut Waveform, handles: &[Signal], range: TimeRange, context: &str) {
    let single_traces: Vec<_> = handles
        .iter()
        .map(|&s| wave.trace(s, range).unwrap())
        .collect();
    let single_samples: Vec<_> = handles
        .iter()
        .map(|&s| match wave.sample(s, range.start()).unwrap().as_ref() {
            SampleRef::Missing { .. } => json!({"missing": true}),
            SampleRef::Event { occurrences, .. } => json!({"occurrences": occurrences.to_string()}),
            SampleRef::Value {
                value, changed_at, ..
            } => json!({
                "value": value_json(value), "changed_at": changed_at.map(|t| t.ticks().to_string())
            }),
            _ => panic!("unknown sample"),
        })
        .collect();
    let batch = wave.samples(handles, range.start()).unwrap();
    assert_eq!(batch.len(), handles.len());
    for ((actual, &signal), expected) in batch.iter().zip(handles).zip(&single_samples) {
        sample(
            actual.as_ref(),
            signal,
            expected,
            range.start().ticks(),
            context,
        );
    }
    let mut direct_rows = Vec::new();
    assert_eq!(
        wave.scan(handles, range, |record| {
            direct_rows.push(row(record));
            ControlFlow::<()>::Continue(())
        })
        .unwrap(),
        ControlFlow::Continue(())
    );
    let mut direct_candidates = Vec::new();
    assert_eq!(
        wave.scan_candidate_times(handles, range, |time| {
            direct_candidates.push(time);
            ControlFlow::<()>::Continue(())
        })
        .unwrap(),
        ControlFlow::Continue(())
    );
    let traces = wave
        .traces(handles, range)
        .unwrap_or_else(|e| panic!("{context}: traces: {e}"));
    assert_eq!(traces.len(), handles.len());
    for (actual, signal) in traces.iter().zip(handles) {
        assert_eq!(actual.signal(), *signal, "{context}: trace selection order");
    }
    let mut selected = wave.select(handles).unwrap();
    assert_eq!(selected.signals(), handles);
    let batch = selected.samples(range.start()).unwrap();
    assert_eq!(batch.len(), handles.len());
    for ((actual, &signal), expected) in batch.iter().zip(handles).zip(&single_samples) {
        sample(
            actual.as_ref(),
            signal,
            expected,
            range.start().ticks(),
            context,
        );
    }
    let mut sample_calls = 0;
    assert_eq!(
        selected
            .visit_samples(range.start(), |actual| {
                sample(
                    actual,
                    handles[sample_calls],
                    &single_samples[sample_calls],
                    range.start().ticks(),
                    context,
                );
                sample_calls += 1;
                ControlFlow::<()>::Continue(())
            })
            .unwrap(),
        ControlFlow::Continue(())
    );
    assert_eq!(
        sample_calls,
        handles.len(),
        "{context}: sample callback selection order"
    );
    let selected_traces = selected.traces(range).unwrap();
    assert_eq!(selected_traces.len(), traces.len());
    let mut rows = Vec::new();
    let result = selected
        .scan(range, |record| {
            rows.push(row(record));
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert_eq!(result, ControlFlow::Continue(()));
    let mut initial_signals = Vec::new();
    let mut seen_change = false;
    let mut previous = range.start();
    for (signal, initial, time, _) in &rows {
        assert!(
            handles.contains(signal),
            "{context}: unselected callback signal"
        );
        if *initial {
            assert!(!seen_change, "{context}: initial after changes");
            assert!(time.is_none_or(|t| t < range.start()));
            initial_signals.push(*signal);
        } else {
            seen_change = true;
            let time = time.unwrap();
            assert!(
                previous <= time && range.end().is_none_or(|end| time <= end),
                "{context}: scan time order/bounds"
            );
            previous = time;
        }
    }
    assert_eq!(
        initial_signals,
        traces
            .iter()
            .filter(|t| t.initial().is_some())
            .map(Trace::signal)
            .collect::<Vec<_>>(),
        "{context}: initial selection order"
    );
    for signal in handles.iter().copied().collect::<HashSet<_>>() {
        let copies = handles.iter().filter(|&&s| s == signal).count();
        let reference = traces.iter().find(|t| t.signal() == signal).unwrap();
        let expected_initial: Vec<_> = reference
            .initial()
            .into_iter()
            .flat_map(|i| std::iter::repeat_n((i.changed_at(), value_json(i.value())), copies))
            .collect();
        let expected_changes: Vec<_> = reference
            .changes()
            .iter()
            .flat_map(|c| std::iter::repeat_n((Some(c.time()), value_json(c.value())), copies))
            .collect();
        for initial in [true, false] {
            let actual: Vec<_> = rows
                .iter()
                .filter(|r| r.0 == signal && r.1 == initial)
                .map(|r| (r.2, value_json(r.3.as_ref())))
                .collect();
            assert_eq!(
                &actual,
                if initial {
                    &expected_initial
                } else {
                    &expected_changes
                },
                "{context}: scan/trace signal {signal:?}, initial={initial}"
            );
            let direct: Vec<_> = direct_rows
                .iter()
                .filter(|r| r.0 == signal && r.1 == initial)
                .map(|r| (r.2, value_json(r.3.as_ref())))
                .collect();
            assert_eq!(
                direct, actual,
                "{context}: Waveform/Selection scan signal {signal:?}"
            );
        }
    }
    assert_eq!(
        direct_rows.len(),
        rows.len(),
        "{context}: direct scan callback count"
    );
    for (a, b) in traces
        .iter()
        .zip(&selected_traces)
        .chain(traces.iter().zip(&single_traces))
    {
        assert_eq!(a.signal(), b.signal());
        assert_eq!(
            a.initial().map(|i| (i.changed_at(), value_json(i.value()))),
            b.initial().map(|i| (i.changed_at(), value_json(i.value())))
        );
        assert_eq!(
            a.changes()
                .iter()
                .map(|c| (c.time(), value_json(c.value())))
                .collect::<Vec<_>>(),
            b.changes()
                .iter()
                .map(|c| (c.time(), value_json(c.value())))
                .collect::<Vec<_>>()
        );
    }
    let required: BTreeSet<_> = traces
        .iter()
        .flat_map(|t| t.changes().iter().map(|c| c.time()))
        .collect();
    let mut candidates = Vec::new();
    let result = selected
        .scan_candidate_times(range, |time| {
            candidates.push(time);
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert_eq!(result, ControlFlow::Continue(()));
    for candidates in [&candidates, &direct_candidates] {
        assert!(
            candidates.windows(2).all(|w| w[0] < w[1]),
            "{context}: unique increasing candidates"
        );
        assert!(
            candidates
                .iter()
                .all(|t| *t >= range.start() && range.end().is_none_or(|end| *t <= end))
        );
        assert!(
            required.is_subset(&candidates.iter().copied().collect()),
            "{context}: missing required candidate ticks"
        );
        if range.is_empty() {
            assert!(rows.is_empty() && candidates.is_empty());
        }
    }
    let mut calls = 0;
    let result = selected
        .scan(range, |_| {
            calls += 1;
            ControlFlow::Break("scan-stop")
        })
        .unwrap();
    assert_eq!(
        calls,
        usize::from(!rows.is_empty()),
        "{context}: scan Break callbacks"
    );
    assert_eq!(
        result,
        if rows.is_empty() {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break("scan-stop")
        }
    );
    calls = 0;
    let result = selected
        .scan_candidate_times(range, |_| {
            calls += 1;
            ControlFlow::Break("candidate-stop")
        })
        .unwrap();
    assert_eq!(
        calls,
        usize::from(!candidates.is_empty()),
        "{context}: candidate Break callbacks"
    );
    assert_eq!(
        result,
        if candidates.is_empty() {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break("candidate-stop")
        }
    );
    calls = 0;
    let result = wave
        .scan(handles, range, |_| {
            calls += 1;
            ControlFlow::Break("direct-stop")
        })
        .unwrap();
    assert_eq!(
        calls,
        usize::from(!direct_rows.is_empty()),
        "{context}: direct scan Break callbacks"
    );
    assert_eq!(
        result,
        if direct_rows.is_empty() {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break("direct-stop")
        }
    );
    calls = 0;
    let result = wave
        .scan_candidate_times(handles, range, |_| {
            calls += 1;
            ControlFlow::Break("direct-stop")
        })
        .unwrap();
    assert_eq!(
        calls,
        usize::from(!direct_candidates.is_empty()),
        "{context}: direct candidate Break callbacks"
    );
    assert_eq!(
        result,
        if direct_candidates.is_empty() {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break("direct-stop")
        }
    );
}

fn window_queries(wave: &mut Waveform, signal: Signal, window: &Json, context: &str) {
    let (start, end) = (tick(&window["start"]), tick(&window["end"]));
    let range = TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end));
    trace(
        &wave
            .trace(signal, range)
            .unwrap_or_else(|e| panic!("{context}: trace: {e}")),
        signal,
        window,
        context,
    );
    scan_queries(wave, &[signal, signal], range, context);
    if start > end {
        return;
    }
    let changes = changes(window);
    let mut times = BTreeSet::from([start, end]);
    for (time, _) in &changes {
        times.insert(*time);
        if *time > start {
            times.insert(time - 1);
        }
        if *time < end {
            times.insert(time + 1);
        }
    }
    for time in times {
        let expected = if signal.encoding() == Encoding::Event {
            json!({"occurrences": changes.iter().filter(|c| c.0 == time).count().to_string()})
        } else if let Some((changed, value)) = changes.iter().rev().find(|c| c.0 <= time) {
            json!({"value": value, "changed_at": changed.to_string()})
        } else if !window["initial"].is_null() {
            window["initial"].clone()
        } else {
            json!({"missing": true})
        };
        sample_queries(wave, signal, &expected, time, context);
    }
}

fn run_case(index: usize, bytes: bool) {
    let fixture = &fixtures()[index];
    let context = format!(
        "{} / {}",
        CASES[index],
        if bytes { "bytes" } else { "file" }
    );
    eprintln!("checking {context}");
    let Some(mut wave) = checked_open(fixture, bytes, &context) else {
        return;
    };
    metadata(&wave, &fixture.oracle["metadata"], bytes, fixture);
    let signals = hierarchy(&wave, &fixture.oracle, true);
    let mut windows = 0;
    let mut samples = 0;
    let mut change_count = 0;
    let mut groups: BTreeMap<(u64, u64), Vec<Signal>> = BTreeMap::new();
    for (id, expected) in fixture.oracle["signals"].as_object().unwrap() {
        let signal = signals[id];
        for expected in list(expected, "samples") {
            sample_queries(
                &mut wave,
                signal,
                expected,
                tick(&expected["time"]),
                &format!("{context} / {id}"),
            );
            samples += 1;
        }
        for (index, window) in list(expected, "windows").iter().enumerate() {
            window_queries(
                &mut wave,
                signal,
                window,
                &format!(
                    "{context} / {id} / window {index} [{}..{}]",
                    window["start"], window["end"]
                ),
            );
            windows += 1;
            change_count += list(window, "changes").len();
            groups
                .entry((tick(&window["start"]), tick(&window["end"])))
                .or_default()
                .push(signal);
        }
    }
    for ((start, end), mut handles) in groups {
        handles.reverse();
        handles.push(handles[0]);
        scan_queries(
            &mut wave,
            &handles,
            TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end)),
            &context,
        );
    }
    scan_queries(
        &mut wave,
        &signals.values().copied().collect::<Vec<_>>(),
        TimeRange::closed(Time::from_ticks(2), Time::from_ticks(1)),
        &context,
    );
    eprintln!(
        "{context}: {} oracle signals, {samples} explicit samples, {windows} windows, {change_count} listed changes",
        signals.len()
    );
}

fn discover(provider: &Path, extension: &str) -> Vec<String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(provider).expect("provider directory") {
        let entry = entry.unwrap();
        let directory = entry.path();
        if !directory.is_dir() {
            continue;
        }
        let sidecar = directory.join("fixture.json");
        let declares_format = sidecar.is_file() && {
            let sidecar: Json = serde_json::from_slice(&fs::read(&sidecar).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", sidecar.display()));
            sidecar["artifact"]["format"] == extension
        };
        // An orphaned artifact is a failing fixture, not an invisible exclusion.
        if declares_format || directory.join(format!("waveform.{extension}")).exists() {
            names.push(entry.file_name().into_string().expect("UTF-8 fixture name"));
        }
    }
    names.sort();
    assert!(
        !names.is_empty(),
        "no {extension} fixtures in {}",
        provider.display()
    );
    names
}

fn pool_queries(fixture: &Fixture, bytes: bool, context: &str) {
    let Some(mut wave) = checked_open(fixture, bytes, context) else {
        return;
    };
    metadata(&wave, &fixture.oracle["metadata"], bytes, fixture);
    let signals = hierarchy(&wave, &fixture.oracle, false);
    let mut samples = BTreeMap::<_, Vec<_>>::new();
    let mut windows = BTreeMap::<_, Vec<_>>::new();
    for (id, expected) in fixture.oracle["signals"].as_object().unwrap() {
        let signal = signals[id];
        for expected in list(expected, "samples") {
            samples
                .entry(tick(&expected["time"]))
                .or_default()
                .push((id, signal, expected));
        }
        for window in list(expected, "windows") {
            windows
                .entry((tick(&window["start"]), tick(&window["end"])))
                .or_default()
                .push((id, signal, window));
        }
    }
    for (time, entries) in samples {
        let handles: Vec<_> = entries.iter().map(|(_, signal, _)| *signal).collect();
        let actual = wave.samples(&handles, Time::from_ticks(time)).unwrap();
        assert_eq!(actual.len(), entries.len(), "{context}: sample count");
        for (actual, (id, signal, expected)) in actual.iter().zip(entries) {
            sample(
                actual.as_ref(),
                signal,
                expected,
                time,
                &format!("{context} / {id}"),
            );
        }
    }
    for ((start, end), entries) in windows {
        let handles: Vec<_> = entries.iter().map(|(_, signal, _)| *signal).collect();
        let actual = wave
            .traces(
                &handles,
                TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end)),
            )
            .unwrap();
        assert_eq!(actual.len(), entries.len(), "{context}: trace count");
        for (actual, (id, signal, expected)) in actual.iter().zip(entries) {
            trace(
                actual,
                signal,
                expected,
                &format!("{context} / {id} / [{start}..{end}]"),
            );
        }
    }
}

fn panic_message(error: Box<dyn std::any::Any + Send>) -> String {
    error
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| error.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| "non-string panic".to_owned())
}

#[test]
#[ignore = "requires ONDAS_FIXTURES; run just conformance"]
fn full_fst_pool() {
    full_pool("fst");
}

#[test]
#[ignore = "requires ONDAS_FIXTURES; run just conformance"]
fn full_vcd_pool() {
    full_pool("vcd");
}

fn full_pool(extension: &str) {
    let provider = provider();
    let names = discover(&provider, extension);
    eprintln!(
        "{extension} pool: {} fixtures, {} file/bytes cases",
        names.len(),
        names.len() * 2
    );
    // Validate the whole selected catalog before opening any waveform. Loading
    // drops artifact bytes after hashing; query inputs are held one case at a time.
    let fixtures: Vec<_> = names
        .iter()
        .map(|name| {
            std::panic::catch_unwind(|| load_fixture(&provider, name)).map_err(panic_message)
        })
        .collect();
    let invalid: Vec<_> = names
        .iter()
        .zip(&fixtures)
        .filter_map(|(name, result)| {
            result
                .as_ref()
                .err()
                .map(|error| format!("{name}: {error}"))
        })
        .collect();
    assert!(
        invalid.is_empty(),
        "{extension} catalog validation failed before conformance:\n{}",
        invalid.join("\n")
    );
    let mut failures = Vec::new();
    let mut passed = 0;
    for (name, fixture) in names.iter().zip(fixtures) {
        let fixture = fixture.unwrap();
        for bytes in [false, true] {
            let context = format!("{name} / {}", if bytes { "bytes" } else { "file" });
            eprintln!("checking {context}");
            let result = std::panic::catch_unwind(|| pool_queries(&fixture, bytes, &context))
                .map_err(panic_message);
            match result {
                Ok(()) => {
                    passed += 1;
                    eprintln!("PASS {context}");
                }
                Err(error) => {
                    eprintln!("FAIL {context}: {error}");
                    failures.push(format!("{context}: {error}"));
                }
            }
        }
    }
    eprintln!(
        "{extension} pool: {} fixtures, {passed} passed, {} failed",
        names.len(),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "{extension} pool failures:\n{}",
        failures.join("\n")
    );
}

macro_rules! cases {
    ($($file:ident, $bytes:ident => $index:expr;)*) => {$ (
        #[test]
        #[ignore = "requires ONDAS_FIXTURES; run just conformance"]
        fn $file() { run_case($index, false); }
        #[test]
        #[ignore = "requires ONDAS_FIXTURES; run just conformance"]
        fn $bytes() { run_case($index, true); }
    )*};
}
cases! {
    fst0041_counter_file, fst0041_counter_bytes => 0;
    fst0035_complex_types_file, fst0035_complex_types_bytes => 1;
    fst0061_shortstring_file, fst0061_shortstring_bytes => 2;
    fst0032_ace_error_file, fst0032_ace_error_bytes => 3;
    fst0033_axi4_error_file, fst0033_axi4_error_bytes => 4;
    fst0046_overlay_error_file, fst0046_overlay_error_bytes => 5;
    fst0048_foo_error_file, fst0048_foo_error_bytes => 6;
}

fn counter_slices(bytes: bool) {
    let fixture = &fixtures()[0];
    let mut wave = open(fixture, bytes).unwrap();
    let counter = wave.hierarchy().signal("tb.dut.counter").unwrap();
    for (msb, lsb) in [(3, 2), (1, 0), (0, 0)] {
        let sliced = counter.slice(msb, lsb).unwrap();
        assert_eq!(sliced.base(), counter);
        assert!(sliced.is_slice());
        assert_eq!(sliced.width(), Some(msb - lsb + 1));
        assert_eq!(
            wave.hierarchy()
                .aliases(sliced)
                .unwrap()
                .map(|v| v.path())
                .collect::<Vec<_>>(),
            wave.hierarchy()
                .aliases(counter)
                .unwrap()
                .map(|v| v.path())
                .collect::<Vec<_>>()
        );
        for original in list(&fixture.oracle["signals"]["counter"], "windows") {
            let mut projected = original.clone();
            let project = |value: &mut Json| {
                let bits = value["bits"].as_str().unwrap();
                *value = json!({"bits": &bits[4 - msb as usize - 1..4 - lsb as usize]});
            };
            if !projected["initial"].is_null() {
                project(&mut projected["initial"]["value"]);
                // The sparse base initial gives no establishment tick for a slice.
                projected["initial"]
                    .as_object_mut()
                    .unwrap()
                    .remove("changed_at");
            }
            for change in projected["changes"].as_array_mut().unwrap() {
                project(&mut change["value"]);
            }
            window_queries(
                &mut wave,
                sliced,
                &projected,
                &format!("counter slice [{msb}:{lsb}], bytes={bytes}"),
            );
        }
    }
    assert_eq!(
        counter.slice(3, 1).unwrap().slice(1, 0).unwrap(),
        counter.slice(2, 1).unwrap()
    );
    assert!(matches!(
        counter.slice(0, 1),
        Err(ondas::SliceError::InvalidRange { .. })
    ));
    assert!(matches!(
        counter.slice(4, 0),
        Err(ondas::SliceError::OutOfBounds { .. })
    ));
    let high = counter.slice(3, 2).unwrap();
    let low = counter.slice(1, 0).unwrap();
    let handles = [high, counter, low, high];
    scan_queries(
        &mut wave,
        &handles,
        TimeRange::closed(Time::from_ticks(380), Time::from_ticks(420)),
        "mixed counter slices",
    );
    // At 390 only the low bits changed; high's establishment must not be 390.
    if let SampleRef::Value {
        changed_at: Some(time),
        ..
    } = wave.sample(high, Time::from_ticks(390)).unwrap().as_ref()
    {
        assert!(
            time.ticks() < 380,
            "high slice changed_at borrowed unrelated base activity"
        );
    }
}

fn foreign_handles(bytes: bool) {
    let fixture = &fixtures()[0];
    let first = open(fixture, bytes).unwrap();
    let foreign = first.hierarchy().signal("tb.dut.counter").unwrap();
    let mut second = open(fixture, bytes).unwrap();
    let own = second.hierarchy().signal("tb.dut.counter").unwrap();
    assert_ne!(
        own, foreign,
        "separate opens must have separate handle identity"
    );
    let range = TimeRange::closed(Time::from_ticks(2), Time::from_ticks(1));
    for signal in [foreign, foreign.slice(1, 0).unwrap()] {
        assert!(matches!(
            second.hierarchy().aliases(signal),
            Err(Error::InvalidSignal { .. })
        ));
        assert!(matches!(
            second.select(&[own, signal]),
            Err(Error::InvalidSignal { .. })
        ));
        assert!(matches!(
            second.sample(signal, Time::ZERO),
            Err(Error::InvalidSignal { .. })
        ));
        assert!(matches!(
            second.samples(&[own, signal], Time::ZERO),
            Err(Error::InvalidSignal { .. })
        ));
        assert!(matches!(
            second.trace(signal, range),
            Err(Error::InvalidSignal { .. })
        ));
        assert!(matches!(
            second.traces(&[own, signal], range),
            Err(Error::InvalidSignal { .. })
        ));
        let mut calls = 0;
        let result = second.scan(&[own, signal], range, |_| {
            calls += 1;
            ControlFlow::<()>::Continue(())
        });
        assert!(matches!(result, Err(Error::InvalidSignal { .. })));
        assert_eq!(calls, 0, "foreign handle scan callback");
        let result = second.scan_candidate_times(&[own, signal], range, |_| {
            calls += 1;
            ControlFlow::<()>::Continue(())
        });
        assert!(matches!(result, Err(Error::InvalidSignal { .. })));
        assert_eq!(calls, 0, "foreign handle candidate callback");
    }
}

#[test]
#[ignore = "requires ONDAS_FIXTURES; run just conformance"]
fn counter_slice_projections_file() {
    counter_slices(false);
}
#[test]
#[ignore = "requires ONDAS_FIXTURES; run just conformance"]
fn counter_slice_projections_bytes() {
    counter_slices(true);
}
#[test]
#[ignore = "requires ONDAS_FIXTURES; run just conformance"]
fn foreign_handle_validation_file() {
    foreign_handles(false);
}
#[test]
#[ignore = "requires ONDAS_FIXTURES; run just conformance"]
fn foreign_handle_validation_bytes() {
    foreign_handles(true);
}

#[test]
#[ignore = "requires ONDAS_FIXTURES; run just conformance"]
fn automatic_opening_uses_content_and_keeps_logical_names() {
    let fixture = &fixtures()[0];
    let file = ondas::open(&fixture.path).unwrap();
    let memory =
        ondas::open_bytes("logical-name.vcd", fs::read(&fixture.path).unwrap().into()).unwrap();
    assert_eq!(memory.metadata().source_name(), "logical-name.vcd");
    let expected = fixture.oracle["signals"]["counter"]["samples"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sample| sample["time"] == "801")
        .unwrap();
    for mut wave in [file, memory] {
        assert_eq!(wave.format(), Format::Fst);
        assert_eq!(wave.backend(), "fst-native");
        let signal = wave.hierarchy().signal("tb.dut.counter").unwrap();
        sample(
            wave.sample(signal, Time::from_ticks(801)).unwrap().as_ref(),
            signal,
            expected,
            801,
            "automatic opening",
        );
    }
}

#[test]
fn discovery_uses_artifacts_not_fixture_names_or_a_whitelist() {
    let root = std::env::temp_dir().join(format!(
        "ondas-fst-discovery-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    for name in [
        "anything",
        "fst-not-selected",
        "orphan",
        "irrelevant/nested",
    ] {
        fs::create_dir_all(root.join(name)).unwrap();
    }
    for (name, format) in [
        ("anything", "fst"),
        ("fst-not-selected", "vcd"),
        ("irrelevant/nested", "fst"),
    ] {
        fs::write(
            root.join(name).join("fixture.json"),
            json!({"artifact": {"format": format}}).to_string(),
        )
        .unwrap();
    }
    fs::write(root.join("orphan/waveform.fst"), []).unwrap();
    fs::write(root.join("catalog.json"), "{}").unwrap();
    assert_eq!(discover(&root, "fst"), ["anything", "orphan"]);
    assert_eq!(discover(&root, "vcd"), ["fst-not-selected"]);
    fs::remove_file(root.join("anything/fixture.json")).unwrap();
    fs::remove_file(root.join("orphan/waveform.fst")).unwrap();
    assert!(std::panic::catch_unwind(|| discover(&root, "fst")).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn oracle_comparison_rules() {
    assert_eq!(
        normalized(&json!({"real_bits": "7ff0000000000001"})),
        normalized(&json!({"real_bits": "fff8000000000002"}))
    );
    assert_ne!(
        normalized(&json!({"real_bits": "0000000000000000"})),
        normalized(&json!({"real_bits": "8000000000000000"}))
    );
    let window = json!({"initial": null, "changes": [
        {"time": "0", "value": {"bits": "0"}}, {"time": "0", "value": {"bits": "1"}},
        {"time": "0", "value": {"bits": "1"}}, {"time": "4", "value": {"bits": "0"}}
    ]});
    assert_eq!(
        changes(&window),
        vec![
            (0, json!({"bits": "0"})),
            (0, json!({"bits": "1"})),
            (4, json!({"bits": "0"}))
        ]
    );
    let events = json!({"initial": null, "changes": [{"time": "0", "value": {"event": true}}, {"time": "0", "value": {"event": true}}]});
    assert_eq!(changes(&events).len(), 2);
    assert_ne!(
        normalized(&json!({"string": "röd "})),
        normalized(&json!({"string": "röd"}))
    );
    changed_at(
        Some(Time::from_ticks(1)),
        &json!({"changed_at": null}),
        2,
        true,
        "unknown oracle timestamp",
    );
}
