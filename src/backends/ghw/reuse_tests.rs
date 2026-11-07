//! Focused tests for validated GHW reuse.
//!
//! Every test drives its own private [`Cache`], so reuse behaviour is observable
//! without depending on which other tests have opened GHW content.
use super::*;
use std::io::{BufRead, Cursor, Read, Seek, SeekFrom};
use std::sync::atomic::Ordering;

/// A value one read delivered, comparable across two logical opens.
#[derive(Debug, PartialEq)]
enum Observation {
    Value {
        signal: usize,
        time: u64,
        value: String,
    },
    Tick {
        time: u64,
    },
}

fn open(
    bytes: &[u8],
    name: &str,
    purpose: Purpose,
    cache: &Cache,
) -> Result<(Reader, Hierarchy, Metadata)> {
    Reader::open_with(
        Box::new(Cursor::new(bytes.to_vec())),
        name.into(),
        purpose,
        cache,
    )
}

fn open_buffer(
    input: Box<dyn Input>,
    name: &str,
    purpose: Purpose,
    cache: &Cache,
) -> Result<(Reader, Hierarchy, Metadata)> {
    Reader::open_with(input, name.into(), purpose, cache)
}

/// Read every selected signal through the reader and record what it delivered.
fn observe(reader: &mut Reader, hierarchy: &Hierarchy, end: u64) -> Vec<Observation> {
    let signals = hierarchy.signals().collect::<Vec<_>>();
    let mut seen = Vec::new();
    let _ = reader
        .read(&signals, Time::from_ticks(end), |notification| {
            match notification {
                Notification::Value { index, time, value } => seen.push(Observation::Value {
                    signal: index,
                    time: time.ticks(),
                    value: format!("{value:?}"),
                }),
                Notification::Tick(time) => seen.push(Observation::Tick { time: time.ticks() }),
            }
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    seen
}

/// The error of a failed open, without requiring `Debug` on the success value.
fn error(result: Result<(Reader, Hierarchy, Metadata)>) -> crate::Error {
    match result {
        Ok(_) => panic!("expected an open error"),
        Err(error) => error,
    }
}

/// Sample content without hierarchy identity, so runs can be compared.
fn sample_text(sample: &crate::Sample) -> String {
    match sample {
        crate::Sample::Event { .. } => "event".into(),
        crate::Sample::Missing { .. } => "missing".into(),
        crate::Sample::Value {
            value, changed_at, ..
        } => {
            let changed = changed_at.map_or("none".to_string(), |time| time.ticks().to_string());
            match value {
                crate::Value::Bits(bits) => format!("bits:{} changed:{changed}", bits.as_ref()),
                crate::Value::Real(value) => {
                    format!("real:{:016x} changed:{changed}", value.to_bits())
                }
                other => format!("{other:?} changed:{changed}"),
            }
        }
    }
}

fn diagnostics(cache: &Cache) -> (usize, usize) {
    (
        cache.hits.load(Ordering::Relaxed),
        cache.parses.load(Ordering::Relaxed),
    )
}

#[test]
fn a_hit_reports_the_same_observations_with_a_fresh_hierarchy() {
    let bytes = test_bytes::recording(4);
    let cache = Cache::new();
    let (mut cold, cold_hierarchy, cold_metadata) =
        open(&bytes, "reuse.ghw", Purpose::Waveform, &cache).unwrap();
    assert_eq!(diagnostics(&cache), (0, 1));
    assert!(cache.parked_identity().is_some());
    let cold_seen = observe(&mut cold, &cold_hierarchy, u64::MAX);
    let cold_paths = cold_hierarchy
        .variables()
        .map(|variable| variable.path().to_string())
        .collect::<Vec<_>>();
    let cold_handle = cold_hierarchy.signal("top.control").unwrap();
    drop(cold);

    let (mut warm, warm_hierarchy, warm_metadata) =
        open(&bytes, "reuse.ghw", Purpose::Waveform, &cache).unwrap();
    assert_eq!(diagnostics(&cache), (1, 1));
    assert_eq!(warm_metadata.time_span, cold_metadata.time_span);
    assert_eq!(
        warm_hierarchy
            .variables()
            .map(|variable| variable.path().to_string())
            .collect::<Vec<_>>(),
        cold_paths
    );
    // Every logical open keeps its own declaration identity.
    let warm_handle = warm_hierarchy.signal("top.control").unwrap();
    assert_ne!(cold_handle, warm_handle);
    assert!(warm_hierarchy.validate(cold_handle).is_err());
    assert_eq!(observe(&mut warm, &warm_hierarchy, u64::MAX), cold_seen);
}

#[test]
fn reused_content_keeps_the_current_source_name() {
    let bytes = test_bytes::recording(0);
    let cache = Cache::new();
    let (_, _, first) = open(&bytes, "first.ghw", Purpose::Waveform, &cache).unwrap();
    let (_, _, second) = open(&bytes, "second.ghw", Purpose::Waveform, &cache).unwrap();
    assert_eq!(diagnostics(&cache), (1, 1));
    assert_eq!(first.source_name, "first.ghw");
    assert_eq!(second.source_name, "second.ghw");
}

#[test]
fn a_changed_byte_at_the_same_length_is_a_miss() {
    let bytes = test_bytes::recording(2);
    let cache = Cache::new();
    let (_, _, cold) = open(&bytes, "suffix.ghw", Purpose::Waveform, &cache).unwrap();
    // Flip one ordinal of the trailing cycle records: same length, same names, a
    // valid waveform, and outside every signal the reuse above selected.
    let mut changed = bytes.clone();
    let ordinal = changed.len() - 7;
    assert!(matches!(changed[ordinal], 2 | 3), "{}", changed[ordinal]);
    changed[ordinal] ^= 0x01;
    let (_, _, warm) = open(&changed, "suffix.ghw", Purpose::Waveform, &cache).unwrap();
    assert_eq!(diagnostics(&cache), (0, 2));
    assert_eq!(cold.time_span, warm.time_span);
}

#[test]
fn truncated_and_extended_inputs_are_misses_with_the_parser_errors() {
    let bytes = test_bytes::recording(0);
    let cache = Cache::new();
    open(&bytes, "length.ghw", Purpose::Waveform, &cache).unwrap();
    for changed in [
        bytes[..bytes.len() - 1].to_vec(),
        [bytes.clone(), vec![0]].concat(),
    ] {
        let reused = open(&changed, "length.ghw", Purpose::Waveform, &cache);
        let fresh = Reader::open_with(
            Box::new(Cursor::new(changed.clone())),
            "length.ghw".into(),
            Purpose::Waveform,
            &Cache::new(),
        );
        let reused = error(reused);
        let fresh = error(fresh);
        assert_eq!(reused.to_string(), fresh.to_string());
    }
    // A cold parse plus the two rejected attempts, both of which really parsed.
    assert_eq!(diagnostics(&cache), (0, 3));
}

#[test]
fn malformed_input_keeps_the_parser_error_with_a_parked_source() {
    let valid = test_bytes::recording(0);
    let cache = Cache::new();
    open(&valid, "parked.ghw", Purpose::Waveform, &cache).unwrap();
    let mut malformed = valid.clone();
    let marker = malformed
        .windows(4)
        .position(|bytes| bytes == b"ESN\0")
        .unwrap();
    malformed[marker] = b'X';
    let reused = error(open(&malformed, "parked.ghw", Purpose::Waveform, &cache));
    let fresh = error(Reader::open_with(
        Box::new(Cursor::new(malformed)),
        "parked.ghw".into(),
        Purpose::Waveform,
        &Cache::new(),
    ));
    assert_eq!(reused.to_string(), fresh.to_string());
    // The parked source was not reused, and the changed content was parsed once.
    assert_eq!(diagnostics(&cache), (0, 2));
}

#[test]
fn metadata_requests_never_leave_a_parked_hierarchy() {
    let bytes = test_bytes::recording(0);
    let cache = Cache::new();
    let metadata = Reader::open_with(
        Box::new(Cursor::new(bytes.clone())),
        "meta.ghw".into(),
        Purpose::Metadata,
        &cache,
    )
    .unwrap()
    .2;
    assert!(cache.parked_identity().is_none());
    assert_eq!(diagnostics(&cache), (0, 1));

    // A metadata request may still reuse content, but releases it afterwards.
    open(&bytes, "meta.ghw", Purpose::Waveform, &cache).unwrap();
    assert!(cache.parked_identity().is_some());
    let reused = Reader::open_with(
        Box::new(Cursor::new(bytes.clone())),
        "meta.ghw".into(),
        Purpose::Metadata,
        &cache,
    )
    .unwrap()
    .2;
    assert_eq!(reused.time_span, metadata.time_span);
    assert_eq!(diagnostics(&cache), (1, 2));
    assert!(cache.parked_identity().is_none());
}

#[test]
fn oversized_results_are_not_parked() {
    let bytes = test_bytes::recording(0);
    let cache = Cache::new();
    let (mut reader, _, _) = Reader::open_with(
        Box::new(Cursor::new(bytes)),
        "big.ghw".into(),
        Purpose::Metadata,
        &cache,
    )
    .unwrap();
    Arc::get_mut(&mut reader.validated)
        .expect("not parked yet")
        .retained_bytes = CACHE_BYTES + 1;
    cache.park(Arc::clone(&reader.validated));
    assert!(cache.parked_identity().is_none());
    Arc::get_mut(&mut reader.validated)
        .expect("still not parked")
        .retained_bytes = 1;
    cache.park(Arc::clone(&reader.validated));
    assert!(cache.parked_identity().is_some());
}

#[test]
fn a_parked_source_without_an_index_still_answers() {
    let bytes = test_bytes::recording(1);
    let cache = Cache::new();
    let (mut reader, _, _) = Reader::open_with(
        Box::new(Cursor::new(bytes)),
        "fallback.ghw".into(),
        Purpose::Metadata,
        &cache,
    )
    .unwrap();
    {
        let validated = Arc::get_mut(&mut reader.validated).expect("not parked yet");
        assert!(validated.index.is_some());
        validated.index = None;
        validated.retained_bytes = 1;
    }
    cache.park(Arc::clone(&reader.validated));
    let (mut warm, hierarchy, _) = open(
        &test_bytes::recording(1),
        "fallback.ghw",
        Purpose::Waveform,
        &cache,
    )
    .unwrap();
    assert_eq!(diagnostics(&cache), (1, 1));
    assert!(!observe(&mut warm, &hierarchy, u64::MAX).is_empty());
}

#[test]
fn an_identity_scan_failure_is_a_miss_and_keeps_the_parse_error() {
    use std::io::{self, BufRead};
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    struct Failing {
        input: Cursor<Vec<u8>>,
        cutoff: Arc<AtomicU64>,
    }
    impl Read for Failing {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let bytes = self.fill_buf()?;
            let n = output.len().min(bytes.len());
            output[..n].copy_from_slice(&bytes[..n]);
            self.consume(n);
            Ok(n)
        }
    }
    impl BufRead for Failing {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            let remaining = self
                .cutoff
                .load(Ordering::Relaxed)
                .checked_sub(self.input.position())
                .ok_or_else(|| io::Error::other("injected read failure"))?;
            if remaining == 0 {
                return Err(io::Error::other("injected read failure"));
            }
            let bytes = self.input.fill_buf()?;
            Ok(&bytes[..bytes
                .len()
                .min(usize::try_from(remaining).unwrap_or(usize::MAX))])
        }
        fn consume(&mut self, n: usize) {
            self.input.consume(n);
        }
    }
    impl Seek for Failing {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            self.input.seek(from)
        }
    }
    let bytes = test_bytes::recording(0);
    let cache = Cache::new();
    open(&bytes, "failing.ghw", Purpose::Waveform, &cache).unwrap();
    let cutoff = Arc::new(AtomicU64::new((bytes.len() / 2) as u64));
    let failed = open_buffer(
        Box::new(Failing {
            input: Cursor::new(bytes),
            cutoff,
        }),
        "failing.ghw",
        Purpose::Waveform,
        &cache,
    );
    let failed = error(failed);
    assert!(matches!(failed, crate::Error::Io(_)), "{failed}");
    // The identity scan failed, so the request really parsed and reported the I/O error.
    assert_eq!(diagnostics(&cache), (0, 2));
}

