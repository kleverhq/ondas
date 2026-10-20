//! Keep every SDK object and call on one OS thread for the process lifetime.
use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_int, c_void},
    ops::ControlFlow,
    ptr::NonNull,
    sync::{OnceLock, mpsc},
};

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
    fn ondas_wlf_next(
        reader: *mut c_void,
        out: *mut NativeRecord,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_wlf_batch_end(reader: *mut c_void) -> c_int;
    fn ondas_wlf_end(reader: *mut c_void);
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
#[derive(Default)]
pub(super) struct Batch {
    pub records: Vec<Record>,
    pub finished: bool,
    pub error: Option<Error>,
}
impl Batch {
    pub fn visit<B>(
        self,
        mut visitor: impl FnMut(Record) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        for record in self.records {
            if let ControlFlow::Break(value) = visitor(record) {
                return Ok(ControlFlow::Break(value));
            }
        }
        if let Some(error) = self.error {
            return Err(error);
        }
        Ok(ControlFlow::Continue(()))
    }
}

// This type never leaves State on the worker thread; no Send/Sync assertion is needed.
struct Handle {
    raw: NonNull<c_void>,
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
struct State {
    files: HashMap<u64, File>,
    next_id: u64,
    resolution: Option<u32>,
}
impl State {
    fn reset(&mut self) {
        // Header inspection can also select a process resolution in this SDK.
        // Suspend native owners before reading a new header, not just before open.
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
                begin_native(&handle, &query.indices, query.start, query.end)?;
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
fn status(code: c_int, error: &[c_char; 512]) -> Result<()> {
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
        _ => backend_error(message),
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
    status(code, &error)?;
    Ok(Handle {
        raw: NonNull::new(raw).ok_or_else(|| backend_error("null WLF reader"))?,
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
        state.reset();
        let handle = open_handle(&path, &name, false)?;
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
pub(super) fn close(id: u64) {
    let _ = call(move |state| {
        state.files.remove(&id);
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
fn begin_native(handle: &Handle, indices: &[usize], start: Time, end: Time) -> Result<()> {
    let mut error = [0; 512];
    // SAFETY: owned call-local selection, copied and validated by the shim.
    let code = unsafe {
        ondas_wlf_begin(
            handle.raw.as_ptr(),
            indices.as_ptr(),
            indices.len(),
            start.ticks(),
            end.ticks(),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    status(code, &error)
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
        begin_native(file.native.as_ref().unwrap(), &indices, start, end)?;
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
    begin_native(handle, indices, start.unwrap_or(Time::ZERO), time)?;
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
        status(code, &error)?;
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
        for (slot, &(index, signal)) in projections.iter().enumerate() {
            if index != record.selection {
                continue;
            }
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
            let recent = if target.ticks() > LOOKBACK {
                point_states(
                    handle,
                    &indices,
                    &encodings,
                    &projections,
                    Some(Time::from_ticks(target.ticks() - LOOKBACK)),
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
            let mut fallback = Vec::new();
            let mut slots = Vec::new();
            for (slot, &(index, signal)) in projections.iter().enumerate() {
                if samples[slot].is_some() {
                    continue;
                }
                let base = bases
                    .iter()
                    .position(|&previous| previous == index)
                    .unwrap_or_else(|| {
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
            if let Err(error) = status(code, &error) {
                batch.error = Some(error);
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
                    // A later conversion failure must not erase the prefix or
                    // override an earlier caller Break.
                    batch.error = Some(error);
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
            error: Some(backend_error("later value conversion failed")),
            finished: false,
        }
    }
    #[test]
    fn wlf_batch_keeps_successful_prefix_before_error() {
        let mut ticks = Vec::new();
        let result = failed_batch().visit(|record| {
            ticks.push(record.tick.ticks());
            ControlFlow::<()>::Continue(())
        });
        assert!(matches!(result, Err(Error::Backend { .. })));
        assert_eq!(ticks, [0, 1]);
    }
    #[test]
    fn wlf_batch_caller_break_wins_over_deferred_error() {
        let result = failed_batch().visit(|_| ControlFlow::Break(std::rc::Rc::new("stop")));
        assert_eq!(*result.unwrap().break_value().unwrap(), "stop");
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
