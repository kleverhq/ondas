use std::{
    collections::HashMap,
    io::{BufRead, Seek},
    ops::ControlFlow,
};

use fst_reader::{
    FstArrayType, FstFilter, FstHierarchyEntry, FstPackType, FstReader, FstSignalHandle,
    FstSignalValue, FstVarDirection, FstVarType, FstVhdlVarType, ReadSignalsError, ReaderError,
};

use crate::{
    BitRange, BitsRef, Direction, Encoding, Error, Format, Hierarchy, Metadata, Packing, Result,
    Signal, Time, TimeSpan, TimeUnit, Timescale, ValueRef,
    hierarchy::{EnumerationData, ScopeData, VariableData},
};

pub(crate) trait Input: BufRead + Seek + Send + Sync {}
impl<T: BufRead + Seek + Send + Sync> Input for T {}

pub(crate) struct Reader {
    inner: FstReader<Box<dyn Input>>,
    handles: Vec<usize>,
    indices: HashMap<usize, usize>,
    encodings: Vec<Encoding>,
}

fn malformed(message: impl Into<String>) -> Error {
    Error::Malformed {
        format: Format::Fst,
        backend: "fst-native".into(),
        message: message.into(),
    }
}

fn reader_error(error: ReaderError) -> Error {
    match error {
        ReaderError::Io(error) if error.kind() != std::io::ErrorKind::UnexpectedEof => {
            Error::Io(error)
        }
        error => malformed(error.to_string()),
    }
}

impl Reader {
    pub(crate) fn open(
        input: Box<dyn Input>,
        source_name: String,
    ) -> Result<(Self, Hierarchy, Metadata)> {
        let mut inner = FstReader::open(input).map_err(reader_error)?;
        let header = inner.get_header();
        if header.start_time > header.end_time {
            return Err(malformed("reversed recorded time span"));
        }
        let mut metadata = Metadata {
            source_name,
            timescale: timescale(header.timescale_exponent),
            time_span: (header.var_count != 0).then(|| {
                TimeSpan::new(
                    Time::from_ticks(header.start_time),
                    Time::from_ticks(header.end_time),
                )
            }),
            writer: Some(header.version.clone()),
            date: Some(header.date.clone()),
            comments: Vec::new(),
        };
        let mut scopes = Vec::<ScopeData>::new();
        let mut variables = Vec::new();
        let mut stack = Vec::new();
        let mut scope_ids = HashMap::new();
        let mut indices = HashMap::new();
        let mut handles = Vec::new();
        let mut encodings = Vec::new();
        let mut enums = HashMap::new();
        let mut pending_enum = None;
        let mut pending_vhdl = None;
        let mut pending_packing = None;
        let mut error = None;
        inner
            .read_hierarchy(|entry| {
                if error.is_some() {
                    return;
                }
                match entry {
                    FstHierarchyEntry::Scope {
                        tpe,
                        name,
                        component,
                    } => {
                        let parent = stack.last().copied();
                        let kind = scope_kind(tpe);
                        let definition_name = (!component.is_empty()).then_some(component);
                        let packing = pending_packing.take();
                        let key = (parent, name.clone());
                        let id = if let Some(&id) = scope_ids.get(&key) {
                            let scope: &mut ScopeData = &mut scopes[id];
                            if scope.kind != kind
                                || (scope.definition_name.is_some()
                                    && definition_name.is_some()
                                    && scope.definition_name != definition_name)
                                || (scope.packing.is_some()
                                    && packing.is_some()
                                    && scope.packing != packing)
                            {
                                error = Some(malformed(format!(
                                    "conflicting scope declaration: {name}"
                                )));
                                return;
                            }
                            if scope.definition_name.is_none() {
                                scope.definition_name = definition_name;
                            }
                            if scope.packing.is_none() {
                                scope.packing = packing;
                            }
                            id
                        } else {
                            let id = scopes.len();
                            scopes.push(ScopeData {
                                name,
                                parent,
                                kind,
                                definition_name,
                                packing,
                            });
                            scope_ids.insert(key, id);
                            id
                        };
                        stack.push(id);
                        pending_enum = None;
                        pending_vhdl = None;
                    }
                    FstHierarchyEntry::UpScope => {
                        if stack.pop().is_none() {
                            error = Some(malformed("hierarchy scope stack underflow"));
                        }
                    }
                    FstHierarchyEntry::Var {
                        tpe,
                        direction,
                        name,
                        length,
                        handle,
                        ..
                    } => {
                        let encoding = match tpe {
                            FstVarType::Event => Encoding::Event,
                            FstVarType::GenericString => Encoding::String,
                            FstVarType::Real
                            | FstVarType::RealTime
                            | FstVarType::RealParameter
                            | FstVarType::ShortReal => Encoding::Real,
                            _ if length == 0 => Encoding::Unsupported,
                            _ => Encoding::Bits { width: length },
                        };
                        let index = if let Some(&index) = indices.get(&handle.get_index()) {
                            if encodings[index] != encoding {
                                error = Some(malformed(format!(
                                    "inconsistent alias encoding for handle {}",
                                    handle.get_index()
                                )));
                                return;
                            }
                            index
                        } else {
                            let index = handles.len();
                            indices.insert(handle.get_index(), index);
                            handles.push(handle.get_index());
                            encodings.push(encoding);
                            index
                        };
                        let (name, range) = declared_name(name, encoding);
                        let (type_name, vhdl_kind) =
                            pending_vhdl.take().unwrap_or((None, FstVhdlVarType::None));
                        let is_constant =
                            matches!(tpe, FstVarType::Parameter | FstVarType::RealParameter)
                                || vhdl_kind == FstVhdlVarType::Constant;
                        let kind = if vhdl_kind != FstVhdlVarType::None {
                            format!("{vhdl_kind:?}").to_ascii_lowercase()
                        } else {
                            var_kind(tpe)
                        };
                        let enumeration = pending_enum
                            .take()
                            .map(|(name, variants)| EnumerationData { name, variants });
                        variables.push(VariableData {
                            name,
                            parent: stack.last().copied(),
                            kind,
                            direction: direction_metadata(direction),
                            range,
                            is_constant,
                            type_name,
                            enumeration,
                            signal: Some(index),
                        });
                        pending_packing = None;
                    }
                    FstHierarchyEntry::Comment { string } => metadata.comments.push(string),
                    FstHierarchyEntry::VhdlVarInfo {
                        type_name,
                        var_type,
                        ..
                    } => {
                        pending_vhdl =
                            Some(((!type_name.is_empty()).then_some(type_name), var_type));
                    }
                    FstHierarchyEntry::EnumTable {
                        name,
                        handle,
                        mapping,
                    } => {
                        enums.insert(handle, ((!name.is_empty()).then_some(name), mapping));
                    }
                    FstHierarchyEntry::EnumTableRef { handle } => match enums.get(&handle) {
                        Some(table) => pending_enum = Some(table.clone()),
                        None => error = Some(malformed("unknown enumeration table reference")),
                    },
                    FstHierarchyEntry::Pack { pack_type, .. } => {
                        pending_packing = match pack_type {
                            FstPackType::None => None,
                            FstPackType::Packed => Some(Packing::Packed),
                            FstPackType::Unpacked => Some(Packing::Unpacked),
                            FstPackType::TaggedPacked => Some(Packing::TaggedPacked),
                        };
                    }
                    FstHierarchyEntry::Array { array_type, .. } => {
                        pending_packing = match array_type {
                            FstArrayType::None => None,
                            FstArrayType::Packed => Some(Packing::Packed),
                            FstArrayType::Unpacked => Some(Packing::Unpacked),
                            FstArrayType::Sparse => Some(Packing::Sparse),
                        };
                    }
                    FstHierarchyEntry::PathName { .. }
                    | FstHierarchyEntry::SourceStem { .. }
                    | FstHierarchyEntry::SVEnum { .. }
                    | FstHierarchyEntry::AttributeEnd => {}
                }
            })
            .map_err(reader_error)?;
        if let Some(error) = error {
            return Err(error);
        }
        if !stack.is_empty() {
            return Err(malformed("unclosed hierarchy scopes"));
        }
        let hierarchy = Hierarchy::new(scopes, variables, encodings.clone());
        Ok((
            Self {
                inner,
                handles,
                indices,
                encodings,
            },
            hierarchy,
            metadata,
        ))
    }

