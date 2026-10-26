mod container;
mod debug;
mod types;
mod wire;

use crate::{Error, Format};

fn malformed(message: impl Into<String>) -> Error {
    Error::Malformed {
        format: Format::Wdb,
        backend: "wdb-native".into(),
        message: message.into(),
    }
}

fn unsupported(message: impl Into<String>) -> Error {
    Error::UnsupportedFeature {
        format: Format::Wdb,
        backend: "wdb-native".into(),
        operation: "open",
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
