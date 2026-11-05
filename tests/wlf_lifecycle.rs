//! Lifecycle and identity checks for sequential WLF opens over two published fixtures.
//!
//! One ignored SDK test exercises public API semantics with the explicit `wlf-lib` backend:
//! exact snapshots after a same-path reopen, external lock access after a logical close,
//! in-place rewrite, atomic replacement, symlink retargeting, previously opened paths that
//! disappear, metadata and another path between lifetimes, caller panic unwinding, and
//! descriptor retention under one temporary root. Reference snapshots of both published
//! originals are computed first.
//!
//! Both published fixtures use a one-nanosecond timescale and bit-vector signals only. This
//! test therefore covers lifecycle, identity and bit-vector observations, including the exact
//! known and unknown `changed_at` values and `Missing` samples that these files actually
//! produce. Other encodings, events and resolution switches are covered by the existing
//! conformance suite, not here.

#![cfg(feature = "wlf-lib")]

use ondas::{Encoding, Sample, Signal, Time, TimeRange, ValueRef, Waveform};
use std::{
    fs,
    os::fd::AsRawFd,
    os::unix::fs::{MetadataExt, symlink},
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU32, Ordering},
};

/// The native backend that these qualification checks must exercise explicitly.
const BACKEND: &str = "wlf-lib";
/// Fixed small same-path reopen cycles; no timing is measured.
const SAME_PATH_CYCLES: usize = 8;

fn fixture(name: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/wlf")
        .join(name)
        .join("waveform.wlf");
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
    /// Creates a fresh directory that this test exclusively owns, or fails.
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let mut attempt = 0;
        loop {
            let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "ondas-wlf-lifecycle-{}-{nonce}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    // Canonicalize only the directory this call created. On failure remove
                    // that directory again and fail instead of keeping an unknown path.
                    match fs::canonicalize(&path) {
                        Ok(canonical) => return Self(canonical),
                        Err(error) => {
                            let _ = fs::remove_dir(&path);
                            panic!(
                                "cannot canonicalize the exclusive temporary root {}: {error}",
                                path.display()
                            );
                        }
                    }
                }
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
        // Removes only the directory this test created.
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn value_text(value: ValueRef<'_>) -> String {
    match value {
        ValueRef::Bits(bits) => format!("bits:{bits}"),
        ValueRef::Real(real) => format!("real:{:016x}", real.to_bits()),
        ValueRef::String(text) => format!("string:{text:?}"),
        ValueRef::Event { occurrences } => format!("event:{occurrences}"),
        other => panic!("unhandled ValueRef variant: {other:?}"),
    }
}

fn sample_text(sample: &Sample) -> String {
    match sample {
        Sample::Missing { .. } => "missing".into(),
        Sample::Value {
            value, changed_at, ..
        } => format!(
            "value:{} changed_at={}",
            value_text(value.as_ref()),
            changed_at_text(*changed_at)
        ),
        Sample::Event { occurrences, .. } => format!("event:{occurrences}"),
        other => panic!("unhandled Sample variant: {other:?}"),
    }
}

fn changed_at_text(changed_at: Option<Time>) -> String {
    match changed_at {
        Some(time) => format!("Some({})", time.ticks()),
        None => "None".into(),
    }
}

fn probe_times(span: Option<ondas::TimeSpan>) -> Vec<Time> {
    match span {
        Some(span) => {
            let (first, last) = (span.first().ticks(), span.last().ticks());
            let quarter = first + (last - first) / 4;
            let middle = first + (last - first) / 2;
            vec![
                Time::from_ticks(first),
                Time::from_ticks(quarter),
                Time::from_ticks(middle),
                Time::from_ticks(last),
            ]
        }
        None => vec![Time::ZERO],
    }
}

fn snapshot_waveform(waveform: &mut Waveform) -> Vec<String> {
    let mut lines = Vec::new();
    let metadata = waveform.metadata();
    lines.push(format!(
        "metadata timescale={:?} span={:?} writer={:?} date={:?} comments={:?}",
        metadata.timescale(),
        metadata
            .time_span()
            .map(|span| (span.first().ticks(), span.last().ticks())),
        metadata.writer(),
        metadata.date(),
        metadata.comments().collect::<Vec<_>>(),
    ));
    let probes = probe_times(metadata.time_span());
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
            lines.push(format!("variable {path} unsupported encoding, not queried"));
            continue;
        }
        lines.push(format!("variable {path}"));
        let trace = waveform
            .trace(signal, TimeRange::all())
            .unwrap_or_else(|error| panic!("trace {path}: {error}"));
        match trace.initial() {
            Some(initial) => lines.push(format!(
                "  initial {} changed_at={}",
                value_text(initial.value()),
                changed_at_text(initial.changed_at())
            )),
            None => lines.push("  initial none".into()),
        }
        for change in trace.changes() {
            lines.push(format!(
                "  change {} {}",
                change.time().ticks(),
                value_text(change.value())
            ));
        }
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
    lines
}