    pub(crate) fn read<B>(
        &mut self,
        signals: &[Signal],
        end: Time,
        mut visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        enum Stop<B> {
            Break(B),
            Error(Error),
        }
        let filter = FstFilter::new(
            0,
            end.ticks(),
            signals
                .iter()
                .map(|signal| FstSignalHandle::from_index(self.handles[signal.index()]))
                .collect(),
        );
        // Read from zero for reliable entering values, including variable-length strings.
        // Event callbacks are preserved verbatim; the reader can expose initialization at the first tick.
        let result = self.inner.read_signals(&filter, |time, handle, raw| {
            if time > end.ticks() {
                return Ok(());
            }
            let index = *self
                .indices
                .get(&handle.get_index())
                .ok_or_else(|| Stop::Error(malformed("unknown value handle")))?;
            let string;
            let value = match (self.encodings[index], raw) {
                (Encoding::Event, _) => ValueRef::Event,
                (Encoding::Real, FstSignalValue::Real(value)) => ValueRef::Real(value),
                (Encoding::Bits { width }, FstSignalValue::String(bytes)) => {
                    let bits = BitsRef::from_ascii(bytes)
                        .filter(|bits| bits.width() == width)
                        .ok_or_else(|| Stop::Error(malformed("invalid bit-vector value")))?;
                    ValueRef::Bits(bits)
                }
                (Encoding::String, FstSignalValue::String(bytes)) => {
                    // FST character bytes map reversibly to Unicode U+0000..U+00FF.
                    string = bytes
                        .iter()
                        .map(|&byte| char::from(byte))
                        .collect::<String>();
                    ValueRef::String(&string)
                }
                _ => {
                    return Err(Stop::Error(malformed(
                        "value encoding disagrees with hierarchy",
                    )));
                }
            };
            match visitor(index, Time::from_ticks(time), value) {
                ControlFlow::Continue(()) => Ok(()),
                ControlFlow::Break(value) => Err(Stop::Break(value)),
            }
        });
        match result {
            Ok(()) => Ok(ControlFlow::Continue(())),
            Err(ReadSignalsError::CallbackError(Stop::Break(value))) => {
                Ok(ControlFlow::Break(value))
            }
            Err(ReadSignalsError::CallbackError(Stop::Error(error))) => Err(error),
            Err(ReadSignalsError::ReadError(error)) => Err(reader_error(error)),
        }
    }
}

