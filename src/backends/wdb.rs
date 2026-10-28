//! Private file-only adapter. Vendor calls and destruction use the same lock.
use crate::{
    BitRange, BitsRef, Direction, Encoding, Error, Format, Hierarchy, LogicDomain, Metadata,
    Packing, Result, Signal, Time, TimeSpan, TimeUnit, Timescale, ValueRef,
    hierarchy::{EnumerationData, ScopeData, VariableData},
};
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    io::Read,
    ops::ControlFlow,
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr::NonNull,
    sync::Mutex,
};
const BACKEND: &str = "wdb-lib";
static SDK: Mutex<()> = Mutex::new(());
fn lock() -> std::sync::MutexGuard<'static, ()> {
    SDK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
#[repr(C)]
#[derive(Default)]
struct Meta {
    last: u64,
    precision: i32,
    has_variables: u32,
}
#[repr(C)]
#[derive(Default)]
struct Declaration {
    id: u64,
    parent: usize,
    scope: u32,
    encoding: u32,
    width: u32,
    has_range: u32,
    is_constant: u32,
    direction: u32,
    signedness: u32,
    logic_domain: u32,
    packing: u32,
    name_syntax: u32,
    msb: i64,
    lsb: i64,
    name: *const c_char,
    reader_name: *const c_char,
    kind: *const c_char,
    type_name: *const c_char,
    definition: *const c_char,
    variants: usize,
    base_len: usize,
}
#[repr(C)]
#[derive(Default)]
struct Record {
    tick: u64,
    id: u64,
    encoding: u32,
    data: *const u8,
    len: usize,
}
const _: () = {
    assert!(std::mem::size_of::<Meta>() == 16);
    assert!(std::mem::offset_of!(Meta, precision) == 8);
    assert!(std::mem::size_of::<Declaration>() == 128);
    assert!(std::mem::offset_of!(Declaration, msb) == 56);
    assert!(std::mem::offset_of!(Declaration, reader_name) == 80);
    assert!(std::mem::offset_of!(Declaration, base_len) == 120);
    assert!(std::mem::size_of::<Record>() == 40);
    assert!(std::mem::offset_of!(Record, data) == 24);
    assert!(std::mem::offset_of!(Record, len) == 32);
};
unsafe extern "C" {
    fn ondas_wdb_open(
        path: *const c_char,
        metadata_only: c_int,
        out: *mut *mut c_void,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_wdb_close(reader: *mut c_void);
    fn ondas_wdb_metadata(reader: *mut c_void, out: *mut Meta);
    fn ondas_wdb_decl_count(reader: *mut c_void) -> usize;
    fn ondas_wdb_scope_count(reader: *mut c_void) -> usize;
    fn ondas_wdb_declaration(reader: *mut c_void, index: usize, out: *mut Declaration) -> c_int;
    fn ondas_wdb_enum_variant(
        reader: *mut c_void,
        declaration: usize,
        variant: usize,
        bits: *mut *const c_char,
        label: *mut *const c_char,
    ) -> c_int;
    fn ondas_wdb_begin(
        reader: *mut c_void,
        ids: *const u64,
        count: usize,
        start: u64,
        end: u64,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_wdb_sample_bits(
        reader: *mut c_void,
        id: u64,
        time: u64,
        lsb: u32,
        width: u32,
        out: *mut Record,
        changed: *mut c_int,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_wdb_next(
        reader: *mut c_void,
        out: *mut Record,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_wdb_end(reader: *mut c_void);
}
fn backend_error(message: impl Into<String>) -> Error {
    Error::Backend {
        backend: BACKEND.into(),
        operation: "decode",
        message: message.into(),
    }
}
fn status(code: c_int, error: &[c_char; 512], operation: &'static str) -> Result<()> {
    if code >= 0 {
        return Ok(());
    }
    // SAFETY: the shim terminates every error buffer with NUL.
    let message = unsafe { CStr::from_ptr(error.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    Err(if code == -4 {
        Error::UnsupportedFeature {
            format: Format::Wdb,
            backend: BACKEND.into(),
            operation,
            message,
        }
    } else {
        Error::Backend {
            backend: BACKEND.into(),
            operation,
            message,
        }
    })
}
// SAFETY: used only for shim-owned immutable strings while their owner is alive.
unsafe fn string(pointer: *const c_char) -> Result<Option<String>> {
    if pointer.is_null() {
        return Ok(None);
    }
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map(|s| Some(s.to_owned()))
        .map_err(|_| backend_error("non-UTF-8 native metadata"))
}
struct Handle(NonNull<c_void>);
// SAFETY: the supported reader owns its native session. Vendor calls, mutable
// traversal and destruction are serialized. Declaration tables stay immutable
// until close. No native pointer or callback escapes.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        let _lock = lock();
        // SAFETY: sole owner; the shim drops traversal state before the reader.
        unsafe { ondas_wdb_close(self.0.as_ptr()) };
    }
}
fn open_handle(path: &Path, metadata_only: bool) -> Result<Handle> {
    let operation = if metadata_only {
        "read metadata"
    } else {
        "open"
    };
    let malformed = || Error::Malformed {
        format: Format::Wdb,
        backend: BACKEND.into(),
        message: "invalid or truncated WDB container header".into(),
    };
    let mut file = std::fs::File::open(path)?;
    if file.metadata()?.len() < 0xc8 {
        return Err(malformed());
    }
    let mut header = [0; 24];
    file.read_exact(&mut header)?;
    if !header.starts_with(b"Xilinx WAVE DATABASE ") {
        return Err(malformed());
    }
    if header != *b"Xilinx WAVE DATABASE 01\0" {
        return Err(Error::UnsupportedFeature {
            format: Format::Wdb,
            backend: BACKEND.into(),
            operation,
            message: "unsupported WDB container version".into(),
        });
    }
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "NUL in WDB path",
        ))
    })?;
    let mut raw = std::ptr::null_mut();
    let mut error = [0; 512];
    {
        let _lock = lock();
        // SAFETY: caller-owned buffers and C string; the shim copies retained state.
        let code = unsafe {
            ondas_wdb_open(
                path.as_ptr(),
                metadata_only.into(),
                &mut raw,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        status(code, &error, operation)?;
    }
    Ok(Handle(
        NonNull::new(raw).ok_or_else(|| backend_error("null native reader"))?,
    ))
}
fn metadata(meta: Meta, source_name: String) -> Result<Metadata> {
    let exponent = meta.precision;
    if !(-21..=2).contains(&exponent) {
        return Err(backend_error("unsupported native timescale exponent"));
    }
    let unit = match exponent.div_euclid(3) * 3 {
        0 => TimeUnit::Second,
        -3 => TimeUnit::Millisecond,
        -6 => TimeUnit::Microsecond,
        -9 => TimeUnit::Nanosecond,
        -12 => TimeUnit::Picosecond,
        -15 => TimeUnit::Femtosecond,
        -18 => TimeUnit::Attosecond,
        -21 => TimeUnit::Zeptosecond,
        _ => return Err(backend_error("unsupported native timescale exponent")),
    };
    Ok(Metadata {
        source_name,
        timescale: Some(Timescale::new(
            10u32.pow(exponent.rem_euclid(3) as u32),
            unit,
        )),
        time_span: (meta.has_variables != 0)
            .then(|| TimeSpan::new(Time::ZERO, Time::from_ticks(meta.last))),
        writer: None,
        date: None,
        comments: Vec::new(),
    })
}
pub(crate) fn read_metadata(path: &Path, name: String) -> Result<Metadata> {
    let handle = open_handle(path, true)?;
    let mut meta = Meta::default();
    // SAFETY: live owner; this reads only the immutable shim-owned descriptor.
    unsafe { ondas_wdb_metadata(handle.0.as_ptr(), &mut meta) };
    metadata(meta, name)
}
struct Traversal<'a>(&'a mut Handle);
impl Drop for Traversal<'_> {
    fn drop(&mut self) {
        let _lock = lock();
        // SAFETY: live borrowed owner, also on a stop, error or Rust callback panic.
        unsafe { ondas_wdb_end(self.0.0.as_ptr()) };
    }
}
pub(crate) struct Reader {
    handle: Handle,
    ids: Vec<u64>,
    indices: Vec<usize>,
    encodings: Vec<Encoding>,
}
impl Reader {
    pub(crate) fn open(path: &Path, name: String) -> Result<(Self, Hierarchy, Metadata)> {
        let handle = open_handle(path, false)?;
        let mut scopes = Vec::new();
        let mut variables = Vec::new();
        let mut ids = Vec::new();
        let mut indices;
        let mut encodings = Vec::new();
        let raw = handle.0.as_ptr();
        let mut meta = Meta::default();
        // SAFETY: all indices come from this immutable shim declaration table.
        // These getters read only shim-owned immutable data; no vendor call runs.
        // The owner stays alive while the strings are copied into Rust storage.
        unsafe {
            ondas_wdb_metadata(raw, &mut meta);
            let count = ondas_wdb_decl_count(raw);
            let scope_count = ondas_wdb_scope_count(raw);
            if scope_count > count {
                return Err(backend_error("invalid native scope count"));
            }
            indices = vec![usize::MAX; count];
            let mut parents = vec![None; count];
            scopes.reserve(scope_count);
            variables.reserve(count - scope_count);
            for (index, scope_index) in parents.iter_mut().enumerate() {
                let mut d = Declaration::default();
                if ondas_wdb_declaration(raw, index, &mut d) != 0 {
                    return Err(backend_error("invalid native declaration index"));
                }
                let parent = (d.parent != usize::MAX).then_some(d.parent);
                let declaration_name_text =
                    string(d.name)?.ok_or_else(|| backend_error("missing native name"))?;
                let (name, name_was_escaped) =
                    declaration_name(declaration_name_text, d.name_syntax, d.base_len)?;
                let reader_name = string(d.reader_name)?;
                let kind = declaration_kind(string(d.kind)?.unwrap_or_default(), d.encoding);
                if d.scope != 0 {
                    *scope_index = Some(scopes.len());
                    scopes.push(ScopeData {
                        name,
                        name_was_escaped,
                        parent,
                        kind,
                        definition_name: string(d.definition)?,
                        packing: match d.packing {
                            1 => Some(Packing::Packed),
                            2 => Some(Packing::Unpacked),
                            _ => None,
                        },
                        is_hidden: false,
                    });
                    continue;
                }
                let encoding = match d.encoding {
                    1 => Encoding::Bits { width: d.width },
                    2 => Encoding::Real,
                    3 => Encoding::String,
                    _ => Encoding::Unsupported,
                };
                let encoding_index = usize::try_from(d.id)
                    .ok()
                    .and_then(|id| indices.get_mut(id))
                    .ok_or_else(|| backend_error("invalid native history identity"))?;
                if *encoding_index == usize::MAX {
                    *encoding_index = encodings.len();
                    encodings.push(encoding);
                    ids.push(d.id);
                }
                let signal = *encoding_index;
                if encodings[signal] != encoding {
                    return Err(backend_error("incompatible native alias encodings"));
                }
                let type_name = string(d.type_name)?;
                let mut variants = Vec::with_capacity(d.variants);
                for variant in 0..d.variants {
                    let (mut bits, mut label) = (std::ptr::null(), std::ptr::null());
                    if ondas_wdb_enum_variant(raw, index, variant, &mut bits, &mut label) != 0 {
                        return Err(backend_error("invalid native enum index"));
                    }
                    variants.push((
                        string(bits)?.ok_or_else(|| backend_error("missing enum code"))?,
                        string(label)?.ok_or_else(|| backend_error("missing enum label"))?,
                    ));
                }
                let enumeration = if variants.is_empty() {
                    None
                } else {
                    EnumerationData::for_encoding(type_name.clone(), variants, encoding)
                };
                variables.push(VariableData {
                    reader_name,
                    name,
                    name_was_escaped,
                    parent,
                    kind,
                    direction: match d.direction {
                        1 => Direction::Input,
                        2 => Direction::Output,
                        3 => Direction::InOut,
                        4 => Direction::Buffer,
                        5 => Direction::Linkage,
                        _ => Direction::Unknown,
                    },
                    range: (d.has_range != 0).then(|| BitRange::new(d.msb, d.lsb)),
                    is_constant: d.is_constant != 0,
                    type_name,
                    signedness: None,
                    logic_domain: match d.logic_domain {
                        1 => Some(LogicDomain::TwoState),
                        2 => Some(LogicDomain::FourState),
                        3 => Some(LogicDomain::NineState),
                        _ => None,
                    },
                    enumeration,
                    signal: Some(signal),
                });
            }
            if scopes.len() != scope_count {
                return Err(backend_error("inconsistent native scope count"));
            }
            for parent in scopes
                .iter_mut()
                .map(|scope| &mut scope.parent)
                .chain(variables.iter_mut().map(|variable| &mut variable.parent))
            {
                if let Some(index) = *parent {
                    *parent = Some(
                        parents
                            .get(index)
                            .copied()
                            .flatten()
                            .ok_or_else(|| backend_error("native parent is not a scope"))?,
                    );
                }
            }
        }
        let metadata = metadata(meta, name)?;
        let hierarchy = Hierarchy::new(scopes, variables, encodings.clone());
        Ok((
            Self {
                handle,
                ids,
                indices,
                encodings,
            },
            hierarchy,
            metadata,
        ))
    }
    pub(crate) fn sample_bits(
        &mut self,
        bases: &[Signal],
        signals: &[Signal],
        time: Time,
    ) -> Result<Vec<crate::Sample>> {
        let ids: Vec<_> = bases
            .iter()
            .map(|signal| self.ids[signal.index()])
            .collect();
        let traversal = Traversal(&mut self.handle);
        let mut error = [0; 512];
        let _lock = lock();
        // SAFETY: live exclusive owner; validated IDs and caller-owned error storage.
        let code = unsafe {
            ondas_wdb_begin(
                traversal.0.0.as_ptr(),
                ids.as_ptr(),
                ids.len(),
                0,
                time.ticks(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        status(code, &error, "begin read")?;
        let mut samples = Vec::with_capacity(signals.len());
        for &signal in signals {
            let width = signal
                .width()
                .ok_or_else(|| backend_error("point sample requires bits"))?;
            let mut record = Record::default();
            let mut changed = 0;
            // SAFETY: selected validated signal/projection; the shim owns the
            // result until its next operation. Copy before unlocking or reentry.
            let code = unsafe {
                ondas_wdb_sample_bits(
                    traversal.0.0.as_ptr(),
                    self.ids[signal.index()],
                    time.ticks(),
                    signal.lsb(),
                    width,
                    &mut record,
                    &mut changed,
                    error.as_mut_ptr(),
                    error.len(),
                )
            };
            status(code, &error, "sample")?;
            samples.push(if code == 0 {
                crate::Sample::Missing { signal }
            } else {
                if record.encoding != 1
                    || record.len != width as usize
                    || record.data.is_null()
                    || record.tick > time.ticks()
                {
                    return Err(backend_error("invalid native point bit buffer"));
                }
                // SAFETY: exactly width native-validated ASCII bits, copied
                // while the SDK lock and live owner protect the borrowed buffer.
                let bytes = unsafe { std::slice::from_raw_parts(record.data, record.len) };
                crate::Sample::Value {
                    signal,
                    value: crate::Value::Bits(BitsRef::from_validated_ascii(bytes).to_owned()),
                    changed_at: (changed != 0).then_some(Time::from_ticks(record.tick)),
                }
            });
        }
        Ok(samples)
    }

    pub(crate) fn read<B>(
        &mut self,
        signals: &[Signal],
        end: Time,
        visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        self.read_from(signals, Time::ZERO, end, visitor)
    }

    // Nonzero starts use the query engine's normalized entering state.
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
        let ids: Vec<_> = signals
            .iter()
            .map(|signal| self.ids[signal.index()])
            .collect();
        let traversal = Traversal(&mut self.handle);
        let mut error = [0; 512];
        {
            let _lock = lock();
            // SAFETY: live exclusive owner; the shim copies the validated selection.
            let code = unsafe {
                ondas_wdb_begin(
                    traversal.0.0.as_ptr(),
                    ids.as_ptr(),
                    ids.len(),
                    start.ticks(),
                    end.ticks(),
                    error.as_mut_ptr(),
                    error.len(),
                )
            };
            status(code, &error, "begin read")?;
        }
        let mut bytes = Vec::new();
        let mut previous = 0;
        loop {
            let mut record = Record::default();
            {
                let _lock = lock();
                // SAFETY: live traversal and caller-owned descriptors. Copy borrowed
                // bytes before any further SDK operation or callback.
                let code = unsafe {
                    ondas_wdb_next(
                        traversal.0.0.as_ptr(),
                        &mut record,
                        error.as_mut_ptr(),
                        error.len(),
                    )
                };
                status(code, &error, "read")?;
                if code == 0 {
                    break;
                }
                if record.len > isize::MAX as usize || (record.len != 0 && record.data.is_null()) {
                    return Err(backend_error("invalid native value buffer"));
                }
                bytes.clear();
                if record.len != 0 {
                    bytes.extend_from_slice(unsafe {
                        std::slice::from_raw_parts(record.data, record.len)
                    });
                }
            }
            if record.tick < previous || record.tick > end.ticks() {
                return Err(backend_error("invalid native chronology"));
            }
            previous = record.tick;
            let index = usize::try_from(record.id)
                .ok()
                .and_then(|id| self.indices.get(id))
                .copied()
                .filter(|index| *index != usize::MAX)
                .ok_or_else(|| backend_error("unknown native history"))?;
            let value = match (self.encodings[index], record.encoding) {
                (Encoding::Bits { width }, 1) if bytes.len() == width as usize => {
                    ValueRef::Bits(BitsRef::from_validated_ascii(&bytes))
                }
                (Encoding::Real, 2) if bytes.len() == 8 => {
                    ValueRef::Real(f64::from_ne_bytes(bytes.as_slice().try_into().unwrap()))
                }
                (Encoding::String, 3) => ValueRef::String(crate::value::ascii_string(
                    &bytes,
                    Format::Wdb,
                    BACKEND,
                    Time::from_ticks(record.tick),
                )?),
                _ => return Err(backend_error("native value disagrees with declaration")),
            };
            if let ControlFlow::Break(value) = visitor(index, Time::from_ticks(record.tick), value)
            {
                return Ok(ControlFlow::Break(value));
            }
        }
        Ok(ControlFlow::Continue(()))
    }
}

fn declaration_kind(mut kind: String, encoding: u32) -> String {
    if encoding == 3 {
        return "string".into();
    }
    kind.make_ascii_lowercase();
    match kind.as_str() {
        "enumeration" => "enum".into(),
        "float" => "real".into(),
        "scope" | "logic" | "bit" | "integer" | "real" | "array" | "record" | "string"
        | "event" => kind,
        _ => format!("wdb:{kind}"),
    }
}

fn declaration_name(mut name: String, syntax: u32, base_len: usize) -> Result<(String, bool)> {
    let boundary = if base_len == 0 { name.len() } else { base_len };
    let base = name
        .get(..boundary)
        .ok_or_else(|| backend_error("invalid native array name boundary"))?;
    if syntax == 2 && base.len() >= 2 && base.starts_with('\\') && base.ends_with('\\') {
        let logical = base[1..base.len() - 1].replace("\\\\", "\\");
        name.replace_range(..boundary, &logical);
        return Ok((name, false));
    }
    if syntax != 1 || !base.starts_with('\\') || !base.ends_with(|c: char| c.is_ascii_whitespace())
    {
        return Ok((name, false));
    }
    let end = base
        .trim_end_matches(|c: char| c.is_ascii_whitespace())
        .len();
    name.replace_range(end..boundary, "");
    name.remove(0);
    Ok((name, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_native_precision_returns_an_error() {
        for precision in [i32::MIN, -22, 3, i32::MAX] {
            assert!(
                metadata(
                    Meta {
                        precision,
                        ..Meta::default()
                    },
                    String::new()
                )
                .is_err()
            );
        }
    }

    #[test]
    #[ignore = "requires the selected reader and installed WDB enum fixture"]
    fn wdb_invalid_native_metadata_indices() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/wdb/wdb0305-enum-core/waveform.wdb");
        let handle = open_handle(&path, false).unwrap();
        let raw = handle.0.as_ptr();
        let mut declaration = Declaration::default();
        let (mut bits, mut label) = (std::ptr::null(), std::ptr::null());
        // SAFETY: a live immutable table and valid output buffers. The invalid
        // indices must return status without a C++ exception crossing the ABI.
        unsafe {
            let count = ondas_wdb_decl_count(raw);
            assert!(count > 0);
            assert_eq!(ondas_wdb_declaration(raw, count, &mut declaration), -1);
            assert_eq!(ondas_wdb_declaration(raw, usize::MAX, &mut declaration), -1);
            assert_eq!(
                ondas_wdb_enum_variant(raw, usize::MAX, 0, &mut bits, &mut label),
                -1
            );
            assert_eq!(
                ondas_wdb_enum_variant(raw, 0, usize::MAX, &mut bits, &mut label),
                -1
            );
            assert_eq!(ondas_wdb_declaration(raw, 0, &mut declaration), 0);
            assert!(!declaration.name.is_null());
        }
    }
}
