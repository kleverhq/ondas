//! Private file-only WLF adapter. Vendor objects stay on a process-wide worker.
mod worker;
use crate::{
    Direction, Encoding, Error, Hierarchy, LogicDomain, Metadata, Result, Signal, Signedness, Time,
    ValueRef,
    hierarchy::{EnumerationData, ScopeData, VariableData},
};
use std::{
    collections::HashMap, ffi::CString, ops::ControlFlow, os::unix::ffi::OsStrExt, path::Path,
};
fn backend_error(message: impl Into<String>) -> Error {
    Error::Backend {
        backend: "wlf-lib".into(),
        operation: "access",
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
            let (name, reader_name) = declaration_name(d.name, d.verilog_name, d.array_base_len)?;
            let name_was_escaped = reader_name.is_some();
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
                    name,
                    name_was_escaped,
                    parent,
                    kind: d.kind,
                    definition_name: d.definition,
                    packing: None,
                    is_hidden: false,
                });
            } else {
                let encoding = if d.id < 0 {
                    d.encoding
                } else {
                    archive_encodings[&d.id]
                };
                let signal = if d.id < 0 {
                    None
                } else {
                    Some(*identities.entry(d.id).or_insert_with(|| {
                        let signal = encodings.len();
                        encodings.push(encoding);
                        declarations.push(index);
                        signal
                    }))
                };
                variables.push(VariableData {
                    reader_name,
                    name,
                    name_was_escaped,
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
                    enumeration: EnumerationData::for_encoding(None, d.variants, encoding),
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
        // Repeated bases share one slot; keep the first slot for a repeated base,
        // as the previous linear scan did.
        let mut slots: HashMap<usize, usize> = HashMap::new();
        for (slot, base) in bases.iter().enumerate() {
            slots.entry(base.index()).or_insert(slot);
        }
        let projections = signals
            .iter()
            .map(|&signal| (slots[&signal.index()], signal))
            .collect();
        worker::sample_bits(self.session.0, indices, projections, time)
    }
    pub(crate) fn read<B>(
        &mut self,
        signals: &[Signal],
        end: Time,
        visitor: impl for<'v> FnMut(usize, Time, Result<ValueRef<'v>>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        self.read_from(signals, Time::ZERO, end, visitor)
    }
    pub(crate) fn read_from<B>(
        &mut self,
        signals: &[Signal],
        start: Time,
        end: Time,
        mut visitor: impl for<'v> FnMut(usize, Time, Result<ValueRef<'v>>) -> ControlFlow<B>,
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
            if let ControlFlow::Break(value) =
                batch.visit(|selection, tick, value| match value {
                    Ok(value) => visitor(signals[selection].index(), tick, Ok(value.as_ref())),
                    Err(error) => visitor(signals[selection].index(), tick, Err(error)),
                })?
            {
                return Ok(ControlFlow::Break(value));
            }
            if finished {
                break;
            }
        }
        Ok(ControlFlow::Continue(()))
    }
}

