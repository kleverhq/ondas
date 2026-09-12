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
    hierarchy::{ScopeData, VariableData},
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
    msb: i64,
    lsb: i64,
    name: *const c_char,
    kind: *const c_char,
    definition: *const c_char,
}
#[repr(C)]
#[derive(Default)]
struct Meta {
    first: u64,
    last: u64,
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
        out: *mut *mut c_void,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_fsdb_close(reader: *mut c_void);
    fn ondas_fsdb_metadata(reader: *mut c_void, out: *mut Meta);
    fn ondas_fsdb_decl_count(reader: *mut c_void) -> usize;
    fn ondas_fsdb_declaration(reader: *mut c_void, index: usize, out: *mut Declaration);
    fn ondas_fsdb_begin(
        reader: *mut c_void,
        ids: *const u64,
        count: usize,
        end: u64,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_fsdb_next(
        reader: *mut c_void,
        out: *mut Record,
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
}
impl Reader {
    pub(crate) fn open(path: &Path, source_name: String) -> Result<(Self, Hierarchy, Metadata)> {
        let path = filename(path)?;
        let mut raw = std::ptr::null_mut();
        let mut error = [0; 512];
        {
            let _lock = lock();
            // SAFETY: pointers are valid for the call, shim owns any retained data.
            let code = unsafe {
                ondas_fsdb_open(path.as_ptr(), &mut raw, error.as_mut_ptr(), error.len())
            };
            status(code, &error)?;
        }
        let handle = Handle(NonNull::new(raw).ok_or_else(|| backend_error("null FSDB reader"))?);
        let mut scopes: Vec<ScopeData> = Vec::new();
        let mut variables = Vec::new();
        let mut seen_variables = HashSet::new();
        let mut stack = Vec::new();
        let mut scope_ids = HashMap::new();
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
            for index in 0..ondas_fsdb_decl_count(raw) {
                let mut d = Declaration::default();
                ondas_fsdb_declaration(raw, index, &mut d);
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
                            let old: &mut ScopeData = &mut scopes[id];
                            if old.kind != kind
                                || old.packing != packing
                                || (old.definition_name.is_some()
                                    && definition_name.is_some()
                                    && old.definition_name != definition_name)
                            {
                                return Err(backend_error("conflicting FSDB scopes"));
                            }
                            if old.definition_name.is_none() {
                                old.definition_name = definition_name;
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
                        let range = (d.has_range != 0
                            || (matches!(encoding, Encoding::Bits { .. })
                                && name.ends_with("[0:0]")))
                        .then(|| BitRange::new(d.msb, d.lsb));
                        let name = declared_name(name, range);
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
                        variables.push(VariableData {
                            name,
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
                            type_name: None,
                            enumeration: None,
                            signal: Some(index),
                        });
                    }
                    _ => return Err(backend_error("invalid FSDB hierarchy entry")),
                }
            }
            if !stack.is_empty() {
                return Err(backend_error("unclosed FSDB scopes"));
            }
            let scale = string(meta.scale)?;
            let scale = match scale.as_deref() {
                None | Some("") => None,
                Some(text) => Some(
                    timescale(text).ok_or_else(|| backend_error("unsupported FSDB timescale"))?,
                ),
            };
            Metadata {
                source_name,
                timescale: scale,
                time_span: (!variables.is_empty()).then(|| {
                    TimeSpan::new(Time::from_ticks(meta.first), Time::from_ticks(meta.last))
                }),
                writer: string(meta.writer)?,
                date: string(meta.date)?,
                comments: Vec::new(),
            }
        };
        drop(guard);
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

    pub(crate) fn read<B>(
        &mut self,
        signals: &[Signal],
        end: Time,
        mut visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        if signals.is_empty() {
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
                    end.ticks(),
                    error.as_mut_ptr(),
                    error.len(),
                )
            };
            status(code, &error)?;
        }
        let mut data = Vec::new();
        let mut text = String::new();
        let mut previous = 0;
        loop {
            let mut record = Record::default();
            {
                let _lock = lock();
                // SAFETY: live cursor; transient bytes copied before any other SDK
                // call or unlock. C++ never invokes the Rust visitor.
                let code = unsafe {
                    ondas_fsdb_next(
                        traversal.0.0.as_ptr(),
                        &mut record,
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
            if record.tick < previous {
                return Err(backend_error("backwards FSDB timestamps"));
            }
            previous = record.tick;
            let index = *self
                .indices
                .get(&record.id)
                .ok_or_else(|| backend_error("unknown FSDB value identity"))?;
            let value = match (self.encodings[index], record.encoding) {
                (Encoding::Bits { width }, 1) => ValueRef::Bits(
                    BitsRef::from_ascii(&data)
                        .filter(|v| v.width() == width)
                        .ok_or_else(|| backend_error("invalid FSDB bits"))?,
                ),
                (Encoding::Real, 2) => ValueRef::Real(real(&data)?),
                (Encoding::String, 3) => {
                    text.clear();
                    text.extend(data.iter().copied().map(char::from));
                    ValueRef::String(&text)
                }
                (Encoding::Event, 4) => ValueRef::Event,
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

fn declared_name(name: String, range: Option<BitRange>) -> String {
    if let Some(range) = range {
        let suffix = format!("[{}:{}]", range.msb(), range.lsb());
        if let Some(base) = name.strip_suffix(&suffix) {
            return base.trim_end().to_owned();
        }
    }
    name
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
    fn exact_scale_and_declared_ranges() {
        let scale = timescale("100fs").unwrap();
        assert_eq!((scale.factor(), scale.unit()), (100, TimeUnit::Femtosecond));
        for bad in ["0ns", "4294967296ns", "1e-13s", "0.1ns", "1unknown"] {
            assert!(timescale(bad).is_none());
        }
        assert_eq!(
            declared_name("bus [-2:-9]".into(), Some(BitRange::new(-2, -9))),
            "bus"
        );
        assert_eq!(declared_name("mem[3]".into(), None), "mem[3]");
        assert!(filename(Path::new("a\0b.fsdb")).is_err());
    }
}
