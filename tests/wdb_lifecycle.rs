//! Lifecycle, identity and reuse checks for sequential WDB opens over published fixtures.
//!
//! One ignored SDK test exercises public API semantics with the explicit `wdb-lib` backend over
//! an exclusively owned temporary copy: exact observations after same-path reopens, stale
//! handles, metadata and another path between lifetimes, same-size in-place mutation with a
//! restored mtime, in-place rewrite, atomic replacement, symlink retargeting, deletion, a
//! caller panic, and deliberate stop.
//!
//! Retained descriptors are the observable for the retained-owner policy: a clean logical close
//! keeps at most one idle owner, while a metadata-only request, a changed path, a failed
//! identity or an unwinding caller must release it. Every clean close releases that session's
//! SDK history indexes before the owner waits. The second ignored test reports cold-miss,
//! reused-owner, descriptor and RSS behaviour; it asserts only invariants, not speed.
//!
//! The published fixtures are small declared-value recordings; other encodings, events and
//! resolution switches remain covered by the existing conformance suite.

#![cfg(feature = "wdb-lib")]

use ondas::{Encoding, Sample, Signal, Time, TimeRange, ValueRef, Waveform};

#[path = "support/wdb_compatibility.rs"]
mod wdb_compatibility;
use std::{
    fs,
    os::unix::fs::{MetadataExt, symlink},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Instant,
};

const BACKEND: &str = "wdb-lib";
/// Fixed small same-path reopen cycles; no timing is measured here.
const SAME_PATH_CYCLES: usize = 6;
// Published recordings that a supported older reader can open: DBG version 4.
const FIXTURE_A: &str = "wdb0091-tesbench-behav";
const FIXTURE_B: &str = "wdb0087-testbench-behav";
// Published recordings that only a newer reader can open: DBG version 6. These scenarios must skip
// with an explicit version reason instead of failing the suite on a supported older reader.
const NEWER_STRINGS: &str = "wdb0317-vhdl-strings-ports";
const NEWER_SAME_SIZE_A: &str = "wdb0042-work-mem1-tb";
const NEWER_SAME_SIZE_B: &str = "wdb0043-work-mem3-tb";

fn fixture(name: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/wdb")
        .join(name)
        .join("waveform.wdb");
    assert!(
        path.is_file(),
        "missing published fixture {}; install the pinned fixtures first",
        path.display()
    );
    path
}

fn open_native(path: &Path) -> Waveform {
    ondas::open_with(path, BACKEND)
        .unwrap_or_else(|error| panic!("open {} with {BACKEND}: {error}", path.display()))
}

/// Opens only when this reader can read the recording, or skips the newer-only scenario with the
/// strict helper: it prints the confirmed version reason and still fails on any other error.
fn compatible(path: &Path) -> Option<Waveform> {
    wdb_compatibility::open(ondas::open_with(path, BACKEND), path)
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let mut attempt = 0;
        loop {
            let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "ondas-wdb-lifecycle-{}-{nonce}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => match fs::canonicalize(&path) {
                    Ok(canonical) => return Self(canonical),
                    Err(error) => {
                        let _ = fs::remove_dir(&path);
                        panic!(
                            "cannot canonicalize the exclusive temporary root {}: {error}",
                            path.display()
                        );
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && attempt < 16 => {
                    attempt += 1;
                }
                Err(error) => panic!(
                    "cannot create an exclusive temporary root {}: {error}",
                    path.display()
                ),
            }
        }
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn value_text(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Bits(bits) => format!("bits:{bits}"),
        ValueRef::Real(real) => format!("real:{:016x}", real.to_bits()),
        ValueRef::String(text) => format!("string:{text:?}"),
        other => panic!("unhandled ValueRef variant: {other:?}"),
    }
}

fn sample_text(sample: &Sample) -> String {
    match sample {
        Sample::Missing { .. } => "missing".into(),
        Sample::Value {
            value, changed_at, ..
        } => format!(
            "value:{} changed_at={:?}",
            value_text(value.as_ref()),
            changed_at.map(|time| time.ticks())
        ),
        other => panic!("unhandled Sample variant: {other:?}"),
    }
}