// Native Verilog identifiers include their terminating whitespace. A name
// without that terminator is not evidence of Verilog escape syntax (for
// example, VHDL extended identifiers and VCD-imported WLF names).
fn declaration_name(
    name: String,
    verilog: bool,
    base_len: usize,
) -> Result<(String, Option<String>)> {
    if base_len == 0 {
        return Ok(logical_name(name, verilog));
    }
    let (base, suffix) = name
        .split_at_checked(base_len)
        .ok_or_else(|| backend_error("invalid WLF array component boundary"))?;
    let (logical, spelling) = logical_name(base.to_owned(), verilog);
    if spelling.is_some() {
        Ok((format!("{logical}{suffix}"), Some(name)))
    } else {
        Ok((name, None))
    }
}
fn logical_name(name: String, verilog: bool) -> (String, Option<String>) {
    if verilog && name.starts_with('\\') && name.ends_with(|c: char| c.is_ascii_whitespace()) {
        let logical = name[1..]
            .trim_end_matches(|c: char| c.is_ascii_whitespace())
            .to_owned();
        (logical, Some(name))
    } else {
        (name, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wlf_array_elements_normalize_only_the_source_component() {
        for (base, logical) in [
            ("\\a.b ", "a.b"),
            ("\\literal[7:0]\t", "literal[7:0]"),
            ("\\\\leading ", r"\leading"),
        ] {
            for suffix in ["[0]", "[1][2]"] {
                let raw = format!("{base}{suffix}");
                assert_eq!(
                    declaration_name(raw.clone(), true, base.len()).unwrap(),
                    (format!("{logical}{suffix}"), Some(raw.clone()))
                );
                assert_eq!(
                    declaration_name(raw.clone(), false, base.len()).unwrap(),
                    (raw, None)
                );
            }
        }
        assert!(declaration_name("é[0]".into(), true, 1).is_err());
        assert!(declaration_name("a[0]".into(), true, 99).is_err());
    }
    #[test]
    fn wlf_identifier_syntax_is_separate_from_literal_characters() {
        for (raw, expected) in [
            ("\\plain ", "plain"),
            ("\\with.dot ", "with.dot"),
            ("\\with/slash\t", "with/slash"),
            ("\\literal[7:0] ", "literal[7:0]"),
            ("\\\\leading ", r"\leading"),
            ("\\a\\b ", r"a\b"),
        ] {
            assert_eq!(
                logical_name(raw.into(), true),
                (expected.into(), Some(raw.into()))
            );
            assert_eq!(logical_name(raw.into(), false), (raw.into(), None));
        }
        for raw in [r"\with.do\", r"\leading", "space in name", r"\VHDL name\"] {
            assert_eq!(logical_name(raw.into(), true), (raw.into(), None));
        }
    }
    use crate::{Sample, Value};
    #[test]
    #[ignore = "requires Questa and installed string fixture; run just conformance-wlf"]
    fn wlf_late_string_error_crosses_native_continuation() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/wlf/wlf0072-strings-and-names/waveform.wlf");
        let (reader, hierarchy, _) = Reader::open(&path, "error continuation".into()).unwrap();
        let bits = hierarchy.signal("top.ascending").unwrap();
        let text = hierarchy.signal("top.message").unwrap();
        let signals = std::iter::repeat_n(bits, 2048)
            .chain(std::iter::once(text))
            .collect::<Vec<_>>();
        let _traversal = Traversal(reader.session.0);
        worker::begin(
            reader.session.0,
            signals
                .iter()
                .map(|signal| reader.declarations[signal.index()])
                .collect(),
            Time::ZERO,
            Time::from_ticks(7),
        )
        .unwrap();
        let mut batches = 0;
        loop {
            let batch = worker::next(reader.session.0).unwrap();
            batches += 1;
            let mut ticks = std::collections::BTreeMap::new();
            for record in &batch.records {
                *ticks.entry(record.tick.ticks()).or_insert(0) += 1;
            }
            eprintln!("batch={batches}, ticks={ticks:?}, error={:?}", batch.error);
            if batches == 1 {
                assert_eq!(batch.records.len(), 2049);
                assert_eq!(ticks.keys().copied().collect::<Vec<_>>(), [0]);
            }
            if let Some(error) = batch.error {
                assert!(matches!(
                    error,
                    worker::Failure::Decode { error: Error::NonAsciiString { format: crate::Format::Wlf, time, .. }, .. }
                        if time == Time::from_ticks(4)
                ));
                break;
            }
            assert!(!batch.finished, "the selected invalid String must fail");
        }
        assert!(batches > 1, "exercise real native continuation");
    }

    #[test]
    #[ignore = "requires Questa and installed history fixture; run just conformance-wlf"]
    fn wlf_batch_continuations_keep_cold_values_and_sparse_updates() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/wlf/wlf0075-history-long/waveform.wlf");
        let (reader, hierarchy, _) = Reader::open(&path, "continuations".into()).unwrap();
        let mut signals = vec![hierarchy.signal("top.clock").unwrap()];
        signals.extend((0..64).map(|i| hierarchy.signal(&format!("top.word_{i:02}")).unwrap()));
        for _ in 0..2 {
            let _traversal = Traversal(reader.session.0);
            worker::begin(
                reader.session.0,
                signals
                    .iter()
                    .map(|signal| reader.declarations[signal.index()])
                    .collect(),
                Time::ZERO,
                Time::from_ticks(600),
            )
            .unwrap();
            let mut rows = vec![Vec::new(); signals.len()];
            let mut batches = 0;
            loop {
                let batch = worker::next(reader.session.0).unwrap();
                batches += 1;
                let finished = batch.finished;
                let _ = batch
                    .visit(|selection, tick, value| {
                        let Value::Bits(value) = value.unwrap() else {
                            panic!("expected Bits");
                        };
                        rows[selection].push((tick.ticks(), value.as_ref().to_string()));
                        ControlFlow::<()>::Continue(())
                    })
                    .unwrap();
                if finished {
                    break;
                }
            }
            assert!(batches > 1, "exercise native continuation boundaries");
            let clock = (0..=600)
                .map(|tick| (tick, (tick % 2).to_string()))
                .collect::<Vec<_>>();
            assert_eq!(rows[0], clock);
            for (index, row) in rows[1..].iter().enumerate() {
                let expected = (0..=600)
                    .step_by(16)
                    .map(|tick| (tick, format!("{:032b}", tick / 16 + index as u64)))
                    .collect::<Vec<_>>();
                assert_eq!(*row, expected, "word {index}");
            }
        }
    }

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
                    .visit(|_, _, value| {
                        assert!(value.is_ok());
                        records += 1;
                        ControlFlow::<()>::Continue(())
                    })
                    .unwrap();
                let (count, bytes, pack) = worker::resources(id).unwrap();
                assert_eq!(count, 1);
                assert!(bytes <= 128 * 1024, "bounded completed-tick storage");
                assert!(pack);
                if finished {
                    break;
                }
            }
            assert!(records > 2048);
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
