//! Skip only the shim's confirmed newer-file opening diagnostic.
//!
//! Keep this match aligned with `native/fsdb.cpp`, `src/backends/fsdb.rs` and
//! `src/error.rs`. Update it when their diagnostic changes.
use std::{fmt::Display, path::Path};

pub fn is_version_error(error: &impl Display) -> bool {
    let error = error.to_string();
    error.starts_with("backend fsdb-lib failed during read FSDB: FSDB file version ")
        && error.contains(" is newer than Reader API ")
}

pub fn open<T, E: Display>(result: Result<T, E>, path: &Path) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) if is_version_error(&error) => {
            eprintln!("SKIP {}: {error}", path.display());
            None
        }
        Err(error) => panic!("{}: {error}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn confirmed_newer_file_is_skipped() {
        use super::*;
        const NEWER: &str = "backend fsdb-lib failed during read FSDB: FSDB file version 2.1 is newer than Reader API 2.0";
        assert!(is_version_error(&NEWER));
        assert!(open::<(), _>(Err(NEWER), Path::new("waveform.fsdb")).is_none());
        assert_eq!(open::<_, &str>(Ok(7), Path::new("waveform.fsdb")), Some(7));
    }

    #[test]
    fn other_failures_are_not_version_skips() {
        use super::*;
        for error in [
            "backend fsdb-lib failed during read FSDB: FSDB Reader could not open the file (check file variant and SDK version)",
            "backend fsdb-lib failed during query: FSDB file version 2.1 is newer than Reader API 2.0",
            "backend another-reader failed during read FSDB: FSDB file version 2.1 is newer than Reader API 2.0",
            "I/O error: file not found",
            "malformed FSDB waveform: invalid version",
        ] {
            assert!(!is_version_error(&error), "{error}");
        }
    }

    #[test]
    #[should_panic(expected = "waveform.fsdb: missing input")]
    fn generic_open_failure_still_fails() {
        use super::*;
        open::<(), _>(Err("missing input"), Path::new("waveform.fsdb"));
    }
}