/// Deterministic observation of everything these fixtures expose through the public API.
fn snapshot_waveform(waveform: &mut Waveform) -> Vec<String> {
    let mut lines = Vec::new();
    let metadata = waveform.metadata();
    // The logical source name is the file name and therefore differs for a copy.
    lines.push(format!(
        "metadata timescale={:?} span={:?}",
        metadata.timescale(),
        metadata
            .time_span()
            .map(|span| (span.first().ticks(), span.last().ticks()))
    ));
    let probes = match metadata.time_span() {
        Some(span) => {
            let (first, last) = (span.first().ticks(), span.last().ticks());
            vec![
                Time::from_ticks(first),
                Time::from_ticks(first + (last - first) / 2),
                Time::from_ticks(last),
            ]
        }
        None => vec![Time::ZERO],
    };
    let hierarchy = waveform.hierarchy().clone();
    for scope in hierarchy.scopes() {
        lines.push(format!("scope {} kind={}", scope.path(), scope.kind()));
    }
    for variable in hierarchy.variables() {
        let path = variable.path().to_string();
        let Some(signal) = variable.signal() else {
            lines.push(format!("variable {path} without signal"));
            continue;
        };
        if matches!(signal.encoding(), Encoding::Unsupported) {
            lines.push(format!("variable {path} unsupported encoding"));
            continue;
        }
        lines.push(format!(
            "variable {path} encoding={:?} range={:?}",
            signal.encoding(),
            variable.range()
        ));
        for probe in &probes {
            let sample = waveform
                .sample(signal, *probe)
                .unwrap_or_else(|error| panic!("sample {path} at {}: {error}", probe.ticks()));
            lines.push(format!(
                "  sample {} {}",
                probe.ticks(),
                sample_text(&sample)
            ));
        }
    }
    // A full scan covers the traversal path, including its cleanup on the next lifetime.
    if let Some(signal) = hierarchy
        .variables()
        .filter_map(|variable| variable.signal())
        .find(|signal| !matches!(signal.encoding(), Encoding::Unsupported))
    {
        let mut records = Vec::new();
        let outcome = waveform
            .scan(&[signal], TimeRange::all(), |record| {
                records.push(match record {
                    ondas::ScanRef::Change { time, value, .. } => {
                        format!("change {} {}", time.ticks(), value_text(value))
                    }
                    other => format!("{other:?}"),
                });
                std::ops::ControlFlow::<()>::Continue(())
            })
            .unwrap_or_else(|error| panic!("scan: {error}"));
        assert!(outcome.is_continue());
        lines.push(format!("scan {} records", records.len()));
        lines.extend(records);
    }
    lines
}

fn snapshot(path: &Path) -> Vec<String> {
    snapshot_waveform(&mut open_native(path))
}

/// Descriptor entries whose target is inside this test's own temporary root.
fn descriptors_under(root: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir("/proc/self/fd") else {
        panic!("no /proc/self/fd; descriptor observation needs Linux procfs");
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let descriptor = entry.file_name().to_string_lossy().into_owned();
        if let Ok(target) = fs::read_link(entry.path())
            && target.starts_with(root)
        {
            found.push((descriptor, target));
        }
    }
    found.sort();
    found
}

fn first_signal(waveform: &Waveform) -> Signal {
    waveform
        .hierarchy()
        .variables()
        .filter_map(|variable| variable.signal())
        .find(|signal| !matches!(signal.encoding(), Encoding::Unsupported))
        .expect("published fixture must expose a queryable signal")
}

