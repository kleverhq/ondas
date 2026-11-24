//! Private native reader. Native cursors and sessions are mutually exclusive.
//! The Rust visitor runs unlocked with an owned, bounded batch.
use crate::{
    BitRange, BitsRef, Direction, Encoding, Error, Format, Hierarchy, Metadata, PathComponent,
    Result, Signal, Time, TimeSpan, Timescale, ValueRef,
    hierarchy::{EnumerationData, ScopeData, VariableData},
};
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    ops::ControlFlow,
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr::NonNull,
    sync::{Mutex, MutexGuard},
};

const BACKEND: &str = "vwdb-lib";
const LIBRARY: &str = concat!(env!("ONDAS_VWDB_READER"), "\0");
static SDK: Mutex<()> = Mutex::new(());

fn lock() -> MutexGuard<'static, ()> {
    SDK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[repr(C)]
#[derive(Default)]
struct Meta {
    first: u64,
    last: u64,
    exponent: i32,
}

#[repr(C)]
#[derive(Default)]
struct Declaration {
    parent: usize,
    name: *const c_char,
    kind: *const c_char,
    native_kind: i32,
    encoding: i32,
    width: u32,
    has_range: u32,
    is_constant: u32,
    indexed_children: u32,
    msb: i64,
    lsb: i64,
    variant_count: usize,
}

#[repr(C)]
#[derive(Default)]
struct Variant {
    encoded: *const c_char,
    label: *const c_char,
}

#[repr(C)]
#[derive(Default)]
struct Record {
    declaration: usize,
    tick: u64,
    encoding: i32,
    data: *const u8,
    len: usize,
}

unsafe extern "C" {
    fn ondas_vwdb_open(
        path: *const c_char,
        library: *const c_char,
        metadata_only: c_int,
        out: *mut *mut c_void,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_vwdb_close(reader: *mut c_void);
    fn ondas_vwdb_metadata(reader: *const c_void, out: *mut Meta);
    fn ondas_vwdb_decl_count(reader: *const c_void) -> usize;
    fn ondas_vwdb_declaration(reader: *const c_void, index: usize, out: *mut Declaration);
    fn ondas_vwdb_enum_variant(
        reader: *const c_void,
        index: usize,
        variant: usize,
        out: *mut Variant,
    );
    fn ondas_vwdb_read(
        reader: *mut c_void,
        ids: *const usize,
        count: usize,
        end: u64,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_vwdb_next(
        reader: *mut c_void,
        out: *mut Record,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_vwdb_end(reader: *mut c_void, keep_session: c_int);
}

fn error(operation: &'static str, message: impl Into<String>) -> Error {
    Error::Backend {
        backend: BACKEND.into(),
        operation,
        message: message.into(),
    }
}

fn status(code: c_int, operation: &'static str, buffer: &[c_char; 512]) -> Result<()> {
    if code >= 0 {
        return Ok(());
    }
    // SAFETY: the shim terminates the caller-owned error buffer.
    let message = unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    Err(match code {
        -3 => Error::BackendUnavailable {
            backend: BACKEND.into(),
            message,
        },
        -2 => Error::UnsupportedFeature {
            format: Format::Vwdb,
            backend: BACKEND.into(),
            operation,
            message,
        },
        _ => error(operation, message),
    })
}

struct Handle(NonNull<c_void>);
// SAFETY: all accesses, including suspension and destruction, use the same SDK
// lock. The owner holds immutable descriptors and an exclusively read stream.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        let _lock = lock();
        // SAFETY: exclusive owner; the shim drops cursors before the database.
        unsafe { ondas_vwdb_close(self.0.as_ptr()) };
    }
}

fn open_handle(path: &Path, metadata_only: bool) -> Result<Handle> {
    let path = path.canonicalize()?;
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;
    let _lock = lock();
    let mut raw = std::ptr::null_mut();
    let mut buffer = [0; 512];
    // SAFETY: both inputs are terminated, outputs are writable, and the shim
    // copies the path. The lock covers all accesses to the vendor library.
    let code = unsafe {
        ondas_vwdb_open(
            path.as_ptr(),
            LIBRARY.as_ptr().cast(),
            metadata_only.into(),
            &mut raw,
            buffer.as_mut_ptr(),
            buffer.len(),
        )
    };
    status(
        code,
        if metadata_only {
            "read metadata"
        } else {
            "open"
        },
        &buffer,
    )?;
    Ok(Handle(
        NonNull::new(raw).ok_or_else(|| error("open", "null VWDB owner"))?,
    ))
}

fn metadata(handle: &Handle, source_name: String) -> Result<Metadata> {
    let _lock = lock();
    let mut meta = Meta::default();
    // SAFETY: the handle is live and the output is writable.
    unsafe { ondas_vwdb_metadata(handle.0.as_ptr(), &mut meta) };
    let timescale = Timescale::decimal_exponent(meta.exponent)
        .ok_or_else(|| error("read metadata", "unsupported VWDB timescale exponent"))?;
    Ok(Metadata {
        source_name,
        timescale: Some(timescale),
        time_span: Some(TimeSpan::new(
            Time::from_ticks(meta.first),
            Time::from_ticks(meta.last),
        )),
        writer: None,
        date: None,
        comments: Vec::new(),
    })
}

pub(crate) fn read_metadata(path: &Path, name: String) -> Result<Metadata> {
    metadata(&open_handle(path, true)?, name)
}

struct ImportedScope {
    id: usize,
    indexed_children: bool,
}

// SAFETY: the descriptor points to a terminated string on the live shim owner.
unsafe fn text(pointer: *const c_char) -> Result<String> {
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| error("open", "non-UTF-8 VWDB declaration text"))
}

