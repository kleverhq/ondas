//! Content identity of the bytes one checked GHW parse consumes.
//!
//! BLAKE3 is used because a warm open pays for exactly one pass over the whole
//! input, so the hash is the floor of every reused opening.
//!
//! A parsed result may be reused only for input whose complete content is
//! cryptographically identical to the content that an earlier checked parse
//! validated. The digest is therefore attached to the input before parsing and
//! covers the bytes the parser actually read, in order.
use std::io::{self, BufRead, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

use blake3::Hasher;

use crate::backends::Input;

/// Cryptographic identity of one complete input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ContentIdentity {
    pub(super) length: u64,
    pub(super) digest: [u8; 32],
}

/// Streams content of one input through BLAKE3 while it is read.
#[derive(Clone)]
pub(super) struct Identity(Arc<Mutex<State>>);

struct State {
    hasher: Hasher,
    /// Bytes hashed so far.
    covered: u64,
    /// False once a seek or a failed read broke the single forward pass.
    sequential: bool,
    /// Set by [`Identity::finish`], so later reads cannot restart the proof.
    finished: bool,
}

impl Identity {
    pub(super) fn new() -> Self {
        Self(Arc::new(Mutex::new(State {
            hasher: Hasher::new(),
            covered: 0,
            sequential: true,
            finished: false,
        })))
    }

    /// Wrap one input so every byte it yields is hashed in read order.
    pub(super) fn watch(&self, input: Box<dyn Input>) -> Box<dyn Input> {
        Box::new(Watched {
            input,
            identity: self.clone(),
        })
    }

    /// The identity of exactly the bytes the parser consumed, or `None` unless the
    /// whole input was read once, forward, without a gap or a seek.
    ///
    /// `length` is the input length that the parse itself used.
    pub(super) fn finish(&self, length: u64) -> Option<ContentIdentity> {
        let mut state = self.lock();
        if state.finished || !state.sequential || state.covered != length {
            return None;
        }
        state.finished = true;
        let digest = std::mem::take(&mut state.hasher).finalize();
        Some(ContentIdentity {
            length,
            digest: *digest.as_bytes(),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn update(&self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let mut state = self.lock();
        if state.finished || !state.sequential {
            return;
        }
        state.hasher.update(bytes);
        state.covered += bytes.len() as u64;
    }

    fn broke_sequence(&self) {
        let mut state = self.lock();
        state.sequential = false;
    }
}

/// Read one input once and rewind it, so a caller can compare the identity before
/// deciding to parse. An I/O failure is only a miss: the caller parses as usual and
/// reports whatever error the parser reaches.
pub(super) fn scan(input: &mut dyn Input) -> io::Result<ContentIdentity> {
    let mut hasher = Hasher::new();
    let mut length = 0u64;
    let mut buffer = [0; 64 * 1024];
    let result = loop {
        match input.read(&mut buffer) {
            Ok(0) => break Ok(()),
            Ok(read) => {
                hasher.update(&buffer[..read]);
                length += read as u64;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => break Err(error),
        }
    };
    // Rewind even after a failure, so the caller's own parse starts at the beginning.
    let rewind = input.seek(SeekFrom::Start(0));
    result?;
    rewind?;
    Ok(ContentIdentity {
        length,
        digest: *hasher.finalize().as_bytes(),
    })
}

struct Watched {
    input: Box<dyn Input>,
    identity: Identity,
}

impl Read for Watched {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let read = self.input.read(output)?;
        self.identity.update(&output[..read]);
        Ok(read)
    }
}

impl BufRead for Watched {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.input.fill_buf()
    }

    fn consume(&mut self, amount: usize) {
        // The inner window still holds the bytes about to be consumed, and a buffer near
        // exhaustion reports them in one call, so the digest is fed in bulk.
        match self.input.fill_buf() {
            Ok(bytes) => {
                let taken = amount.min(bytes.len());
                self.identity.update(&bytes[..taken]);
            }
            Err(_) => self.identity.broke_sequence(),
        }
        self.input.consume(amount);
    }
}

impl Seek for Watched {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        // Positioning before the first byte is how an opening finds the length.
        if self.identity.lock().covered > 0 {
            self.identity.broke_sequence();
        }
        self.input.seek(position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn watched(bytes: &[u8]) -> (Identity, Box<dyn Input>) {
        let identity = Identity::new();
        let input: Box<dyn Input> =
            Box::new(BufReader::with_capacity(4, Cursor::new(bytes.to_vec())));
        (identity.clone(), identity.watch(input))
    }

    #[test]
    fn identity_covers_the_bytes_a_reader_consumes() {
        let bytes = b"content over several small buffers";
        let (identity, mut input) = watched(bytes);
        let mut read = Vec::new();
        input.read_to_end(&mut read).unwrap();
        assert_eq!(read, bytes);
        let proven = identity.finish(bytes.len() as u64).unwrap();
        assert_eq!(proven.length, bytes.len() as u64);
        let mut hasher = Hasher::new();
        hasher.update(bytes);
        assert_eq!(proven.digest, *hasher.finalize().as_bytes());
    }

    #[test]
    fn partial_reads_and_seeks_cannot_complete_a_proof() {
        let bytes = b"only part of this content is consumed";
        let (identity, mut input) = watched(bytes);
        let mut part = [0; 8];
        input.read_exact(&mut part).unwrap();
        assert!(identity.finish(bytes.len() as u64).is_none());
        let (identity, mut input) = watched(bytes);
        input.seek(SeekFrom::End(0)).unwrap();
        input.seek(SeekFrom::Start(0)).unwrap();
        let mut all = Vec::new();
        input.read_to_end(&mut all).unwrap();
        let proven = identity.finish(bytes.len() as u64).unwrap();
        assert_eq!(proven.length, bytes.len() as u64);
        let (identity, mut input) = watched(bytes);
        let mut all = Vec::new();
        input.read_to_end(&mut all).unwrap();
        input.seek(SeekFrom::Start(0)).unwrap();
        assert!(identity.finish(bytes.len() as u64).is_none());
    }

    #[test]
    fn a_short_length_claim_is_not_a_proof() {
        let bytes = b"length is part of the identity";
        let (identity, mut input) = watched(bytes);
        let mut all = Vec::new();
        input.read_to_end(&mut all).unwrap();
        assert!(identity.finish(bytes.len() as u64 + 1).is_none());
        assert!(identity.finish(bytes.len() as u64 - 1).is_none());
    }

    #[test]
    fn scan_reports_content_and_rewinds() {
        let bytes = b"scan sees the whole input and leaves it at the start";
        let mut input: Box<dyn Input> =
            Box::new(BufReader::with_capacity(5, Cursor::new(bytes.to_vec())));
        let scanned = scan(&mut *input).unwrap();
        assert_eq!(scanned.length, bytes.len() as u64);
        let mut read = Vec::new();
        input.read_to_end(&mut read).unwrap();
        assert_eq!(read, bytes);
        let (identity, mut watched_input) = watched(bytes);
        let mut all = Vec::new();
        watched_input.read_to_end(&mut all).unwrap();
        assert_eq!(scanned, identity.finish(bytes.len() as u64).unwrap());
    }
}