fn temp_entries(root: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(root)
        .expect("list the temporary root")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[derive(Debug)]
struct VisitorSentinel;

#[test]
#[ignore = "requires the selected reader and installed WDB fixtures; run just conformance-wdb"]
fn wdb_sequential_open_lifecycle_over_published_fixtures() {
    let fixture_a = fixture(FIXTURE_A);
    let fixture_b = fixture(FIXTURE_B);
    // The core lifecycle must run for real: both recordings are from the compatible DBG class.
    if compatible(&fixture_a).is_none() || compatible(&fixture_b).is_none() {
        eprintln!("SKIP core lifecycle: this reader cannot open the compatible recordings");
        return;
    }
    let reference_a = snapshot(&fixture_a);
    let reference_b = snapshot(&fixture_b);
    assert_ne!(
        reference_a, reference_b,
        "the two published fixtures must expose different observations"
    );

    let root = TempRoot::new();
    let path = root.file("copy.wdb");
    fs::copy(&fixture_a, &path).expect("copy published fixture A");

    eprintln!("PHASE 1");
    // Phase 1: same-path reopens must reproduce exact observations and keep at most one idle
    // owner: the descriptor count may stay at one but never grows.
    let mut baseline = None;
    for cycle in 0..SAME_PATH_CYCLES {
        let mut waveform = open_native(&path);
        assert_eq!(
            snapshot_waveform(&mut waveform),
            reference_a,
            "same-path cycle {cycle}"
        );
        drop(waveform);
        let retained = descriptors_under(root.path());
        assert!(
            retained.len() <= 1,
            "more than one retained descriptor after cycle {cycle}: {retained:?}"
        );
        match baseline {
            None => baseline = Some(retained.len()),
            Some(first) => assert_eq!(
                retained.len(),
                first,
                "retained descriptors changed: cycle {cycle} has {retained:?}"
            ),
        }
    }
    assert_eq!(
        baseline,
        Some(1),
        "a clean logical close must keep exactly one idle owner"
    );

    eprintln!("PHASE 2");
    // Phase 2: a handle from a dropped logical open is stale even though the idle owner and its
    // immutable declarations are reused.
    let stale = {
        let waveform = open_native(&path);
        let signal = first_signal(&waveform);
        let clone = waveform.hierarchy().clone();
        (signal, clone)
    };
    {
        let mut waveform = open_native(&path);
        assert!(
            waveform.select(&[stale.0]).is_err(),
            "a Signal from a dropped Waveform must be rejected"
        );
        assert_ne!(
            stale.1.scopes().count(),
            0,
            "a retained hierarchy clone must stay readable"
        );
        assert_eq!(snapshot_waveform(&mut waveform), reference_a);
    }

    eprintln!("PHASE 3");
    // Phase 3: a metadata-only request invalidates the idle owner before its own SDK work.
    assert_eq!(
        descriptors_under(root.path()).len(),
        1,
        "prefill the idle owner before the metadata request"
    );
    let metadata = ondas::read_metadata(&path).expect("read metadata of the temporary copy");
    assert_eq!(
        metadata
            .time_span()
            .map(|span| (span.first().ticks(), span.last().ticks())),
        ondas::read_metadata(&fixture_a)
            .unwrap()
            .time_span()
            .map(|span| (span.first().ticks(), span.last().ticks()))
    );
    assert!(
        descriptors_under(root.path()).is_empty(),
        "a metadata-only request must release the idle owner: {:?}",
        descriptors_under(root.path())
    );
    assert_eq!(snapshot(&path), reference_a, "after read_metadata");

    eprintln!("PHASE 4");
    // Phase 4: another normal path replaces the idle owner instead of keeping both.
    let other = root.file("other.wdb");
    fs::copy(&fixture_b, &other).expect("copy published fixture B");
    assert_eq!(snapshot(&other), reference_b, "second path");
    let retained = descriptors_under(root.path());
    assert_eq!(
        retained.len(),
        1,
        "one idle owner must remain after two paths: {retained:?}"
    );
    assert!(
        retained[0].1.ends_with("other.wdb"),
        "the idle owner must belong to the last opened path: {retained:?}"
    );

    eprintln!("PHASE 5");
    // Phase 5: a same-size in-place rewrite with a restored mtime must never return the
    // previously imported state. The published equal-size pair is newer-only, so this phase skips
    // with an explicit version reason on a supported older reader. Size, inode and mtime all stay
    // unchanged there; only the content change time and the streamed digest can tell the files
    // apart.
    let equal_a = fixture(NEWER_SAME_SIZE_A);
    let equal_b = fixture(NEWER_SAME_SIZE_B);
    let same_size_supported = compatible(&equal_a).is_some() && compatible(&equal_b).is_some();
    if !same_size_supported {
        eprintln!("SKIP same-size invalidation: the published pair needs a newer reader");
    }
    let equal = root.file("equal.wdb");
    if same_size_supported {
        assert_eq!(
            fs::metadata(&equal_a).unwrap().len(),
            fs::metadata(&equal_b).unwrap().len(),
            "the published same-size pair must keep its size"
        );
        fs::copy(&equal_a, &equal).expect("copy same-size fixture A");
        let reference_equal_a = snapshot(&equal);
        let reference_equal_b = snapshot(&equal_b);
        assert_ne!(
            reference_equal_a, reference_equal_b,
            "the same-size pair must expose different observations"
        );
        let before = fs::metadata(&equal).expect("stat the same-size copy");
        fs::write(
            &equal,
            fs::read(&equal_b).expect("read same-size fixture B"),
        )
        .expect("rewrite the same-size copy in place");
        let file = fs::OpenOptions::new()
            .write(true)
            .open(&equal)
            .expect("open the same-size copy for a timestamp reset");
        file.set_modified(before.modified().expect("copy mtime"))
            .expect("restore the copy mtime");
        drop(file);
        let after = fs::metadata(&equal).expect("stat the rewritten same-size copy");
        assert_eq!(before.len(), after.len(), "the rewrite must keep the size");
        assert_eq!(before.ino(), after.ino(), "the rewrite must keep the inode");
        assert_eq!(
            before.modified().unwrap(),
            after.modified().unwrap(),
            "the rewrite keeps the recorded mtime"
        );
        assert_eq!(
            snapshot(&equal),
            reference_equal_b,
            "a same-size rewrite with a restored mtime must not return the previous observations"
        );
    }

    eprintln!("PHASE 6");
    // Phase 6: an in-place rewrite to other content, then an atomic replacement, then a
    // retargeted symlink.
    let before_rewrite = fs::metadata(&path).expect("stat copy A");
    fs::write(
        &path,
        fs::read(&fixture_b).expect("read published fixture B"),
    )
    .expect("rewrite the temporary file in place");
    let after_rewrite = fs::metadata(&path).expect("stat the rewritten file");
    assert_eq!(
        before_rewrite.ino(),
        after_rewrite.ino(),
        "in-place rewrite must keep the inode"
    );
    assert_eq!(snapshot(&path), reference_b, "rewritten content");

    let replacement = root.file("replacement.wdb");
    fs::copy(&fixture_a, &replacement).expect("copy published fixture A for replacement");
    fs::rename(&replacement, &path).expect("atomically replace the temporary path");
    assert_eq!(snapshot(&path), reference_a, "replacement content");

    let link = root.file("link.wdb");
    symlink(&path, &link).expect("create a symlink to the temporary copy");
    let target_b = root.file("target-b.wdb");
    fs::copy(&fixture_b, &target_b).expect("copy published fixture B as a new target");
    assert_eq!(snapshot(&link), reference_a, "symlink target A");
    fs::remove_file(&link).expect("remove the old symlink");
    symlink(&target_b, &link).expect("retarget the symlink to B");
    assert_eq!(snapshot(&link), reference_b, "retargeted symlink");

    eprintln!("PHASE 7");
    // Phase 7: a previously opened path that disappears must not return a cached success.
    assert_eq!(snapshot(&path), reference_a, "prefill before deletion");
    assert_eq!(
        descriptors_under(root.path()).len(),
        1,
        "prefill the idle owner before deleting its file"
    );
    fs::remove_file(&path).expect("delete the previously opened temporary file");
    assert!(
        ondas::open_with(&path, BACKEND).is_err(),
        "a deleted path must stay an error"
    );
    assert!(
        descriptors_under(root.path()).is_empty(),
        "a failed preflight must release the idle owner: {:?}",
        descriptors_under(root.path())
    );
    fs::copy(&fixture_a, &path).expect("recreate published fixture A at the same path");
    assert_eq!(snapshot(&path), reference_a, "recreated copy");
    fs::remove_file(&target_b).expect("delete the symlink target");
    assert!(
        ondas::open_with(&link, BACKEND).is_err(),
        "a dangling symlink must stay an error"
    );
    assert!(
        descriptors_under(root.path()).is_empty(),
        "a dangling-symlink failure must release the idle owner: {:?}",
        descriptors_under(root.path())
    );

    eprintln!("PHASE 8");
    // Phase 8: a deliberate stop keeps the owner clean and reusable.
    {
        let mut waveform = open_native(&path);
        let signal = first_signal(&waveform);
        let outcome = waveform
            .scan(&[signal], TimeRange::all(), |_| {
                std::ops::ControlFlow::Break(())
            })
            .unwrap();
        assert!(outcome.is_break());
    }
    assert_eq!(snapshot(&path), reference_a, "after a deliberate stop");

    eprintln!("PHASE 9");
    // Phase 9: a caller panic drops the Waveform while unwinding; that owner must not be kept.
    assert_eq!(
        descriptors_under(root.path()).len(),
        1,
        "prefill the idle owner before the panic"
    );
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut waveform = open_native(&path);
        let signals = vec![first_signal(&waveform)];
        let _ = waveform.scan(
            &signals,
            TimeRange::all(),
            |_| -> std::ops::ControlFlow<()> { std::panic::panic_any(VisitorSentinel) },
        );
    }));
    match panicked {
        Err(payload) => assert!(
            payload.downcast_ref::<VisitorSentinel>().is_some(),
            "the unwind must carry the intentional visitor sentinel"
        ),
        Ok(()) => panic!("the visitor panic did not unwind"),
    }
    assert!(
        descriptors_under(root.path()).is_empty(),
        "an unwinding owner must not be retained: {:?}",
        descriptors_under(root.path())
    );
    assert_eq!(
        snapshot(&path),
        reference_a,
        "recovery after the visitor panic"
    );

    eprintln!("PHASE 10");
    // Phase 10: nested and parallel owners over the same unchanged file stay independent.
    {
        let outer = open_native(&path);
        let mut inner = open_native(&path);
        assert_eq!(snapshot_waveform(&mut inner), reference_a, "nested owner");
        drop(inner);
        drop(outer);
    }
    let shared = Arc::new(path.clone());
    let expected = Arc::new(reference_a.clone());
    let workers: Vec<_> = (0..2)
        .map(|worker| {
            let shared = Arc::clone(&shared);
            let expected = Arc::clone(&expected);
            std::thread::spawn(move || {
                for round in 0..2 {
                    let mut waveform = open_native(&shared);
                    assert_eq!(
                        snapshot_waveform(&mut waveform),
                        *expected,
                        "parallel owner {worker} round {round}"
                    );
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().expect("parallel owner thread");
    }

    eprintln!("PHASE 11");
    // Phase 11: no temporary files were created next to the waveform.
    let mut expected_entries = vec![
        "copy.wdb".to_owned(),
        "link.wdb".to_owned(),
        "other.wdb".to_owned(),
    ];
    if same_size_supported {
        expected_entries.push("equal.wdb".to_owned());
    }
    expected_entries.sort();
    assert_eq!(
        temp_entries(root.path()),
        expected_entries,
        "unexpected files in the owned temporary root"
    );
    // The last logical close keeps at most one idle owner and drops the descriptor with it.
    drop(root);
}

/// Queryable signals of one open hierarchy.
fn queryable(waveform: &Waveform) -> Vec<Signal> {
    let signals: Vec<_> = waveform
        .hierarchy()
        .variables()
        .filter_map(|variable| variable.signal())
        .filter(|signal| !matches!(signal.encoding(), Encoding::Unsupported))
        .take(3)
        .collect();
    assert!(!signals.is_empty(), "fixture must expose queryable signals");
    signals
}

fn point_rows(waveform: &mut Waveform, signals: &[Signal], times: &[Time]) -> Vec<String> {
    times
        .iter()
        .map(|time| {
            waveform
                .samples(signals, *time)
                .unwrap_or_else(|error| panic!("samples at {}: {error}", time.ticks()))
                .iter()
                .map(sample_text)
                .collect::<Vec<_>>()
                .join(" | ")
        })
        .collect()
}

fn selection_rows(selection: &mut ondas::Selection<'_>, time: Time) -> Vec<String> {
    let rows = selection
        .samples(time)
        .unwrap_or_else(|error| panic!("samples at {}: {error}", time.ticks()));
    rows.iter().map(sample_text).collect()
}

// Signal identity is source-bound, so the text keeps only slot, time and value.
fn scan_rows(selection: &mut ondas::Selection<'_>, range: TimeRange) -> Vec<String> {
    let mut rows = Vec::new();
    let outcome = selection
        .scan_each(range, |slot, record| {
            rows.push(match record {
                ondas::ScanRef::Initial {
                    value, changed_at, ..
                } => format!(
                    "initial {slot} {} changed_at={:?}",
                    value_text(value),
                    changed_at.map(|time| time.ticks())
                ),
                ondas::ScanRef::Change { time, value, .. } => {
                    format!("change {slot} {} {}", time.ticks(), value_text(value))
                }
                other => panic!("unhandled ScanRef variant: {other:?}"),
            });
            std::ops::ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert!(outcome.is_continue());
    rows
}

fn change_count(selection: &mut ondas::Selection<'_>, range: TimeRange) -> usize {
    let mut changes = 0;
    let outcome = selection
        .scan_each(range, |_, record| {
            if matches!(record, ondas::ScanRef::Change { .. }) {
                changes += 1;
            }
            std::ops::ControlFlow::<()>::Continue(())
        })
        .unwrap();
    assert!(outcome.is_continue());
    changes
}

/// Late-delivery panic point inside a fully materialized result.
#[derive(Clone, Copy, Debug)]
enum LateCallbackPanic {
    /// Borrowed per-slot delivery through `Selection::visit_samples`.
    VisitSamples,
    /// Only the final change of a scan.
    LastChange,
    /// A scan whose only records are entering states.
    InitialsOnly,
}

/// A caller-caught panic in a late callback must leave exact observations on the same selection
/// and waveform, and after drop and reopen with a fresh source identity. It does not forbid
/// parking: the native traversal is already gone and the copied values are Rust-owned.
#[test]
#[ignore = "requires the selected reader and installed WDB fixtures; run just conformance-wdb"]
fn wdb_late_callback_panics_keep_exact_observations() {
    let fixture_a = fixture(FIXTURE_A);
    let root = TempRoot::new();
    let path = root.file("late.wdb");
    fs::copy(&fixture_a, &path).expect("copy published fixture");

    let (times, after_eof) = {
        let waveform = open_native(&path);
        let span = waveform.metadata().time_span().expect("recorded span");
        let (first, last) = (span.first().ticks(), span.last().ticks());
        (
            [
                Time::from_ticks(first),
                Time::from_ticks(first + (last - first) / 2),
                Time::from_ticks(last),
            ],
            TimeRange::closed(Time::from_ticks(last + 1), Time::from_ticks(last + 1)),
        )
    };

    for case in [
        LateCallbackPanic::VisitSamples,
        LateCallbackPanic::LastChange,
        LateCallbackPanic::InitialsOnly,
    ] {
        let mut waveform = open_native(&path);
        let signals: Vec<Signal> = {
            let mut candidates = queryable(&waveform);
            // The last-change case needs a signal that actually changes.
            if matches!(case, LateCallbackPanic::LastChange) {
                let mut selection = waveform.select(&candidates).unwrap();
                let changes = change_count(&mut selection, TimeRange::all());
                drop(selection);
                if changes == 0 {
                    candidates.retain(|_| false);
                }
            }
            candidates
        };
        if matches!(case, LateCallbackPanic::LastChange) && signals.is_empty() {
            panic!("fixture has no signal with recorded changes");
        }
        let stale = signals[0];
        let expected_points = point_rows(&mut waveform, &signals, &times);

        let mut selection = waveform.select(&signals).unwrap();
        let expected_rows = selection_rows(&mut selection, times[1]);
        let expected_scan = match case {
            LateCallbackPanic::VisitSamples => None,
            LateCallbackPanic::LastChange => {
                let rows = scan_rows(&mut selection, TimeRange::all());
                assert!(
                    rows.iter().any(|row| row.starts_with("change")),
                    "the last-change case needs at least one change"
                );
                Some((TimeRange::all(), rows))
            }
            LateCallbackPanic::InitialsOnly => {
                let rows = scan_rows(&mut selection, after_eof);
                assert!(
                    !rows.is_empty() && rows.iter().all(|row| row.starts_with("initial")),
                    "the initials-only case needs entering states and no changes: {rows:?}"
                );
                Some((after_eof, rows))
            }
        };

        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match case {
            LateCallbackPanic::VisitSamples => {
                let _ = selection.visit_samples(times[1], |_| -> std::ops::ControlFlow<()> {
                    std::panic::panic_any(VisitorSentinel)
                });
            }
            LateCallbackPanic::LastChange => {
                let total = change_count(&mut selection, TimeRange::all());
                let mut seen = 0;
                let _ = selection.scan_each(TimeRange::all(), |_, record| {
                    if matches!(record, ondas::ScanRef::Change { .. }) {
                        seen += 1;
                        if seen == total {
                            std::panic::panic_any(VisitorSentinel);
                        }
                    }
                    std::ops::ControlFlow::<()>::Continue(())
                });
            }
            LateCallbackPanic::InitialsOnly => {
                let _ = selection.scan_each(after_eof, |_, record| {
                    if matches!(record, ondas::ScanRef::Initial { .. }) {
                        std::panic::panic_any(VisitorSentinel);
                    }
                    std::ops::ControlFlow::<()>::Continue(())
                });
            }
        }));
        assert!(caught.is_err(), "case {case:?} must unwind");
        assert!(
            caught
                .unwrap_err()
                .downcast_ref::<VisitorSentinel>()
                .is_some(),
            "case {case:?}: the unwind must carry the intentional sentinel"
        );

        // The same selection and waveform still produce the exact same observations.
        assert_eq!(
            selection_rows(&mut selection, times[1]),
            expected_rows,
            "case {case:?}: selection observations after the caught panic"
        );
        if let Some((range, rows)) = &expected_scan {
            assert_eq!(
                &scan_rows(&mut selection, *range),
                rows,
                "case {case:?}: scan observations after the caught panic"
            );
        }
        drop(selection);
        assert_eq!(
            point_rows(&mut waveform, &signals, &times),
            expected_points,
            "case {case:?}: waveform observations after the caught panic"
        );
        drop(waveform);

        // Reopen: same observations, fresh source identity, earlier handles rejected.
        let mut waveform = open_native(&path);
        assert!(
            waveform.select(&[stale]).is_err(),
            "case {case:?}: a Signal from the earlier open must be rejected"
        );
        let signals = queryable(&waveform);
        assert_eq!(
            point_rows(&mut waveform, &signals, &times),
            expected_points,
            "case {case:?}: observations after reopen"
        );
    }
}

/// A panic that passes through the active backend keeps the owner unparked, even when the caller
/// catches it and drops the waveform later, outside the unwind.
#[test]
#[ignore = "requires the selected reader and installed WDB fixtures; run just conformance-wdb"]
fn wdb_active_panic_keeps_the_owner_unparked() {
    let fixture_a = fixture(FIXTURE_A);
    let root = TempRoot::new();
    let path = root.file("active.wdb");
    fs::copy(&fixture_a, &path).expect("copy the compatible published fixture");
    if compatible(&path).is_none() {
        eprintln!("SKIP active panic: this reader cannot open the compatible recording");
        return;
    }

    // A clean lifetime first, so the failing one runs over a retained owner.
    {
        let mut waveform = open_native(&path);
        let signal = first_signal(&waveform);
        assert!(waveform.sample(signal, Time::ZERO).is_ok());
    }
    assert_eq!(
        descriptors_under(root.path()).len(),
        1,
        "prefill the idle owner before the failing lifetime"
    );

    {
        let mut waveform = open_native(&path);
        let signals = vec![first_signal(&waveform)];
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = waveform.scan(
                &signals,
                TimeRange::all(),
                |_| -> std::ops::ControlFlow<()> { std::panic::panic_any(VisitorSentinel) },
            );
        }));
        assert!(caught.is_err(), "the visitor panic must unwind");
        // The native traversal was removed during the unwind: a later query works, and the later
        // drop is an ordinary drop with no active unwind.
        let signal = first_signal(&waveform);
        assert!(
            waveform.sample(signal, Time::ZERO).is_ok(),
            "the native traversal must be cleared during the unwind"
        );
    }
    assert!(
        descriptors_under(root.path()).is_empty(),
        "a panic through the active backend must stay unparked after a later ordinary drop: {:?}",
        descriptors_under(root.path())
    );
    let mut waveform = open_native(&path);
    assert!(waveform.sample(first_signal(&waveform), Time::ZERO).is_ok());
}

