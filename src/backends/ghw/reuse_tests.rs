//! Focused tests for validated GHW reuse.
//!
//! Every test drives its own private [`Cache`], so reuse behaviour is observable
//! without depending on which other tests have opened GHW content.
use super::*;
use std::io::{Cursor, Read, Seek, SeekFrom};
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