#[test]
fn opening_inside_a_query_callback_is_not_blocked_by_the_cache() {
    let bytes = test_bytes::recording(1);
    let cache = Cache::new();
    let (mut reader, hierarchy, _) =
        open(&bytes, "reentrant.ghw", Purpose::Waveform, &cache).unwrap();
    let signals = hierarchy.signals().collect::<Vec<_>>();
    let mut opened = 0;
    let _ = reader
        .read(&signals, Time::from_ticks(u64::MAX), |_| {
            if opened == 0 {
                opened += 1;
                // Opens the same parked content from inside the callback.
                let (_, nested, _) =
                    open(&bytes, "reentrant.ghw", Purpose::Waveform, &cache).expect("nested open");
                assert!(nested.signals().count() > 0);
            }
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert_eq!(opened, 1);
    assert_eq!(diagnostics(&cache), (1, 1));
}

#[test]
#[ignore = "requires the installed GHW corpus"]
fn file_and_byte_inputs_share_one_content_identity() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/ghw/ghw0001-wellen-ghw-bug/waveform.ghw");
    let cache = Cache::new();
    let name = path.to_string_lossy().into_owned();
    let file = |purpose: Purpose| {
        Reader::open_with(
            Box::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap())),
            name.clone(),
            purpose,
            &cache,
        )
    };
    let (mut cold, cold_hierarchy, _) = file(Purpose::Waveform).unwrap();
    let cold_seen = observe(&mut cold, &cold_hierarchy, u64::MAX);
    assert!(!cold_seen.is_empty());
    let bytes = std::fs::read(&path).unwrap();
    // A byte input with the same content reuses the file parse: identity is content.
    let (mut warm, warm_hierarchy, _) =
        open(&bytes, "bytes.ghw", Purpose::Waveform, &cache).unwrap();
    assert_eq!(diagnostics(&cache), (1, 1));
    assert_eq!(
        warm_hierarchy.signals().count(),
        cold_hierarchy.signals().count()
    );
    assert_ne!(
        cold_hierarchy.signals().next().unwrap(),
        warm_hierarchy.signals().next().unwrap()
    );
    assert_eq!(observe(&mut warm, &warm_hierarchy, u64::MAX), cold_seen);
}