/// A decode error through the active backend keeps the owner unparked. The recorded non-ASCII
/// string is newer-only, so a supported older reader skips this scenario with a version reason.
#[test]
#[ignore = "requires the selected reader and installed WDB fixtures; run just conformance-wdb"]
fn wdb_decode_error_keeps_the_owner_unparked() {
    let fixture_strings = fixture(NEWER_STRINGS);
    let root = TempRoot::new();
    let path = root.file("strings.wdb");
    fs::copy(&fixture_strings, &path).expect("copy the published fixture");
    if compatible(&path).is_none() {
        eprintln!("SKIP decode error: the recorded non-ASCII string needs a newer reader");
        return;
    }

    {
        let mut waveform = open_native(&path);
        let good = waveform.hierarchy().signal("top.message").unwrap();
        assert!(waveform.sample(good, Time::from_ticks(9000)).is_ok());
    }
    {
        let mut waveform = open_native(&path);
        let bad = waveform.hierarchy().signal("top.non_ascii").unwrap();
        assert!(
            matches!(
                waveform.trace(bad, TimeRange::all()),
                Err(ondas::Error::NonAsciiString { .. })
            ),
            "the published non-ASCII string must produce a decode error"
        );
        let good = waveform.hierarchy().signal("top.message").unwrap();
        assert!(
            waveform.sample(good, Time::from_ticks(9000)).is_ok(),
            "a decode failure must not break other histories of the live reader"
        );
    }
    assert!(
        descriptors_under(root.path()).is_empty(),
        "a decode error must keep the owner unparked: {:?}",
        descriptors_under(root.path())
    );
}