pub(crate) struct Reader {
    handle: Handle,
    ids: Vec<usize>,
    by_declaration: Vec<Option<usize>>,
}

impl Reader {
    pub(crate) fn open(path: &Path, name: String) -> Result<(Self, Hierarchy, Metadata)> {
        let handle = open_handle(path, false)?;
        let meta = metadata(&handle, name)?;
        let mut scopes = Vec::<ScopeData>::new();
        let mut variables = Vec::new();
        let mut ids = Vec::new();
        let mut encodings = Vec::new();
        let mut parents = Vec::<Option<ImportedScope>>::new();
        let mut by_declaration = Vec::new();
        let raw = handle.0.as_ptr();
        let guard = lock();
        // SAFETY: all indices come from the live immutable descriptor table,
        // and its strings are copied while the native owner and lock are held.
        unsafe {
            let count = ondas_vwdb_decl_count(raw);
            parents.reserve(count);
            by_declaration.resize(count, None);
            for (id, signal) in by_declaration.iter_mut().enumerate() {
                let mut decl = Declaration::default();
                ondas_vwdb_declaration(raw, id, &mut decl);
                let name = text(decl.name)?;
                let mut parent = None;
                let mut component = PathComponent::new(name.as_str());
                if decl.parent != usize::MAX {
                    let scope = parents
                        .get(decl.parent)
                        .and_then(Option::as_ref)
                        .ok_or_else(|| error("open", "invalid VWDB parent declaration"))?;
                    let container = &scopes[scope.id];
                    if scope.indexed_children
                        && container.component.index().is_none()
                        && let Ok(index) = name.parse::<i64>()
                        && name == index.to_string()
                    {
                        component = PathComponent::indexed(container.component.name(), index);
                        parent = container.parent;
                    } else {
                        parent = Some(scope.id);
                    }
                }
                if decl.encoding == -1 {
                    parents.push(Some(ImportedScope {
                        id: scopes.len(),
                        indexed_children: decl.indexed_children != 0,
                    }));
                    let kind = if decl.kind.is_null() {
                        format!("vwdb:scope:{}", decl.native_kind)
                    } else {
                        text(decl.kind)?
                    };
                    scopes.push(ScopeData {
                        component,
                        name_was_escaped: false,
                        parent,
                        kind,
                        definition_name: None,
                        packing: None,
                        is_hidden: false,
                    });
                    continue;
                }
                let encoding = match decl.encoding {
                    0 => Encoding::Unsupported,
                    1 if decl.width > 0 => Encoding::Bits { width: decl.width },
                    2 => Encoding::Real,
                    4 => Encoding::Event,
                    _ => return Err(error("open", "invalid VWDB value encoding")),
                };
                let mut variants = Vec::with_capacity(decl.variant_count);
                for variant in 0..decl.variant_count {
                    let mut v = Variant::default();
                    ondas_vwdb_enum_variant(raw, id, variant, &mut v);
                    variants.push((text(v.encoded)?, text(v.label)?));
                }
                let index = ids.len();
                *signal = Some(index);
                ids.push(id);
                encodings.push(encoding);
                parents.push(None);
                variables.push(VariableData {
                    component,
                    reader_name: None,
                    name_was_escaped: false,
                    parent,
                    kind: format!("vwdb:decl:{}", decl.native_kind),
                    direction: Direction::Unknown,
                    range: (decl.has_range != 0).then(|| BitRange::new(decl.msb, decl.lsb)),
                    is_constant: decl.is_constant != 0,
                    type_name: None,
                    signedness: None,
                    logic_domain: None,
                    enumeration: EnumerationData::for_encoding(None, variants, encoding),
                    signal: Some(index),
                });
            }
        }
        drop(guard);
        Ok((
            Self {
                handle,
                ids,
                by_declaration,
            },
            Hierarchy::new(scopes, variables, encodings),
            meta,
        ))
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
        let ids = signals
            .iter()
            .map(|signal| self.ids[signal.index()])
            .collect::<Vec<_>>();
        let raw = self.handle.0.as_ptr();
        let mut traversal = Traversal {
            handle: &mut self.handle,
            keep_session: false,
        };
        let mut buffer = [0; 512];
        {
            let _lock = lock();
            // SAFETY: the owner is live, ids and error buffer are valid for the call.
            let code = unsafe {
                ondas_vwdb_read(
                    raw,
                    ids.as_ptr(),
                    ids.len(),
                    end.ticks(),
                    buffer.as_mut_ptr(),
                    buffer.len(),
                )
            };
            status(code, "start read", &buffer)?;
        }
        let mut batch = Vec::new();
        let mut limit = 64;
        loop {
            let mut payload_len = 0;
            let mut terminal = Ok(false);
            let mut batch_failed = false;
            {
                let _lock = lock();
                while batch.len() < limit && payload_len < 64 * 1024 {
                    let mut record = Record::default();
                    // SAFETY: output and buffers are valid, and the stream is live.
                    let code = unsafe {
                        ondas_vwdb_next(raw, &mut record, buffer.as_mut_ptr(), buffer.len())
                    };
                    if code == 0 {
                        terminal = Ok(true);
                        break;
                    }
                    let result = status(code, "read history", &buffer);
                    let Some(index) = self
                        .by_declaration
                        .get(record.declaration)
                        .copied()
                        .flatten()
                    else {
                        batch_failed = true;
                        terminal = result.and_then(|()| {
                            Err(error("read history", "invalid VWDB history identity"))
                        });
                        break;
                    };
                    let time = Time::from_ticks(record.tick);
                    let value = result.map(|()| {
                        // SAFETY: nonempty bytes are shim-owned until its next
                        // operation. The copy stays valid across callback reentry.
                        let value = if record.len == 0 {
                            Vec::new()
                        } else {
                            unsafe { std::slice::from_raw_parts(record.data, record.len) }.to_vec()
                        };
                        payload_len += value.len();
                        value
                    });
                    let failed = value.is_err();
                    batch.push((index, time, record.encoding, value));
                    if failed {
                        batch_failed = true;
                        terminal = Ok(true);
                        break;
                    }
                }
            }
            for (index, time, encoding, value) in batch.drain(..) {
                let decoded = match value.as_deref() {
                    Ok(bytes) => decode(encoding, bytes),
                    Err(_) => Err(value.expect_err("timed VWDB read failure")),
                };
                let failed = decoded.is_err();
                if let ControlFlow::Break(value) = visitor(index, time, decoded) {
                    traversal.keep_session = !batch_failed && !failed;
                    return Ok(ControlFlow::Break(value));
                }
                if failed {
                    return Err(error(
                        "read history",
                        "read stopped after a failed VWDB record",
                    ));
                }
            }
            if terminal? {
                traversal.keep_session = true;
                return Ok(ControlFlow::Continue(()));
            }
            limit = 1024;
        }
    }
}

fn decode(encoding: i32, bytes: &[u8]) -> Result<Option<ValueRef<'_>>> {
    let value = match encoding {
        0 => return Ok(None),
        1 => ValueRef::Bits(
            BitsRef::from_ascii(bytes)
                .ok_or_else(|| error("read history", "invalid VWDB logic state"))?,
        ),
        2 => ValueRef::Real(match bytes.len() {
            4 => f64::from(f32::from_bits(u32::from_be_bytes(
                bytes.try_into().unwrap(),
            ))),
            8 => f64::from_bits(u64::from_be_bytes(bytes.try_into().unwrap())),
            _ => return Err(error("read history", "invalid VWDB real width")),
        }),
        4 => ValueRef::Event { occurrences: 1 },
        _ => return Err(error("read history", "unknown VWDB value encoding")),
    };
    Ok(Some(value))
}

struct Traversal<'a> {
    handle: &'a mut Handle,
    keep_session: bool,
}
impl Drop for Traversal<'_> {
    fn drop(&mut self) {
        let _lock = lock();
        // SAFETY: this lease ends before the owner is destroyed.
        unsafe { ondas_vwdb_end(self.handle.0.as_ptr(), self.keep_session.into()) };
    }
}