#[test]
fn accounting_covers_declarations_and_lazy_tables() {
    let bytes = test_bytes::padded_recording(0);
    let (reader, hierarchy, _) = Reader::open_with(
        Box::new(Cursor::new(bytes)),
        "acct.ghw".into(),
        Purpose::Metadata,
        &Cache::new(),
    )
    .unwrap();
    let counted = reader.validated.retained_bytes;
    assert!(counted > 0 && counted <= CACHE_BYTES, "{counted}");
    // The hierarchy bound counts declarations that exist and the lookup tables that
    // are only built on first use.
    let variables = hierarchy.variables().count();
    assert!(variables > 0);
    assert!(
        hierarchy.validated_bytes()
            >= variables * std::mem::size_of::<crate::hierarchy::VariableData>()
    );
}

// ---------------------------------------------------------------------------
// The public query surface must observe the same things on a guaranteed hit.
// ---------------------------------------------------------------------------

use crate::{ScanRef, TimeRange};

#[derive(Debug, PartialEq)]
struct Engine {
    control: Vec<(u64, String)>,
    wide: Vec<String>,
    broad_records: usize,
    indexed_payloads: usize,
    indexed_frames: usize,
}

fn indexed_work(wave: &crate::Waveform) -> (usize, usize) {
    let crate::backends::Reader::Ghw(reader) = &wave.reader else {
        panic!("GHW")
    };
    reader
        .work()
        .map_or((0, 0), |work| (work.payloads_decoded, work.frames_scanned))
}

