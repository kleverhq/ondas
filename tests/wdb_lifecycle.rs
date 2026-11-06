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
//! identity or an unwinding caller must release it. The second ignored test reports cold-miss,
//! warm-hit, descriptor and RSS behaviour; it asserts only invariants, not speed.
//!
//! The published fixtures are small declared-value recordings; other encodings, events and
//! resolution switches remain covered by the existing conformance suite.

#![cfg(feature = "wdb-lib")]

use ondas::{Encoding, Sample, Signal, Time, TimeRange, ValueRef, Waveform};
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
const FIXTURE_A: &str = "wdb0305-enum-core";
const FIXTURE_B: &str = "wdb0310-packed-shapes";

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
    // previously imported state. wdb0042 and wdb0043 are published recordings of equal size, so
    // size, inode and mtime all stay unchanged and only the content change time and the streamed
    // digest can distinguish the file.
    let equal_a = fixture("wdb0042-work-mem1-tb");
    let equal_b = fixture("wdb0043-work-mem3-tb");
    assert_eq!(
        fs::metadata(&equal_a).unwrap().len(),
        fs::metadata(&equal_b).unwrap().len(),
        "the published same-size pair must keep its size"
    );
    let equal = root.file("equal.wdb");
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
    assert_eq!(
        temp_entries(root.path()),
        vec![
            "copy.wdb".to_owned(),
            "equal.wdb".to_owned(),
            "link.wdb".to_owned(),
            "other.wdb".to_owned()
        ],
        "unexpected files in the owned temporary root"
    );
    // The last logical close keeps at most one idle owner and drops the descriptor with it.
    drop(root);
}

/// A read or decode failure and a caller-caught panic must both keep the owner unparked.
#[test]
#[ignore = "requires the selected reader and installed WDB fixtures; run just conformance-wdb"]
fn wdb_failed_reads_and_caught_panics_keep_the_owner_unparked() {
    let fixture_strings = fixture("wdb0317-vhdl-strings-ports");
    let root = TempRoot::new();
    let path = root.file("strings.wdb");
    fs::copy(&fixture_strings, &path).expect("copy published fixture");
    // A clean lifetime first, so the failure below runs over a retained owner.
    {
        let mut waveform = open_native(&path);
        let good = waveform.hierarchy().signal("top.message").unwrap();
        assert!(waveform.sample(good, Time::from_ticks(9000)).is_ok());
    }
    assert_eq!(
        descriptors_under(root.path()).len(),
        1,
        "prefill the idle owner before the failing lifetimes"
    );

    // Phase A: a decode error (a non-ASCII recorded string) taints reuse only; the same reader
    // still serves other histories and the owner is not parked by an ordinary drop.
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

    // Phase B: the caller catches a visitor panic, keeps the waveform and drops it later, when the
    // thread is no longer unwinding. The traversal is cleared during the unwind and the owner must
    // still stay unparked.
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
        let good = waveform.hierarchy().signal("top.message").unwrap();
        assert!(
            waveform.sample(good, Time::from_ticks(9000)).is_ok(),
            "the native traversal must be cleared during the unwind"
        );
    }
    assert!(
        descriptors_under(root.path()).is_empty(),
        "a caught panic must stay unparked after a later ordinary drop: {:?}",
        descriptors_under(root.path())
    );
    let mut waveform = open_native(&path);
    let good = waveform.hierarchy().signal("top.message").unwrap();
    assert!(
        waveform.sample(good, Time::from_ticks(9000)).is_ok(),
        "a clean open after the failures still works"
    );
}

/// Cold-miss, warm-hit, descriptor and RSS behaviour. Prints measurements; asserts invariants.
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
    let warm = {
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
        "wdb reuse resources: cold={cold:?} warm={warm:?} fds {fd_before}->{fd_after} rss_kib {rss_before}->{rss_after}"
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