fn snapshot(path: &Path) -> Vec<String> {
    let mut waveform = open_native(path);
    snapshot_waveform(&mut waveform)
}

fn span_of(path: &Path) -> Option<(u64, u64)> {
    let waveform = open_native(path);
    waveform
        .metadata()
        .time_span()
        .map(|span| (span.first().ticks(), span.last().ticks()))
}

/// Descriptor entries whose target is inside this test's own temporary root, with their
/// descriptor numbers. Duplicate targets stay separate so a duplicate descriptor is visible.
fn descriptors_under(root: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir("/proc/self/fd") else {
        panic!("no /proc/self/fd; descriptor observation needs Linux procfs");
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let descriptor = entry.file_name().to_string_lossy().into_owned();
        if let Ok(target) = fs::read_link(entry.path()) {
            if target.starts_with(root) {
                found.push((descriptor, target));
            }
        }
    }
    found.sort();
    found
}

/// Ordinary external write-lock access to the given file, using the existing Python helper.
///
/// This is functional validation tooling for the test, not a product backend or a subprocess
/// interface of the reader.
fn lock_access(path: &Path) {
    let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wlf_lifecycle_lock.py");
    let output = Command::new("python3")
        .arg(&helper)
        .arg(path)
        .output()
        .unwrap_or_else(|error| panic!("run python3 lock helper: {error}"));
    assert!(
        output.status.success(),
        "exclusive non-blocking lock or access failed for {}: status {:?} stdout {:?} stderr {:?}",
        path.display(),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Payload that identifies the intentional visitor panic exactly.
#[derive(Debug)]
struct VisitorSentinel;

#[test]
#[ignore = "requires the installed WLF SDK, published fixtures and python3"]
fn wlf_sequential_open_lifecycle_over_published_fixtures() {
    let fixture_a = fixture("wlf0074-history-short");
    let fixture_b = fixture("wlf0076-topology-small");

    // Reference snapshots of both originals, before any temporary copy exists.
    let reference_a = snapshot(&fixture_a);
    let reference_b = snapshot(&fixture_b);
    assert_ne!(
        reference_a, reference_b,
        "the two published fixtures must expose different snapshots"
    );

    let root = TempRoot::new();
    let path = root.file("copy.wlf");

    // Phase 1: owned temporary copy, same-path reopen cycles and stale-signal rejection.
    fs::copy(&fixture_a, &path).expect("copy published fixture A");
    // Observer guard: a test-owned descriptor on this path must appear under the canonical root.
    {
        let observed =
            fs::File::open(&path).expect("open the temporary copy for the descriptor guard");
        let descriptor = observed.as_raw_fd().to_string();
        let entries = descriptors_under(root.path());
        assert!(
            entries.iter().any(|(entry, _)| entry == &descriptor),
            "the test-owned descriptor {descriptor} is not reported under the canonical root; entries were {entries:?}"
        );
        drop(observed);
    }
    assert_eq!(snapshot(&path), reference_a, "copy of A");
    let stale = {
        let waveform = open_native(&path);
        waveform
            .hierarchy()
            .variables()
            .filter_map(|variable| variable.signal())
            .next()
            .expect("published fixture must expose a signal")
    };
    let mut baseline = None;
    for cycle in 0..SAME_PATH_CYCLES {
        let mut waveform = open_native(&path);
        assert_eq!(
            snapshot_waveform(&mut waveform),
            reference_a,
            "same-path cycle {cycle}"
        );
        drop(waveform);
        let retained = descriptors_under(root.path()).len();
        match baseline {
            None => baseline = Some(retained),
            Some(first) => assert!(
                retained <= first,
                "descriptor entries under the temporary root grew: cycle {cycle} retained {retained}, first cycle {first}"
            ),
        }
    }
    {
        let mut waveform = open_native(&path);
        assert!(
            waveform.select(&[stale]).is_err(),
            "a Signal from a dropped Waveform must be rejected"
        );
        assert_eq!(snapshot_waveform(&mut waveform), reference_a);
    }

    // Phase 2: ordinary external write-lock access after the logical close and before any reset.
    lock_access(&path);

    // Phase 3: in-place rewrite of the same inode exposes the other published content.
    let before_rewrite = fs::metadata(&path).expect("stat copy A");
    fs::write(
        &path,
        fs::read(&fixture_b).expect("read published fixture B"),
    )
    .expect("rewrite the temporary file in place");
    let after_rewrite = fs::metadata(&path).expect("stat rewritten file");
    assert_eq!(
        before_rewrite.ino(),
        after_rewrite.ino(),
        "in-place rewrite must keep the inode"
    );
    assert_eq!(snapshot(&path), reference_b, "rewritten content");

    // Phase 4: atomic replacement uses a new inode; a retargeted symlink follows its target.
    let replacement = root.file("replacement.wlf");
    fs::copy(&fixture_a, &replacement).expect("copy published fixture A for replacement");
    fs::rename(&replacement, &path).expect("atomically replace the temporary path");
    let replaced = fs::metadata(&path).expect("stat replaced file");
    assert_ne!(
        after_rewrite.ino(),
        replaced.ino(),
        "atomic replacement must use a new inode"
    );
    assert_eq!(snapshot(&path), reference_a, "replacement content");
    let link = root.file("link.wlf");
    symlink(&path, &link).expect("create a symlink to the temporary copy");
    assert_eq!(snapshot(&link), reference_a, "symlink target A");
    let target_b = root.file("target-b.wlf");
    fs::copy(&fixture_b, &target_b).expect("copy published fixture B as a new target");
    fs::remove_file(&link).expect("remove the old symlink");
    symlink(&target_b, &link).expect("retarget the symlink to B");
    assert_eq!(snapshot(&link), reference_b, "retargeted symlink");

    // Phase 5: previously opened paths that disappear cannot return a cached successful open.
    // The last open before this point may have bound the idle owner to another path.
    assert_eq!(
        snapshot(&path),
        reference_a,
        "prefill before deleting the copy"
    );
    fs::remove_file(&path).expect("delete the previously opened temporary file");
    assert!(
        ondas::open_with(&path, BACKEND).is_err(),
        "a previously opened path that was deleted must stay an error"
    );
    fs::copy(&fixture_a, &path).expect("recreate published fixture A at the same path");
    // A failed open above cleared any earlier idle owner; resolve this link successfully first.
    assert_eq!(
        snapshot(&link),
        reference_b,
        "prefill before deleting the symlink target"
    );
    fs::remove_file(&target_b).expect("delete the previously opened symlink target");
    assert!(
        ondas::open_with(&link, BACKEND).is_err(),
        "a previously resolved symlink that became dangling must stay an error"
    );
    fs::copy(&fixture_b, &target_b).expect("recreate published fixture B at the symlink target");
    assert_eq!(snapshot(&link), reference_b, "recreated symlink target B");

    // Phase 6: metadata and another normal path between lifetimes must not leave stale state.
    assert_eq!(snapshot(&path), reference_a);
    assert_eq!(
        span_of(&path),
        span_of(&fixture_a),
        "metadata of the temporary copy must match published A"
    );
    let metadata = ondas::read_metadata(&path).expect("read metadata of the temporary copy");
    assert_eq!(
        metadata
            .time_span()
            .map(|span| (span.first().ticks(), span.last().ticks())),
        span_of(&fixture_a)
    );
    assert_eq!(snapshot(&path), reference_a, "after read_metadata");
    assert_eq!(
        snapshot(&fixture_b),
        reference_b,
        "interleaved published path"
    );
    assert_eq!(snapshot(&path), reference_a, "after interleaved open");

    // Phase 7: a caller panic inside a visitor drops the Waveform while unwinding.
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut waveform = open_native(&path);
        let signals: Vec<Signal> = waveform.hierarchy().signals().take(1).collect();
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
    lock_access(&path);
    assert_eq!(
        snapshot(&path),
        reference_a,
        "recovery after the visitor panic"
    );

    // Phase 8: a different published path releases the temporary owners and guard.
    assert_eq!(snapshot(&path), reference_a);
    let _other = open_native(&fixture_b);
    let retained = descriptors_under(root.path());
    assert!(
        retained.is_empty(),
        "descriptor entries under the temporary root were not released: {retained:?}"
    );
}