/// Drive a waveform whose reader comes from `cache`, so cold and hit runs can be
/// compared through the public engine rather than through private events.
fn engine(bytes: &[u8], name: &str, cache: &Cache) -> Engine {
    let (reader, _, _) = Reader::open_with(
        Box::new(Cursor::new(bytes.to_vec())),
        name.into(),
        Purpose::Waveform,
        cache,
    )
    .unwrap();
    let mut wave = crate::open_bytes(name, bytes.to_vec().into()).unwrap();
    wave.reader = crate::backends::Reader::Ghw(Box::new(reader));
    let control = wave.hierarchy().signal("top.control").unwrap();
    let wide = wave.hierarchy().signal("top.wide").unwrap();
    let mut observations = Vec::new();
    let _ = wave
        .scan(&[control], TimeRange::all(), |record| {
            if let ScanRef::Change { time, value, .. } = record {
                observations.push((time.ticks(), format!("{value:?}")));
            }
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    let (indexed_payloads, indexed_frames) = indexed_work(&wave);
    // Exact change time for a point sample.
    let sample = wave.sample(control, Time::from_ticks(1)).unwrap();
    assert!(
        matches!(sample, crate::Sample::Value { changed_at: Some(time), .. } if time.ticks() == 1),
        "{sample:?}"
    );
    let mut values = Vec::new();
    for tick in 0..=4 {
        let samples = wave.samples(&[wide], Time::from_ticks(tick)).unwrap();
        values.push(sample_text(&samples[0]));
    }
    // The fixture cancels a same-tick excursion and then changes at ticks 3 and 4.
    assert_eq!(values[3], "bits:111 changed:3", "{}", values[3]);
    assert_eq!(values[4], "bits:110 changed:4", "{}", values[4]);
    // Aliases stay one handle with one shared history.
    assert_eq!(
        wave.hierarchy().signal("top.wide").unwrap(),
        wave.hierarchy().signal("top.alias").unwrap()
    );
    // Broad replay over every signal uses the sequential walker.
    let signals = wave.hierarchy().signals().collect::<Vec<_>>();
    let mut broad_records = 0;
    let _ = wave
        .scan(&signals, TimeRange::all(), |_| {
            broad_records += 1;
            ControlFlow::<()>::Continue(())
        })
        .unwrap();
    // A stopped read and a panicking visitor must not affect later queries.
    let flow = wave
        .scan(&[control], TimeRange::all(), |_| ControlFlow::Break(7))
        .unwrap();
    assert_eq!(flow, ControlFlow::Break(7));
    assert!(wave.sample(control, Time::from_ticks(1)).is_ok());
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = wave.scan(&[control], TimeRange::all(), |_| -> ControlFlow<()> {
            panic!("visitor")
        });
    }));
    assert!(wave.sample(control, Time::from_ticks(1)).is_ok());
    Engine {
        control: observations,
        wide: values,
        broad_records,
        indexed_payloads,
        indexed_frames,
    }
}

