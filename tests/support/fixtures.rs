//! Shared sidecar loading and artifact checks for tests and benchmarks.
use std::{
    collections::BTreeSet,
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
        matches!(
            extension,
            "fst" | "vcd" | "fsdb" | "ghw" | "wlf" | "shm" | "wdb"
        ),
        "{name}: invalid artifact format"
    );
    let filename = format!("waveform.{extension}");
    assert_eq!(artifact["file"], filename, "{name}: artifact filename");
    let path = directory.join(filename).canonicalize().unwrap();
    if extension == "shm" {
        assert!(
            path.starts_with(directory) && path.is_dir(),
            "{name}: SHM directory containment/type"
        );
        let components = artifact["files"]
            .as_array()
            .expect("SHM component manifest");
        assert!(!components.is_empty(), "{name}: empty SHM manifest");
        let mut names = BTreeSet::new();
        let mut total = 0_u64;
        for component in components {
            let object = component.as_object().expect("SHM component object");
            assert_eq!(object.len(), 3, "{name}: component fields");
            let filename = component["file"].as_str().unwrap();
            let relative = Path::new(filename);
            assert!(
                relative.file_name().and_then(|s| s.to_str()) == Some(filename)
                    && !filename.contains('\\')
                    && !matches!(filename, "." | "..")
                    && matches!(
                        relative.extension().and_then(|s| s.to_str()),
                        Some("dsn" | "trn")
                    ),
                "{name}: invalid SHM component name"
            );
            assert!(
                names.insert(filename.to_owned()),
                "{name}: duplicate SHM component"
            );
            let size = component["size"]
                .as_u64()
                .filter(|&size| size > 0)
                .expect("positive SHM component size");
            total = total.checked_add(size).expect("SHM size overflow");
            let component_path = path.join(filename).canonicalize().unwrap();
            assert!(
                component_path.starts_with(&path) && component_path.is_file(),
                "{name}: component containment/type"
            );
            check_artifact(&component_path, component, name);
        }
        let entries = fs::read_dir(&path)
            .unwrap()
            .map(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .into_string()
                    .expect("UTF-8 SHM filename")
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(entries, names, "{name}: SHM component set");
        assert_eq!(
            Some(total),
            artifact["size"].as_u64(),
            "{name}: SHM total size"
        );
        assert_eq!(
            manifest_sha256(components),
            artifact["sha256"].as_str().unwrap(),
            "{name}: SHM manifest hash"
        );
        return path;
    }
    assert!(
        path.starts_with(directory) && path.is_file(),
        "{name}: artifact containment/type"
    );
    check_artifact(&path, artifact, name);
    path
}

pub(crate) fn manifest_sha256(components: &[Json]) -> String {
    let mut components = components.to_vec();
    components.sort_by(|a, b| a["file"].as_str().unwrap().cmp(b["file"].as_str().unwrap()));
    // serde_json's default object map sorts keys. Python's ensure_ascii=True
    // additionally escapes non-ASCII characters, including UTF-16 surrogate pairs.
    let json = serde_json::to_string(&components).unwrap();
    let mut ascii = String::new();
    for character in json.chars() {
        if (' '..='~').contains(&character) {
            ascii.push(character);
        } else {
            use std::fmt::Write;
            for unit in character.encode_utf16(&mut [0; 2]) {
                write!(ascii, "\\u{unit:04x}").unwrap();
            }
        }
    }
    format!("{:x}", Sha256::digest(ascii.as_bytes()))
}

pub fn load_artifact(root: &Path, name: &str) -> (PathBuf, Json) {
    let (directory, sidecar) = read_sidecar(root, name);
    let path = verify_artifact(&directory, &sidecar["artifact"], name);
    (path, sidecar)
}