fn declared_name(name: String, encoding: Encoding) -> (String, Option<BitRange>) {
    if let Encoding::Bits { .. } = encoding
        && let Some((base, suffix)) = name.rsplit_once(" [")
        && let Some(suffix) = suffix.strip_suffix(']')
        && let Some((msb, lsb)) = suffix.split_once(':')
        && let (Ok(msb), Ok(lsb)) = (msb.parse::<i64>(), lsb.parse::<i64>())
    {
        return (base.into(), Some(BitRange::new(msb, lsb)));
    }
    (name, None)
}

fn var_kind(kind: FstVarType) -> String {
    match kind {
        FstVarType::GenericString => "string".into(),
        FstVarType::RealParameter => "real-parameter".into(),
        _ => format!("{kind:?}").to_ascii_lowercase(),
    }
}

fn scope_kind(kind: fst_reader::FstScopeType) -> String {
    use fst_reader::FstScopeType::*;
    match kind {
        VhdlForGenerate => return "for-generate".into(),
        VhdlIfGenerate => return "if-generate".into(),
        // Preserve the format-specific code used by the independent oracle.
        SvArray => return "fst:22".into(),
        _ => {}
    }
    let name = format!("{kind:?}");
    name.strip_prefix("Vhdl")
        .unwrap_or(&name)
        .to_ascii_lowercase()
}

fn direction_metadata(direction: FstVarDirection) -> Direction {
    match direction {
        FstVarDirection::Implicit => Direction::Implicit,
        FstVarDirection::Input => Direction::Input,
        FstVarDirection::Output => Direction::Output,
        FstVarDirection::InOut => Direction::InOut,
        FstVarDirection::Buffer => Direction::Buffer,
        FstVarDirection::Linkage => Direction::Linkage,
    }
}

fn timescale(exponent: i8) -> Option<Timescale> {
    let units = [
        (0, TimeUnit::Second),
        (-3, TimeUnit::Millisecond),
        (-6, TimeUnit::Microsecond),
        (-9, TimeUnit::Nanosecond),
        (-12, TimeUnit::Picosecond),
        (-15, TimeUnit::Femtosecond),
        (-18, TimeUnit::Attosecond),
        (-21, TimeUnit::Zeptosecond),
    ];
    let (base, unit) = units.into_iter().find(|(base, _)| exponent >= *base)?;
    Some(Timescale::new(
        10u32.checked_pow(u32::try_from(i16::from(exponent) - i16::from(base)).ok()?)?,
        unit,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compound_scope_kinds_are_canonical() {
        use fst_reader::FstScopeType::*;
        for (kind, expected) in [
            (VhdlForGenerate, "for-generate"),
            (VhdlIfGenerate, "if-generate"),
            (SvArray, "fst:22"),
            (VhdlArchitecture, "architecture"),
            (Module, "module"),
        ] {
            assert_eq!(scope_kind(kind), expected);
        }
    }

    #[test]
    fn real_parameter_kind_is_canonical() {
        assert_eq!(var_kind(FstVarType::RealParameter), "real-parameter");
    }

    #[test]
    fn ranges_require_an_explicit_separated_bit_suffix() {
        let (name, range) = declared_name("word [-2:-9]".into(), Encoding::Bits { width: 8 });
        assert_eq!(name, "word");
        assert_eq!(range, Some(BitRange::new(-2, -9)));
        assert_eq!(
            declared_name("word[7:0]".into(), Encoding::Bits { width: 8 }),
            ("word[7:0]".into(), None)
        );
        assert_eq!(
            declared_name("text [1:50]".into(), Encoding::String),
            ("text [1:50]".into(), None)
        );
    }

    #[test]
    fn exact_timescales_do_not_wrap_or_use_floating_point() {
        let scale = timescale(-14).unwrap();
        assert_eq!(scale.factor(), 10);
        assert_eq!(scale.unit(), TimeUnit::Femtosecond);
        assert!(timescale(-22).is_none());
        assert!(timescale(10).is_none());
        assert_eq!(timescale(9).unwrap().factor(), 1_000_000_000);
    }
}