#[test]
fn a_hit_keeps_public_engine_observations() {
    let bytes = test_bytes::recording(0);
    let cache = Cache::new();
    let cold = engine(&bytes, "engine.ghw", &cache);
    assert_eq!(diagnostics(&cache), (0, 1));
    let warm = engine(&bytes, "engine.ghw", &cache);
    // Exactly one guaranteed local hit, without any conformance race.
    assert_eq!(diagnostics(&cache), (1, 1));
    assert_eq!(warm, cold);
    assert!(!cold.control.is_empty());
    assert!(
        cold.indexed_payloads > 0,
        "selective read must use the index"
    );
    assert!(cold.indexed_frames > 0);
    assert!(cold.broad_records > cold.indexed_payloads);
}

#[test]
fn a_hit_keeps_missing_and_projection_change_times() {
    let mut bytes = test_bytes::recording(0);
    for (declaration, ids) in [
        ([16, 14, 4, 3, 1, 4], [3, 3, 4]),
        ([16, 15, 4, 3, 1, 4], [3, 3, 4]),
        ([16, 16, 4, 4, 1, 3], [4, 3, 3]),
    ] {
        let at = bytes
            .windows(declaration.len())
            .position(|candidate| candidate == declaration)
            .unwrap();
        bytes[at + 3..at + 6].copy_from_slice(&ids);
    }
    let body = bytes.windows(4).position(|b| b == b"SNP\0").unwrap();
    bytes.truncate(body);
    bytes.extend(b"CYC\0");
    bytes.extend(0i64.to_le_bytes());
    bytes.extend([1, 3, 0, 1, 1, 2, 0, 1, 2, 3, 0, 1, 2, 2, 0, 127]);
    bytes.extend(b"ECY\0");
    let cache = Cache::new();
    let observe = |cache: &Cache| {
        let (reader, _, _) = Reader::open_with(
            Box::new(Cursor::new(bytes.clone())),
            "partial.ghw".into(),
            Purpose::Waveform,
            cache,
        )
        .unwrap();
        let mut wave = crate::open_bytes("partial.ghw", bytes.clone().into()).unwrap();
        wave.reader = crate::backends::Reader::Ghw(Box::new(reader));
        let wide = wave.hierarchy().signal("top.wide").unwrap();
        let low = wide.slice(0, 0).unwrap();
        let mut seen = Vec::new();
        for tick in [0, 1, 2, 3, 1, 0, 3, 2] {
            let samples = wave.samples(&[wide, low], Time::from_ticks(tick)).unwrap();
            seen.push(
                samples
                    .iter()
                    .map(sample_text)
                    .collect::<Vec<_>>()
                    .join(" | "),
            );
        }
        seen
    };
    let cold = observe(&cache);
    assert_eq!(diagnostics(&cache), (0, 1));
    let warm = observe(&cache);
    assert_eq!(diagnostics(&cache), (1, 1));
    assert_eq!(warm, cold);
    // Ticks 0 and 1 have an incomplete recipe, so the value is Missing.
    assert_eq!(cold[0], "missing | missing", "{}", cold[0]);
    assert_eq!(cold[1], "missing | missing", "{}", cold[1]);
    assert_eq!(
        cold[2], "bits:001 changed:none | bits:1 changed:none",
        "{}",
        cold[2]
    );
}