/// Cold-miss, reused-owner, descriptor and RSS behaviour. Prints measurements; asserts
/// invariants. Each close releases the session's SDK histories, so a reopened owner pays the
/// selected-history loading cost again.
#[test]
#[ignore = "requires the selected reader and installed WDB fixtures; run just conformance-wdb"]
fn wdb_reuse_resource_behaviour() {
    let fixture_a = fixture(FIXTURE_A);
    let root = TempRoot::new();
    let path = root.file("copy.wdb");
    fs::copy(&fixture_a, &path).expect("copy published fixture A");

    let cold = {
        let started = Instant::now();
        let _waveform = open_native(&path);
        started.elapsed()
    };
    let reused = {
        let started = Instant::now();
        let _waveform = open_native(&path);
        started.elapsed()
    };
    drop(open_native(&path));

    let (fd_before, rss_before) = resource_usage();
    for _ in 0..4 {
        let mut waveform = open_native(&path);
        let signal = first_signal(&waveform);
        let _ = waveform
            .scan(&[signal], TimeRange::all(), |_| {
                std::ops::ControlFlow::<()>::Continue(())
            })
            .unwrap();
    }
    let (fd_after, rss_after) = resource_usage();
    eprintln!(
        "wdb reuse resources: cold={cold:?} reused={reused:?} fds {fd_before}->{fd_after} rss_kib {rss_before}->{rss_after}"
    );

    // A retained owner is one owner, not a resource ceiling: descriptors must not grow per
    // logical open, and a changed path must release the previous owner.
    assert!(
        fd_after <= fd_before + 1,
        "descriptor count grew from {fd_before} to {fd_after}"
    );
    let retained = descriptors_under(root.path());
    assert!(retained.len() <= 1, "retained owners: {retained:?}");
    let other = root.file("other.wdb");
    fs::copy(fixture(FIXTURE_B), &other).expect("copy published fixture B");
    let _ = open_native(&other);
    assert!(
        descriptors_under(root.path()).len() <= 1,
        "a changed path must replace the idle owner: {:?}",
        descriptors_under(root.path())
    );
}

