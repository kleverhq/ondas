//! Skip only the adapter's confirmed newer-DBG opening diagnostic.
use std::{fmt::Display, path::Path};

pub fn is_version_error(error: &impl Display) -> bool {
    let error = error.to_string();
    (error.starts_with("unsupported Wdb feature in wdb-lib during open: WDB DBG version ")
        || error.starts_with(
            "unsupported Wdb feature in wdb-lib during read metadata: WDB DBG version ",
        ))
        && error.contains(" is newer than Reader DBG ")
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
    fn confirmed_newer_dbg_is_skipped() {
        for operation in ["open", "read metadata"] {
            let newer = format!(
                "unsupported Wdb feature in wdb-lib during {operation}: WDB DBG version 006 is newer than Reader DBG 004"
            );
            assert!(super::open::<(), _>(Err(newer), super::Path::new("waveform.wdb")).is_none());
        }
        assert_eq!(
            super::open::<_, &str>(Ok(7), super::Path::new("waveform.wdb")),
            Some(7)
        );
    }

    #[test]
    fn other_errors_are_not_version_skips() {
        for error in [
            "backend wdb-lib failed during open: native reader exception",
            "unsupported Wdb feature in wdb-lib during open: unsupported WDB container version",
            "unsupported Wdb feature in wdb-lib during query: WDB DBG version 006 is newer than Reader DBG 004",
            "unsupported Wdb feature in other-reader during open: WDB DBG version 006 is newer than Reader DBG 004",
            "I/O error: file not found",
            "malformed WDB waveform: invalid version",
        ] {
            assert!(!super::is_version_error(&error), "{error}");
        }
    }

    #[test]
    #[should_panic(expected = "waveform.wdb: missing input")]
    fn generic_open_failure_still_fails() {
        super::open::<(), _>(Err("missing input"), super::Path::new("waveform.wdb"));
    }
}