#[test]
fn a_hit_keeps_binary64_and_big_endian_values() {
    for big in [false, true] {
        let bytes = test_bytes::real_recording(big, 1, 0x400921fb54442d18);
        let cache = Cache::new();
        let observe = |cache: &Cache| {
            let (reader, _, _) = Reader::open_with(
                Box::new(Cursor::new(bytes.clone())),
                "real.ghw".into(),
                Purpose::Waveform,
                cache,
            )
            .unwrap();
            let mut wave = crate::open_bytes("real.ghw", bytes.clone().into()).unwrap();
            wave.reader = crate::backends::Reader::Ghw(Box::new(reader));
            let real = wave.hierarchy().signals().next().unwrap();
            let mut bits = Vec::new();
            for tick in [3, 4] {
                let sample = wave.sample(real, Time::from_ticks(tick)).unwrap();
                match sample {
                    crate::Sample::Value {
                        value: crate::Value::Real(value),
                        ..
                    } => bits.push(value.to_bits()),
                    other => panic!("expected a real value, got {other:?}"),
                }
            }
            bits
        };
        let cold = observe(&cache);
        assert_eq!(diagnostics(&cache), (0, 1));
        let warm = observe(&cache);
        assert_eq!(diagnostics(&cache), (1, 1));
        assert_eq!(warm, cold);
        assert_eq!(cold, vec![0x8000_0000_0000_0000u64, 0x400921fb54442d18u64]);
    }
}

// ---------------------------------------------------------------------------
// Identity failures, rewinds and required payload errors.
// ---------------------------------------------------------------------------

/// What one faulty input actually did, observable after it is consumed by an open.
#[derive(Default)]
struct Observed {
    seeks: std::sync::atomic::AtomicUsize,
    read_bytes: std::sync::atomic::AtomicUsize,
}

/// An input that can fail or shorten its reads and seeks on demand.
struct Faulty {
    bytes: Vec<u8>,
    position: usize,
    fail_read_at: Option<usize>,
    eof_at: Option<usize>,
    /// Fail the seek with this one-based index, and no other.
    fail_seek_at: Option<usize>,
    /// Fail this seek and every later one.
    fail_seeks_from: Option<usize>,
    observed: Arc<Observed>,
}

impl Faulty {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            position: 0,
            fail_read_at: None,
            eof_at: None,
            fail_seek_at: None,
            fail_seeks_from: None,
            observed: Arc::new(Observed::default()),
        }
    }
}

