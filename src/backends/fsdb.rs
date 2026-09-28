//! Private FSDB Reader adapter. The SDK only receives file paths.
use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, CString, c_char, c_int, c_void},
    ops::ControlFlow,
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr::NonNull,
    sync::{Mutex, MutexGuard},
};

use crate::{
    BitRange, BitsRef, Direction, Encoding, Error, Hierarchy, Metadata, Result, Signal, Time,
    TimeSpan, TimeUnit, Timescale, ValueRef,
    hierarchy::{EnumerationData, ScopeData, VariableData},
};

const BACKEND: &str = "fsdb-lib";
// ponytail: serialize SDK calls process-wide; relax only with documented SDK
// guarantees and measured contention. Never hold this lock across Rust visitors.
static SDK: Mutex<()> = Mutex::new(());
fn lock() -> MutexGuard<'static, ()> {
    SDK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[repr(C)]
#[derive(Default)]
struct Declaration {
    id: u64,
    entry: u32,
    encoding: u32,
    width: u32,
    direction: u32,
    is_constant: u32,
    has_range: u32,
    packing: u32,
    is_hidden: u32,
    msb: i64,
    lsb: i64,
    name: *const c_char,
    kind: *const c_char,
    definition: *const c_char,
    type_name: *const c_char,
    enum_count: usize,
}
#[repr(C)]
#[derive(Default)]
struct Meta {
    first: u64,
    last: u64,
    has_variables: u32,
    scale: *const c_char,
    writer: *const c_char,
    date: *const c_char,
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
unsafe extern "C" {
    fn ondas_fsdb_probe(
        path: *const c_char,
        yes: *mut c_int,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_fsdb_open(
        path: *const c_char,
        metadata_only: c_int,
        out: *mut *mut c_void,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_fsdb_close(reader: *mut c_void);
    fn ondas_fsdb_metadata(reader: *mut c_void, out: *mut Meta);
    fn ondas_fsdb_decl_count(reader: *mut c_void) -> usize;
    fn ondas_fsdb_declaration(reader: *mut c_void, index: usize, out: *mut Declaration);
    fn ondas_fsdb_enum_variant(
        reader: *mut c_void,
        declaration: usize,
        variant: usize,
        bits: *mut *const c_char,
        label: *mut *const c_char,
    );
    fn ondas_fsdb_begin(
        reader: *mut c_void,
        ids: *const u64,
        count: usize,
        start: u64,
        end: u64,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_fsdb_next(
        reader: *mut c_void,
        out: *mut Record,
        bits: *mut u8,
        bits_cap: usize,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_fsdb_sample_bits(
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
    fn ondas_fsdb_end(reader: *mut c_void);
}

fn backend_error(message: impl Into<String>) -> Error {
    Error::Backend {
        backend: BACKEND.into(),
        operation: "read FSDB",
        message: message.into(),
    }
}
fn status(code: c_int, error: &[c_char; 512]) -> Result<()> {
    if code < 0 {
        // SAFETY: all shim errors terminate the caller-owned buffer with NUL.
        return Err(backend_error(
            unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy(),
        ));
    }
    Ok(())
}
fn filename(path: &Path) -> Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "NUL in FSDB path",
        ))
    })
}
// SAFETY: only called on shim-owned NUL-terminated strings under SDK lock, while
// their owner lives. Copy before the next FFI operation or unlock.
unsafe fn string(ptr: *const c_char) -> Result<Option<String>> {
    if ptr.is_null() {
        return Ok(None);
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map(|s| Some(s.to_owned()))
        .map_err(|_| backend_error("non-UTF-8 FSDB hierarchy/metadata text"))
}

pub(crate) fn probe(path: &Path) -> Result<bool> {
    let path = filename(path)?;
    let _lock = lock();
    let mut yes = 0;
    let mut error = [0; 512];
    // SAFETY: valid caller-owned pointers; calls are serialized.
    let code =
        unsafe { ondas_fsdb_probe(path.as_ptr(), &mut yes, error.as_mut_ptr(), error.len()) };
    status(code, &error)?;
    Ok(yes != 0)
}

struct Handle(NonNull<c_void>);
// SAFETY: ffrOpenNonSharedObj creates an independently owned reader and is the
// SDK's documented multi-thread open API (FSDB Reader guide, ffrOpenNonSharedObj).
// No thread-local arguments/callback data are retained: callbacks point into the C++ heap
// owner. All calls, including destruction, are serialized by SDK. No SDK pointer
// escapes the adapter. Shared access cannot mutate without this lock, and reads
// additionally require &mut Reader. Rust visitors run after unlocking.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        let _lock = lock();
        // SAFETY: sole owner; shim releases cursor before values and reader.
        unsafe { ondas_fsdb_close(self.0.as_ptr()) };
    }
}
fn open_handle(path: &Path, metadata_only: bool) -> Result<Handle> {
    let path = filename(path)?;
    let mut raw = std::ptr::null_mut();
    let mut error = [0; 512];
    {
        let _lock = lock();
        // SAFETY: pointers are valid for the call; the shim owns any retained data.
        let code = unsafe {
            ondas_fsdb_open(
                path.as_ptr(),
                metadata_only.into(),
                &mut raw,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        status(code, &error)?;
    }
    Ok(Handle(
        NonNull::new(raw).ok_or_else(|| backend_error("null FSDB reader"))?,
    ))
}

// SAFETY: copied while the owner is alive and the SDK lock is held.
unsafe fn copy_metadata(meta: &Meta, source_name: String, has_variables: bool) -> Result<Metadata> {
    let scale = unsafe { string(meta.scale)? };
    let timescale = match scale.as_deref() {
        None | Some("") => None,
        Some(text) => {
            Some(timescale(text).ok_or_else(|| backend_error("unsupported FSDB timescale"))?)
        }
    };
    Ok(Metadata {
        source_name,
        timescale,
        time_span: has_variables
            .then(|| TimeSpan::new(Time::from_ticks(meta.first), Time::from_ticks(meta.last))),
        writer: unsafe { string(meta.writer)? },
        date: unsafe { string(meta.date)? },
        comments: Vec::new(),
    })
}

pub(crate) fn read_metadata(path: &Path, source_name: String) -> Result<Metadata> {
    let handle = open_handle(path, true)?;
    // The lock is declared after Handle so errors unlock before Handle::drop.
    let guard = lock();
    let mut meta = Meta::default();
    // SAFETY: live owner; SDK strings are copied before unlocking.
    let result = unsafe {
        ondas_fsdb_metadata(handle.0.as_ptr(), &mut meta);
        copy_metadata(&meta, source_name, meta.has_variables != 0)
    };
    drop(guard);
    result
}

struct Traversal<'a>(&'a mut Handle);
impl Drop for Traversal<'_> {
    fn drop(&mut self) {
        let _lock = lock();
        // SAFETY: borrowed live owner, including on early return or visitor panic.
        unsafe { ondas_fsdb_end(self.0.0.as_ptr()) };
    }
}

