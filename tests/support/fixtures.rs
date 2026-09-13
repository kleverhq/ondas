//! Shared catalog and artifact checks for conformance tests and benchmarks.
use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde_json::Value as Json;
use sha2::{Digest, Sha256};

pub const PROVIDER: &str = "kleverhq.ondas-fixtures";

pub fn provider() -> PathBuf {
    checked_provider(PROVIDER, include_str!("../../fixtures.lock.toml"))
        .expect("required public fixture provider is absent; run just fixtures-install")
}

pub fn provider_directory(path: &Path) -> Option<PathBuf> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        result => {
            result.unwrap();
            assert!(
                path.is_dir(),
                "invalid provider directory: {}",
                path.display()
            );
        }
    }
    Some(path.canonicalize().unwrap())
}

pub fn checked_provider(name: &str, lock_text: &str) -> Option<PathBuf> {
    let lock: toml::Value = toml::from_str(lock_text).expect("fixture lock TOML");
    let provider_version = lock["providers"][name]
        .as_str()
        .expect("selected provider version must be a string");
    assert!(!provider_version.is_empty(), "empty provider version");
    let root = PathBuf::from(
        std::env::var_os("ONDAS_FIXTURES")
            .expect("ONDAS_FIXTURES is required; run just conformance"),
    );
    let provider = provider_directory(&root.join(name))?;
    let catalog: Json =
        serde_json::from_slice(&fs::read(provider.join("catalog.json")).expect("provider catalog"))
            .unwrap();
    assert_eq!(catalog["schema"], 1, "catalog schema");
    assert_eq!(catalog["provider"], name, "provider identity");
    assert_eq!(
        catalog["version"].as_str(),
        Some(provider_version),
        "provider version mismatch"
    );
    Some(provider)
}

pub fn check_artifact(path: &Path, artifact: &Json, name: &str) {
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

pub fn load_artifact(provider: &Path, name: &str) -> (PathBuf, Json) {
    let directory = provider
        .join(name)
        .canonicalize()
        .unwrap_or_else(|e| panic!("{name}: fixture directory: {e}"));
    assert!(
        directory.starts_with(provider),
        "{name}: fixture escapes provider"
    );
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
    let artifact = &sidecar["artifact"];
    let extension = artifact["format"].as_str().unwrap();
    assert!(
        matches!(extension, "fst" | "vcd" | "fsdb" | "ghw" | "wlf"),
        "{name}: invalid artifact format"
    );
    let filename = format!("waveform.{extension}");
    assert_eq!(artifact["file"], filename, "{name}: artifact filename");
    let path = directory.join(filename).canonicalize().unwrap();
    assert!(
        path.starts_with(&directory) && path.is_file(),
        "{name}: artifact containment/type"
    );
    check_artifact(&path, artifact, name);
    (path, sidecar)
}
