use std::{fs, path::PathBuf};

use ondas::{HierarchyPath, Signal, Time, TimeRange, Trace, Waveform};
use serde_json::Value;

pub struct Job {
    pub backend: String,
    pub artifact: PathBuf,
    pub id: String,
    pub workload: String,
    pub shape: String,
    pub boundary: String,
    pub history: String,
    pub api: String,
    pub paths: Vec<HierarchyPath>,
    pub lookup: HierarchyPath,
    pub lookup_text: String,
    pub scope: HierarchyPath,
    pub lookup_many: Vec<HierarchyPath>,
    pub metadata: Vec<HierarchyPath>,
    pub session: Vec<Vec<HierarchyPath>>,
    pub choices: Vec<usize>,
    pub independent: Vec<HierarchyPath>,
    pub prior: Vec<HierarchyPath>,
    pub times: Vec<Time>,
    pub range: TimeRange,
    pub point: Time,
    pub late_point: Time,
    pub prior_point: Time,
    pub prior_start: Time,
    pub limit: usize,
    pub drivers: usize,
    pub step_ticks: u64,
    pub raw: PathBuf,
    pub settings: Value,
}

fn path(v: &Value) -> HierarchyPath {
    HierarchyPath::from_components(v.as_array().unwrap().iter().map(|s| s.as_str().unwrap()))
}
fn paths(v: &Value) -> Vec<HierarchyPath> {
    v.as_array().unwrap().iter().map(path).collect()
}

impl Job {
    pub fn read(file: &str, index: usize) -> Self {
        let run: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
        let job = &run["jobs"][index];
        let input = &job["input"];
        let case = &input["case"];
        let time = |key: &str| Time::from_ticks(input[key].as_u64().unwrap());
        Self {
            backend: input["backend"].as_str().unwrap().into(),
            artifact: input["artifact"].as_str().unwrap().into(),
            id: case["id"].as_str().unwrap().into(),
            workload: case["workload"].as_str().unwrap().into(),
            shape: case["selection"].as_str().unwrap().into(),
            boundary: case["boundary"].as_str().unwrap().into(),
            history: case["history"].as_str().unwrap().into(),
            api: case["api"].as_str().unwrap().into(),
            paths: paths(&input["paths"]),
            lookup: path(&input["lookup_path"]),
            lookup_text: path(&input["lookup_path"]).to_verilog().unwrap(),
            scope: path(&input["scope_path"]),
            lookup_many: paths(&input["lookup_paths"]),
            metadata: paths(&input["metadata_paths"]),
            session: input["session_paths"]
                .as_array()
                .unwrap()
                .iter()
                .map(paths)
                .collect(),
            choices: input["session_choices"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as usize)
                .collect(),
            independent: paths(&input["independent_paths"]),
            prior: paths(&input["prior_paths"]),
            times: input["times"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| Time::from_ticks(v.as_u64().unwrap()))
                .collect(),
            range: TimeRange::closed(time("start"), time("end")),
            point: time("point"),
            late_point: time("late_point"),
            prior_point: time("prior_point"),
            prior_start: time("prior_start"),
            limit: case["parameters"]["n"].as_u64().unwrap_or(
                if matches!(
                    case["workload"].as_str(),
                    Some("inspect-window" | "break-then-independent-point")
                ) {
                    100
                } else {
                    0
                },
            ) as usize,
            drivers: case["parameters"]["drivers"].as_u64().unwrap_or(0) as usize,
            step_ticks: input["step_ticks"].as_u64().unwrap(),
            raw: PathBuf::from(run["directory"].as_str().unwrap())
                .join(job["raw"].as_str().unwrap()),
            settings: run["criterion"].clone(),
        }
    }

    pub fn open(&self) -> Waveform {
        let wave = ondas::open_with(&self.artifact, &self.backend).unwrap();
        assert_eq!(wave.backend(), self.backend);
        wave
    }
}

pub fn resolve(wave: &Waveform, paths: &[HierarchyPath]) -> Vec<Signal> {
    paths
        .iter()
        .map(|p| wave.hierarchy().signal_path(p).unwrap())
        .collect()
}

pub fn compose(wave: &mut Waveform, signals: &[Signal], nested: bool) -> Signal {
    if nested {
        let mut head = signals[0];
        for &next in &signals[1..] {
            head = wave.concat(&[head, next]).unwrap();
        }
        head
    } else {
        wave.concat(signals).unwrap()
    }
}

pub struct Prepared {
    pub wave: Waveform,
    pub signals: Vec<Signal>,
    pub session: Vec<Vec<Signal>>,
    pub independent: Vec<Signal>,
    pub prior: Vec<Signal>,
    pub saved: Vec<Trace>,
}

impl Prepared {
    pub fn new(job: &Job) -> Self {
        let mut wave = job.open();
        let mut signals = resolve(&wave, &job.paths);
        match job.shape.as_str() {
            "active-bit" => signals = vec![signals[0].slice(0, 0).unwrap()],
            "stable-high-slice" => signals = vec![signals[0].slice(127, 64).unwrap()],
            "concat8x64" => signals = vec![compose(&mut wave, &signals, false)],
            "nested16-result-bit" => {
                signals = vec![compose(&mut wave, &signals, true).slice(0, 0).unwrap()]
            }
            "overlapping64-of1024" => {
                signals = (0..100)
                    .map(|i| signals[0].slice(i + 63, i).unwrap())
                    .collect()
            }
            "query-shared-base" => signals[0] = signals[0].slice(0, 0).unwrap(),
            "concat8-identical-repeat" => {
                compose(&mut wave, &signals, false);
            }
            _ => {}
        }
        let session = job
            .session
            .iter()
            .map(|paths| resolve(&wave, paths))
            .collect();
        let independent = if job.workload == "break-then-independent-point" {
            resolve(&wave, &job.independent)
        } else {
            vec![]
        };
        let prior = if matches!(
            job.history.as_str(),
            "distant-disjoint" | "background-10000"
        ) {
            resolve(&wave, &job.prior)
        } else {
            vec![]
        };
        let saved = if job.boundary == "T" {
            wave.traces(&signals, job.range).unwrap()
        } else {
            vec![]
        };
        Self {
            wave,
            signals,
            session,
            independent,
            prior,
            saved,
        }
    }
}
