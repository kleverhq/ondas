pub(crate) mod fst;
pub(crate) mod vcd;

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
    #[cfg(test)]
    Memory {
        records: Vec<(usize, Time, crate::Value)>,
        fail_after: Option<usize>,
    },
}

impl Reader {
    pub(crate) fn read<B>(
        &mut self,
        signals: &[Signal],
        end: Time,
        visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        if signals.is_empty() {
            return Ok(ControlFlow::Continue(()));
        }
        match self {
            Self::Fst(reader) => reader.read(signals, end, visitor),
            Self::Vcd(reader) => reader.read(signals, end, visitor),
            #[cfg(test)]
            Self::Memory {
                records,
                fail_after,
            } => {
                let mut visitor = visitor;
                for (position, (index, time, value)) in records.iter().enumerate() {
                    if *time > end {
                        break;
                    }
                    if *fail_after == Some(position) {
                        return Err(crate::Error::Backend {
                            backend: "memory".into(),
                            operation: "read",
                            message: "injected read failure".into(),
                        });
                    }
                    if signals.iter().any(|signal| signal.index() == *index)
                        && let ControlFlow::Break(value) = visitor(*index, *time, value.as_ref())
                    {
                        return Ok(ControlFlow::Break(value));
                    }
                }
                Ok(ControlFlow::Continue(()))
            }
        }
    }
}