/// Open descriptor count and resident set size of this process.
fn resource_usage() -> (usize, u64) {
    let descriptors = fs::read_dir("/proc/self/fd")
        .map(|entries| entries.count())
        .unwrap_or(0);
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let rss = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    (descriptors, rss)
}

/// A parked owner must be released by the normal-exit cleanup, even when an earlier logical open
/// registered that cleanup before any history existed.
#[test]
#[ignore = "requires the selected reader and installed WDB fixtures; run just conformance-wdb"]
fn wdb_normal_exit_releases_a_parked_owner() {
    let fixture = fixture("wdb0301-opb-m");
    if std::env::var_os("ONDAS_WDB_EXIT_CHILD").is_some() {
        // First logical open has no history and registers the exit cleanup.
        drop(open_native(&fixture));
        // Second logical open creates a history and parks it; the process then exits normally.
        let mut waveform = open_native(&fixture);
        let signal = first_signal(&waveform);
        let _ = waveform
            .sample(signal, Time::from_ticks(5_000_000))
            .unwrap();
        drop(waveform);
        return;
    }
    let root = TempRoot::new();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "wdb_normal_exit_releases_a_parked_owner",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("ONDAS_WDB_EXIT_CHILD", "1")
        .env("TMPDIR", root.path())
        .output()
        .expect("run the exit child");
    assert!(
        output.status.success(),
        "exit child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        temp_entries(root.path()).is_empty(),
        "normal exit left temporary files: {:?}",
        temp_entries(root.path())
    );
}

