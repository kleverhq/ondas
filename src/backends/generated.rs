//! Fixed test stream: no record storage, index, clock or public metrics API.
use crate::{Error, Result, Signal, Time, ValueRef};
use std::{
    ops::ControlFlow,
    sync::{Arc, Mutex},
};

pub(crate) const PAYLOAD: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
pub(crate) type Probe = Arc<Mutex<Counts>>;

#[derive(Default, Debug, Clone)]
pub(crate) struct Counts {
    pub starts: u64,
    pub releases: u64,
    pub active: u64,
    pub reader_drops: u64,
    pub advances: u64,
    pub payload_decodes: u64,
    pub unused_decodes: u64,
    pub requested_bases: Vec<usize>,
    pub advance_budget: Option<u64>,
    pub max_pending_records: usize,
    pub max_pending_bytes: usize,
    pub max_slots: usize,
}

pub(crate) struct Reader {
    pub ticks: u64,
    pub fail_at: Option<(u64, usize)>,
    pub probe: Probe,
}

struct Lease(Probe);
impl Drop for Lease {
    fn drop(&mut self) {
        let mut counts = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        counts.active -= 1;
        counts.releases += 1;
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.probe
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .reader_drops += 1;
    }
}

impl Reader {
    pub(crate) fn read<B>(
        &mut self,
        signals: &[Signal],
        end: Time,
        mut visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        {
            let mut counts = self.probe.lock().unwrap();
            counts.starts += 1;
            counts.active += 1;
            counts.requested_bases = signals.iter().map(|signal| signal.index()).collect();
        }
        let _lease = Lease(self.probe.clone());
        for tick in 0..self.ticks {
            let time = Time::from_ticks(tick);
            if time > end {
                break;
            }
            for (position, base) in [0, 1, 1, 2, 3].into_iter().enumerate() {
                if !signals.iter().any(|signal| signal.index() == base) {
                    continue;
                }
                // Unselected channels need not be validated by this narrow reader.
                if self.fail_at == Some((tick, position)) {
                    return Err(Error::Backend {
                        backend: "generated".into(),
                        operation: "read",
                        message: "injected source failure".into(),
                    });
                }
                let (advances, budget) = {
                    let mut counts = self.probe.lock().unwrap();
                    counts.advances += 1;
                    counts.payload_decodes += u64::from(base == 2);
                    counts.unused_decodes += u64::from(base == 3);
                    (counts.advances, counts.advance_budget)
                };
                // Assert outside the lock so failed regressions still release leases.
                assert!(
                    budget.is_none_or(|limit| advances <= limit),
                    "advanced beyond logical early-stop budget"
                );
                let value = match base {
                    0 => ValueRef::Real(tick as f64),
                    1 => ValueRef::Event { occurrences: 1 },
                    2 => ValueRef::String(PAYLOAD),
                    3 => ValueRef::String("unused"),
                    _ => unreachable!(),
                };
                if let ControlFlow::Break(value) = visitor(base, time, value) {
                    return Ok(ControlFlow::Break(value));
                }
            }
        }
        Ok(ControlFlow::Continue(()))
    }
}
