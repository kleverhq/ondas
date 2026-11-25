#[cfg(feature = "fsdb-lib")]
#[allow(unsafe_code)] // The private, audited C ABI boundary only.
pub(crate) mod fsdb;
pub(crate) mod fst;
#[cfg(test)]
pub(crate) mod generated;
pub(crate) mod ghw;
#[cfg(feature = "shm-lib")]
#[allow(unsafe_code)] // Private C ABI; native calls are serialized and user callbacks run unlocked.
pub(crate) mod shm;
pub(crate) mod vcd;
#[cfg(feature = "vwdb-lib")]
#[allow(unsafe_code)] // Private C ABI with serialized native access.
pub(crate) mod vwdb;
#[cfg(feature = "wdb-lib")]
#[allow(unsafe_code)] // Private C ABI; native calls are serialized and callbacks run unlocked.
pub(crate) mod wdb;
#[cfg(feature = "wlf-lib")]
#[allow(unsafe_code)] // The private, audited C ABI boundary only.
pub(crate) mod wlf;

use std::{
    io::{BufRead, Seek},
    ops::ControlFlow,
};

pub(crate) trait Input: BufRead + Seek + Send + Sync {}
impl<T: BufRead + Seek + Send + Sync> Input for T {}

use crate::{Result, Signal, Time, ValueRef};

pub(crate) enum Reader {
    Fst(Box<fst::Reader>),
    Vcd(Box<vcd::Reader>),
    Ghw(Box<ghw::Reader>),
    #[cfg(feature = "fsdb-lib")]
    Fsdb(Box<fsdb::Reader>),
    #[cfg(feature = "shm-lib")]
    Shm(Box<shm::Reader>),
    #[cfg(feature = "vwdb-lib")]
    Vwdb(Box<vwdb::Reader>),
    #[cfg(feature = "wlf-lib")]
    Wlf(Box<wlf::Reader>),
    #[cfg(feature = "wdb-lib")]
    Wdb(Box<wdb::Reader>),
    #[cfg(test)]
    Generated(generated::Reader),
    #[cfg(test)]
    Memory {
        records: Vec<(usize, Time, crate::Value)>,
        fail_after: Option<usize>,
        failure: Option<fn() -> crate::Error>,
    },
}

impl Reader {
    #[inline]
    pub(crate) fn visit_after_read<B>(&mut self, visitor: impl FnOnce() -> B) -> B {
        #[cfg(feature = "vwdb-lib")]
        if let Self::Vwdb(reader) = self {
            return reader.visit_after_read(visitor);
        }
        visitor()
    }

    pub(crate) fn read<B>(
        &mut self,
        signals: &[Signal],
        end: Time,
        mut visitor: impl for<'v> FnMut(usize, Time, Result<Option<ValueRef<'v>>>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        if signals.is_empty() {
            return Ok(ControlFlow::Continue(()));
        }
        match self {
            Self::Fst(reader) => {
                reader.read(signals, Time::from_ticks(0), end, |index, time, value| {
                    visitor(index, time, Ok(Some(value)))
                })
            }
            Self::Vcd(reader) => reader.read(signals, end, |index, time, value| {
                visitor(index, time, Ok(Some(value)))
            }),
            Self::Ghw(reader) => reader.read(signals, end, |event| match event {
                ghw::Notification::Value { index, time, value } => {
                    visitor(index, time, Ok(Some(value)))
                }
                ghw::Notification::Tick(_) => ControlFlow::Continue(()),
            }),
            #[cfg(feature = "fsdb-lib")]
            Self::Fsdb(reader) => reader.read(signals, end, |index, time, value| {
                visitor(index, time, Ok(Some(value)))
            }),
            #[cfg(feature = "shm-lib")]
            Self::Shm(reader) => reader.read(signals, end, |index, time, value| {
                visitor(index, time, value.map(Some))
            }),
            #[cfg(feature = "vwdb-lib")]
            Self::Vwdb(reader) => reader.read(signals, Time::ZERO, end, visitor),
            #[cfg(feature = "wlf-lib")]
            Self::Wlf(reader) => reader.read(signals, end, |index, time, value| {
                visitor(index, time, value.map(Some))
            }),
            #[cfg(feature = "wdb-lib")]
            Self::Wdb(reader) => reader.read(signals, end, |index, time, value| {
                visitor(index, time, Ok(Some(value)))
            }),
            #[cfg(test)]
            Self::Generated(reader) => reader.read(signals, end, |index, time, value| {
                visitor(index, time, Ok(Some(value)))
            }),
            #[cfg(test)]
            Self::Memory {
                records,
                fail_after,
                failure,
            } => {
                for (position, (index, time, value)) in records.iter().enumerate() {
                    if *time > end {
                        break;
                    }
                    if *fail_after == Some(position) {
                        return Err(failure.map(|make| make()).unwrap_or_else(|| {
                            crate::Error::Backend {
                                backend: "memory".into(),
                                operation: "read",
                                message: "injected read failure".into(),
                            }
                        }));
                    }
                    if signals.iter().any(|signal| signal.index() == *index)
                        && let ControlFlow::Break(value) =
                            visitor(*index, *time, Ok(Some(value.as_ref())))
                    {
                        return Ok(ControlFlow::Break(value));
                    }
                }
                Ok(ControlFlow::Continue(()))
            }
        }
    }
}