fn bank_paths(bank: &str, count: usize) -> Vec<ondas::HierarchyPath> {
    (0..count)
        .map(|i| {
            ondas::HierarchyPath::from_components([
                "tb_opb".to_string(),
                bank.to_string(),
                format!("word[{i}].data"),
                "q".to_string(),
            ])
        })
        .collect()
}

fn resolve_paths(wave: &Waveform, paths: &[ondas::HierarchyPath]) -> Vec<Signal> {
    paths
        .iter()
        .map(|path| wave.hierarchy().signal_path(path).unwrap())
        .collect()
}

fn report_resource(fixture: &str, phase: &str, elapsed: Option<Instant>, peak: bool) {
    let (fds, rss) = resource_usage();
    let root = PathBuf::from(std::env::var_os("TMPDIR").unwrap_or_default());
    let mut files = Vec::new();
    let mut bytes = 0u64;
    let mut blocks = 0u64;
    if let Ok(entries) = fs::read_dir(&root) {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata()
                && metadata.is_file()
            {
                bytes += metadata.len();
                blocks += metadata.blocks() * 512;
                files.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    let elapsed_ms = elapsed
        .map(|start| format!("{:.3}", start.elapsed().as_secs_f64() * 1000.0))
        .unwrap_or_default();
    println!(
        "RESOURCE,fixture={fixture},phase={phase},elapsed_ms={elapsed_ms},peak={peak},rss_kib={rss},rss_mib={:.3},fds={fds},tmp_files={},tmp_bytes={bytes},tmp_blocks_bytes={blocks},files={}",
        rss as f64 / 1024.0,
        files.len(),
        files.join("|")
    );
}

/// Cold, reused-owner and read-set resource control at a substantial 10 000-signal read set.
/// Counters are measured by the private native probe; this test measures the actual Rust
/// consumer (resident set, fds, SDK temporary backing files) and the normal-exit cleanup in an
/// isolated TMPDIR. Every close releases the session indexes, so each reopen builds them again.
#[test]
#[ignore = "requires the selected reader and installed WDB fixtures; run just conformance-wdb"]
fn wdb_readset_resources_at_k10000() {
    if std::env::var_os("ONDAS_WDB_RESOURCE_CHILD").is_some() {
        for name in ["wdb0301-opb-m", "wdb0302-opb-l"] {
            let path = fixture(name);
            let regular = bank_paths("regular64", 10_000);
            let staggered = bank_paths("staggered64", 10_000);
            let tick = Time::from_ticks(5_000_000);
            let late = Time::from_ticks(50_000_000);

            let started = Instant::now();
            let mut wave = open_native(&path);
            let open = started.elapsed();
            let signals = resolve_paths(&wave, &regular);
            let started = Instant::now();
            let rows = wave.samples(&signals, tick).unwrap();
            assert_eq!(rows.len(), 10_000);
            report_resource(name, "cold-open", Some(started), false);
            println!(
                "RESOURCE,fixture={name},phase=cold-open-ms,elapsed_ms={:.3}",
                open.as_secs_f64() * 1000.0
            );
            drop(wave);
            report_resource(name, "cold-park", None, false);

            let mut wave = open_native(&path);
            let signals = resolve_paths(&wave, &regular);
            let started = Instant::now();
            let rows = wave.samples(&signals, late).unwrap();
            assert_eq!(rows.len(), 10_000);
            report_resource(name, "reuse-hit", Some(started), false);
            let mut selection = wave.select(&signals).unwrap();
            let started = Instant::now();
            let _ = selection.samples(tick).unwrap();
            report_resource(name, "live-repeat", Some(started), false);
            drop(selection);
            drop(wave);
            report_resource(name, "reuse-park", None, false);

            // A full disjoint bank: the previous session released its histories, so this one
            // creates and loads its own staggered64 histories.
            let mut wave = open_native(&path);
            let other = resolve_paths(&wave, &staggered);
            let started = Instant::now();
            let rows = wave.samples(&other, tick).unwrap();
            assert_eq!(rows.len(), 10_000);
            report_resource(name, "disjoint-full", Some(started), true);
            drop(wave);
            report_resource(name, "disjoint-park", None, false);

            let mut wave = open_native(&path);
            let half = resolve_paths(&wave, &staggered[..5_000]);
            let started = Instant::now();
            let rows = wave.samples(&half, tick).unwrap();
            assert_eq!(rows.len(), 5_000);
            report_resource(name, "subset", Some(started), false);
            drop(wave);
            report_resource(name, "subset-park", None, false);

            drop(open_native(&path)); // no query
            report_resource(name, "no-query-park", None, false);
            drop(open_native(&fixture("wdb0311-partial-logging"))); // evicts the parked owner
            report_resource(name, "evict", None, false);
        }
        return;
    }
    let root = TempRoot::new();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "wdb_readset_resources_at_k10000",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("ONDAS_WDB_RESOURCE_CHILD", "1")
        .env("TMPDIR", root.path())
        .status()
        .expect("run the resource child");
    assert!(output.success(), "resource child failed");
    assert!(
        temp_entries(root.path()).is_empty(),
        "normal exit left temporary files: {:?}",
        temp_entries(root.path())
    );
}