impl Read for Faulty {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        let bytes = self.fill_buf()?;
        let n = output.len().min(bytes.len());
        output[..n].copy_from_slice(&bytes[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl std::io::BufRead for Faulty {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self
            .fail_read_at
            .is_some_and(|limit| self.position >= limit)
        {
            return Err(std::io::Error::other("injected read failure"));
        }
        let end = self.bytes.len().min(self.eof_at.unwrap_or(usize::MAX));
        Ok(&self.bytes[self.position.min(end)..end])
    }

    fn consume(&mut self, amount: usize) {
        self.observed
            .read_bytes
            .fetch_add(amount, Ordering::Relaxed);
        self.position = (self.position + amount).min(self.bytes.len());
    }
}

impl Seek for Faulty {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        let seek = self.observed.seeks.fetch_add(1, Ordering::Relaxed) + 1;
        if self.fail_seek_at == Some(seek)
            || self.fail_seeks_from.is_some_and(|first| seek >= first)
        {
            return Err(std::io::Error::other("injected seek failure"));
        }
        self.position = match from {
            SeekFrom::Start(offset) => offset as usize,
            SeekFrom::End(offset) => (self.bytes.len() as i64 + offset).max(0) as usize,
            SeekFrom::Current(offset) => (self.position as i64 + offset).max(0) as usize,
        }
        .min(self.bytes.len());
        Ok(self.position as u64)
    }
}

#[test]
fn a_late_identity_read_failure_leaves_the_early_malformed_error() {
    let valid = test_bytes::recording(0);
    let cache = Cache::new();
    open(&valid, "order.ghw", Purpose::Waveform, &cache).unwrap();
    let mut malformed = valid.clone();
    let marker = malformed.windows(4).position(|b| b == b"ESN\0").unwrap();
    malformed[marker] = b'X';
    // The identity pass fails long after the byte that is already malformed.
    let mut input = Faulty::new(&malformed);
    input.fail_read_at = Some(malformed.len() - 8);
    let reused = error(open_buffer(
        Box::new(input),
        "order.ghw",
        Purpose::Waveform,
        &cache,
    ));
    let fresh = error(Reader::open_with(
        Box::new(Cursor::new(malformed.clone())),
        "order.ghw".into(),
        Purpose::Waveform,
        &Cache::new(),
    ));
    assert!(matches!(reused, crate::Error::Malformed { .. }), "{reused}");
    assert_eq!(reused.to_string(), fresh.to_string());
    // The parked source was not reused and the rejected input was parsed once.
    assert_eq!(diagnostics(&cache), (0, 2));
}

#[test]
fn a_rewind_failure_after_a_complete_scan_falls_back_to_parsing() {
    // Opening seeks twice before the identity pass (end, then start), so the third
    // seek is the rewind that the scan performs after reading the whole input.
    let bytes = test_bytes::recording(0);
    let cache = Cache::new();
    open(&bytes, "rewind.ghw", Purpose::Waveform, &cache).unwrap();
    let mut input = Faulty::new(&bytes);
    input.fail_seek_at = Some(3);
    let observed = Arc::clone(&input.observed);
    let (mut reader, hierarchy, _) =
        open_buffer(Box::new(input), "rewind.ghw", Purpose::Waveform, &cache).unwrap();
    // The scan really ran over the whole input and its rewind was the failing seek.
    assert_eq!(
        observed.read_bytes.load(Ordering::Relaxed),
        2 * bytes.len(),
        "one complete identity pass plus one complete parse"
    );
    assert!(observed.seeks.load(Ordering::Relaxed) >= 3);
    // It was a miss, not a hit, and the fallback parse answers normally.
    assert_eq!(diagnostics(&cache), (0, 2));
    assert!(reader.validated.identity.is_some());
    let mut seen = Vec::new();
    let _ = reader
        .read(
            &hierarchy.signals().collect::<Vec<_>>(),
            Time::from_ticks(2),
            |event| {
                if let Notification::Value { value, .. } = event {
                    seen.push(format!("{value:?}"));
                }
                ControlFlow::<()>::Continue(())
            },
        )
        .unwrap();
    assert!(!seen.is_empty());
}

#[test]
fn a_persistent_seek_failure_after_the_scan_propagates_as_an_io_error() {
    let bytes = test_bytes::recording(0);
    let cache = Cache::new();
    open(&bytes, "rewind.ghw", Purpose::Waveform, &cache).unwrap();
    let mut input = Faulty::new(&bytes);
    input.fail_seeks_from = Some(3);
    let observed = Arc::clone(&input.observed);
    let failed = error(open_buffer(
        Box::new(input),
        "rewind.ghw",
        Purpose::Waveform,
        &cache,
    ));
    assert!(matches!(failed, crate::Error::Io(_)), "{failed}");
    // The identity pass completed before the failing rewind; the fallback parse was
    // entered and failed on its own positioning seek, and nothing was reused.
    assert!(observed.read_bytes.load(Ordering::Relaxed) >= bytes.len());
    assert_eq!(diagnostics(&cache), (0, 2));
}

#[test]
fn a_short_read_at_the_same_length_is_a_miss_with_the_parser_error() {
    let bytes = test_bytes::recording(0);
    let cache = Cache::new();
    open(&bytes, "short.ghw", Purpose::Waveform, &cache).unwrap();
    let short = bytes[..bytes.len() - 1].to_vec();
    let mut input = Faulty::new(&bytes);
    input.eof_at = Some(short.len());
    let reused = error(open_buffer(
        Box::new(input),
        "short.ghw",
        Purpose::Waveform,
        &cache,
    ));
    let fresh = error(Reader::open_with(
        Box::new(Cursor::new(short)),
        "short.ghw".into(),
        Purpose::Waveform,
        &Cache::new(),
    ));
    assert_eq!(reused.to_string(), fresh.to_string());
    // The parked source was not reused, and the short read was parsed once.
    assert_eq!(diagnostics(&cache), (0, 2));
}

#[test]
fn a_hit_reports_required_payload_errors_and_recovers() {
    use std::sync::atomic::AtomicBool;
    struct Switchable {
        inner: Faulty,
        fail: Arc<AtomicBool>,
    }
    impl Read for Switchable {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            if self.fail.load(Ordering::Relaxed) {
                return Err(std::io::Error::other("injected read failure"));
            }
            self.inner.read(output)
        }
    }
    impl std::io::BufRead for Switchable {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            if self.fail.load(Ordering::Relaxed) {
                return Err(std::io::Error::other("injected read failure"));
            }
            self.inner.fill_buf()
        }
        fn consume(&mut self, amount: usize) {
            self.inner.consume(amount);
        }
    }
    impl Seek for Switchable {
        fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(from)
        }
    }
    let bytes = test_bytes::recording(0);
    let cache = Cache::new();
    open(&bytes, "payload.ghw", Purpose::Waveform, &cache).unwrap();
    let fail = Arc::new(AtomicBool::new(false));
    let input = Switchable {
        inner: Faulty::new(&bytes),
        fail: Arc::clone(&fail),
    };
    let (reader, _, _) =
        open_buffer(Box::new(input), "payload.ghw", Purpose::Waveform, &cache).unwrap();
    assert_eq!(diagnostics(&cache), (1, 1));
    let mut wave = crate::open_bytes("payload.ghw", bytes.clone().into()).unwrap();
    wave.reader = crate::backends::Reader::Ghw(Box::new(reader));
    let control = wave.hierarchy().signal("top.control").unwrap();
    fail.store(true, Ordering::Relaxed);
    let failed = wave.sample(control, Time::from_ticks(1));
    assert!(matches!(failed, Err(crate::Error::Io(_))), "{failed:?}");
    fail.store(false, Ordering::Relaxed);
    assert!(wave.sample(control, Time::from_ticks(1)).is_ok());
}