pub(crate) struct Reader {
    handle: Handle,
    ids: Vec<u64>,
    indices: HashMap<u64, usize>,
    encodings: Vec<Encoding>,
    #[cfg(test)]
    pub(crate) records_read: usize,
    #[cfg(test)]
    pub(crate) point_queries: usize,
}
impl Reader {
    pub(crate) fn open(path: &Path, source_name: String) -> Result<(Self, Hierarchy, Metadata)> {
        let handle = open_handle(path, false)?;
        let raw = handle.0.as_ptr();
        let mut scopes: Vec<ScopeData> = Vec::new();
        let mut variables = Vec::new();
        let mut seen_variables = HashSet::new();
        let mut stack = Vec::new();
        let mut scope_ids: HashMap<(Option<usize>, String), usize> = HashMap::new();
        let mut ids = Vec::new();
        let mut indices = HashMap::new();
        let mut encodings = Vec::new();
        // The lock is declared after Handle so errors unlock before Handle::drop.
        let guard = lock();
        // SAFETY: live owner, all descriptor pointers copied while locked. The
        // count and indices are from the same immutable shim hierarchy vector.
        let metadata = unsafe {
            let mut meta = Meta::default();
            ondas_fsdb_metadata(raw, &mut meta);
            for declaration_index in 0..ondas_fsdb_decl_count(raw) {
                let mut d = Declaration::default();
                ondas_fsdb_declaration(raw, declaration_index, &mut d);
                let name = string(d.name)?.unwrap_or_default();
                let kind = string(d.kind)?.unwrap_or_default();
                match d.entry {
                    0 => {
                        let parent = stack.last().copied();
                        let definition_name = string(d.definition)?;
                        let packing = match d.packing {
                            0 => None,
                            1 => Some(crate::Packing::Packed),
                            2 => Some(crate::Packing::Unpacked),
                            3 => Some(crate::Packing::TaggedPacked),
                            _ => return Err(backend_error("invalid FSDB packing")),
                        };
                        let key = (parent, name.clone());
                        let id = if let Some(&id) = scope_ids.get(&key) {
                            let old = &scopes[id];
                            let differences = [
                                (old.kind != kind)
                                    .then(|| format!("kind {:?} vs {:?}", old.kind, kind)),
                                (old.is_hidden != (d.is_hidden != 0)).then(|| {
                                    format!("hidden {} vs {}", old.is_hidden, d.is_hidden != 0)
                                }),
                                (old.packing != packing)
                                    .then(|| format!("packing {:?} vs {:?}", old.packing, packing)),
                                (old.definition_name.is_some()
                                    && definition_name.is_some()
                                    && old.definition_name != definition_name)
                                    .then(|| {
                                        format!(
                                            "definition {:?} vs {:?}",
                                            old.definition_name, definition_name
                                        )
                                    }),
                            ]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>();
                            if !differences.is_empty() {
                                let path = crate::HierarchyPath::from_components(
                                    stack
                                        .iter()
                                        .map(|&parent| scopes[parent].name.as_str())
                                        .chain(std::iter::once(name.as_str())),
                                );
                                return Err(backend_error(format!(
                                    "conflicting FSDB scope {path}: {}",
                                    differences.join(", ")
                                )));
                            }
                            if old.definition_name.is_none() {
                                scopes[id].definition_name = definition_name;
                            }
                            id
                        } else {
                            let id = scopes.len();
                            scopes.push(ScopeData {
                                name,
                                name_was_escaped: false,
                                parent,
                                kind,
                                definition_name,
                                packing,
                                is_hidden: d.is_hidden != 0,
                            });
                            scope_ids.insert(key, id);
                            id
                        };
                        stack.push(id);
                    }
                    1 => {
                        stack
                            .pop()
                            .ok_or_else(|| backend_error("FSDB scope stack underflow"))?;
                    }
                    2 => {
                        let encoding = match d.encoding {
                            0 => Encoding::Unsupported,
                            1 if d.width > 0 => Encoding::Bits { width: d.width },
                            2 => Encoding::Real,
                            3 => Encoding::String,
                            4 => Encoding::Event,
                            _ => return Err(backend_error("invalid FSDB encoding descriptor")),
                        };
                        let index = if let Some(&index) = indices.get(&d.id) {
                            if encodings[index] != encoding {
                                return Err(backend_error("incompatible FSDB alias"));
                            }
                            index
                        } else {
                            let index = ids.len();
                            ids.push(d.id);
                            indices.insert(d.id, index);
                            encodings.push(encoding);
                            index
                        };
                        let range = (d.has_range != 0).then(|| BitRange::new(d.msb, d.lsb));
                        let reader_name = name.clone();
                        let (name, range) = declared_name(name, range, encoding);
                        // A file can append hierarchy trees repeating an identical
                        // declaration. Coalesce repetitions, not distinct aliases.
                        if !seen_variables.insert((
                            stack.last().copied(),
                            name.clone(),
                            kind.clone(),
                            d.direction,
                            range.map(|r| (r.msb(), r.lsb())),
                            d.is_constant,
                            index,
                        )) {
                            continue;
                        }
                        let type_name = string(d.type_name)?;
                        let enumeration = if d.enum_count == 0 {
                            None
                        } else {
                            let mut variants = Vec::with_capacity(d.enum_count);
                            for variant in 0..d.enum_count {
                                let (mut bits, mut label) = (std::ptr::null(), std::ptr::null());
                                // Indices originate from this immutable native hierarchy;
                                // borrowed strings are copied while the SDK lock is held.
                                ondas_fsdb_enum_variant(
                                    raw,
                                    declaration_index,
                                    variant,
                                    &mut bits,
                                    &mut label,
                                );
                                variants.push((
                                    string(bits)?
                                        .ok_or_else(|| backend_error("missing enum bits"))?,
                                    string(label)?
                                        .ok_or_else(|| backend_error("missing enum label"))?,
                                ));
                            }
                            Some(EnumerationData {
                                name: type_name.clone(),
                                variants,
                            })
                        };
                        variables.push(VariableData {
                            name,
                            reader_name: Some(reader_name),
                            name_was_escaped: false,
                            parent: stack.last().copied(),
                            kind,
                            direction: match d.direction {
                                0 => Direction::Implicit,
                                1 => Direction::Input,
                                2 => Direction::Output,
                                3 => Direction::InOut,
                                4 => Direction::Buffer,
                                5 => Direction::Linkage,
                                _ => return Err(backend_error("unknown FSDB direction")),
                            },
                            range,
                            is_constant: d.is_constant != 0,
                            type_name,
                            signedness: None,
                            logic_domain: None,
                            enumeration,
                            signal: Some(index),
                        });
                    }
                    _ => return Err(backend_error("invalid FSDB hierarchy entry")),
                }
            }
            if !stack.is_empty() {
                return Err(backend_error("unclosed FSDB scopes"));
            }
            copy_metadata(&meta, source_name, !variables.is_empty())?
        };
        drop(guard);
        let hierarchy = Hierarchy::new(scopes, variables, encodings.clone());
        Ok((
            Self {
                handle,
                ids,
                indices,
                encodings,
                #[cfg(test)]
                records_read: 0,
                #[cfg(test)]
                point_queries: 0,
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
        // SAFETY: live exclusive reader, validated base handles and writable
        // error buffer. Traversal releases loaded signals on all exits.
        let code = unsafe {
            ondas_fsdb_begin(
                traversal.0.0.as_ptr(),
                ids.as_ptr(),
                ids.len(),
                0,
                time.ticks(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        status(code, &error)?;
        let mut samples = Vec::with_capacity(signals.len());
        for &signal in signals {
            let mut record = Record::default();
            let mut changed = 0;
            let width = signal
                .width()
                .ok_or_else(|| backend_error("point sample requires bits"))?;
            // SAFETY: selected, validated signal/projection and caller-owned
            // output descriptors. Borrowed bytes are copied before another call.
            let code = unsafe {
                ondas_fsdb_sample_bits(
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
            status(code, &error)?;
            #[cfg(test)]
            {
                self.point_queries += 1;
            }
            samples.push(if code == 0 {
                crate::Sample::Missing { signal }
            } else {
                if record.encoding != 1 || record.len != width as usize || record.data.is_null() {
                    return Err(backend_error("invalid FSDB point bit buffer"));
                }
                // SAFETY: the native descriptor supplies exactly width validated
                // bytes, retained on the live owner until its next operation.
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

    // Nonzero starts require normalized entering state owned by the query engine.
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
        let ids: Vec<_> = signals.iter().map(|s| self.ids[s.index()]).collect();
        let traversal = Traversal(&mut self.handle);
        let mut error = [0; 512];
        {
            let _lock = lock();
            // SAFETY: live exclusive owner and caller-owned selection; shim copies ids.
            let code = unsafe {
                ondas_fsdb_begin(
                    traversal.0.0.as_ptr(),
                    ids.as_ptr(),
                    ids.len(),
                    start.ticks(),
                    end.ticks(),
                    error.as_mut_ptr(),
                    error.len(),
                )
            };
            status(code, &error)?;
        }
        let width = signals
            .iter()
            .filter_map(|signal| match self.encodings[signal.index()] {
                Encoding::Bits { width } => Some(width as usize),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        let mut bits = vec![0; width];
        let mut data = Vec::new();
        let mut text = String::new();
        let mut previous = 0;
        loop {
            let mut record = Record::default();
            {
                let _lock = lock();
                // SAFETY: live cursor and writable caller-owned bit buffer. Other
                // transient bytes are copied before any further SDK call/unlock.
                // C++ never retains the buffer or invokes the Rust visitor.
                let code = unsafe {
                    ondas_fsdb_next(
                        traversal.0.0.as_ptr(),
                        &mut record,
                        bits.as_mut_ptr(),
                        bits.len(),
                        error.as_mut_ptr(),
                        error.len(),
                    )
                };
                status(code, &error)?;
                if code == 0 {
                    break;
                }
                if record.tick > end.ticks() {
                    break;
                }
                if record.encoding == 1 {
                    if record.len > bits.len() {
                        return Err(backend_error("invalid FSDB bit buffer"));
                    }
                } else {
                    data.clear();
                    if record.len > 0 {
                        if record.data.is_null() || record.len > isize::MAX as usize {
                            return Err(backend_error("invalid FSDB buffer"));
                        }
                        data.extend_from_slice(unsafe {
                            std::slice::from_raw_parts(record.data, record.len)
                        });
                    }
                }
            }
            if record.tick < previous {
                return Err(backend_error("backwards FSDB timestamps"));
            }
            previous = record.tick;
            #[cfg(test)]
            {
                self.records_read += 1;
            }
            let index = *self
                .indices
                .get(&record.id)
                .ok_or_else(|| backend_error("unknown FSDB value identity"))?;
            let bytes = if record.encoding == 1 {
                &bits[..record.len]
            } else {
                &data[..record.len]
            };
            let value = match (self.encodings[index], record.encoding) {
                (Encoding::Bits { width }, 1) => {
                    if bytes.len() != width as usize {
                        return Err(backend_error("invalid FSDB bits"));
                    }
                    // The private shim validates every SDK digit before writing
                    // its normalized ASCII form. Don't parse those digits again.
                    ValueRef::Bits(BitsRef::from_validated_ascii(bytes))
                }
                (Encoding::Real, 2) => ValueRef::Real(real(bytes)?),
                (Encoding::String, 3) => {
                    text.clear();
                    text.extend(bytes.iter().copied().map(char::from));
                    ValueRef::String(&text)
                }
                (Encoding::Event, 4) => ValueRef::Event { occurrences: 1 },
                _ => return Err(backend_error("FSDB value disagrees with declaration")),
            };
            if let ControlFlow::Break(value) = visitor(index, Time::from_ticks(record.tick), value)
            {
                return Ok(ControlFlow::Break(value));
            }
        }
        Ok(ControlFlow::Continue(()))
    }
}

fn real(bytes: &[u8]) -> Result<f64> {
    match bytes.len() {
        4 => Ok(f32::from_ne_bytes(bytes.try_into().unwrap()) as f64),
        8 => Ok(f64::from_ne_bytes(bytes.try_into().unwrap())),
        _ => Err(backend_error("invalid FSDB real size")),
    }
}

fn declared_name(
    name: String,
    range: Option<BitRange>,
    encoding: Encoding,
) -> (String, Option<BitRange>) {
    // The SDK omits [0:0] bounds for scalars. An attached suffix on an
    // escaped identifier is literal even when the SDK supplies matching bounds;
    // a separately printed range has separating whitespace.
    let range = range.or_else(|| {
        (matches!(encoding, Encoding::Bits { .. })
            && name
                .strip_suffix("[0:0]")
                .is_some_and(|base| !base.starts_with('\\') || base.ends_with(char::is_whitespace)))
        .then_some(BitRange::new(0, 0))
    });
    if let Some(range) = range {
        let suffix = format!("[{}:{}]", range.msb(), range.lsb());
        if let Some(base) = name
            .strip_suffix(&suffix)
            .filter(|base| !name.starts_with('\\') || base.ends_with(char::is_whitespace))
        {
            return (base.trim_end().to_owned(), Some(range));
        }
    }
    (name, range)
}
fn timescale(text: &str) -> Option<Timescale> {
    let text = text.trim();
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let factor = text[..digits].parse::<u32>().ok()?;
    if factor == 0 {
        return None;
    }
    let unit = match text[digits..].trim() {
        "s" => TimeUnit::Second,
        "ms" => TimeUnit::Millisecond,
        "us" => TimeUnit::Microsecond,
        "ns" => TimeUnit::Nanosecond,
        "ps" => TimeUnit::Picosecond,
        "fs" => TimeUnit::Femtosecond,
        "as" => TimeUnit::Attosecond,
        "zs" => TimeUnit::Zeptosecond,
        _ => return None,
    };
    Some(Timescale::new(factor, unit))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_storage_preserves_binary_values() {
        for value in [0.0_f32, -0.0, -3.25, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                real(&value.to_ne_bytes()).unwrap().to_bits(),
                (value as f64).to_bits()
            );
        }
        assert!(real(&f32::NAN.to_ne_bytes()).unwrap().is_nan());
        for bits in [0_u64, 1 << 63, 0x7ff0000000000000, 0x7ff8000000000001] {
            assert_eq!(real(&bits.to_ne_bytes()).unwrap().to_bits(), bits);
        }
        for invalid in [0, 1, 3, 5, 9] {
            assert!(real(&vec![0; invalid]).is_err());
        }
    }

    #[test]
    #[ignore = "requires real FSDB runtime and locked public fixtures"]
    fn fsdb_bits_use_caller_storage() {
        let root = std::path::PathBuf::from(std::env::var_os("ONDAS_FIXTURES").unwrap());
        let path = root.join("kleverhq.ondas-fixtures/fsdb0015-wide-compact-toggle/waveform.fsdb");
        let (mut reader, hierarchy, _) = Reader::open(&path, "bits".into()).unwrap();
        let signal = hierarchy.signal("top.wide").unwrap();
        let id = reader.ids[signal.index()];
        let width = signal.width().unwrap() as usize;
        // A short destination fails inside the shim; the next session still works.
        for capacity in [0, width] {
            let traversal = Traversal(&mut reader.handle);
            let _lock = lock();
            let mut error = [0; 512];
            let mut bits = vec![0; capacity];
            let mut records = 0;
            let mut bytes = 0;
            // SAFETY: live exclusive owner, writable caller buffers, SDK lock.
            let code = unsafe {
                ondas_fsdb_begin(
                    traversal.0.0.as_ptr(),
                    &id,
                    1,
                    0,
                    4096,
                    error.as_mut_ptr(),
                    error.len(),
                )
            };
            status(code, &error).unwrap();
            loop {
                let mut record = Record::default();
                // SAFETY: the destination length is passed exactly, including
                // the intentional zero-capacity error case; no pointer escapes.
                let code = unsafe {
                    ondas_fsdb_next(
                        traversal.0.0.as_ptr(),
                        &mut record,
                        bits.as_mut_ptr(),
                        bits.len(),
                        error.as_mut_ptr(),
                        error.len(),
                    )
                };
                if capacity == 0 {
                    assert!(status(code, &error).is_err());
                    break;
                }
                status(code, &error).unwrap();
                if code == 0 {
                    break;
                }
                assert_eq!(record.encoding, 1);
                assert_eq!(record.len, width);
                assert_eq!(
                    record.data,
                    bits.as_ptr(),
                    "no native scratch/copy for bits"
                );
                let value = BitsRef::from_ascii(&bits).unwrap();
                assert_eq!(
                    value.bit(0),
                    Some(if record.tick % 2 == 0 {
                        crate::Logic::Zero
                    } else {
                        crate::Logic::One
                    })
                );
                records += 1;
                bytes += record.len;
            }
            if capacity != 0 {
                assert_eq!(records, 4097);
                assert_eq!(bytes, 4097 * 4096);
                eprintln!(
                    "FSDB bit-buffer counters: {records} records, {bytes} directly written digits; no intermediate bit-buffer copy"
                );
            }
        }
    }

    #[test]
    #[ignore = "requires installed SDK demo FSDB"]
    fn fsdb_datatype_enum_is_queryable() {
        let sdk = std::path::PathBuf::from(std::env::var_os("VERDI_HOME").unwrap());
        let mut wave = crate::open(sdk.join("share/VIA/demo/waveform/cpu.fsdb")).unwrap();
        let variable = wave
            .hierarchy()
            .variables()
            .find(|var| var.name() == "assertControlType")
            .unwrap();
        assert_eq!(variable.kind(), "enum");
        assert_eq!(variable.range(), Some(BitRange::new(1, 0)));
        assert!(variable.type_name().is_some());
        let enumeration = variable.enumeration().unwrap();
        assert_eq!(enumeration.name(), variable.type_name());
        let variants: Vec<_> = enumeration.variants().collect();
        assert_eq!(variants.len(), 4);
        assert!(
            variants
                .iter()
                .all(|variant| variant.encoded.len() == 2 && !variant.label.is_empty())
        );
        assert!(variants.iter().any(|variant| variant.encoded == "00"));
        let signal = variable.signal().unwrap();
        assert_eq!(signal.encoding(), Encoding::Bits { width: 2 });
        let sample = wave.sample(signal, Time::ZERO).unwrap();
        let crate::Sample::Value {
            value: crate::Value::Bits(bits),
            ..
        } = sample
        else {
            panic!("expected integral enum sample");
        };
        assert_eq!(
            bits.as_ref().iter_msb().collect::<Vec<_>>(),
            vec![crate::Logic::Zero; 2]
        );
    }

    #[test]
    #[ignore = "requires installed SDK demo FSDB"]
    fn fsdb_hidden_scopes_remain_accessible() {
        let sdk = std::path::PathBuf::from(std::env::var_os("VERDI_HOME").unwrap());
        let wave = crate::open(sdk.join("share/VIA/demo/waveform/cpu.fsdb")).unwrap();
        let hierarchy = wave.hierarchy();
        assert!(hierarchy.scopes().any(|scope| scope.is_hidden()));
        assert!(hierarchy.scopes().any(|scope| !scope.is_hidden()));
        for scope in hierarchy.scopes().filter(|scope| scope.is_hidden()) {
            assert!(hierarchy.scope_path(&scope.path()).unwrap().is_hidden());
        }
    }

    #[test]
    fn escaped_scalar_suffix_preserves_ambiguous_lookup() {
        let encodings = vec![
            Encoding::Bits { width: 1 },
            Encoding::Bits { width: 1 },
            Encoding::Bits { width: 8 },
            Encoding::Bits { width: 32 },
            Encoding::Bits { width: 8 },
        ];
        let raw_names = [
            r"\flags[0:0]",
            r"\flags[0:0]",
            "flags[7:0]",
            r"\awaddr[0] [31:0]",
            "maprom[0][7:0]",
        ];
        let variables = raw_names
            .into_iter()
            .enumerate()
            .map(|(index, raw)| {
                let range = match index {
                    2 | 4 => Some(BitRange::new(7, 0)),
                    3 => Some(BitRange::new(31, 0)),
                    _ => None,
                };
                let (name, range) = declared_name(raw.into(), range, encodings[index]);
                VariableData {
                    name,
                    reader_name: Some(raw.into()),
                    name_was_escaped: false,
                    parent: None,
                    kind: "wire".into(),
                    direction: Direction::Implicit,
                    range,
                    is_constant: false,
                    type_name: None,
                    signedness: None,
                    logic_domain: None,
                    enumeration: None,
                    signal: Some(index),
                }
            })
            .collect();
        let hierarchy = Hierarchy::new(Vec::new(), variables, encodings);
        let path = crate::HierarchyPath::from_components([r"\flags[0:0]"]);
        assert!(matches!(
            hierarchy.signal(&path.to_string()),
            Err(crate::LookupError::Ambiguous { .. })
        ));
        assert_eq!(hierarchy.signal("flags").unwrap().width(), Some(8));
        assert_eq!(
            hierarchy
                .variables()
                .map(|var| var.reader_name().unwrap())
                .collect::<Vec<_>>(),
            raw_names
        );
        assert_eq!(
            hierarchy
                .variable_path(&crate::HierarchyPath::from_components([r"\awaddr[0]"]))
                .unwrap()
                .range(),
            Some(BitRange::new(31, 0))
        );
        assert_eq!(
            hierarchy.variable("maprom[0]").unwrap().reader_name(),
            Some("maprom[0][7:0]")
        );
        assert_eq!(
            hierarchy
                .variables()
                .filter(|var| var.range().is_none())
                .count(),
            2
        );
    }

    #[test]
    fn exact_scale_and_declared_ranges() {
        let scale = timescale("100fs").unwrap();
        assert_eq!((scale.factor(), scale.unit()), (100, TimeUnit::Femtosecond));
        for bad in ["0ns", "4294967296ns", "1e-13s", "0.1ns", "1unknown"] {
            assert!(timescale(bad).is_none());
        }
        assert_eq!(
            declared_name(
                "bus [-2:-9]".into(),
                Some(BitRange::new(-2, -9)),
                Encoding::Bits { width: 8 }
            ),
            ("bus".into(), Some(BitRange::new(-2, -9)))
        );
        for (name, native_range, expected_name, expected_range) in [
            (r"\flags[0:0]", None, r"\flags[0:0]", None),
            (
                r"\range.dot[31:0]",
                Some(BitRange::new(31, 0)),
                r"\range.dot[31:0]",
                Some(BitRange::new(31, 0)),
            ),
            (
                r"\awaddr[0] [31:0]",
                Some(BitRange::new(31, 0)),
                r"\awaddr[0]",
                Some(BitRange::new(31, 0)),
            ),
            (
                "maprom[0][7:0]",
                Some(BitRange::new(7, 0)),
                "maprom[0]",
                Some(BitRange::new(7, 0)),
            ),
            (
                r"\flags[0:0] [0:0]",
                None,
                r"\flags[0:0]",
                Some(BitRange::new(0, 0)),
            ),
            ("\\flags\t[0:0]", None, r"\flags", Some(BitRange::new(0, 0))),
            ("flags[0:0]", None, "flags", Some(BitRange::new(0, 0))),
            (
                "flags[7:0]",
                Some(BitRange::new(7, 0)),
                "flags",
                Some(BitRange::new(7, 0)),
            ),
            (
                r"\flags[0:0] [7:0]",
                Some(BitRange::new(7, 0)),
                r"\flags[0:0]",
                Some(BitRange::new(7, 0)),
            ),
            ("mem[3]", None, "mem[3]", None),
        ] {
            assert_eq!(
                declared_name(name.into(), native_range, Encoding::Bits { width: 1 }),
                (expected_name.into(), expected_range),
            );
        }
        assert!(filename(Path::new("a\0b.fsdb")).is_err());
    }
}
