//! Private file-only WLF adapter. Vendor objects stay on a process-wide worker.
mod worker;
use crate::{
    Direction, Encoding, Error, Hierarchy, LogicDomain, Metadata, Result, Signal, Signedness, Time,
    ValueRef,
    hierarchy::{ScopeData, VariableData},
};
use std::{
    collections::HashMap, ffi::CString, ops::ControlFlow, os::unix::ffi::OsStrExt, path::Path,
};
fn backend_error(message: impl Into<String>) -> Error {
    Error::Backend {
        backend: "wlf-lib".into(),
        operation: "read WLF",
        message: message.into(),
    }
}
fn filename(path: &Path) -> Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "NUL in WLF path",
        ))
    })
}
pub(crate) fn read_metadata(path: &Path, source_name: String) -> Result<Metadata> {
    worker::read_metadata(filename(path)?, source_name)
}
struct Session(u64);
impl Drop for Session {
    fn drop(&mut self) {
        worker::close(self.0);
    }
}
struct Traversal(u64);
impl Drop for Traversal {
    fn drop(&mut self) {
        worker::end(self.0);
    }
}
pub(crate) struct Reader {
    session: Session,
    declarations: Vec<usize>,
}
impl Reader {
    pub(crate) fn open(path: &Path, source_name: String) -> Result<(Self, Hierarchy, Metadata)> {
        let opened = worker::open(filename(path)?, source_name)?;
        let session = Session(opened.id);
        let mut scopes = Vec::new();
        let mut variables = Vec::new();
        let mut parents = HashMap::new();
        let mut identities = HashMap::new();
        let mut declarations = Vec::new();
        let mut encodings = Vec::new();
        // A shared archive with incompatible declaration widths/classes is not queryable.
        let mut archive_encodings = HashMap::new();
        for d in &opened.declarations {
            if !d.scope && d.id >= 0 {
                archive_encodings
                    .entry(d.id)
                    .and_modify(|old| {
                        if *old != d.encoding {
                            *old = Encoding::Unsupported;
                        }
                    })
                    .or_insert(d.encoding);
            }
        }
        for (index, d) in opened.declarations.into_iter().enumerate() {
            let parent = d
                .parent
                .map(|parent| {
                    parents
                        .get(&parent)
                        .copied()
                        .ok_or_else(|| backend_error("WLF parent is not a scope"))
                })
                .transpose()?;
            if d.scope {
                parents.insert(index, scopes.len());
                scopes.push(ScopeData {
                    name: d.name,
                    name_was_escaped: false,
                    parent,
                    kind: d.kind,
                    definition_name: d.definition,
                    packing: None,
                    is_hidden: false,
                });
            } else {
                let signal = if d.id < 0 {
                    None
                } else {
                    let encoding = archive_encodings[&d.id];
                    Some(*identities.entry(d.id).or_insert_with(|| {
                        let signal = encodings.len();
                        encodings.push(encoding);
                        declarations.push(index);
                        signal
                    }))
                };
                variables.push(VariableData {
                    reader_name: None,
                    name: d.name,
                    name_was_escaped: false,
                    parent,
                    kind: d.kind,
                    direction: match d.direction {
                        0 => Direction::Implicit,
                        1 => Direction::Input,
                        2 => Direction::Output,
                        3 => Direction::InOut,
                        _ => Direction::Unknown,
                    },
                    range: d.range,
                    is_constant: d.is_constant,
                    type_name: None,
                    enumeration: None,
                    signedness: match d.signedness {
                        1 => Some(Signedness::Unsigned),
                        2 => Some(Signedness::Signed),
                        _ => None,
                    },
                    logic_domain: match d.logic_domain {
                        1 => Some(LogicDomain::TwoState),
                        2 => Some(LogicDomain::FourState),
                        3 => Some(LogicDomain::NineState),
                        _ => None,
                    },
                    signal,
                });
            }
        }
        let hierarchy = Hierarchy::new(scopes, variables, encodings);
        Ok((
            Self {
                session,
                declarations,
            },
            hierarchy,
            opened.metadata,
        ))
    }
    pub(crate) fn sample_bits(
        &mut self,
        bases: &[Signal],
        signals: &[Signal],
        time: Time,
    ) -> Result<Vec<crate::Sample>> {
        let indices = bases.iter().map(|s| self.declarations[s.index()]).collect();
        let projections = signals
            .iter()
            .map(|&signal| {
                let index = bases
                    .iter()
                    .position(|base| base.index() == signal.index())
                    .unwrap();
                (index, signal)
            })
            .collect();
        worker::sample_bits(self.session.0, indices, projections, time)
    }
    pub(crate) fn read<B>(
        &mut self,
        signals: &[Signal],
        end: Time,
        visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        self.read_from(signals, Time::ZERO, end, visitor)
    }
    pub(crate) fn read_from<B>(
        &mut self,
        signals: &[Signal],
        start: Time,
        end: Time,
        mut visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        if signals.is_empty() || start > end {
            return Ok(ControlFlow::Continue(()));
        }
        let _traversal = Traversal(self.session.0);
        let indices = signals
            .iter()
            .map(|s| self.declarations[s.index()])
            .collect();
        worker::begin(self.session.0, indices, start, end)?;
        loop {
            let batch = worker::next(self.session.0)?;
            let finished = batch.finished;
            if let ControlFlow::Break(value) = batch.visit(|record| {
                visitor(
                    signals[record.selection].index(),
                    record.tick,
                    record.value.as_ref(),
                )
            })? {
                return Ok(ControlFlow::Break(value));
            }
            if finished {
                break;
            }
        }
        Ok(ControlFlow::Continue(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Sample, Value};
    #[test]
    #[ignore = "requires Questa and installed fixtures"]
    fn wlf_counter_boundaries() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/wlf/wlf0038-count4b-3-2-vsim/waveform.wlf");
        let mut wave = crate::open_with(&path, "wlf-lib").unwrap();
        assert_eq!(
            wave.hierarchy().scope("count4b").unwrap().kind(),
            "architecture"
        );
        let reset = wave.hierarchy().signal("count4b.reset").unwrap();
        let clock = wave.hierarchy().signal("count4b.clk").unwrap();
        for (signal, tick, digit) in [
            (reset, 0, "1"),
            (clock, 0, "u"),
            (clock, 50, "1"),
            (reset, 100, "0"),
            (clock, 100, "0"),
        ] {
            let Sample::Value {
                value: Value::Bits(bits),
                ..
            } = wave.sample(signal, Time::from_ticks(tick)).unwrap()
            else {
                panic!("expected bits")
            };
            assert_eq!(bits.as_ref().to_string(), digit);
        }
        assert_eq!(
            wave.hierarchy().variable("count4b.q").unwrap().range(),
            Some(crate::BitRange::new(3, 0))
        );
        assert_eq!(wave.metadata().time_span().unwrap().last().ticks(), 1000150);
        wave.sample(reset, Time::from_ticks(u64::MAX)).unwrap();
    }
    #[test]
    #[ignore = "requires Questa and installed fixtures"]
    fn wlf_selected_storage_and_error_cleanup() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/wlf/wlf0038-count4b-3-2-vsim/waveform.wlf");
        let (reader, hierarchy, _) = Reader::open(&path, "resources".into()).unwrap();
        let clock = hierarchy.signal("count4b.clk").unwrap();
        let id = reader.session.0;
        {
            let _guard = Traversal(id);
            worker::begin(
                id,
                vec![reader.declarations[clock.index()]],
                Time::ZERO,
                Time::from_ticks(1000150),
            )
            .unwrap();
            let mut records = 0;
            loop {
                let batch = worker::next(id).unwrap();
                let finished = batch.finished;
                let _ = batch
                    .visit(|_| {
                        records += 1;
                        ControlFlow::<()>::Continue(())
                    })
                    .unwrap();
                if matches!(records, 1 | 64 | 1024) {
                    let (count, bytes, pack) = worker::resources(id).unwrap();
                    assert_eq!(count, 1);
                    assert!(bytes <= 128 * 1024, "bounded completed-tick storage");
                    assert!(pack);
                }
                if finished {
                    break;
                }
            }
            assert!(records > 1024);
            let (count, bytes, pack) = worker::resources(id).unwrap();
            assert_eq!(count, 1);
            assert!(bytes <= 128 * 1024, "bounded completed-tick storage");
            assert!(pack);
            assert!(worker::begin(id, vec![usize::MAX], Time::ZERO, Time::ZERO).is_err());
        }
        assert_eq!(worker::resources(id).unwrap(), (0, 0, false));
        drop(reader);
        assert!(worker::resources(id).is_err());
    }
}
