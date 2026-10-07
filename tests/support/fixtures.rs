//! Shared sidecar loading and artifact checks for tests and benchmarks.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde_json::Value as Json;
use sha2::{Digest, Sha256};

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .canonicalize()
        .expect("fixtures are absent; run git submodule update --init fixtures on the host, then ./dev just fixtures-install")
}

fn check_artifact(path: &Path, artifact: &Json, name: &str) {
    let mut file = fs::File::open(path).unwrap();
    let mut digest = Sha256::new();
    let size = io::copy(&mut file, &mut digest).unwrap();
    assert_eq!(
        Some(size),
        artifact["size"].as_u64(),
        "{name}: artifact size"
    );
    assert_eq!(
        format!("{:x}", digest.finalize()),
        artifact["sha256"].as_str().unwrap(),
        "{name}: artifact SHA256"
    );
}

pub fn read_sidecar(root: &Path, name: &str) -> (PathBuf, Json) {
    let directory = root
        .join(name)
        .canonicalize()
        .unwrap_or_else(|e| panic!("{name}: fixture directory: {e}"));
    assert!(directory.starts_with(root), "{name}: fixture escapes root");
    let sidecar_path = directory
        .join("fixture.json")
        .canonicalize()
        .expect("fixture sidecar path");
    assert!(
        sidecar_path.starts_with(&directory) && sidecar_path.is_file(),
        "{name}: sidecar containment/type"
    );
    let sidecar: Json = serde_json::from_slice(
        &fs::read(sidecar_path).unwrap_or_else(|e| panic!("{name}: sidecar: {e}")),
    )
    .expect("fixture JSON");
    assert_eq!(sidecar["schema"], 1, "{name}: sidecar schema");
    (directory, sidecar)
}

pub fn verify_artifact(directory: &Path, artifact: &Json, name: &str) -> PathBuf {
    let extension = artifact["format"].as_str().unwrap();
    assert!(
        matches!(extension, "fst" | "vcd" | "fsdb" | "ghw" | "wlf"),
        "{name}: invalid artifact format"
    );
    let filename = format!("waveform.{extension}");
    assert_eq!(artifact["file"], filename, "{name}: artifact filename");
    let path = directory.join(filename).canonicalize().unwrap();
    assert!(
        path.starts_with(directory) && path.is_file(),
        "{name}: artifact containment/type"
    );
    check_artifact(&path, artifact, name);
    path
}

pub fn load_artifact(root: &Path, name: &str) -> (PathBuf, Json) {
    let (directory, sidecar) = read_sidecar(root, name);
    let path = verify_artifact(&directory, &sidecar["artifact"], name);
    (path, sidecar)
}
