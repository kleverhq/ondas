//! Source-controlled GHW workload coordinates shared by conformance and benches.
use ondas::{Time, TimeRange};

pub const WIDE: &str = "ghw/ghw0097-wide-toggle";
pub const WIDE_PATH: &str = "top.wide";
pub const CONTROL_PATH: &str = "top.control";
pub const CONSTANT_PATH: &str = "top.constant_probe";
pub const NOISE_PATH: &str = "top.noise";
pub const TOPOLOGIES: [&str; 4] = [
    "ghw/ghw0098-topology-short",
    "ghw/ghw0099-topology-long",
    "ghw/ghw0100-topology-many",
    "ghw/ghw0101-topology-dense",
];

pub fn range(start: u64, end: u64) -> TimeRange {
    TimeRange::closed(Time::from_ticks(start), Time::from_ticks(end))
}

pub fn probe(index: usize) -> String {
    format!("top.probes({index}).probe")
}