#[test]
fn identity_depends_on_content_not_on_the_input_object() {
    // Same content through two different input implementations, one of them with
    // tiny read-ahead, must reuse the same parked result. This is the CI-safe
    // equivalent of the ignored published-fixture file/bytes test.
    let bytes = test_bytes::recording(0);
    let cache = Cache::new();
    let (_, _, _) = open(&bytes, "kind.ghw", Purpose::Waveform, &cache).unwrap();
    assert_eq!(diagnostics(&cache), (0, 1));
    struct Small {
        inner: Cursor<Vec<u8>>,
    }
    impl Read for Small {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            let window = self.inner.fill_buf()?;
            let n = output.len().min(window.len()).min(3);
            output[..n].copy_from_slice(&window[..n]);
            self.inner.consume(n);
            Ok(n)
        }
    }
    impl std::io::BufRead for Small {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            let window = self.inner.fill_buf()?;
            Ok(&window[..window.len().min(3)])
        }
        fn consume(&mut self, amount: usize) {
            self.inner.consume(amount);
        }
    }
    impl Seek for Small {
        fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(from)
        }
    }
    let input = Box::new(Small {
        inner: Cursor::new(bytes),
    });
    let (_, _, metadata) = open_buffer(input, "kind.ghw", Purpose::Waveform, &cache).unwrap();
    assert_eq!(diagnostics(&cache), (1, 1));
    assert!(metadata.time_span.is_some());
}
