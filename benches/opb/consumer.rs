use std::{
    collections::{HashMap, HashSet},
    hint::black_box,
    io::{BufWriter, Write},
};

use ondas::{HierarchyPath, Item, SampleRef, ScanRef, Scope, Signal, Time, ValueRef, Waveform};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub trait Consumer {
    fn sample(&mut self, slot: usize, time: Time, sample: SampleRef<'_>);
    fn scan(&mut self, slot: usize, record: ScanRef<'_>);
    fn candidate(&mut self, time: Time);
    fn item(&mut self, item: &Item<'_>);
    fn hierarchy_end(&mut self);
    fn open(&mut self, wave: &Waveform);
    fn lookup(&mut self, wave: &Waveform, path: &HierarchyPath, signal: Signal);
    fn selection(&mut self, requested: &[Signal], signals: &[Signal]);
    fn scope(&mut self, scope: Scope<'_>);
    fn missing(&mut self, missing: bool);
    fn metadata(&mut self, item: &Item<'_>);
    fn sources(&mut self, wave: &Waveform, signals: &[Signal]);
    fn inventory(&mut self, wave: &Waveform, signals: impl Iterator<Item = Signal>);
    fn accepted(&mut self, time: Time);
    fn query_end(&mut self, stopped: bool);
    fn definition(&mut self, wave: &mut Waveform, signal: Signal, time: Time);
}

pub struct Timing;
impl Consumer for Timing {
    fn sample(&mut self, slot: usize, time: Time, sample: SampleRef<'_>) {
        black_box((slot, time, sample));
    }
    fn scan(&mut self, slot: usize, record: ScanRef<'_>) {
        black_box((slot, record));
    }
    fn candidate(&mut self, time: Time) {
        black_box(time);
    }
    fn item(&mut self, item: &Item<'_>) {
        black_box(item);
    }
    fn hierarchy_end(&mut self) {}
    fn open(&mut self, wave: &Waveform) {
        black_box(wave);
    }
    fn lookup(&mut self, _wave: &Waveform, path: &HierarchyPath, signal: Signal) {
        black_box((path, signal));
    }
    fn selection(&mut self, _requested: &[Signal], signals: &[Signal]) {
        black_box(signals);
    }
    fn scope(&mut self, scope: Scope<'_>) {
        black_box(scope);
    }
    fn missing(&mut self, missing: bool) {
        black_box(missing);
    }
    fn metadata(&mut self, item: &Item<'_>) {
        match item {
            Item::Scope(s) => {
                black_box((s.name(), s.kind(), s.parent(), s.path()));
            }
            Item::Variable(v) => {
                black_box((
                    v.name(),
                    v.kind(),
                    v.parent(),
                    v.path(),
                    v.signal().map(Signal::encoding),
                    v.range(),
                    v.direction(),
                    v.signedness(),
                    v.logic_domain(),
                    v.enumeration().is_some(),
                ));
            }
        }
    }
    fn sources(&mut self, _wave: &Waveform, signals: &[Signal]) {
        black_box(signals);
    }
    fn inventory(&mut self, _wave: &Waveform, signals: impl Iterator<Item = Signal>) {
        for signal in signals {
            black_box(signal);
        }
    }
    fn accepted(&mut self, time: Time) {
        black_box(time);
    }
    fn query_end(&mut self, stopped: bool) {
        black_box(stopped);
    }
    fn definition(&mut self, _wave: &mut Waveform, signal: Signal, _time: Time) {
        black_box(signal);
    }
}

pub struct Verify<W: Write> {
    writer: BufWriter<W>,
    scopes: Vec<Vec<String>>,
    variables: usize,
}

impl<W: Write> Verify<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer: BufWriter::new(writer),
            scopes: vec![],
            variables: 0,
        }
    }
    pub fn emit(&mut self, value: Value) {
        write!(&mut self.writer, "OPB:").unwrap();
        serde_json::to_writer(&mut self.writer, &value).unwrap();
        writeln!(&mut self.writer).unwrap();
    }
    pub fn finish(mut self) {
        self.emit(json!({"kind":"done"}));
        self.writer.flush().unwrap();
    }
}

fn bits(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Bits(bits) => bits.to_string(),
        _ => panic!("OPB expects bits"),
    }
}

impl<W: Write> Consumer for Verify<W> {
    fn sample(&mut self, slot: usize, time: Time, sample: SampleRef<'_>) {
        let SampleRef::Value {
            value, changed_at, ..
        } = sample
        else {
            panic!("OPB persistent sample missing")
        };
        self.emit(json!({"kind":"sample", "slot":slot, "time":time.ticks(), "bits":bits(value), "changed_at":changed_at.map(Time::ticks)}));
    }
    fn scan(&mut self, slot: usize, record: ScanRef<'_>) {
        match record {
            ScanRef::Initial { value, changed_at, .. } => self.emit(json!({"kind":"initial", "slot":slot, "bits":bits(value), "changed_at":changed_at.map(Time::ticks)})),
            ScanRef::Change { time, value, .. } => self.emit(json!({"kind":"change", "slot":slot, "time":time.ticks(), "bits":bits(value)})),
            _ => panic!("unknown scan record"),
        }
    }
    fn candidate(&mut self, time: Time) {
        self.emit(json!({"kind":"candidate", "time":time.ticks()}));
    }
    fn item(&mut self, item: &Item<'_>) {
        match item {
            Item::Scope(scope) => self
                .scopes
                .push(scope.path().components().map(str::to_owned).collect()),
            Item::Variable(var) => {
                self.variables += 1;
                self.emit(json!({"kind":"variable", "path":var.path().components().collect::<Vec<_>>(), "width":var.signal().and_then(Signal::width)}));
            }
        }
    }
    fn hierarchy_end(&mut self) {
        self.scopes.sort();
        let encoded = serde_json::to_vec(&self.scopes).unwrap();
        let hash = format!("{:x}", Sha256::digest(encoded));
        self.emit(json!({"kind":"hierarchy", "scopes":self.scopes.len(), "variables":self.variables, "scope_sha256":hash}));
        self.scopes.clear();
        self.variables = 0;
    }
    fn open(&mut self, wave: &Waveform) {
        let span = wave.metadata().time_span().unwrap();
        let scale = wave
            .metadata()
            .timescale()
            .map(|s| json!([s.factor(), format!("{:?}", s.unit()).to_lowercase()]));
        self.emit(json!({"kind":"open", "backend":wave.backend(), "first":span.first().ticks(), "last":span.last().ticks(), "timescale":scale}));
    }
    fn lookup(&mut self, wave: &Waveform, path: &HierarchyPath, signal: Signal) {
        let observed = wave
            .hierarchy()
            .aliases(signal)
            .unwrap()
            .find(|variable| variable.path() == *path)
            .map(|variable| {
                variable
                    .path()
                    .components()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            });
        self.emit(json!({"kind":"lookup", "path":observed, "width":signal.width()}));
    }
    fn selection(&mut self, requested: &[Signal], signals: &[Signal]) {
        let bases = signals
            .iter()
            .map(|s| s.base())
            .collect::<HashSet<_>>()
            .len();
        self.emit(json!({"kind":"selection", "slots":signals.len(), "bases":bases, "widths":signals.iter().map(|s| s.width()).collect::<Vec<_>>(), "same_order":signals == requested}));
    }
    fn scope(&mut self, scope: Scope<'_>) {
        self.emit(json!({"kind":"scope", "path":scope.path().components().collect::<Vec<_>>() }));
    }
    fn missing(&mut self, missing: bool) {
        self.emit(json!({"kind":"missing", "missing":missing}));
    }
    fn metadata(&mut self, item: &Item<'_>) {
        match item {
            Item::Scope(s) => self.emit(json!({"kind":"metadata", "object":"scope", "path":s.path().components().collect::<Vec<_>>(), "name":s.name(), "parent":s.parent().map(|p| p.path().components().map(str::to_owned).collect::<Vec<_>>()), "native_kind":s.kind()})),
            Item::Variable(v) => self.emit(json!({"kind":"metadata", "object":"variable", "path":v.path().components().collect::<Vec<_>>(), "name":v.name(), "parent":v.parent().map(|p| p.path().components().map(str::to_owned).collect::<Vec<_>>()), "native_kind":v.kind(), "width":v.signal().and_then(Signal::width), "encoding":v.signal().map(|s|format!("{:?}",s.encoding())), "range":v.range().map(|r|format!("{:?}",r)), "direction":format!("{:?}",v.direction()), "signedness":v.signedness().map(|s|format!("{:?}",s)), "logic_domain":v.logic_domain().map(|s|format!("{:?}",s)), "has_enum":v.enumeration().is_some()})),
        }
    }
    fn sources(&mut self, wave: &Waveform, signals: &[Signal]) {
        self.emit(json!({"kind":"sources", "paths":observation_paths(wave,signals)}));
    }
    fn inventory(&mut self, wave: &Waveform, signals: impl Iterator<Item = Signal>) {
        let mut count = 0;
        let mut returned = HashSet::new();
        for signal in signals {
            count += 1;
            returned.insert(signal);
        }
        let mut declared = HashSet::new();
        for variable in wave.hierarchy().variables() {
            if let Some(signal) = variable.signal() {
                declared.insert(signal);
                if returned.contains(&signal) {
                    self.emit(json!({"kind":"signal", "path":variable.path().components().collect::<Vec<_>>(), "width":signal.width()}));
                }
            }
        }
        self.emit(json!({"kind":"signals", "count":count, "unique":returned.len()==count, "same_set":returned==declared}));
    }
    fn accepted(&mut self, time: Time) {
        self.emit(json!({"kind":"accepted", "time":time.ticks()}));
    }
    fn query_end(&mut self, stopped: bool) {
        self.emit(json!({"kind":"query-end", "stopped":stopped}));
    }
    fn definition(&mut self, wave: &mut Waveform, signal: Signal, time: Time) {
        self.emit(json!({"kind":"definition", "width":signal.width()}));
        let sample = wave.sample(signal, time).unwrap();
        self.sample(0, time, sample.as_ref());
    }
}

fn observation_paths(wave: &Waveform, signals: &[Signal]) -> Vec<Vec<String>> {
    let wanted = signals.iter().copied().collect::<HashSet<_>>();
    let mut declarations = HashMap::new();
    for variable in wave.hierarchy().variables() {
        if let Some(signal) = variable.signal()
            && wanted.contains(&signal)
        {
            declarations.entry(signal).or_insert(variable);
        }
    }
    signals
        .iter()
        .map(|signal| {
            declarations
                .get(signal)
                .unwrap()
                .path()
                .components()
                .map(str::to_owned)
                .collect()
        })
        .collect()
}
