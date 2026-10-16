//! Private build-selected SST2 adapter. No vendor handles survive an FFI call.
use crate::{
    BitRange, BitsRef, Direction, Encoding, Error, Hierarchy, Metadata, Result, Signal, Time,
    TimeSpan, TimeUnit, Timescale, ValueRef,
    hierarchy::{ScopeData, VariableData},
};
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    ops::ControlFlow,
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr::NonNull,
    sync::Mutex,
};
const BACKEND: &str = "shm-lib";
// SST2 shares open files and a process-wide time unit. Serialize complete native
// sessions, copy selected records, and close before unlocking or calling users.
static SDK: Mutex<()> = Mutex::new(());
fn lock() -> std::sync::MutexGuard<'static, ()> {
    SDK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
#[repr(C)]
#[derive(Default)]
struct Declaration {
    id: u64,
    entry: u32,
    encoding: u32,
    width: u32,
    has_range: u32,
    is_constant: u32,
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
    exponent: i32,
}
#[repr(C)]
#[derive(Default)]
struct Record {
    id: u64,
    tick: u64,
    encoding: u32,
    data: *const u8,
    len: usize,
}
#[repr(C)]
#[derive(Clone, Default)]
struct Point {
    changed_at: u64,
    data: *const u8,
    len: usize,
    changed: c_int,
    certified: c_int,
}
#[cfg(test)]
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Work {
    pub(crate) sessions: u64,
    pub(crate) hierarchy_nodes: u64,
    pub(crate) histories: u64,
    pub(crate) history_records: u64,
}
unsafe extern "C" {
    fn ondas_shm_open(
        path: *const c_char,
        metadata_only: c_int,
        out: *mut *mut c_void,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_shm_close(reader: *mut c_void);
    fn ondas_shm_metadata(reader: *const c_void, out: *mut Meta);
    #[cfg(test)]
    fn ondas_shm_work_done(reader: *const c_void, out: *mut Work);
    fn ondas_shm_decl_count(reader: *const c_void) -> usize;
    fn ondas_shm_declaration(reader: *const c_void, index: usize, out: *mut Declaration);
    fn ondas_shm_read(
        reader: *mut c_void,
        ids: *const u64,
        count: usize,
        from: u64,
        end: u64,
        validated_resume: c_int,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_shm_next(
        reader: *mut c_void,
        out: *mut Record,
        error: *mut c_char,
        cap: usize,
    ) -> c_int;
    fn ondas_shm_end(reader: *mut c_void);
    fn ondas_shm_sample_bits(
        reader: *mut c_void,
        ids: *const u64,
        count: usize,
        end: u64,
        prepare_fallback: c_int,
        out: *mut Point,
    ) -> c_int;
}
fn error(message: impl Into<String>) -> Error {
    Error::Backend {
        backend: BACKEND.into(),
        operation: "read SHM",
        message: message.into(),
    }
}
fn status(code: c_int, buffer: &[c_char; 512]) -> Result<()> {
    if code < 0 {
        // SAFETY: the shim terminates every caller-owned error buffer.
        Err(error(
            unsafe { CStr::from_ptr(buffer.as_ptr()) }.to_string_lossy(),
        ))
    } else {
        Ok(())
    }
}
// SAFETY: the shim's owned hierarchy strings are NUL terminated and remain
// alive on the owner. Every caller copies them under the adapter lock.
unsafe fn string(pointer: *const c_char) -> Result<Option<String>> {
    if pointer.is_null() {
        return Ok(None);
    }
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map(|s| Some(s.to_owned()))
        .map_err(|_| error("non-UTF-8 SHM declaration text"))
}
struct Handle(NonNull<c_void>);
// SAFETY: this pointer owns only C++ strings/vectors and a path. The shim closes
// every SDK session before returning. No vendor handle, thread-local buffer or
// callback survives. All native operations/destruction are serialized; mutable
// traversal additionally requires &mut Reader. User callbacks run unlocked.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        let _lock = lock();
        // SAFETY: sole owner of an Ondas C++ object, with no SDK session.
        unsafe { ondas_shm_close(self.0.as_ptr()) };
    }
}
fn open_handle(path: &Path, metadata_only: bool) -> Result<Handle> {
    if !path.is_dir() {
        return Err(error("SHM input requires a database directory"));
    }
    let path = path.canonicalize()?;
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "NUL in SHM path",
        ))
    })?;
    let mut raw = std::ptr::null_mut();
    let mut buffer = [0; 512];
    let _lock = lock();
    // SAFETY: valid caller-owned buffers. No pointers into Rust are retained.
    let code = unsafe {
        ondas_shm_open(
            path.as_ptr(),
            metadata_only.into(),
            &mut raw,
            buffer.as_mut_ptr(),
            buffer.len(),
        )
    };
    status(code, &buffer)?;
    Ok(Handle(
        NonNull::new(raw).ok_or_else(|| error("null SHM owner"))?,
    ))
}
fn metadata(handle: &Handle, source_name: String) -> Result<Metadata> {
    let mut meta = Meta::default();
    let _lock = lock();
    // SAFETY: a live Ondas-owned immutable metadata object and writable output.
    unsafe { ondas_shm_metadata(handle.0.as_ptr(), &mut meta) };
    let (factor, unit) = match meta.exponent {
        -15..=-13 => (
            10_u32.pow((meta.exponent + 15) as u32),
            TimeUnit::Femtosecond,
        ),
        -12..=-10 => (
            10_u32.pow((meta.exponent + 12) as u32),
            TimeUnit::Picosecond,
        ),
        -9..=-7 => (10_u32.pow((meta.exponent + 9) as u32), TimeUnit::Nanosecond),
        -6..=-4 => (
            10_u32.pow((meta.exponent + 6) as u32),
            TimeUnit::Microsecond,
        ),
        -3..=-1 => (
            10_u32.pow((meta.exponent + 3) as u32),
            TimeUnit::Millisecond,
        ),
        0..=2 => (10_u32.pow(meta.exponent as u32), TimeUnit::Second),
        _ => return Err(error("unsupported SHM timescale exponent")),
    };
    Ok(Metadata {
        source_name,
        timescale: Some(Timescale::new(factor, unit)),
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
    let handle = open_handle(path, true)?;
    metadata(&handle, name)
}
pub(crate) struct Reader {
    handle: Handle,
    ids: Vec<u64>,
    indices: Vec<Option<usize>>,
    encodings: Vec<Encoding>,
    point_fallback: Option<(Vec<u64>, Time)>,
    #[cfg(test)]
    pub(crate) native_reads: usize,
}
impl Reader {
    #[cfg(test)]
    pub(crate) fn work(&self) -> Work {
        let _lock = lock();
        let mut work = Work::default();
        // SAFETY: the live owner copies its own counters without SDK access.
        unsafe { ondas_shm_work_done(self.handle.0.as_ptr(), &mut work) };
        work
    }
    pub(crate) fn open(path: &Path, name: String) -> Result<(Self, Hierarchy, Metadata)> {
        let handle = open_handle(path, false)?;
        let meta = metadata(&handle, name)?;
        let (mut scopes, mut variables, mut stack) = (Vec::new(), Vec::new(), Vec::new());
        let (mut ids, mut indices, mut encodings) = (Vec::new(), Vec::new(), Vec::new());
        let guard = lock();
        // SAFETY: count and descriptors belong to this immutable live owner.
        unsafe {
            let count = ondas_shm_decl_count(handle.0.as_ptr());
            indices.resize(count, None);
            for index in 0..count {
                let mut d = Declaration::default();
                ondas_shm_declaration(handle.0.as_ptr(), index, &mut d);
                let name = string(d.name)?.unwrap_or_default();
                let kind = string(d.kind)?.unwrap_or_default();
                match d.entry {
                    0 => {
                        let id = scopes.len();
                        scopes.push(ScopeData {
                            name,
                            name_was_escaped: false,
                            parent: stack.last().copied(),
                            kind,
                            definition_name: string(d.definition)?,
                            packing: None,
                            is_hidden: false,
                        });
                        stack.push(id);
                    }
                    1 => {
                        stack
                            .pop()
                            .ok_or_else(|| error("SHM scope stack underflow"))?;
                    }
                    2 => {
                        let encoding = match d.encoding {
                            0 => Encoding::Unsupported,
                            1 if d.width > 0 => Encoding::Bits { width: d.width },
                            2 => Encoding::Real,
                            3 => Encoding::String,
                            4 => Encoding::Event,
                            _ => return Err(error("invalid SHM encoding descriptor")),
                        };
                        let identity = usize::try_from(d.id)
                            .ok()
                            .and_then(|id| indices.get_mut(id))
                            .ok_or_else(|| error("invalid SHM declaration identity"))?;
                        let signal = if let Some(signal) = *identity {
                            signal
                        } else {
                            let signal = ids.len();
                            ids.push(d.id);
                            *identity = Some(signal);
                            encodings.push(encoding);
                            signal
                        };
                        if encodings[signal] != encoding {
                            return Err(error("incompatible SHM alias"));
                        }
                        variables.push(VariableData {
                            name,
                            reader_name: None,
                            name_was_escaped: false,
                            parent: stack.last().copied(),
                            kind,
                            direction: Direction::Unknown,
                            range: (d.has_range != 0).then(|| BitRange::new(d.msb, d.lsb)),
                            is_constant: d.is_constant != 0,
                            type_name: None,
                            signedness: None,
                            logic_domain: None,
                            enumeration: None,
                            signal: Some(signal),
                        });
                    }
                    _ => return Err(error("invalid SHM declaration entry")),
                }
            }
        }
        if !stack.is_empty() {
            return Err(error("unclosed SHM scopes"));
        }
        drop(guard);
        let hierarchy = Hierarchy::new(scopes, variables, encodings.clone());
        Ok((
            Self {
                handle,
                ids,
                indices,
                encodings,
                point_fallback: None,
                #[cfg(test)]
                native_reads: 0,
            },
            hierarchy,
            meta,
        ))
    }
    pub(crate) fn sample_bits(
        &mut self,
        bases: &[Signal],
        time: Time,
        prepare_fallback: bool,
    ) -> Result<Option<Vec<Option<crate::Sample>>>> {
        let ids: Vec<_> = bases.iter().map(|s| self.ids[s.index()]).collect();
        // A mismatch continues into a new native point call below. That call
        // discards every previous prepared batch before opening its session.
        if self
            .point_fallback
            .take()
            .is_some_and(|(prepared_ids, at)| prepared_ids == ids && at == time)
        {
            // A mixed point already collected this subset in the native call.
            // Let the normalizer consume that handoff without trying it again.
            return Ok(None);
        }
        let mut points = vec![Point::default(); bases.len()];
        #[cfg(test)]
        {
            self.native_reads += 1;
        }
        let traversal = Traversal(&mut self.handle);
        let mut keep_fallback = false;
        let samples = {
            let _lock = lock();
            // SAFETY: exclusive live owner, validated identities and a writable
            // descriptor array. Copy Ondas-owned bytes before unlocking/end;
            // no SDK handle survives. Partial success prepares only its subset.
            let code = unsafe {
                ondas_shm_sample_bits(
                    traversal.0.0.as_ptr(),
                    ids.as_ptr(),
                    ids.len(),
                    time.ticks(),
                    c_int::from(prepare_fallback),
                    points.as_mut_ptr(),
                )
            };
            if code == 0 {
                // The shim cleared point data or prepared owned ordinary rows
                // for the immediate fallback. Its read guard will release them.
                if prepare_fallback {
                    self.point_fallback = Some((ids, time));
                    keep_fallback = true;
                }
                None
            } else {
                if !matches!(code, 1 | 2) {
                    return Err(error("invalid SHM point result"));
                }
                let samples = bases
                    .iter()
                    .zip(points)
                    .map(|(&signal, point)| {
                        if point.certified == 0 {
                            if !point.data.is_null()
                                || point.len != 0
                                || point.changed != 0
                                || point.changed_at != 0
                            {
                                return Err(error("invalid SHM fallback descriptor"));
                            }
                            return Ok(None);
                        }
                        let Encoding::Bits { width } = signal.encoding() else {
                            return Err(error("invalid SHM point encoding"));
                        };
                        if point.certified != 1
                            || point.len != width as usize
                            || point.data.is_null()
                            || point.len > isize::MAX as usize
                            || !matches!(point.changed, 0 | 1)
                            || (point.changed == 1
                                && (point.changed_at == 0 || point.changed_at > time.ticks()))
                        {
                            return Err(error("invalid SHM point value"));
                        }
                        // SAFETY: the successful shim result supplies exactly
                        // len readable bytes; owner is locked through the copy.
                        let data = unsafe { std::slice::from_raw_parts(point.data, point.len) };
                        if data
                            .iter()
                            .any(|byte| !matches!(byte, b'0' | b'1' | b'x' | b'z'))
                        {
                            return Err(error("invalid SHM point logic state"));
                        }
                        Ok(Some(crate::Sample::Value {
                            signal,
                            value: crate::Value::Bits(
                                BitsRef::from_validated_ascii(data).to_owned(),
                            ),
                            changed_at: (point.changed == 1)
                                .then_some(Time::from_ticks(point.changed_at)),
                        }))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let missing = samples.iter().filter(|sample| sample.is_none()).count();
                if (code == 1 && missing != 0)
                    || (code == 2 && (missing == 0 || missing == bases.len()))
                {
                    return Err(error("invalid SHM partial point result"));
                }
                if code == 2 {
                    self.point_fallback = Some((
                        ids.into_iter()
                            .zip(&samples)
                            .filter_map(|(id, sample)| sample.is_none().then_some(id))
                            .collect(),
                        time,
                    ));
                    keep_fallback = true;
                }
                Some(samples)
            }
        };
        if keep_fallback {
            // Reader owns these buffers until the immediate matching read.
            // Other native reads/points and empty reads discard the handoff;
            // dropping Reader also releases it. No SDK state is retained.
            std::mem::forget(traversal);
        }
        Ok(samples)
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
        from: Time,
        end: Time,
        visitor: impl for<'v> FnMut(usize, Time, Result<ValueRef<'v>>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        self.read_from_with_resume(signals, from, end, false, visitor)
    }
    // Resume requires a successful immutable-prefix checkpoint with known
    // entering values for every selected base. Ordinary delivery filters do not.
    pub(crate) fn read_from_with_resume<B>(
        &mut self,
        signals: &[Signal],
        from: Time,
        end: Time,
        resume: bool,
        mut visitor: impl for<'v> FnMut(usize, Time, Result<ValueRef<'v>>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        self.point_fallback = None;
        if from > end {
            // No SDK session is needed, but any pending owned rows must end.
            let _traversal = Traversal(&mut self.handle);
            return Ok(ControlFlow::Continue(()));
        }
        #[cfg(test)]
        {
            self.native_reads += 1;
        }
        let ids: Vec<_> = signals.iter().map(|s| self.ids[s.index()]).collect();
        let traversal = Traversal(&mut self.handle);
        let mut buffer = [0; 512];
        {
            let _lock = lock();
            // SAFETY: live exclusive owner, validated identities and output buffers.
            // Resume uses the caller's successful immutable-prefix checkpoint.
            // The shim closes its session after copying selected histories.
            let code = unsafe {
                ondas_shm_read(
                    traversal.0.0.as_ptr(),
                    ids.as_ptr(),
                    ids.len(),
                    from.ticks(),
                    end.ticks(),
                    c_int::from(resume),
                    buffer.as_mut_ptr(),
                    buffer.len(),
                )
            };
            status(code, &buffer)?;
        }
        let mut data = Vec::new();
        let mut previous = 0;
        loop {
            let mut record = Record::default();
            let failure;
            {
                let _lock = lock();
                // SAFETY: live owner and writable output. Copy the private buffer
                // before unlocking or calling users; no vendor call occurs here.
                let code = unsafe {
                    ondas_shm_next(
                        traversal.0.0.as_ptr(),
                        &mut record,
                        buffer.as_mut_ptr(),
                        buffer.len(),
                    )
                };
                failure = status(code, &buffer).err();
                if code == 0 {
                    break;
                }
                if record.tick < previous || record.tick > end.ticks() {
                    return Err(error("invalid SHM record order"));
                }
                previous = record.tick;
                if record.tick < from.ticks() {
                    if let Some(failure) = failure {
                        return Err(failure);
                    }
                    continue;
                }
                data.clear();
                if record.len > 0 {
                    if record.data.is_null() || record.len > isize::MAX as usize {
                        return Err(error("invalid SHM value buffer"));
                    }
                    data.extend_from_slice(unsafe {
                        std::slice::from_raw_parts(record.data, record.len)
                    });
                }
            }
            let index = usize::try_from(record.id)
                .ok()
                .and_then(|id| self.indices.get(id))
                .copied()
                .flatten()
                .ok_or_else(|| error("unknown SHM history identity"))?;
            let value = if let Some(failure) = failure {
                Err(failure)
            } else {
                (|| {
                    Ok(match (self.encodings[index], record.encoding) {
                        (Encoding::Bits { width }, 1) if data.len() == width as usize => {
                            ValueRef::Bits(BitsRef::from_validated_ascii(&data))
                        }
                        (Encoding::Real, 2) if data.len() == 8 => {
                            ValueRef::Real(f64::from_ne_bytes(data.as_slice().try_into().unwrap()))
                        }
                        (Encoding::String, 3) => ValueRef::String(
                            std::str::from_utf8(&data)
                                .map_err(|_| error("non-UTF-8 SHM string value"))?,
                        ),
                        (Encoding::Event, 4) => ValueRef::Event { occurrences: 1 },
                        _ => return Err(error("SHM value disagrees with declaration")),
                    })
                })()
            };
            let failed = value.is_err();
            if let ControlFlow::Break(value) = visitor(index, Time::from_ticks(record.tick), value)
            {
                return Ok(ControlFlow::Break(value));
            }
            if failed {
                return Err(error("SHM visitor continued after a decoding failure"));
            }
        }
        Ok(ControlFlow::Continue(()))
    }
}
struct Traversal<'a>(&'a mut Handle);
impl Drop for Traversal<'_> {
    fn drop(&mut self) {
        let _lock = lock();
        // SAFETY: exclusive live owner; includes Break, error and visitor panic.
        unsafe { ondas_shm_end(self.0.0.as_ptr()) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(
        reader: &mut Reader,
        signals: &[Signal],
        from: u64,
        end: u64,
        resume: bool,
    ) -> (String, Vec<(usize, u64, String)>) {
        let mut records = Vec::new();
        let result = reader.read_from_with_resume(
            signals,
            Time::from_ticks(from),
            Time::from_ticks(end),
            resume,
            |index, time, value| {
                let failed = value.is_err();
                records.push((index, time.ticks(), format!("{value:?}")));
                if failed {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        );
        (format!("{result:?}"), records)
    }

    #[test]
    #[ignore = "requires real SHM runtime and installed fixtures"]
    fn shm_point_fallback_preserves_read_boundaries() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/shm");
        let path = root.join("shm0080-history-short/waveform.shm");
        for (names, from, end, resume, prepare, clear) in [
            (vec!["top.phase_0", "top.step"], 0, 7, false, true, false),
            (vec!["top.step", "top.phase_0"], 0, 7, false, true, false),
            (vec!["top.phase_0"], 0, 7, false, true, false),
            (vec!["top.step"], 0, 4, false, true, false),
            (vec!["top.phase_0", "top.step"], 0, 6, false, true, false),
            (vec!["top.phase_0", "top.step"], 2, 7, false, true, false),
            (vec!["top.phase_0", "top.step"], 6, 7, true, true, false),
            (vec!["top.step"], 8, 7, false, true, false),
            (vec!["top.phase_0", "top.step"], 0, 7, false, false, false),
            (vec!["top.phase_0", "top.step"], 0, 7, false, true, true),
        ] {
            let (mut reader, hierarchy, _) = Reader::open(&path, "handoff".into()).unwrap();
            let bases = [hierarchy.signal("top.step").unwrap()];
            assert!(
                reader
                    .sample_bits(&bases, Time::from_ticks(7), prepare)
                    .unwrap()
                    .is_none()
            );
            {
                let _lock = lock();
                let mut record = Record::default();
                let mut buffer = [0; 512];
                // SAFETY: exclusive live owner and writable outputs. A preparing
                // rejection must survive the Rust method; its matching read
                // rewinds this one inspected record before consuming the batch.
                let code = unsafe {
                    ondas_shm_next(
                        reader.handle.0.as_ptr(),
                        &mut record,
                        buffer.as_mut_ptr(),
                        buffer.len(),
                    )
                };
                assert_eq!(code, if prepare { 1 } else { 0 });
                if prepare {
                    assert_eq!(record.id, reader.ids[bases[0].index()]);
                    assert_eq!(record.tick, 0);
                    assert_eq!(record.len, 32);
                    assert!(!record.data.is_null());
                    assert!(matches!(unsafe { *record.data }, b'0' | b'1' | b'x' | b'z'));
                }
            }
            if clear {
                let _lock = lock();
                // SAFETY: exclusive live owner; end discards its private buffer.
                unsafe { ondas_shm_end(reader.handle.0.as_ptr()) };
            }
            let signals: Vec<_> = names
                .iter()
                .map(|name| hierarchy.signal(name).unwrap())
                .collect();
            let (mut reference, _, _) = Reader::open(&path, "handoff".into()).unwrap();
            let expected = records(&mut reference, &signals, from, end, resume);
            assert_eq!(records(&mut reader, &signals, from, end, resume), expected);
            if from > end {
                let _lock = lock();
                let mut record = Record::default();
                let mut buffer = [0; 512];
                // SAFETY: exclusive owner and writable outputs. Even an empty
                // read must release any earlier prepared ordinary rows.
                assert_eq!(
                    unsafe {
                        ondas_shm_next(
                            reader.handle.0.as_ptr(),
                            &mut record,
                            buffer.as_mut_ptr(),
                            buffer.len(),
                        )
                    },
                    0
                );
            }
            // Consuming or rejecting a handoff must leave a fresh read valid.
            assert_eq!(records(&mut reader, &signals, from, end, resume), expected);
        }

        let path = root.join("shm0089-probe-gap/waveform.shm");
        let (mut reader, hierarchy, _) = Reader::open(&path, "gap".into()).unwrap();
        let signals = [hierarchy.signal("top.glitch").unwrap()];
        assert!(
            reader
                .sample_bits(&signals, Time::from_ticks(3), true)
                .unwrap()
                .is_none()
        );
        let (mut reference, _, _) = Reader::open(&path, "gap".into()).unwrap();
        let expected = records(&mut reference, &signals, 0, 3, false);
        assert!(
            expected
                .1
                .iter()
                .any(|(_, tick, value)| *tick == 2 && value.starts_with("Err("))
        );
        assert_eq!(records(&mut reader, &signals, 0, 3, false), expected);
        assert!(
            reader
                .sample_bits(&signals, Time::from_ticks(1), true)
                .unwrap()
                .is_some()
        );
    }
}
