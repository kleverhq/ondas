//! Keep every SDK object and call on one OS thread for the process lifetime.
use std::{
    cell::Cell,
    collections::HashMap,
    ffi::{CStr, CString, OsStr, c_char, c_int, c_void},
    io::Read,
    ops::ControlFlow,
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, OpenOptionsExt},
    },
    ptr::NonNull,
    sync::{OnceLock, mpsc},
};

use sha2::{Digest, Sha256};

use super::backend_error;
use crate::{
    BitRange, BitsRef, Encoding, Error, Format, Metadata, Result, Sample, Signal, Time, TimeSpan,
    TimeUnit, Timescale, Value,
};

#[repr(C)]
#[derive(Default)]
struct Meta {
    first: u64,
    last: u64,
    resolution: u32,
    writer: *const c_char,
}
#[repr(C)]
#[derive(Default)]
struct NativeDeclaration {
    id: i64,
    parent: usize,
    entry: u32,
    encoding: u32,
    width: u32,
    direction: u32,
    is_constant: u32,
    has_range: u32,
    signedness: u32,
    logic_domain: u32,
    msb: i64,
    lsb: i64,
    name: *const c_char,
    kind: *const c_char,
    definition: *const c_char,
    verilog_name: u32,
    array_base_len: u32,
}
#[repr(C)]
#[derive(Default)]
struct NativeRecord {
    selection: usize,
    tick: u64,
    occurrences: u64,
    data: *const u8,
    len: usize,
    real_bits: u64,
}
unsafe extern "C" {
    fn ondas_wlf_open(
        path: *const c_char,
        name: *const c_char,
        metadata_only: c_int,
        out: *mut *mut c_void,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_wlf_close(reader: *mut c_void);
    fn ondas_wlf_metadata(reader: *mut c_void, out: *mut Meta);
    fn ondas_wlf_decl_count(reader: *mut c_void) -> usize;
    fn ondas_wlf_declaration(reader: *mut c_void, index: usize, out: *mut NativeDeclaration);
    fn ondas_wlf_enum_count(reader: *mut c_void, index: usize) -> usize;
    fn ondas_wlf_enum_variant(
        reader: *mut c_void,
        index: usize,
        variant: usize,
        bits: *mut *const c_char,
        label: *mut *const c_char,
    );
    fn ondas_wlf_begin(
        reader: *mut c_void,
        indices: *const usize,
        count: usize,
        start: u64,
        end: u64,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_wlf_begin_point(
        reader: *mut c_void,
        indices: *const usize,
        count: usize,
        start: u64,
        end: u64,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_wlf_point_start(
        reader: *mut c_void,
        index: usize,
        target: u64,
        start: *mut u64,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_wlf_next(
        reader: *mut c_void,
        out: *mut NativeRecord,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_wlf_batch_end(reader: *mut c_void) -> c_int;
    fn ondas_wlf_end(reader: *mut c_void);
    fn ondas_wlf_park(reader: *mut c_void) -> c_int;
    #[cfg(test)]
    fn ondas_wlf_resources(
        reader: *mut c_void,
        count: *mut usize,
        bytes: *mut usize,
        pack: *mut c_int,
    );
}

#[cfg(test)]
pub(super) fn resources(id: u64) -> Result<(usize, usize, bool)> {
    call(move |state| {
        // Inspect an active owner even if another test suspended this session.
        let file = state.file(id)?;
        let handle = file.native.as_ref().unwrap();
        let (mut count, mut bytes, mut pack) = (0, 0, 0);
        // SAFETY: live worker-owned handle and writable, correctly typed outputs.
        unsafe { ondas_wlf_resources(handle.raw.as_ptr(), &mut count, &mut bytes, &mut pack) };
        Ok((count, bytes, pack != 0))
    })
}

pub(super) struct Declaration {
    pub id: i64,
    pub parent: Option<usize>,
    pub scope: bool,
    pub encoding: Encoding,
    pub direction: u32,
    pub is_constant: bool,
    pub range: Option<BitRange>,
    pub signedness: u32,
    pub logic_domain: u32,
    pub name: String,
    pub kind: String,
    pub definition: Option<String>,
    pub verilog_name: bool,
    pub variants: Vec<(String, String)>,
    pub array_base_len: usize,
}
pub(super) struct Opened {
    pub id: u64,
    pub metadata: Metadata,
    pub declarations: Vec<Declaration>,
}
pub(super) struct Record {
    pub selection: usize,
    pub tick: Time,
    pub value: Value,
}
#[derive(Debug)]
pub(super) enum Failure {
    Read(Error),
    Decode {
        selection: usize,
        tick: Time,
        error: Error,
    },
}
#[derive(Default)]
pub(super) struct Batch {
    pub records: Vec<Record>,
    pub finished: bool,
    pub error: Option<Failure>,
}
impl Batch {
    pub fn visit<B>(
        self,
        mut visitor: impl FnMut(usize, Time, Result<Value>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        for record in self.records {
            if let ControlFlow::Break(value) =
                visitor(record.selection, record.tick, Ok(record.value))
            {
                return Ok(ControlFlow::Break(value));
            }
        }
        match self.error {
            Some(Failure::Read(error)) => return Err(error),
            Some(Failure::Decode {
                selection,
                tick,
                error,
            }) => {
                if let ControlFlow::Break(value) = visitor(selection, tick, Err(error)) {
                    return Ok(ControlFlow::Break(value));
                }
                return Err(backend_error(
                    "WLF visitor continued after a decoding failure",
                ));
            }
            None => (),
        }
        Ok(ControlFlow::Continue(()))
    }
}

// Lightweight identity of a regular file; coarse timestamps alone are not freshness proof.
#[derive(Clone, PartialEq, Eq)]
struct NativeMetadata {
    dev: u64,
    ino: u64,
    size: u64,
    mtime: i64,
    mtime_nsec: i64,
    ctime: i64,
    ctime_nsec: i64,
}
// Full identity of a regular file, captured for the native open that created its owner.
#[derive(Clone, PartialEq, Eq)]
struct NativeIdentity {
    metadata: NativeMetadata,
    digest: [u8; 32],
}
fn metadata_key(meta: &std::fs::Metadata) -> Option<NativeMetadata> {
    meta.file_type().is_file().then(|| NativeMetadata {
        dev: meta.dev(),
        ino: meta.ino(),
        size: meta.size(),
        mtime: meta.mtime(),
        mtime_nsec: meta.mtime_nsec(),
        ctime: meta.ctime(),
        ctime_nsec: meta.ctime_nsec(),
    })
}
// Metadata of a regular file, or an optional miss on any stat, access or type failure.
fn file_metadata(path: &CString) -> Option<NativeMetadata> {
    metadata_key(&std::fs::metadata(OsStr::from_bytes(path.as_bytes())).ok()?)
}
// Streamed content identity of a regular file. Any stat, access, type or IO failure is an
// optional miss; the whole file is read once with a fixed buffer and no bytes are retained.
// O_NONBLOCK keeps a replacement FIFO from blocking the optional fingerprint open.
fn content_identity(path: &CString) -> Option<NativeIdentity> {
    let before = file_metadata(path)?;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(OsStr::from_bytes(path.as_bytes()))
        .ok()?;
    // The descriptor must still be the regular file named by the path metadata.
    if metadata_key(&file.metadata().ok()?)? != before {
        return None;
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut remaining = before.size;
    while remaining > 0 {
        let want = remaining.min(buffer.len() as u64) as usize;
        let read = file.read(&mut buffer[..want]).ok()?;
        if read == 0 {
            return None;
        }
        hasher.update(&buffer[..read]);
        remaining -= read as u64;
    }
    // One extra byte proves that the content did not grow past the recorded size.
    let mut extra = [0u8; 1];
    if file.read(&mut extra).ok()? != 0 {
        return None;
    }
    if metadata_key(&file.metadata().ok()?)? != before
        || file_metadata(path) != Some(before.clone())
    {
        return None;
    }
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&hasher.finalize());
    Some(NativeIdentity {
        metadata: before,
        digest,
    })
}
// This type never leaves State on the worker thread; no Send/Sync assertion is needed.
struct Handle {
    raw: NonNull<c_void>,
    // Identity captured around the open that created this owner.
    identity: Option<NativeIdentity>,
    // Cleared permanently by any native or decode failure; a tainted owner is never parked.
    reusable: Cell<bool>,
}
impl Handle {
    fn taint(&self) {
        self.reusable.set(false);
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: sole owner on the SDK thread. Close first releases query resources.
        unsafe { ondas_wlf_close(self.raw.as_ptr()) };
    }
}
struct Query {
    indices: Vec<usize>,
    encodings: Vec<Encoding>,
    start: Time,
    end: Time,
    last: Option<(u64, usize)>,
    finished: bool,
}
struct File {
    path: CString,
    name: CString,
    resolution: u32,
    last: Time,
    encodings: Vec<Encoding>,
    native: Option<Handle>,
    query: Option<Query>,
}
// One fully ended native owner, kept only while no logical File is live.
struct Idle {
    path: CString,
    resolution: u32,
    handle: Handle,
}
struct State {
    files: HashMap<u64, File>,
    next_id: u64,
    resolution: Option<u32>,
    idle: Option<Idle>,
}
impl State {
    fn reset(&mut self) {
        // Header inspection can also select a process resolution in this SDK.
        // Suspend native owners before reading a new header, not just before open.
        self.idle.take();
        for file in self.files.values_mut() {
            file.native.take();
        }
        self.resolution = None;
    }
    fn select_resolution(&mut self, resolution: u32) {
        if self.resolution != Some(resolution) {
            self.reset();
            self.resolution = Some(resolution);
        }
    }
    fn file(&mut self, id: u64) -> Result<&mut File> {
        let resolution = self
            .files
            .get(&id)
            .ok_or_else(|| backend_error("closed WLF session"))?
            .resolution;
        self.select_resolution(resolution);
        let file = self.files.get_mut(&id).unwrap();
        if file.native.is_none() {
            let handle = open_handle(&file.path, &file.name, false)?;
            if let Some(query) = &file.query {
                begin_native(&handle, &query.indices, query.start, query.end, false)?;
            }
            file.native = Some(handle);
        }
        Ok(file)
    }
}
type Job = Box<dyn FnOnce(&mut State) + Send>;
static WORKER: OnceLock<std::result::Result<mpsc::Sender<Job>, String>> = OnceLock::new();
fn call<T: Send + 'static>(
    work: impl FnOnce(&mut State) -> Result<T> + Send + 'static,
) -> Result<T> {
    let sender = WORKER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<Job>();
            std::thread::Builder::new()
                .name("ondas-wlf".into())
                .spawn(move || {
                    let mut state = State {
                        files: HashMap::new(),
                        next_id: 0,
                        resolution: None,
                        idle: None,
                    };
                    while let Ok(job) = receiver.recv() {
                        job(&mut state);
                    }
                })
                .map(|_| sender)
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|message| backend_error(message.clone()))?;
    let (reply, receive) = mpsc::channel();
    sender
        .send(Box::new(move |state| {
            let _ = reply.send(work(state));
        }))
        .map_err(|_| backend_error("WLF worker stopped"))?;
    receive
        .recv()
        .map_err(|_| backend_error("WLF worker stopped"))?
}
fn status(code: c_int, error: &[c_char; 512], operation: &'static str) -> Result<()> {
    if code >= 0 {
        return Ok(());
    }
    // SAFETY: initialized caller buffer; the shim writes bounded NUL-terminated diagnostics.
    let message = unsafe { CStr::from_ptr(error.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    Err(match code {
        -2 => Error::Malformed {
            format: Format::Wlf,
            backend: "wlf-lib".into(),
            message,
        },
        -3 => Error::BackendUnavailable {
            backend: "wlf-lib".into(),
            message,
        },
        _ => Error::Backend {
            backend: "wlf-lib".into(),
            operation,
            message,
        },
    })
}
// SAFETY: called only for live shim-owned NUL-terminated text on the SDK thread;
// every string is copied before the next SDK operation or owner destruction.
unsafe fn string(ptr: *const c_char) -> Result<Option<String>> {
    if ptr.is_null() {
        return Ok(None);
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .map(|s| Some(s.to_owned()))
        .map_err(|_| backend_error("non-UTF-8 WLF declaration or metadata text"))
}
// SAFETY: the pointers obey string's lifetime and terminator requirements.
// An unusable optional table must not disable the signal's bit encoding.
unsafe fn enum_strings(bits: *const c_char, label: *const c_char) -> Option<(String, String)> {
    unsafe {
        string(bits)
            .ok()
            .flatten()
            .zip(string(label).ok().flatten())
    }
}
fn open_handle(path: &CString, name: &CString, metadata_only: bool) -> Result<Handle> {
    let before = if metadata_only {
        None
    } else {
        file_metadata(path)
    };
    let mut raw = std::ptr::null_mut();
    let mut error = [0; 512];
    // SAFETY: call-local owned strings and writable output buffers; shim retains no inputs.
    let code = unsafe {
        ondas_wlf_open(
            path.as_ptr(),
            name.as_ptr(),
            metadata_only.into(),
            &mut raw,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    status(
        code,
        &error,
        if metadata_only {
            "read metadata"
        } else {
            "open"
        },
    )?;
    let identity = if metadata_only {
        None
    } else {
        // The identity must describe this actual open: metadata must agree across the native
        // open and the streamed digest, and the pathname must still name the hashed file.
        match (before, content_identity(path)) {
            (Some(before), Some(hashed)) if before == hashed.metadata => Some(hashed),
            _ => None,
        }
    };
    Ok(Handle {
        raw: NonNull::new(raw).ok_or_else(|| backend_error("null WLF reader"))?,
        identity,
        reusable: Cell::new(true),
    })
}
fn metadata(handle: &Handle, source_name: String) -> Result<Metadata> {
    let mut meta = Meta::default();
    // SAFETY: live owner and correctly laid out writable descriptor, worker only.
    unsafe { ondas_wlf_metadata(handle.raw.as_ptr(), &mut meta) };
    let units = [
        TimeUnit::Femtosecond,
        TimeUnit::Picosecond,
        TimeUnit::Nanosecond,
        TimeUnit::Microsecond,
        TimeUnit::Millisecond,
        TimeUnit::Second,
    ];
    let unit = *units
        .get(meta.resolution as usize / 3)
        .ok_or_else(|| backend_error("invalid WLF resolution"))?;
    Ok(Metadata {
        source_name,
        timescale: Some(Timescale::new(10_u32.pow(meta.resolution % 3), unit)),
        time_span: Some(TimeSpan::new(
            Time::from_ticks(meta.first),
            Time::from_ticks(meta.last),
        )),
        // SAFETY: shim-owned header string, copied while handle is live.
        writer: unsafe { string(meta.writer)? },
        date: None,
        comments: Vec::new(),
    })
}
pub(super) fn read_metadata(path: CString, source_name: String) -> Result<Metadata> {
    call(move |state| {
        state.reset();
        let handle = open_handle(&path, &CString::new("metadata").unwrap(), true)?;
        metadata(&handle, source_name)
    })
}
pub(super) fn open(path: CString, source_name: String) -> Result<Opened> {
    call(move |state| {
        let id = state
            .next_id
            .checked_add(1)
            .ok_or_else(|| backend_error("WLF session identifier overflow"))?;
        state.next_id = id;
        let name = CString::new(format!("ondas-{id}")).unwrap();
        // The idle owner is used only for the exact raw path, its full original identity and an
        // unchanged resolution. Every other case drops it before header or open activity.
        let reused = match state.idle.take() {
            Some(idle) => {
                // Cheap path, resolution, key and metadata checks happen before any whole-file hash.
                let candidate = state.resolution == Some(idle.resolution)
                    && idle.path.as_bytes() == path.as_bytes()
                    && idle.handle.identity.as_ref().is_some_and(|saved| {
                        file_metadata(&path).as_ref() == Some(&saved.metadata)
                    });
                if candidate && idle.handle.identity == content_identity(&path) {
                    Some(idle)
                } else {
                    None
                }
            }
            None => None,
        };
        state.reset();
        let handle = match reused {
            Some(idle) => idle.handle,
            None => open_handle(&path, &name, false)?,
        };
        let mut meta = Meta::default();
        // SAFETY: live owner and correctly laid out output. Open reads the header.
        unsafe { ondas_wlf_metadata(handle.raw.as_ptr(), &mut meta) };
        let resolution = meta.resolution;
        state.resolution = Some(resolution);
        let metadata = metadata(&handle, source_name)?;
        let mut encodings = Vec::new();
        // SAFETY: count and indices come from the same immutable live owner.
        let count = unsafe { ondas_wlf_decl_count(handle.raw.as_ptr()) };
        let mut declarations = Vec::with_capacity(count);
        for index in 0..count {
            let mut d = NativeDeclaration::default();
            // SAFETY: index < native count; strings copied before another SDK call.
            unsafe { ondas_wlf_declaration(handle.raw.as_ptr(), index, &mut d) };
            let encoding = match d.encoding {
                0 => Encoding::Unsupported,
                1 if d.width > 0 => Encoding::Bits { width: d.width },
                2 => Encoding::Real,
                3 => Encoding::String,
                4 => Encoding::Event,
                _ => return Err(backend_error("invalid WLF encoding")),
            };
            if d.parent != usize::MAX && d.parent >= index {
                return Err(backend_error("invalid WLF hierarchy parent"));
            }
            // SAFETY: immutable worker-owned declaration; copy all borrowed
            // variant bytes before returning data to the caller thread.
            let variant_count = unsafe { ondas_wlf_enum_count(handle.raw.as_ptr(), index) };
            let mut variants = Vec::with_capacity(variant_count);
            for variant in 0..variant_count {
                let (mut bits, mut label) = (std::ptr::null(), std::ptr::null());
                // SAFETY: both indices come from this same live native owner.
                let pair = unsafe {
                    ondas_wlf_enum_variant(
                        handle.raw.as_ptr(),
                        index,
                        variant,
                        &mut bits,
                        &mut label,
                    );
                    enum_strings(bits, label)
                };
                if let Some(pair) = pair {
                    variants.push(pair);
                } else {
                    variants.clear();
                    break;
                }
            }
            declarations.push(Declaration {
                id: d.id,
                parent: (d.parent != usize::MAX).then_some(d.parent),
                scope: d.entry == 0,
                encoding,
                direction: d.direction,
                is_constant: d.is_constant != 0,
                range: (d.has_range != 0).then(|| BitRange::new(d.msb, d.lsb)),
                signedness: d.signedness,
                logic_domain: d.logic_domain,
                // SAFETY: live shim-owned strings, copied into thread-independent data.
                name: unsafe { string(d.name)? }
                    .ok_or_else(|| backend_error("missing WLF name"))?,
                kind: unsafe { string(d.kind)? }
                    .ok_or_else(|| backend_error("missing WLF kind"))?,
                definition: unsafe { string(d.definition)? },
                verilog_name: d.verilog_name != 0,
                variants,
                array_base_len: d.array_base_len as usize,
            });
            encodings.push(encoding);
        }
        state.files.insert(
            id,
            File {
                path,
                name,
                resolution,
                last: metadata.time_span().unwrap().last(),
                encodings,
                native: Some(handle),
                query: None,
            },
        );
        Ok(Opened {
            id,
            metadata,
            declarations,
        })
    })
}
pub(super) fn close(id: u64, allow_idle: bool) {
    let _ = call(move |state| {
        if let Some(mut file) = state.files.remove(&id) {
            // Keep one clean, identity-bound owner only for a fully validated logical Reader.
            if allow_idle
                && state.files.is_empty()
                && let Some(handle) = file.native.take()
            {
                let parked = handle.identity.is_some()
                    && handle.reusable.get()
                    && state.resolution == Some(file.resolution)
                    // SAFETY: sole worker-owned owner; park keeps the file and declarations.
                    && unsafe { ondas_wlf_park(handle.raw.as_ptr()) } != 0;
                if parked {
                    state.idle = Some(Idle {
                        path: file.path.clone(),
                        resolution: file.resolution,
                        handle,
                    });
                }
                // Otherwise the handle is dropped here and closes completely.
            }
        }
        Ok(())
    });
}
pub(super) fn end(id: u64) {
    let _ = call(move |state| {
        if let Some(file) = state.files.get_mut(&id) {
            if let Some(handle) = &file.native {
                // SAFETY: live owner on the SDK thread; inactive reads are allowed.
                unsafe { ondas_wlf_end(handle.raw.as_ptr()) };
            }
            file.query = None;
        }
        Ok(())
    });
}
fn begin_native(
    handle: &Handle,
    indices: &[usize],
    start: Time,
    end: Time,
    point: bool,
) -> Result<()> {
    let mut error = [0; 512];
    // A streaming query answers the first request with a smaller quota; point
    // histories need the full quota from their first request.
    let begin = if point {
        ondas_wlf_begin_point
    } else {
        ondas_wlf_begin
    };
    // SAFETY: owned call-local selection, copied and validated by the shim.
    let code = unsafe {
        begin(
            handle.raw.as_ptr(),
            indices.as_ptr(),
            indices.len(),
            start.ticks(),
            end.ticks(),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    status(code, &error, "begin read").inspect_err(|_| handle.taint())
}
pub(super) fn begin(id: u64, indices: Vec<usize>, start: Time, end: Time) -> Result<()> {
    call(move |state| {
        let file = state.file(id)?;
        let encodings = indices
            .iter()
            .map(|&index| {
                file.encodings
                    .get(index)
                    .copied()
                    .ok_or_else(|| backend_error("invalid WLF declaration index"))
            })
            .collect::<Result<_>>()?;
        file.query = None;
        begin_native(file.native.as_ref().unwrap(), &indices, start, end, false)?;
        file.query = Some(Query {
            indices,
            encodings,
            start,
            end,
            last: None,
            finished: false,
        });
        Ok(())
    })
}
#[derive(Clone)]
struct PointState {
    value: Value,
    changed_at: Option<Time>,
}

// A short suffix proves recent changes. Unresolved projections share one
// chronological pass, retaining only their current value and change time.
fn point_states(
    handle: &Handle,
    indices: &[usize],
    encodings: &[Encoding],
    projections: &[(usize, Signal)],
    start: Option<Time>,
    time: Time,
) -> Result<Vec<Option<PointState>>> {
    let mut states: Vec<Option<PointState>> = vec![None; projections.len()];
    // Group the projections by native selection so a record visits only its own
    // slots, in their previous order.
    let mut slots: Vec<Vec<usize>> = vec![Vec::new(); indices.len()];
    for (slot, &(index, _)) in projections.iter().enumerate() {
        slots[index].push(slot);
    }
    begin_native(handle, indices, start.unwrap_or(Time::ZERO), time, true)?;
    let mut out = NativeRecord::default();
    let mut error = [0; 512];
    loop {
        // SAFETY: live worker-owned cursor and writable call-local descriptor.
        let code = unsafe {
            ondas_wlf_next(
                handle.raw.as_ptr(),
                &mut out,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        status(code, &error, "sample")?;
        if code == 0 {
            break;
        }
        // The suffix's first tick can contain a synthetic entering-state STARTLOG.
        if start.is_some_and(|start| out.tick <= start.ticks()) {
            continue;
        }
        let record = owned_record(&out, encodings)?;
        let crate::ValueRef::Bits(bits) = record.value.as_ref() else {
            return Err(backend_error("WLF point requires bits"));
        };
        let Some(bucket) = slots.get(record.selection) else {
            continue;
        };
        for &slot in bucket {
            let (_, signal) = projections[slot];
            let lsb = signal.lsb();
            let value = bits.slice(lsb + signal.width().unwrap() - 1, lsb);
            match &mut states[slot] {
                Some(previous) => {
                    if !previous
                        .value
                        .as_ref()
                        .same_value(crate::ValueRef::Bits(value))
                    {
                        previous.value = Value::Bits(value.to_owned());
                        previous.changed_at = Some(record.tick);
                    }
                }
                state => {
                    *state = Some(PointState {
                        value: Value::Bits(value.to_owned()),
                        changed_at: None,
                    })
                }
            }
        }
    }
    Ok(states)
}
pub(super) fn sample_bits(
    id: u64,
    indices: Vec<usize>,
    projections: Vec<(usize, Signal)>,
    time: Time,
) -> Result<Vec<Sample>> {
    call(move |state| {
        let file = state.file(id)?;
        file.query = None;
        let handle = file.native.as_ref().unwrap();
        let result = (|| {
            let encodings = indices
                .iter()
                .map(|&index| file.encodings[index])
                .collect::<Vec<_>>();
            let target = time.min(file.last);
            const LOOKBACK: u64 = 32;
            // One whole base can ask the SDK for a suffix start hint. The hint only
            // chooses a candidate start; the point traversal below still proves
            // values, change times and missing samples.
            let hint = if target.ticks() > LOOKBACK
                && indices.len() == 1
                && projections.iter().all(|&(_, signal)| !signal.is_slice())
            {
                let mut start = 0;
                let mut error = [0; 512];
                // SAFETY: live worker-owned handle and writable call-local outputs.
                let code = unsafe {
                    ondas_wlf_point_start(
                        handle.raw.as_ptr(),
                        indices[0],
                        target.ticks(),
                        &mut start,
                        error.as_mut_ptr(),
                        error.len(),
                    )
                };
                status(code, &error, "point start hint")?;
                (code == 1).then(|| Time::from_ticks(start))
            } else {
                None
            };
            let recent = if target.ticks() > LOOKBACK {
                let start = hint.unwrap_or(Time::from_ticks(target.ticks() - LOOKBACK));
                point_states(
                    handle,
                    &indices,
                    &encodings,
                    &projections,
                    Some(start),
                    target,
                )?
            } else {
                vec![None; projections.len()]
            };
            let mut samples: Vec<_> = recent
                .into_iter()
                .zip(&projections)
                .map(|(state, &(_, signal))| {
                    state.and_then(|state| {
                        state.changed_at.map(|at| Sample::Value {
                            signal,
                            value: state.value,
                            changed_at: Some(at),
                        })
                    })
                })
                .collect();
            let mut bases = Vec::new();
            // Maps a native base index to its fallback entry; the first appearance
            // keeps its position.
            let mut base_of = vec![None; indices.len()];
            let mut fallback = Vec::new();
            let mut slots = Vec::new();
            for (slot, &(index, signal)) in projections.iter().enumerate() {
                if samples[slot].is_some() {
                    continue;
                }
                let base = *base_of[index].get_or_insert_with(|| {
                    bases.push(index);
                    bases.len() - 1
                });
                fallback.push((base, signal));
                slots.push(slot);
            }
            if !fallback.is_empty() {
                let indices = bases.iter().map(|&base| indices[base]).collect::<Vec<_>>();
                let encodings = bases
                    .iter()
                    .map(|&base| encodings[base])
                    .collect::<Vec<_>>();
                let states = point_states(handle, &indices, &encodings, &fallback, None, target)?;
                for ((slot, (_, signal)), state) in slots.into_iter().zip(fallback).zip(states) {
                    samples[slot] = Some(match state {
                        Some(PointState { value, changed_at }) => Sample::Value {
                            signal,
                            value,
                            changed_at,
                        },
                        None => Sample::Missing { signal },
                    });
                }
            }
            Ok(samples.into_iter().map(Option::unwrap).collect())
        })();
        // SAFETY: release partial or completed point resources on every Result path.
        unsafe { ondas_wlf_end(handle.raw.as_ptr()) };
        if result.is_err() {
            handle.taint();
        }
        result
    })
}
pub(super) fn next(id: u64) -> Result<Batch> {
    call(move |state| {
        let file = state.file(id)?;
        let handle = file.native.as_ref().unwrap();
        let query = file
            .query
            .as_mut()
            .ok_or_else(|| backend_error("inactive WLF query"))?;
        let mut batch = Batch::default();
        if query.finished {
            batch.finished = true;
            return Ok(batch);
        }
        let mut out = NativeRecord::default();
        let mut error = [0; 512];
        loop {
            // SAFETY: live worker cursor and writable call-local descriptor.
            let code = unsafe {
                ondas_wlf_next(
                    handle.raw.as_ptr(),
                    &mut out,
                    error.as_mut_ptr(),
                    error.len(),
                )
            };
            if let Err(error) = status(code, &error, "read") {
                handle.taint();
                batch.error = Some(Failure::Read(error));
                return Ok(batch);
            }
            if code == 0 {
                query.finished = true;
                batch.finished = true;
                return Ok(batch);
            }
            // Resume from the last transferred ordered slot. Earlier batch slots
            // are caller-owned even if a nested query closes this native owner.
            if query
                .last
                .is_some_and(|last| (out.tick, out.selection) <= last)
            {
                continue;
            }
            match owned_record(&out, &query.encodings) {
                Ok(record) => {
                    query.last = Some((out.tick, out.selection));
                    batch.records.push(record);
                }
                Err(error) => {
                    handle.taint();
                    // Keep the failed record's time so the consumer can finish
                    // an earlier tick before deciding between Break and error.
                    batch.error = Some(if out.selection < query.encodings.len() {
                        Failure::Decode {
                            selection: out.selection,
                            tick: Time::from_ticks(out.tick),
                            error,
                        }
                    } else {
                        Failure::Read(error)
                    });
                    return Ok(batch);
                }
            }
            // SAFETY: same live owner; this check only inspects the completed-tick batch.
            let end = unsafe { ondas_wlf_batch_end(handle.raw.as_ptr()) };
            if end != 0 {
                batch.finished = end == 2;
                query.finished = batch.finished;
                return Ok(batch);
            }
        }
    })
}

fn owned_record(out: &NativeRecord, encodings: &[Encoding]) -> Result<Record> {
    let encoding = *encodings
        .get(out.selection)
        .ok_or_else(|| backend_error("invalid WLF selection index"))?;
    let value = match encoding {
        Encoding::Bits { width } => {
            if out.data.is_null() || out.len != width as usize {
                return Err(backend_error("invalid WLF bit buffer"));
            }
            // SAFETY: validated bounded shim-owned bytes, copied before next operation.
            let bytes = unsafe { std::slice::from_raw_parts(out.data, out.len) };
            Value::Bits(BitsRef::from_validated_ascii(bytes).to_owned())
        }
        Encoding::Real => Value::Real(f64::from_bits(out.real_bits)),
        Encoding::String => {
            if out.len > isize::MAX as usize || (out.len != 0 && out.data.is_null()) {
                return Err(backend_error("invalid WLF string buffer"));
            }
            let bytes = if out.len == 0 {
                &[]
            } else {
                // SAFETY: bounded live shim storage, copied before the next operation.
                unsafe { std::slice::from_raw_parts(out.data, out.len) }
            };
            Value::String(
                crate::value::ascii_string(
                    bytes,
                    Format::Wlf,
                    "wlf-lib",
                    Time::from_ticks(out.tick),
                )?
                .into(),
            )
        }
        Encoding::Event if out.occurrences > 0 => Value::Event {
            occurrences: out.occurrences,
        },
        _ => return Err(backend_error("invalid WLF value descriptor")),
    };
    Ok(Record {
        selection: out.selection,
        tick: Time::from_ticks(out.tick),
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wlf_status_preserves_known_categories_and_unknown_diagnostics() {
        let mut buffer = [0; 512];
        for (out, byte) in buffer.iter_mut().zip(b"SDK status 4: sentinel") {
            *out = *byte as c_char;
        }
        assert!(matches!(status(-2, &buffer, "open"), Err(Error::Malformed {
            format: Format::Wlf, backend, message
        }) if backend == "wlf-lib" && message == "SDK status 4: sentinel"));
        assert!(
            matches!(status(-3, &buffer, "open"), Err(Error::BackendUnavailable { backend, .. })
            if backend == "wlf-lib")
        );
        for code in [-1, -71] {
            assert!(matches!(status(code, &buffer, "read"), Err(Error::Backend {
                backend, operation: "read", message
            }) if backend == "wlf-lib" && message == "SDK status 4: sentinel"));
        }
    }

    #[test]
    fn wlf_optional_enum_text_preserves_unicode_and_rejects_invalid_text() {
        let bits = c"001x";
        let label = CString::new("état").unwrap();
        let invalid = [255u8, 0];
        // SAFETY: all inputs are live and NUL-terminated, including invalid UTF-8.
        unsafe {
            assert_eq!(
                enum_strings(bits.as_ptr(), label.as_ptr()),
                Some(("001x".into(), "état".into()))
            );
            assert!(enum_strings(bits.as_ptr(), invalid.as_ptr().cast()).is_none());
            assert!(enum_strings(std::ptr::null(), label.as_ptr()).is_none());
        }
    }
    fn failed_batch() -> Batch {
        Batch {
            records: vec![
                Record {
                    selection: 0,
                    tick: Time::ZERO,
                    value: Value::Real(1.0),
                },
                Record {
                    selection: 0,
                    tick: Time::from_ticks(1),
                    value: Value::Real(2.0),
                },
            ],
            error: Some(Failure::Read(backend_error(
                "later value conversion failed",
            ))),
            finished: false,
        }
    }
    #[test]
    fn wlf_batch_keeps_successful_prefix_before_error() {
        let mut ticks = Vec::new();
        let result = failed_batch().visit(|_, tick, value| {
            assert!(value.is_ok());
            ticks.push(tick.ticks());
            ControlFlow::<()>::Continue(())
        });
        assert!(matches!(result, Err(Error::Backend { .. })));
        assert_eq!(ticks, [0, 1]);
    }
    #[test]
    fn wlf_batch_caller_break_wins_over_deferred_error() {
        let result = failed_batch().visit(|_, _, _| ControlFlow::Break(std::rc::Rc::new("stop")));
        assert_eq!(*result.unwrap().break_value().unwrap(), "stop");
    }
    #[test]
    fn wlf_batch_timed_failure_keeps_exact_location_and_successful_prefix() {
        for failed_tick in [0, 1, 2] {
            let mut batch = failed_batch();
            batch
                .records
                .retain(|record| record.tick.ticks() <= failed_tick);
            if failed_tick == 0 {
                batch.records.clear();
            }
            batch.error = Some(Failure::Decode {
                selection: 7,
                tick: Time::from_ticks(failed_tick),
                error: backend_error("sentinel"),
            });
            let mut prefix = Vec::new();
            let result = batch
                .visit(|selection, tick, value| match value {
                    Ok(_) => {
                        prefix.push(tick.ticks());
                        ControlFlow::Continue(())
                    }
                    Err(error) => {
                        assert_eq!(selection, 7);
                        assert_eq!(tick, Time::from_ticks(failed_tick));
                        ControlFlow::Break(error)
                    }
                })
                .unwrap();
            assert!(
                matches!(result, ControlFlow::Break(Error::Backend { message, .. }) if message == "sentinel")
            );
            assert_eq!(prefix, if failed_tick == 0 { vec![] } else { vec![0, 1] });
        }
    }
    #[test]
    fn wlf_batch_timed_failure_cannot_be_ignored_or_override_break() {
        for stop in [false, true] {
            let mut batch = failed_batch();
            batch.error = Some(Failure::Decode {
                selection: 0,
                tick: Time::from_ticks(2),
                error: backend_error("later failure"),
            });
            let result = batch.visit(|_, _, _| {
                if stop {
                    ControlFlow::Break(17)
                } else {
                    ControlFlow::Continue(())
                }
            });
            if stop {
                assert_eq!(result.unwrap(), ControlFlow::Break(17));
            } else {
                assert!(
                    matches!(result, Err(Error::Backend { message, .. }) if message == "WLF visitor continued after a decoding failure")
                );
            }
        }
    }
    #[test]
    fn wlf_private_abi_layouts() {
        assert_eq!(std::mem::size_of::<Meta>(), 32);
        assert_eq!(std::mem::size_of::<NativeDeclaration>(), 96);
        assert_eq!(std::mem::size_of::<NativeRecord>(), 48);
        assert_eq!(std::mem::offset_of!(NativeDeclaration, name), 64);
        assert_eq!(std::mem::offset_of!(NativeDeclaration, verilog_name), 88);
        assert_eq!(std::mem::offset_of!(NativeDeclaration, array_base_len), 92);
        assert_eq!(std::mem::offset_of!(NativeRecord, data), 24);
    }
}
