use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::SeekFrom,
    ops::ControlFlow,
    sync::Arc,
};

#[cfg(unix)]
use std::{
    cell::RefCell,
    io::{self, BufReader, Read, Seek},
    os::unix::fs::FileExt,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};

use super::Input;
use crate::{
    BitRange, BitsRef, Direction, Encoding, Error, Format, Hierarchy, Metadata, Result, Signal,
    Time, TimeSpan, TimeUnit, Timescale, ValueRef,
    hierarchy::{ScopeData, VariableData},
};

#[cfg(unix)]
use crate::{Sample, Value};

mod index;

// Backend budgets, not platform or resident-set limits. The transient records
// of one opening and the retained position lists are bounded separately, and
// the merge peak covers both. An input that does not fit keeps the
// chronological paths.
const INDEX_BYTES: usize = 512 * 1024 * 1024;
const BUILDER_BYTES: usize = 256 * 1024 * 1024;
const BUILD_PEAK_BYTES: usize = INDEX_BYTES + BUILDER_BYTES;
// One record decode may walk back over earlier records to prove `changed_at`.
// An unproven timestamp keeps the chronological path, never a guess.
const PROOF_STEPS: usize = 64;

fn malformed(message: impl Into<String>) -> Error {
    Error::Malformed {
        format: Format::Vcd,
        backend: "vcd-native".into(),
        message: message.into(),
    }
}

struct Tokens {
    input: Box<dyn Input>,
    offset: u64,
    #[cfg(test)]
    growths: usize,
}

impl Tokens {
    fn take(&mut self, whitespace: bool, mut output: Option<&mut Vec<u8>>) -> Result<()> {
        loop {
            let bytes = self.input.fill_buf()?;
            let n = bytes
                .iter()
                .take_while(|b| b.is_ascii_whitespace() == whitespace)
                .count();
            if let Some(output) = output.as_mut() {
                #[cfg(test)]
                if output.len() + n > output.capacity() {
                    self.growths += 1;
                }
                output.extend_from_slice(&bytes[..n]);
            }
            let done = n < bytes.len() || bytes.is_empty();
            self.input.consume(n);
            self.offset += n as u64;
            if done {
                return Ok(());
            }
        }
    }

    fn next_into(&mut self, word: &mut Vec<u8>) -> Result<bool> {
        word.clear();
        loop {
            let bytes = self.input.fill_buf()?;
            if bytes.is_empty() {
                return Ok(!word.is_empty());
            }
            let skip = if word.is_empty() {
                bytes.iter().take_while(|b| b.is_ascii_whitespace()).count()
            } else {
                0
            };
            let len = bytes[skip..]
                .iter()
                .take_while(|b| !b.is_ascii_whitespace())
                .count();
            #[cfg(test)]
            if word.len() + len > word.capacity() {
                self.growths += 1;
            }
            word.extend_from_slice(&bytes[skip..skip + len]);
            let consumed = skip + len;
            let done = consumed < bytes.len();
            self.input.consume(consumed);
            self.offset += consumed as u64;
            if done {
                return Ok(true);
            }
        }
    }

    fn next(&mut self) -> Result<Option<Vec<u8>>> {
        let mut word = Vec::new();
        Ok(self.next_into(&mut word)?.then_some(word))
    }

    fn required(&mut self) -> Result<Vec<u8>> {
        self.next()?.ok_or_else(|| self.error("unexpected EOF"))
    }

    fn end(&mut self) -> Result<()> {
        if self.required()? != b"$end" {
            return Err(self.error("expected $end"));
        }
        Ok(())
    }

    fn text(&mut self) -> Result<String> {
        let mut text = Vec::new();
        loop {
            self.take(true, Some(&mut text))?;
            let word = self.required()?;
            if word == b"$end" {
                break;
            }
            text.extend(word);
        }
        utf8(text.trim_ascii())
    }

    fn error(&self, message: &str) -> Error {
        malformed(format!("byte {}: {message}", self.offset))
    }
}

fn utf8(bytes: &[u8]) -> Result<String> {
    String::from_utf8(bytes.to_vec())
        .map_err(|_| malformed("invalid UTF-8 declaration or metadata"))
}

// Printable identifier codes are reversible base-94 numbers, with the first byte least significant.
fn identifier_number(id: &[u8]) -> Option<usize> {
    let mut number = 0_usize;
    for &byte in id.iter().rev() {
        if !(33..=126).contains(&byte) {
            return None;
        }
        number = number
            .checked_mul(94)?
            .checked_add(usize::from(byte - 32))?;
    }
    Some(number)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Position {
    offset: u64,
    pub(crate) time: u64,
    block: bool,
}

pub(crate) struct Reader {
    tokens: Tokens,
    position: Option<Position>,
    #[cfg(test)]
    pub(crate) records_read: usize,
    body: u64,
    ids: Arc<HashMap<Vec<u8>, usize>>,
    dense_ids: Arc<Vec<Option<usize>>>,
    encodings: Vec<Encoding>,
    index: Option<index::Index>,
    #[cfg(all(test, unix))]
    pub(crate) index_queries: AtomicUsize,
    #[cfg(all(test, unix))]
    pub(crate) traversal_queries: AtomicUsize,
    #[cfg(all(test, unix))]
    pub(crate) traversal_records: AtomicUsize,
    #[cfg(unix)]
    parallel: Option<ParallelSource>,
}

#[cfg(unix)]
struct ParallelSource {
    file: Arc<File>,
    cuts: Vec<usize>,
    last_time: u64,
}

#[cfg(unix)]
struct ChunkValue {
    first: Option<Value>,
    first_time: Time,
    last: Value,
    last_change: Option<Time>,
    tick: Time,
    pending: Value,
}

#[cfg(unix)]
impl ChunkValue {
    fn record(&mut self, time: Time, value: Value) {
        if time != self.tick {
            self.finish_tick();
            self.tick = time;
        }
        self.pending = value;
    }

    fn finish_tick(&mut self) {
        if self.first.is_none() {
            self.first = Some(self.pending.clone());
            self.last = self.pending.clone();
        } else if !self.last.as_ref().same_value(self.pending.as_ref()) {
            self.last = self.pending.clone();
            self.last_change = Some(self.tick);
        }
    }
}

#[cfg(unix)]
struct ChunkSamples {
    span: Option<TimeSpan>,
    values: HashMap<usize, ChunkValue>,
    position: Position,
}

#[cfg(unix)]
enum FullValue {
    Bit(u8),
    Other(Value),
}

#[cfg(unix)]
impl FullValue {
    fn from_ref(value: ValueRef<'_>) -> Self {
        match value {
            ValueRef::Bits(bits) if bits.width() == 1 => {
                Self::Bit(bits.ascii()[0].to_ascii_lowercase())
            }
            _ => Self::Other(value.to_owned()),
        }
    }

    fn as_ref(&self) -> ValueRef<'_> {
        match self {
            Self::Bit(byte) => {
                ValueRef::Bits(BitsRef::from_validated_ascii(std::slice::from_ref(byte)))
            }
            Self::Other(value) => value.as_ref(),
        }
    }
}

#[cfg(unix)]
struct FullRecord {
    index: usize,
    time: Time,
    value: FullValue,
    position: Position,
}

#[cfg(unix)]
fn flush_startup(first_time: &mut Option<Time>, first_transition: &mut bool, time: Time) -> bool {
    let first_record = first_time.is_none();
    let next_tick = !*first_transition && first_time.is_some_and(|first| first != time);
    first_time.get_or_insert(time);
    *first_transition |= next_tick;
    first_record || next_tick
}

#[cfg(unix)]
struct RangeReader {
    file: Arc<File>,
    start: u64,
    len: u64,
    cursor: u64,
    canceled: Option<Arc<AtomicBool>>,
}

#[cfg(unix)]
impl Read for RangeReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self
            .canceled
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            return Err(io::Error::other("VCD query canceled"));
        }
        let remaining = self.len - self.cursor;
        let count = bytes.len().min(remaining as usize);
        let read = self
            .file
            .read_at(&mut bytes[..count], self.start + self.cursor)?;
        self.cursor += read as u64;
        Ok(read)
    }
}

#[cfg(unix)]
impl Seek for RangeReader {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let next = match position {
            SeekFrom::Start(value) => i128::from(value),
            SeekFrom::Current(value) => i128::from(self.cursor) + i128::from(value),
            SeekFrom::End(value) => i128::from(self.len) + i128::from(value),
        };
        if !(0..=i128::from(self.len)).contains(&next) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek outside VCD chunk",
            ));
        }
        self.cursor = next as u64;
        Ok(self.cursor)
    }
}

/// Opening-only sinks: collected comments and optional position records.
struct OpeningSinks<'a> {
    comments: Option<&'a mut Vec<String>>,
    records: Option<&'a mut index::Builder>,
}

type Opening = (
    Option<TimeSpan>,
    Vec<String>,
    Vec<Encoding>,
    Option<index::Index>,
);

#[cfg(unix)]
struct OpeningChunk {
    span: Option<TimeSpan>,
    seen: Vec<bool>,
    encodings: Vec<Encoding>,
    comments: Vec<String>,
    records: Option<index::Builder>,
}

impl Reader {
    pub(crate) fn open(
        input: Box<dyn Input>,
        source_name: String,
        file: Option<File>,
    ) -> Result<(Self, Hierarchy, Metadata)> {
        Self::open_inner(input, source_name, file, true)
    }

    /// Validates the same body without retaining a query position index.
    pub(crate) fn open_metadata(
        input: Box<dyn Input>,
        source_name: String,
        file: Option<File>,
    ) -> Result<(Self, Hierarchy, Metadata)> {
        Self::open_inner(input, source_name, file, false)
    }

    fn open_inner(
        input: Box<dyn Input>,
        source_name: String,
        file: Option<File>,
        query: bool,
    ) -> Result<(Self, Hierarchy, Metadata)> {
        let mut tokens = Tokens {
            input,
            offset: 0,
            #[cfg(test)]
            growths: 0,
        };
        let mut metadata = Metadata {
            source_name,
            timescale: None,
            time_span: None,
            writer: None,
            date: None,
            comments: Vec::new(),
        };
        let mut scopes = Vec::<ScopeData>::new();
        let mut variables = Vec::new();
        let mut stack = Vec::new();
        let mut scope_ids = HashMap::<_, usize>::new();
        let mut ids = HashMap::new();
        let mut encodings = Vec::new();
        let mut declarations = HashSet::new();
        let body = loop {
            let start = tokens.offset;
            let mut command = tokens.required()?;
            if start == 0 && command.starts_with(b"\xef\xbb\xbf") {
                command.drain(..3);
                if command.is_empty() {
                    command = tokens.required()?;
                }
            }
            match command.as_slice() {
                b"$date" => metadata.date = Some(tokens.text()?),
                b"$version" => metadata.writer = Some(tokens.text()?),
                b"$comment" => metadata.comments.push(tokens.text()?),
                b"$timescale" => metadata.timescale = Some(timescale(&tokens.text()?)?),
                // Known producer annotations do not define value storage.
                b"$attrbegin" | b"$attrend" => {
                    tokens.text()?;
                }
                b"$timezero" => {
                    if integer(tokens.text()?.as_bytes())? != 0 {
                        return Err(tokens.error("nonzero timezero is unsupported"));
                    }
                }
                // Migen emits a complete declaration list without enddefinitions.
                b"$dumpvars" if stack.is_empty() => {
                    tokens.input.seek(SeekFrom::Start(start))?;
                    tokens.offset = start;
                    break start;
                }
                b"$scope" => {
                    let kind = scope_kind(&utf8(&tokens.required()?)?)?;
                    let spelling = tokens.required()?;
                    let name_was_escaped = spelling.starts_with(b"\\");
                    let name = name(&spelling)?;
                    tokens.end()?;
                    let parent = stack.last().copied();
                    // Coalesce repeated source spellings, not equal logical paths.
                    let key = (parent, spelling);
                    let index = if let Some(&index) = scope_ids.get(&key) {
                        if scopes[index].kind != kind {
                            return Err(tokens.error("conflicting scope kind"));
                        }
                        index
                    } else {
                        let index = scopes.len();
                        scopes.push(ScopeData {
                            name,
                            name_was_escaped,
                            parent,
                            kind,
                            definition_name: None,
                            packing: None,
                            is_hidden: false,
                        });
                        scope_ids.insert(key, index);
                        index
                    };
                    stack.push(index);
                }
                b"$upscope" => {
                    tokens.end()?;
                    stack
                        .pop()
                        .ok_or_else(|| tokens.error("scope stack underflow"))?;
                }
                b"$var" => {
                    let kind = utf8(&tokens.required()?)?;
                    let width_token = tokens.required()?;
                    if !width_token.iter().all(u8::is_ascii_digit) {
                        return Err(tokens.error("invalid width"));
                    }
                    let width = utf8(&width_token)?
                        .parse::<u32>()
                        .map_err(|_| tokens.error("invalid width"))?;
                    let id = tokens.required()?;
                    if !id.iter().all(|b| (33..=126).contains(b)) {
                        return Err(tokens.error("invalid identifier code"));
                    }
                    let reference = tokens.required()?;
                    let mut suffix = Vec::new();
                    loop {
                        let word = tokens.required()?;
                        if word == b"$end" {
                            break;
                        }
                        suffix.extend(word);
                    }
                    let encoding = match kind.as_str() {
                        "event" => Encoding::Event,
                        "real" | "realtime" | "shortreal" | "real_parameter" => Encoding::Real,
                        "string" => Encoding::String,
                        "wire" | "reg" | "parameter" | "integer" | "time" | "supply0"
                        | "supply1" | "tri" | "triand" | "trior" | "trireg" | "tri0" | "tri1"
                        | "wand" | "wor" | "bit" | "logic" | "int" | "shortint" | "longint"
                        | "byte" | "enum" | "port" => {
                            if width == 0 {
                                Encoding::Unsupported
                            } else {
                                Encoding::Bits { width }
                            }
                        }
                        _ => return Err(tokens.error("unsupported variable kind")),
                    };
                    let index = if let Some(&index) = ids.get(&id) {
                        if encodings[index] != encoding {
                            return Err(tokens.error("incompatible aliases"));
                        }
                        index
                    } else {
                        let index = encodings.len();
                        encodings.push(encoding);
                        ids.insert(id, index);
                        index
                    };
                    let (name, range) = reference_name(&reference, &suffix, encoding)?;
                    let name_was_escaped = reference.starts_with(b"\\");
                    let parent = stack.last().copied();
                    let kind = kind.replace('_', "-");
                    if declarations.insert((
                        parent,
                        reference,
                        kind.clone(),
                        index,
                        range.map(|r| (r.msb(), r.lsb())),
                    )) {
                        variables.push(VariableData {
                            name,
                            reader_name: None,
                            name_was_escaped,
                            parent,
                            is_constant: matches!(kind.as_str(), "parameter" | "real-parameter"),
                            kind,
                            direction: Direction::Implicit,
                            range,
                            type_name: None,
                            signedness: None,
                            logic_domain: None,
                            enumeration: None,
                            signal: Some(index),
                        });
                    }
                }
                b"$enddefinitions" => {
                    tokens.end()?;
                    break tokens.offset;
                }
                _ => return Err(tokens.error("unsupported header command")),
            }
        };
        if !stack.is_empty() {
            return Err(tokens.error("unclosed scopes"));
        }
        if variables.is_empty() {
            return Err(tokens.error("no variable declarations"));
        }
        // Arbitrary sparse codes still use the map; bound the direct table by the declared IDs.
        let mut dense_ids = Vec::new();
        let max_id = ids.keys().try_fold(0, |max, id| {
            identifier_number(id).map(|number| max.max(number))
        });
        if let Some(max_id) = max_id
            && max_id < ids.len().saturating_mul(2)
        {
            dense_ids.resize(max_id + 1, None);
            for (id, &index) in &ids {
                dense_ids[identifier_number(id).unwrap()] = Some(index);
            }
        }
        let mut reader = Self {
            tokens,
            position: None,
            #[cfg(test)]
            records_read: 0,
            body,
            ids: Arc::new(ids),
            dense_ids: Arc::new(dense_ids),
            encodings,
            index: None,
            #[cfg(all(test, unix))]
            index_queries: AtomicUsize::new(0),
            #[cfg(all(test, unix))]
            traversal_queries: AtomicUsize::new(0),
            #[cfg(all(test, unix))]
            traversal_records: AtomicUsize::new(0),
            #[cfg(unix)]
            parallel: None,
        };
        // Only the parallel opening path consumes the position index, so a
        // serial opening retains none.
        metadata.time_span = if let Some((span, comments, encodings, index)) =
            file.and_then(|file| reader.parallel_open(file, query))
        {
            metadata.comments.extend(comments);
            reader.encodings = encodings;
            reader.index = index;
            span
        } else {
            let (_, span, _) = reader.walk(
                u64::MAX,
                None,
                OpeningSinks {
                    comments: Some(&mut metadata.comments),
                    records: None,
                },
                |_, _, _, _| ControlFlow::<()>::Continue(()),
            )?;
            span
        };
        let hierarchy = Hierarchy::new(scopes, variables, reader.encodings.clone());
        Ok((reader, hierarchy, metadata))
    }

    #[cfg(unix)]
    fn parallel_open(&mut self, file: File, collect: bool) -> Option<Opening> {
        let end = usize::try_from(file.metadata().ok()?.len()).ok()?;
        let start = usize::try_from(self.body).ok()?;
        let length = end.checked_sub(start)?;
        let workers = thread::available_parallelism()
            .ok()?
            .get()
            .min(length / (8 * 1024 * 1024));
        if workers < 2 {
            return None;
        }
        let mut cuts = vec![start];
        let mut probe = vec![0; 1024 * 1024];
        for index in 1..workers {
            let target = start + (length / workers) * index;
            let limit = (end - target).min(probe.len());
            let count = file.read_at(&mut probe[..limit], target as u64).ok()?;
            let newline = probe[..count].windows(2).position(|pair| pair == b"\n#")?;
            let cut = target + newline + 1;
            if cut <= *cuts.last().unwrap() {
                return None;
            }
            cuts.push(cut);
        }
        cuts.push(end);
        let result = self.parallel_open_with_cuts(file.try_clone().ok()?, &cuts, collect)?;
        self.parallel = Some(ParallelSource {
            file: Arc::new(file),
            cuts,
            last_time: result.0.map_or(0, |span| span.last().ticks()),
        });
        Some(result)
    }

    #[cfg(unix)]
    fn parallel_open_with_cuts(
        &self,
        file: File,
        cuts: &[usize],
        collect: bool,
    ) -> Option<Opening> {
        let file = Arc::new(file);
        let workers = cuts.len().saturating_sub(1).max(1);
        let parts = thread::scope(|scope| {
            let handles = cuts
                .windows(2)
                .map(|range| {
                    let ids = self.ids.clone();
                    let dense_ids = self.dense_ids.clone();
                    let encodings = self.encodings.clone();
                    let body = self.body;
                    let input = BufReader::with_capacity(
                        256 * 1024,
                        RangeReader {
                            file: Arc::clone(&file),
                            start: range[0] as u64,
                            len: (range[1] - range[0]) as u64,
                            cursor: 0,
                            canceled: None,
                        },
                    );
                    let offset = range[0] as u64;
                    thread::Builder::new().spawn_scoped(scope, move || {
                        let mut reader = Self {
                            tokens: Tokens {
                                input: Box::new(input),
                                offset,
                                #[cfg(test)]
                                growths: 0,
                            },
                            position: None,
                            #[cfg(test)]
                            records_read: 0,
                            body,
                            ids,
                            dense_ids,
                            encodings,
                            index: None,
                            #[cfg(all(test, unix))]
                            index_queries: AtomicUsize::new(0),
                            #[cfg(all(test, unix))]
                            traversal_queries: AtomicUsize::new(0),
                            #[cfg(all(test, unix))]
                            traversal_records: AtomicUsize::new(0),
                            parallel: None,
                        };
                        let mut comments = Vec::new();
                        let mut records =
                            collect.then(|| index::Builder::new(BUILDER_BYTES / workers));
                        let (_, span, seen) = reader.walk(
                            u64::MAX,
                            None,
                            OpeningSinks {
                                comments: Some(&mut comments),
                                records: records.as_mut(),
                            },
                            |_, _, _, _| ControlFlow::<()>::Continue(()),
                        )?;
                        Ok::<_, Error>(OpeningChunk {
                            span,
                            seen,
                            encodings: reader.encodings,
                            comments,
                            records,
                        })
                    })
                })
                .collect::<io::Result<Vec<_>>>()
                .ok()?;
            Some(
                handles
                    .into_iter()
                    .map(|handle| handle.join().expect("VCD opening worker panicked"))
                    .collect::<Vec<_>>(),
            )
        })?;
        let mut span: Option<TimeSpan> = None;
        let mut comments = Vec::new();
        let mut records = Vec::with_capacity(parts.len());
        let mut saw_real = vec![false; self.encodings.len()];
        let mut saw_string = vec![false; self.encodings.len()];
        for (index, part) in parts.into_iter().enumerate() {
            let part = part.ok()?;
            if index > 0 && part.span.is_none() {
                return None;
            }
            if let Some(next) = part.span {
                if span.is_some_and(|previous| previous.last() > next.first()) {
                    return None;
                }
                span = Some(TimeSpan::new(
                    span.map_or(next.first(), |previous| previous.first()),
                    next.last(),
                ));
            }
            for (index, &seen) in part.seen.iter().enumerate() {
                if seen && self.encodings[index] == Encoding::Real {
                    saw_real[index] |= part.encodings[index] == Encoding::Real;
                    saw_string[index] |= part.encodings[index] == Encoding::String;
                }
            }
            comments.extend(part.comments);
            records.push(part.records);
        }
        if saw_real
            .iter()
            .zip(&saw_string)
            .any(|(real, string)| *real && *string)
        {
            return None;
        }
        let mut encodings = self.encodings.clone();
        for (index, &string) in saw_string.iter().enumerate() {
            if string {
                encodings[index] = Encoding::String;
            }
        }
        let index = if collect {
            let records = records.into_iter().collect::<Option<Vec<_>>>()?;
            index::Index::build(records, encodings.len(), INDEX_BYTES, BUILD_PEAK_BYTES)
        } else {
            None
        };
        Some((span, comments, encodings, index))
    }

    #[cfg(unix)]
    pub(crate) fn parallel_prefix(
        &self,
        signals: &[Signal],
        end: Time,
    ) -> Option<(Vec<Sample>, Position)> {
        let source = self.parallel.as_ref()?;
        if signals.is_empty() || end == Time::ZERO {
            return None;
        }
        if let Some(prefix) = self.indexed_prefix(source, signals, end) {
            return Some(prefix);
        }
        if end.ticks() < source.last_time.div_ceil(8) {
            return None;
        }
        // A chunk keeps up to three owned values per signal. Account for the
        // parser's current value and a replacement copy before spawning workers.
        // Variable-length strings cannot be bounded from their declarations.
        let mut selected = HashSet::new();
        let mut bytes = 0_usize;
        for signal in signals {
            if signal.is_slice() {
                return None;
            }
            if selected.insert(signal.index()) {
                let width = match signal.encoding() {
                    Encoding::Bits { width } => width as usize,
                    Encoding::Real => std::mem::size_of::<f64>(),
                    _ => return None,
                };
                bytes = bytes.checked_add(
                    width
                        .checked_mul(5)?
                        .checked_add(std::mem::size_of::<ChunkValue>() + 64)?,
                )?;
            }
        }
        if bytes.checked_mul(source.cuts.len() - 1)? > 4 * 1024 * 1024 {
            return None;
        }
        let selected = Arc::new(selected);
        let file = Arc::clone(&source.file);
        let body = self.body;
        let chunks = thread::scope(|scope| {
            let handles = source
                .cuts
                .windows(2)
                .map(|range| {
                    let input = BufReader::with_capacity(
                        256 * 1024,
                        RangeReader {
                            file: Arc::clone(&file),
                            start: range[0] as u64,
                            len: (range[1] - range[0]) as u64,
                            cursor: 0,
                            canceled: None,
                        },
                    );
                    let offset = range[0] as u64;
                    let ids = Arc::clone(&self.ids);
                    let dense_ids = Arc::clone(&self.dense_ids);
                    let encodings = self.encodings.clone();
                    let selected = Arc::clone(&selected);
                    thread::Builder::new().spawn_scoped(scope, move || {
                        let mut reader = Self {
                            tokens: Tokens {
                                input: Box::new(input),
                                offset,
                                #[cfg(test)]
                                growths: 0,
                            },
                            position: None,
                            #[cfg(test)]
                            records_read: 0,
                            body,
                            ids,
                            dense_ids,
                            encodings,
                            index: None,
                            #[cfg(all(test, unix))]
                            index_queries: AtomicUsize::new(0),
                            #[cfg(all(test, unix))]
                            traversal_queries: AtomicUsize::new(0),
                            #[cfg(all(test, unix))]
                            traversal_records: AtomicUsize::new(0),
                            parallel: None,
                        };
                        let mut values = HashMap::<usize, ChunkValue>::new();
                        let (_, span, _) = reader.walk(
                            end.ticks(),
                            Some(&selected),
                            OpeningSinks {
                                comments: None,
                                records: None,
                            },
                            |index, time, value, _| {
                                let value = value.to_owned();
                                if let Some(previous) = values.get_mut(&index) {
                                    previous.record(time, value);
                                } else {
                                    values.insert(
                                        index,
                                        ChunkValue {
                                            first: None,
                                            first_time: time,
                                            last: value.clone(),
                                            last_change: None,
                                            tick: time,
                                            pending: value,
                                        },
                                    );
                                }
                                ControlFlow::<()>::Continue(())
                            },
                        )?;
                        for value in values.values_mut() {
                            value.finish_tick();
                        }
                        Ok::<_, Error>(ChunkSamples {
                            span,
                            values,
                            position: reader.position.unwrap(),
                        })
                    })
                })
                .collect::<io::Result<Vec<_>>>()
                .ok()?;
            Some(
                handles
                    .into_iter()
                    .map(|handle| handle.join().expect("VCD sample worker panicked"))
                    .collect::<Vec<_>>(),
            )
        })?;
        let mut previous = None;
        let mut resume = None;
        let mut last_position = None;
        let mut values = HashMap::<usize, (Value, Option<Time>)>::new();
        for (chunk, &boundary) in chunks.iter().zip(source.cuts.iter().skip(1)) {
            let chunk = chunk.as_ref().ok()?;
            last_position = Some(chunk.position);
            if resume.is_none() && chunk.position.offset < boundary as u64 {
                resume = Some(chunk.position);
            }
            if let Some(span) = chunk.span {
                // A cut through the same tick needs serial normalization.
                if previous.is_some_and(|time| time >= span.first()) {
                    return None;
                }
                previous = Some(span.last());
            }
            for (&index, part) in &chunk.values {
                let first = part.first.as_ref()?;
                let change = if let Some((last, changed_at)) = values.get(&index) {
                    part.last_change.or_else(|| {
                        if last.as_ref().same_value(first.as_ref()) {
                            *changed_at
                        } else {
                            Some(part.first_time)
                        }
                    })
                } else {
                    part.last_change
                };
                values.insert(index, (part.last.clone(), change));
            }
        }
        Some((
            signals
                .iter()
                .map(|&signal| {
                    if let Some((value, changed_at)) = values.get(&signal.index()) {
                        Sample::Value {
                            signal,
                            value: value.clone(),
                            changed_at: *changed_at,
                        }
                    } else {
                        Sample::Missing { signal }
                    }
                })
                .collect(),
            resume.or(last_position)?,
        ))
    }

    /// Answers an entering state and resume position from validated record positions.
    ///
    /// Every value is decoded from its recorded source offset and every
    /// `changed_at` is proven from preceding records. A missing index, an
    /// unsupported encoding, an unreadable position or an unproven timestamp
    /// returns `None`, so the caller keeps the chronological paths. The index
    /// is a position map, not a value history.
    #[cfg(unix)]
    fn indexed_prefix(
        &self,
        source: &ParallelSource,
        signals: &[Signal],
        end: Time,
    ) -> Option<(Vec<Sample>, Position)> {
        let index = self.index.as_ref()?;
        let file = source.file.as_ref();
        let file_len = file.metadata().ok()?.len();
        // The same bound as the chunk path: declared widths and one replacement
        // copy. Refuse before decoding any value.
        let mut selected = HashSet::new();
        let mut bytes = 0_usize;
        for signal in signals {
            if signal.is_slice() {
                return None;
            }
            if selected.insert(signal.index()) {
                let width = match signal.encoding() {
                    Encoding::Bits { width } => width as usize,
                    Encoding::Real => std::mem::size_of::<f64>(),
                    _ => return None,
                };
                bytes = bytes.checked_add(
                    width
                        .checked_mul(5)?
                        .checked_add(std::mem::size_of::<ChunkValue>() + 64)?,
                )?;
            }
        }
        if bytes > 4 * 1024 * 1024 {
            return None;
        }
        let mut lookup = Lookup {
            index,
            file,
            file_len,
            scratch: Scratch::default(),
        };
        let mut samples = Vec::with_capacity(signals.len());
        for &signal in signals {
            let signal_index = signal.index();
            let Some(at) = index.last_at_or_before(signal_index, end.ticks()) else {
                samples.push(Sample::Missing { signal });
                continue;
            };
            let offset = index.entries(signal_index)[at].offset;
            let value = self.indexed_value(&mut lookup, offset, signal_index)?;
            let changed_at = self.indexed_changed_at(&mut lookup, signal_index, at, &value)?;
            samples.push(Sample::Value {
                signal,
                value,
                changed_at,
            });
        }
        // Resume at the first selected record after `end`. Starting there keeps
        // that record's tick, so a later `#time` still stops the walk. A block
        // snapshot position keeps the chronological path.
        let mut resume = None;
        for &signal in signals {
            let signal_index = signal.index();
            let list = index.entries(signal_index);
            let Some(entry) = list.get(index.first_after(signal_index, end.ticks())) else {
                continue;
            };
            if resume.is_none_or(|(offset, _)| entry.offset < offset) {
                resume = Some((entry.offset, entry.time));
            }
        }
        let position = match resume {
            Some((offset, time)) if !index.in_block(offset) => Position {
                offset,
                time,
                block: false,
            },
            Some(_) => return None,
            None => Position {
                offset: file_len,
                time: end.ticks(),
                block: false,
            },
        };
        #[cfg(all(test, unix))]
        self.index_queries.fetch_add(1, Ordering::Relaxed);
        Some((samples, position))
    }

    /// Reads selected records in source order from validated positions.
    ///
    /// The byte lower bound, not the time, is the consumed-record boundary: a
    /// checkpoint can sit inside a tick. The sparse-selection estimate and all
    /// encoding checks happen before the first callback; a dense or unsupported
    /// request returns `None` and keeps the walk. A failure after delivery is an
    /// error, never a restart.
    #[cfg(unix)]
    fn indexed_traversal<B>(
        &mut self,
        signals: &[Signal],
        end: Time,
        lower: u64,
        visitor: &mut impl for<'v> FnMut(usize, Time, ValueRef<'v>, Position) -> ControlFlow<B>,
    ) -> Option<Result<ControlFlow<B>>> {
        use std::{cmp::Reverse, collections::BinaryHeap};

        const WORD_LIMIT: usize = 64 * 1024;
        // One merged record is estimated as no cheaper than this much
        // sequential window. It is a backend estimate, not a platform limit.
        const MERGE_BYTES: usize = 4096;
        let index = self.index.as_ref()?;
        let source = self.parallel.as_ref()?;
        let file = source.file.as_ref();
        let file_len = file.metadata().ok()?.len();
        let mut lookup = Lookup {
            index,
            file,
            file_len,
            scratch: Scratch::default(),
        };
        let mut selected = Vec::new();
        let mut seen = HashSet::new();
        for &signal in signals {
            let signal_index = signal.index();
            if !seen.insert(signal_index) {
                continue;
            }
            match self.encodings[signal_index] {
                Encoding::Bits { width } if width as usize + 2 < WORD_LIMIT => {}
                Encoding::Real | Encoding::Event => {}
                Encoding::Bits { .. } | Encoding::String | Encoding::Unsupported => return None,
            }
            selected.push(signal_index);
        }
        if selected.is_empty() {
            return None;
        }
        let mut streams = Vec::with_capacity(selected.len());
        let mut records = 0_usize;
        let mut last = lower;
        for &signal_index in &selected {
            let list = index.entries(signal_index);
            let start = list.partition_point(|entry| entry.offset < lower);
            let end_at = list.partition_point(|entry| entry.time <= end.ticks());
            let stop = end_at.max(start);
            if stop > start {
                records = records.checked_add(stop - start)?;
                last = last.max(list[stop - 1].offset);
            }
            streams.push((signal_index, start, stop));
        }
        if records == 0 {
            return Some(Ok(ControlFlow::Continue(())));
        }
        if (records as u64).saturating_mul(MERGE_BYTES as u64) > last.saturating_sub(lower) {
            return None;
        }
        #[cfg(all(test, unix))]
        {
            self.traversal_queries.fetch_add(1, Ordering::Relaxed);
            self.traversal_records.fetch_add(records, Ordering::Relaxed);
        }
        let mut heap = BinaryHeap::new();
        let mut cursors = Vec::with_capacity(streams.len());
        for (stream, &(signal_index, start, stop)) in streams.iter().enumerate() {
            cursors.push(start);
            if start < stop {
                let entry = index.entries(signal_index)[start];
                heap.push(Reverse((entry.time, entry.offset, stream)));
            }
        }
        let mut delivered = false;
        while let Some(Reverse((time, offset, stream))) = heap.pop() {
            let signal_index = streams[stream].0;
            let value = match self.indexed_value(&mut lookup, offset, signal_index) {
                Some(value) => value,
                None if delivered => {
                    return Some(Err(malformed("indexed record cannot be decoded")));
                }
                None => return None,
            };
            delivered = true;
            if let ControlFlow::Break(value) = visitor(
                signal_index,
                Time::from_ticks(time),
                value.as_ref(),
                Position {
                    offset,
                    time,
                    block: index.in_block(offset),
                },
            ) {
                return Some(Ok(ControlFlow::Break(value)));
            }
            cursors[stream] += 1;
            if cursors[stream] < streams[stream].2 {
                let entry = index.entries(signal_index)[cursors[stream]];
                heap.push(Reverse((entry.time, entry.offset, stream)));
            }
        }
        Some(Ok(ControlFlow::Continue(())))
    }

    /// Decodes one record at a validated source offset.
    #[cfg(unix)]
    fn indexed_value(
        &self,
        lookup: &mut Lookup<'_>,
        offset: u64,
        expected: usize,
    ) -> Option<Value> {
        let next = read_word(
            lookup.file,
            lookup.file_len,
            offset,
            &mut lookup.scratch.word,
            &mut lookup.scratch.buffer,
        )?;
        let prefix = lookup.scratch.word.first()?.to_ascii_lowercase();
        let (payload, id): (&[u8], &[u8]) = if matches!(prefix, b'b' | b'r' | b's') {
            read_word(
                lookup.file,
                lookup.file_len,
                next,
                &mut lookup.scratch.id,
                &mut lookup.scratch.buffer,
            )?;
            (&lookup.scratch.word[1..], lookup.scratch.id.as_slice())
        } else {
            (&lookup.scratch.word[..1], &lookup.scratch.word[1..])
        };
        let index = if self.dense_ids.is_empty() {
            self.ids.get(id).copied()
        } else {
            identifier_number(id)
                .and_then(|number| self.dense_ids.get(number).copied().flatten())
                .or_else(|| self.ids.get(id).copied())
        }?;
        if index != expected {
            return None;
        }
        match (self.encodings[index], prefix) {
            (Encoding::Real, b'r') => Some(Value::Real(real(payload).ok()?)),
            (Encoding::Event, _) if !matches!(prefix, b'r' | b's') => {
                if payload.is_empty() || BitsRef::from_ascii(payload).is_none() {
                    return None;
                }
                Some(Value::Event { occurrences: 1 })
            }
            (Encoding::Bits { width }, _) if !matches!(prefix, b'r' | b's') => {
                if payload.is_empty() || BitsRef::from_ascii(payload).is_none() {
                    return None;
                }
                let width = width as usize;
                let fill = match payload[0].to_ascii_lowercase() {
                    b'0' | b'1' => b'0',
                    other => other,
                };
                let mut bits = vec![fill; width.saturating_sub(payload.len())];
                bits.extend_from_slice(&payload[payload.len().saturating_sub(width)..]);
                Some(Value::Bits(crate::Bits::from_validated_ascii(bits)))
            }
            _ => None,
        }
    }

    /// Proves the tick that established the current value.
    ///
    /// The search compares tick-final values, so an excursion that returns to
    /// the entering value does not advance `changed_at`. `Some(None)` means no
    /// preceding observation proves an earlier state. `None` means the bounded
    /// proof ran out; the caller must keep the chronological attribution.
    #[cfg(unix)]
    fn indexed_changed_at(
        &self,
        lookup: &mut Lookup<'_>,
        signal: usize,
        mut at: usize,
        value: &Value,
    ) -> Option<Option<Time>> {
        for _ in 0..PROOF_STEPS {
            let time = lookup.index.entries(signal)[at].time;
            let before =
                lookup.index.entries(signal)[..at].partition_point(|entry| entry.time < time);
            let Some(previous) = before.checked_sub(1) else {
                return Some(None);
            };
            let offset = lookup.index.entries(signal)[previous].offset;
            let earlier = self.indexed_value(lookup, offset, signal)?;
            if earlier.as_ref().same_value(value.as_ref()) {
                at = previous;
                continue;
            }
            return Some(Some(Time::from_ticks(time)));
        }
        None
    }

    #[cfg(all(test, unix))]
    pub(crate) fn disable_index(&mut self) {
        self.index = None;
    }

    #[cfg(all(test, unix))]
    pub(crate) fn has_index(&self) -> bool {
        self.index.is_some()
    }

    #[cfg(all(test, unix))]
    pub(crate) fn force_parallel_for_test(
        &mut self,
        file: File,
        mut cuts: Vec<usize>,
        last_time: u64,
    ) {
        // A serial test opening has no index. Build the one a parallel opening
        // would retain so the reached-path tests use the production lookup.
        if self.index.is_none() {
            self.rebuild_index_for_test();
        }
        cuts.insert(0, self.body as usize);
        self.parallel = Some(ParallelSource {
            file: Arc::new(file),
            cuts,
            last_time,
        });
    }

    #[cfg(all(test, unix))]
    fn rebuild_index_for_test(&mut self) {
        let start = self.body;
        if self.tokens.input.seek(SeekFrom::Start(start)).is_err() {
            return;
        }
        self.tokens.offset = start;
        self.position = None;
        let mut records = index::Builder::new(BUILDER_BYTES);
        let _ = self.walk(
            u64::MAX,
            None,
            OpeningSinks {
                comments: None,
                records: Some(&mut records),
            },
            |_, _, _, _| ControlFlow::<()>::Continue(()),
        );
        self.index = index::Index::build(
            [records],
            self.encodings.len(),
            INDEX_BYTES,
            BUILD_PEAK_BYTES,
        );
        self.position = None;
    }

    #[cfg(not(unix))]
    fn parallel_open(&self, _: File, _: bool) -> Option<Opening> {
        None
    }

    pub(crate) fn read<B>(
        &mut self,
        signals: &[Signal],
        end: Time,
        mut visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        self.read_from(signals, end, None, |index, time, value, _| {
            visitor(index, time, value)
        })
    }

    pub(crate) fn position(&self) -> Option<Position> {
        self.position
    }

    pub(crate) fn read_from<B>(
        &mut self,
        signals: &[Signal],
        end: Time,
        position: Option<Position>,
        mut visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>, Position) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        #[cfg(unix)]
        if position.is_none()
            && let Some(result) = self.parallel_full(signals, end, &mut visitor)
        {
            return result;
        }
        let position = position.unwrap_or(Position {
            offset: self.body,
            time: 0,
            block: false,
        });
        self.position = None;
        #[cfg(unix)]
        if let Some(result) = self.indexed_traversal(signals, end, position.offset, &mut visitor) {
            return result;
        }
        self.tokens.input.seek(SeekFrom::Start(position.offset))?;
        self.tokens.offset = position.offset;
        self.position = Some(position);
        // A resumed position can name a tick past a bounded request end. The
        // later `#time` line would stop the walk, but a record-first position
        // must not publish records outside the request.
        if position.time > end.ticks() {
            return Ok(ControlFlow::Continue(()));
        }
        let selected = signals.iter().map(|s| s.index()).collect();
        let result = self.walk(
            end.ticks(),
            Some(&selected),
            OpeningSinks {
                comments: None,
                records: None,
            },
            visitor,
        );
        if !matches!(result, Ok((ControlFlow::Continue(()), _, _))) {
            self.position = None;
        }
        result.map(|(flow, _, _)| flow)
    }

    #[cfg(unix)]
    fn parallel_full<B>(
        &mut self,
        signals: &[Signal],
        end: Time,
        visitor: &mut impl for<'v> FnMut(usize, Time, ValueRef<'v>, Position) -> ControlFlow<B>,
    ) -> Option<Result<ControlFlow<B>>> {
        let source = self.parallel.as_ref()?;
        if signals.is_empty() || end.ticks() < source.last_time {
            return None;
        }
        // Fixed budget across channel slots, active batches and the consumer.
        // Declared widths bound payloads; strings have no such bound.
        let width = signals.iter().try_fold(0_usize, |width, signal| {
            let bytes = match signal.encoding() {
                Encoding::Bits { width } => width as usize,
                Encoding::Real => std::mem::size_of::<f64>(),
                Encoding::Event | Encoding::Unsupported => 0,
                Encoding::String => return None,
            };
            Some(width.max(bytes))
        })?;
        let workers = source.cuts.len() - 1;
        let bytes = std::mem::size_of::<FullRecord>().checked_add(width)?;
        const QUEUED_BATCHES: usize = 24;
        let batch_len = (192 * 1024 * 1024 / workers / (QUEUED_BATCHES + 2) / bytes).min(4096);
        if batch_len == 0 {
            return None;
        }
        let selected = Arc::new(signals.iter().map(|s| s.index()).collect::<HashSet<_>>());
        let canceled = Arc::new(AtomicBool::new(false));
        let active_chunk = Arc::new(AtomicUsize::new(0));
        let result = thread::scope(|scope| {
            let mut receivers = Vec::with_capacity(workers);
            let mut handles = Vec::with_capacity(workers);
            for (index, range) in source.cuts.windows(2).enumerate() {
                let (tx, rx) = mpsc::sync_channel::<Result<Vec<FullRecord>>>(QUEUED_BATCHES);
                let input = BufReader::with_capacity(
                    256 * 1024,
                    RangeReader {
                        file: Arc::clone(&source.file),
                        start: range[0] as u64,
                        len: (range[1] - range[0]) as u64,
                        cursor: 0,
                        canceled: Some(Arc::clone(&canceled)),
                    },
                );
                let mut reader = Self {
                    tokens: Tokens {
                        input: Box::new(input),
                        offset: range[0] as u64,
                        #[cfg(test)]
                        growths: 0,
                    },
                    position: None,
                    #[cfg(test)]
                    records_read: 0,
                    body: self.body,
                    ids: Arc::clone(&self.ids),
                    dense_ids: Arc::clone(&self.dense_ids),
                    encodings: self.encodings.clone(),
                    index: None,
                    #[cfg(all(test, unix))]
                    index_queries: AtomicUsize::new(0),
                    #[cfg(all(test, unix))]
                    traversal_queries: AtomicUsize::new(0),
                    #[cfg(all(test, unix))]
                    traversal_records: AtomicUsize::new(0),
                    parallel: None,
                };
                let stop = range[1] as u64;
                let selected = Arc::clone(&selected);
                let active_chunk = Arc::clone(&active_chunk);
                let handle = thread::Builder::new().spawn_scoped(scope, move || {
                    let batch = RefCell::new(Vec::with_capacity(batch_len));
                    let mut first_time = None;
                    let mut first_transition = false;
                    let send = |batch: &mut Vec<FullRecord>| {
                        if !batch.is_empty()
                            && tx
                                .send(Ok(std::mem::replace(batch, Vec::with_capacity(batch_len))))
                                .is_err()
                        {
                            ControlFlow::Break(())
                        } else {
                            ControlFlow::Continue(())
                        }
                    };
                    let read = reader.walk_with_progress(
                        u64::MAX,
                        Some(&selected),
                        None,
                        |index, time, value, position| {
                            let flush = flush_startup(&mut first_time, &mut first_transition, time);
                            let mut batch = batch.borrow_mut();
                            batch.push(FullRecord {
                                index,
                                time,
                                value: FullValue::from_ref(value),
                                position,
                            });
                            // The first record may finish a tick from the previous chunk;
                            // the next selected tick can finish this chunk's first tick.
                            if batch.len() == batch_len || flush {
                                send(&mut batch)
                            } else {
                                ControlFlow::Continue(())
                            }
                        },
                        || {
                            // Future chunks must not fill their queues with small batches.
                            if active_chunk.load(Ordering::Acquire) == index {
                                send(&mut batch.borrow_mut())
                            } else {
                                ControlFlow::Continue(())
                            }
                        },
                    );
                    let batch = batch.into_inner();
                    let error = match read {
                        Ok((ControlFlow::Continue(()), _, _)) if reader.tokens.offset == stop => {
                            None
                        }
                        Ok((ControlFlow::Continue(()), _, _)) => {
                            Some(malformed("unexpected EOF in VCD chunk"))
                        }
                        Err(error) => Some(error),
                        Ok((ControlFlow::Break(()), _, _)) => return,
                    };
                    if !batch.is_empty() && tx.send(Ok(batch)).is_err() {
                        return;
                    }
                    if let Some(error) = error {
                        let _ = tx.send(Err(error));
                    }
                });
                match handle {
                    Ok(handle) => {
                        receivers.push(rx);
                        handles.push(handle);
                    }
                    Err(_) => {
                        canceled.store(true, Ordering::Relaxed);
                        drop(receivers);
                        for handle in handles {
                            let _ = handle.join();
                        }
                        return None;
                    }
                }
            }
            let mut output = Ok(ControlFlow::Continue(()));
            'chunks: for (index, rx) in receivers.iter().enumerate() {
                active_chunk.store(index, Ordering::Release);
                while let Ok(batch) = rx.recv() {
                    match batch {
                        Ok(batch) => {
                            for record in batch {
                                if let ControlFlow::Break(value) = visitor(
                                    record.index,
                                    record.time,
                                    record.value.as_ref(),
                                    record.position,
                                ) {
                                    output = Ok(ControlFlow::Break(value));
                                    break 'chunks;
                                }
                            }
                        }
                        Err(error) => {
                            output = Err(error);
                            break 'chunks;
                        }
                    }
                }
            }
            if !matches!(&output, Ok(ControlFlow::Continue(()))) {
                canceled.store(true, Ordering::Relaxed);
            }
            drop(receivers);
            for handle in handles {
                if handle.join().is_err() && output.is_ok() {
                    output = Err(malformed("VCD query worker panicked"));
                }
            }
            Some(output)
        });
        self.position = result.as_ref().and_then(|output| {
            output
                .as_ref()
                .ok()
                .filter(|flow| flow.is_continue())
                .map(|_| Position {
                    offset: *source.cuts.last().unwrap() as u64,
                    time: source.last_time,
                    block: false,
                })
        });
        result
    }

    fn walk<B>(
        &mut self,
        end: u64,
        selected: Option<&HashSet<usize>>,
        opening: OpeningSinks<'_>,
        visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>, Position) -> ControlFlow<B>,
    ) -> Result<(ControlFlow<B>, Option<TimeSpan>, Vec<bool>)> {
        self.walk_inner(end, selected, opening, visitor, None, || {
            ControlFlow::Continue(())
        })
    }

    #[cfg(unix)]
    fn walk_with_progress<B>(
        &mut self,
        end: u64,
        selected: Option<&HashSet<usize>>,
        comments: Option<&mut Vec<String>>,
        visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>, Position) -> ControlFlow<B>,
        progress: impl FnMut() -> ControlFlow<B>,
    ) -> Result<(ControlFlow<B>, Option<TimeSpan>, Vec<bool>)> {
        self.walk_inner(
            end,
            selected,
            OpeningSinks {
                comments,
                records: None,
            },
            visitor,
            Some(1024 * 1024),
            progress,
        )
    }

    fn walk_inner<B>(
        &mut self,
        end: u64,
        selected: Option<&HashSet<usize>>,
        opening: OpeningSinks<'_>,
        mut visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>, Position) -> ControlFlow<B>,
        progress_bytes: Option<u64>,
        mut progress: impl FnMut() -> ControlFlow<B>,
    ) -> Result<(ControlFlow<B>, Option<TimeSpan>, Vec<bool>)> {
        let OpeningSinks { comments, records } = opening;
        let mut comments = comments;
        let mut records = records;
        let mut last_progress = self.tokens.offset;
        let position = self.position.take();
        let mut time = position.map_or(0, |position| position.time);
        let mut span = None::<TimeSpan>;
        let mut block = position.is_some_and(|position| position.block);
        let mut block_start = None;
        let indexing = records.is_some() && selected.is_none();
        let mut bits = Vec::new();
        let mut string = String::new();
        let mut word = Vec::new();
        let mut id = Vec::new();
        let mut seen = if selected.is_none() {
            vec![false; self.encodings.len()]
        } else {
            Vec::new()
        };
        loop {
            let offset = self.tokens.offset;
            if progress_bytes.is_some_and(|limit| offset - last_progress >= limit) {
                if let ControlFlow::Break(value) = progress() {
                    return Ok((ControlFlow::Break(value), span, seen));
                }
                last_progress = offset;
            }
            if !self.tokens.next_into(&mut word)? {
                break;
            }
            if word[0] == b'#' {
                if block {
                    return Err(self.tokens.error("timestamp inside dump block"));
                }
                let next = integer(&word[1..])?;
                if next < time {
                    return Err(self.tokens.error("backwards timestamp"));
                }
                if next > end {
                    self.position = Some(Position {
                        offset,
                        time,
                        block,
                    });
                    break;
                }
                time = next;
                span = Some(TimeSpan::new(
                    span.map_or(Time::from_ticks(time), |s| s.first()),
                    Time::from_ticks(time),
                ));
                continue;
            }
            if word[0] == b'$' {
                match word.as_slice() {
                    b"$dumpvars" | b"$dumpall" | b"$dumpoff" | b"$dumpon" if !block => {
                        block = true;
                        block_start = Some(offset);
                    }
                    b"$end" if block => {
                        block = false;
                        if let (Some(start), Some(builder)) =
                            (block_start.take(), records.as_deref_mut())
                        {
                            builder.block(start, self.tokens.offset);
                        }
                    }
                    b"$comment" if !block => {
                        let text = self.tokens.text()?;
                        if let Some(comments) = comments.as_mut() {
                            comments.push(text);
                        }
                    }
                    _ => return Err(self.tokens.error("unexpected body command")),
                }
                continue;
            }
            #[cfg(test)]
            {
                self.records_read += 1;
            }
            let prefix = word[0].to_ascii_lowercase();
            let (payload, id) = if matches!(prefix, b'b' | b'r' | b's') {
                if !self.tokens.next_into(&mut id)? {
                    return Err(self.tokens.error("unexpected EOF"));
                }
                (&word[1..], id.as_slice())
            } else {
                (&word[..1], &word[1..])
            };
            let index = if self.dense_ids.is_empty() {
                self.ids.get(id).copied()
            } else {
                identifier_number(id)
                    .and_then(|number| self.dense_ids.get(number).copied().flatten())
                    .or_else(|| self.ids.get(id).copied())
            }
            .ok_or_else(|| self.tokens.error("unknown identifier code"))?;
            if selected.is_none() {
                if prefix == b's' && self.encodings[index] == Encoding::Real && !seen[index] {
                    self.encodings[index] = Encoding::String;
                }
                seen[index] = true;
            }
            let selected = selected.is_some_and(|selected| selected.contains(&index));
            if span.is_none() {
                span = Some(TimeSpan::new(Time::ZERO, Time::ZERO));
            }
            let value = match (self.encodings[index], prefix) {
                (Encoding::Real, b'r') => {
                    let value = ValueRef::Real(real(payload)?);
                    if indexing {
                        records
                            .as_deref_mut()
                            .expect("indexing needs a builder")
                            .record(index, time, offset);
                    }
                    value
                }
                (Encoding::String, b's') => {
                    decode_string(payload, &mut string, selected, Time::from_ticks(time))?;
                    if indexing {
                        records
                            .as_deref_mut()
                            .expect("indexing needs a builder")
                            .record(index, time, offset);
                    }
                    ValueRef::String(&string)
                }
                (
                    encoding @ (Encoding::Bits { .. } | Encoding::Event | Encoding::Unsupported),
                    _,
                ) if !matches!(prefix, b'r' | b's') => {
                    if (payload.is_empty()
                        && !(encoding == Encoding::Unsupported && prefix == b'b'))
                        || BitsRef::from_ascii(payload).is_none()
                    {
                        return Err(self.tokens.error("invalid bit value"));
                    }
                    if let Encoding::Bits { width } = encoding {
                        // Never revalidate a retained wide buffer for unrelated records.
                        if !selected {
                            // Dump-block event records are snapshots, not occurrences.
                            if indexing && !(block && encoding == Encoding::Event) {
                                records
                                    .as_deref_mut()
                                    .expect("indexing needs a builder")
                                    .record(index, time, offset);
                            }
                            continue;
                        }
                        let width = width as usize;
                        let fill = match payload[0].to_ascii_lowercase() {
                            b'0' | b'1' => b'0',
                            other => other,
                        };
                        bits.clear();
                        bits.resize(width.saturating_sub(payload.len()), fill);
                        bits.extend_from_slice(&payload[payload.len().saturating_sub(width)..]);
                        // Padding uses a validated source state; truncation keeps
                        // validated digits. Do not rescan the declared width.
                        ValueRef::Bits(BitsRef::from_validated_ascii(&bits))
                    } else {
                        if indexing && !(block && encoding == Encoding::Event) {
                            records
                                .as_deref_mut()
                                .expect("indexing needs a builder")
                                .record(index, time, offset);
                        }
                        ValueRef::Event { occurrences: 1 }
                    }
                }
                _ => return Err(self.tokens.error("value conflicts with storage encoding")),
            };
            // Checkpoints describe event snapshots, not observable triggers.
            if selected
                && !(block && matches!(value, ValueRef::Event { .. }))
                && let ControlFlow::Break(value) = visitor(
                    index,
                    Time::from_ticks(time),
                    value,
                    Position {
                        offset,
                        time,
                        block,
                    },
                )
            {
                return Ok((ControlFlow::Break(value), span, seen));
            }
        }
        if block {
            return Err(self.tokens.error("unterminated dump block"));
        }
        self.position.get_or_insert(Position {
            offset: self.tokens.offset,
            time,
            block,
        });
        Ok((ControlFlow::Continue(()), span, seen))
    }
}

/// Reusable token and IO scratch for position lookups.
#[cfg(unix)]
#[derive(Default)]
struct Scratch {
    word: Vec<u8>,
    id: Vec<u8>,
    buffer: Vec<u8>,
}

/// One validated position lookup context.
#[cfg(unix)]
struct Lookup<'a> {
    index: &'a index::Index,
    file: &'a File,
    file_len: u64,
    scratch: Scratch,
}

/// Reads one whitespace-delimited token from a validated source position.
///
/// Returns the offset of the first byte after the token. A short read before
/// the end of the file is not a token boundary, so the window grows instead.
/// A token that does not fit a bounded window or an unexplained short read
/// returns `None`, keeping the chronological path.
#[cfg(unix)]
fn read_word(
    file: &File,
    file_len: u64,
    offset: u64,
    word: &mut Vec<u8>,
    buffer: &mut Vec<u8>,
) -> Option<u64> {
    const FIRST: usize = 128;
    const LIMIT: usize = 64 * 1024;
    let mut window = FIRST;
    loop {
        let remaining = file_len.checked_sub(offset)?;
        if remaining == 0 {
            return None;
        }
        let take = window.min(remaining as usize);
        if buffer.len() < take {
            buffer.resize(take, 0);
        }
        let count = file.read_at(&mut buffer[..take], offset).ok()?;
        if count == 0 {
            return None;
        }
        let end_of_file = offset + count as u64 >= file_len;
        if count < take && !end_of_file {
            if window >= LIMIT || take == remaining as usize {
                return None;
            }
            window *= 2;
            continue;
        }
        let Some(start) = buffer[..count]
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())
        else {
            if end_of_file || window >= LIMIT {
                return None;
            }
            window *= 2;
            continue;
        };
        match buffer[start..count]
            .iter()
            .position(|byte| byte.is_ascii_whitespace())
        {
            Some(len) => {
                word.clear();
                word.extend_from_slice(&buffer[start..start + len]);
                return Some(offset + (start + len) as u64);
            }
            None if end_of_file => {
                word.clear();
                word.extend_from_slice(&buffer[start..count]);
                return Some(offset + count as u64);
            }
            None => {
                if window >= LIMIT {
                    return None;
                }
                window *= 2;
            }
        }
    }
}

fn name(bytes: &[u8]) -> Result<String> {
    Ok(crate::hierarchy::normalize_name(utf8(bytes)?).0)
}

fn scope_kind(raw: &str) -> Result<String> {
    let kind = raw.strip_prefix("vhdl_").unwrap_or(raw).replace('_', "-");
    if !matches!(
        kind.as_str(),
        "module"
            | "task"
            | "function"
            | "begin"
            | "fork"
            | "generate"
            | "struct"
            | "union"
            | "class"
            | "interface"
            | "package"
            | "program"
            | "architecture"
            | "procedure"
            | "record"
            | "process"
            | "block"
            | "for-generate"
            | "if-generate"
    ) {
        return Err(malformed("unsupported scope kind"));
    }
    Ok(kind)
}

fn reference_name(
    reference: &[u8],
    suffix: &[u8],
    encoding: Encoding,
) -> Result<(String, Option<BitRange>)> {
    let original = reference;
    let attached = suffix.is_empty();
    if !matches!(encoding, Encoding::Bits { .. }) && attached {
        return Ok((name(reference)?, None));
    }
    let (reference, suffix) = if attached
        && !reference.starts_with(b"\\")
        && reference.ends_with(b"]")
        && reference
            .iter()
            .rposition(|b| *b == b'[')
            .is_some_and(|i| reference[i..].contains(&b':'))
    {
        match reference.iter().rposition(|b| *b == b'[') {
            Some(index) => (&reference[..index], &reference[index..]),
            None => (reference, suffix),
        }
    } else {
        (reference, suffix)
    };
    let range = if suffix.is_empty() {
        None
    } else {
        let text = utf8(suffix)?;
        let text = text
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .ok_or_else(|| malformed("invalid reference range"))?;
        let (msb, lsb) = text.split_once(':').unwrap_or((text, text));
        Some(BitRange::new(
            msb.parse().map_err(|_| malformed("invalid range index"))?,
            lsb.parse().map_err(|_| malformed("invalid range index"))?,
        ))
    };
    if let (Encoding::Bits { width }, Some(range)) = (encoding, range) {
        if range.msb().abs_diff(range.lsb()).checked_add(1) != Some(u64::from(width)) {
            if attached {
                return Ok((name(original)?, None));
            }
            return Err(malformed("range conflicts with declared width"));
        }
    } else if !matches!(encoding, Encoding::Bits { .. }) {
        return Ok((name(reference)?, None));
    }
    Ok((name(reference)?, range))
}

fn integer(bytes: &[u8]) -> Result<u64> {
    let text = std::str::from_utf8(bytes).map_err(|_| malformed("invalid time"))?;
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if whole.is_empty()
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b == b'0')
        || (text.contains('.') && fraction.is_empty())
    {
        return Err(malformed("nonintegral or invalid time"));
    }
    whole.parse().map_err(|_| malformed("time overflow"))
}

fn real(bytes: &[u8]) -> Result<f64> {
    let text = std::str::from_utf8(bytes).map_err(|_| malformed("invalid real value"))?;
    let value: f64 = text.parse().map_err(|_| malformed("invalid real value"))?;
    let named = text.trim_start_matches(['+', '-']).to_ascii_lowercase();
    if !value.is_finite() && !matches!(named.as_str(), "nan" | "inf" | "infinity") {
        return Err(malformed("real value overflow"));
    }
    Ok(value)
}

fn decode_string(bytes: &[u8], output: &mut String, materialize: bool, time: Time) -> Result<()> {
    output.clear();
    let mut index = 0;
    while index < bytes.len() {
        let mut byte = bytes[index];
        index += 1;
        if byte == b'\\' {
            byte = *bytes
                .get(index)
                .ok_or_else(|| malformed("truncated string escape"))?;
            index += 1;
            byte = match byte {
                b'a' => 7,
                b'b' => 8,
                b'f' => 12,
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'v' => 11,
                b'\\' | b'\"' | b'\'' | b'?' => byte,
                b'0'..=b'7' => {
                    let mut octet = u16::from(byte - b'0');
                    for _ in 0..2 {
                        let Some(&next @ b'0'..=b'7') = bytes.get(index) else {
                            break;
                        };
                        octet = octet * 8 + u16::from(next - b'0');
                        index += 1;
                    }
                    u8::try_from(octet).map_err(|_| malformed("string octal escape overflow"))?
                }
                b'x' => {
                    let digits = bytes
                        .get(index..index + 2)
                        .ok_or_else(|| malformed("truncated hex escape"))?;
                    if !digits.iter().all(u8::is_ascii_hexdigit) {
                        return Err(malformed("invalid hex escape"));
                    }
                    let text =
                        std::str::from_utf8(digits).map_err(|_| malformed("invalid hex escape"))?;
                    let byte = u8::from_str_radix(text, 16)
                        .map_err(|_| malformed("invalid hex escape"))?;
                    index += 2;
                    byte
                }
                _ => return Err(malformed("unknown string escape")),
            };
        }
        if !byte.is_ascii() {
            return Err(Error::NonAsciiString {
                format: Format::Vcd,
                backend: "vcd-native".into(),
                time,
            });
        }
        if materialize {
            output.push(char::from(byte));
        }
    }
    Ok(())
}

pub(crate) fn timescale(text: &str) -> Result<Timescale> {
    let text: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let split = text
        .find(|c: char| c.is_ascii_alphabetic())
        .ok_or_else(|| malformed("missing timescale unit"))?;
    let units = [
        TimeUnit::Second,
        TimeUnit::Millisecond,
        TimeUnit::Microsecond,
        TimeUnit::Nanosecond,
        TimeUnit::Picosecond,
        TimeUnit::Femtosecond,
        TimeUnit::Attosecond,
        TimeUnit::Zeptosecond,
    ];
    let mut unit = match &text[split..] {
        "s" | "sec" => 0,
        "ms" => 1,
        "us" => 2,
        "ns" => 3,
        "ps" => 4,
        "fs" => 5,
        "as" => 6,
        "zs" => 7,
        _ => return Err(malformed("unknown timescale unit")),
    };
    let (whole, fraction) = text[..split]
        .split_once('.')
        .unwrap_or((&text[..split], ""));
    if whole.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return Err(malformed("invalid timescale number"));
    }
    let fraction = fraction.trim_end_matches('0');
    let mut numerator = format!("{whole}{fraction}")
        .parse::<u128>()
        .map_err(|_| malformed("timescale overflow"))?;
    let denominator = u32::try_from(fraction.len())
        .ok()
        .and_then(|n| 10u128.checked_pow(n))
        .ok_or_else(|| malformed("timescale precision overflow"))?;
    while numerator % denominator != 0 && unit < units.len() - 1 {
        numerator = numerator
            .checked_mul(1000)
            .ok_or_else(|| malformed("timescale overflow"))?;
        unit += 1;
    }
    if numerator == 0 || numerator % denominator != 0 {
        return Err(malformed("timescale is zero or not exactly representable"));
    }
    let factor = u32::try_from(numerator / denominator)
        .map_err(|_| malformed("timescale factor overflow"))?;
    Ok(Timescale::new(factor, units[unit]))
}

/// A small author-created source with a wide gap of unselected records.
#[cfg(all(test, unix))]
pub(crate) fn traversal_test_source() -> Vec<u8> {
    let mut source = Vec::from(
        b"$var wire 2 ! a $end $var wire 1 \" b $end $var wire 1 n noise $end \
          $var event 1 # e $end $enddefinitions $end \
          $dumpvars b00 ! 0\" 0n 1# $end\n#2 b00 ! 0\"\n#3 b01 ! 1\"\n\
          #5 b11 !\n#5 b01 ! 1#\n#6 " as &[u8],
    );
    for _ in 0..50_000 {
        source.extend_from_slice(b"0n ");
    }
    source.extend_from_slice(b"\n#7 b11 ! 0\"\n#7 b00 !\n");
    source
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    #[test]
    fn identifier_numbers_do_not_alias_or_overflow() {
        assert_ne!(identifier_number(b"!"), identifier_number(b"!!"));
        assert_ne!(identifier_number(b"~!"), identifier_number(b"!~"));
        assert_eq!(identifier_number(b" "), None);
        assert_eq!(identifier_number(&[b'~'; 32]), None);
    }

    #[cfg(unix)]
    #[test]
    fn full_batches_flush_first_record_and_first_transition() {
        let mut first_time = None;
        let mut first_transition = false;
        let mut flush = |tick| {
            flush_startup(
                &mut first_time,
                &mut first_transition,
                Time::from_ticks(tick),
            )
        };
        assert!(flush(0));
        assert!(!flush(0));
        assert!(flush(1));
        assert!(!flush(2));
    }

    #[cfg(unix)]
    #[test]
    fn full_batches_flush_before_a_large_unselected_suffix() {
        let mut source = b"$var wire 1 ! selected $end $var wire 1 \" other $end $enddefinitions $end #0 0! #1 1! #2 0! ".to_vec();
        for _ in 0..300_000 {
            source.extend_from_slice(b"#3 0\" ");
        }
        let length = source.len() as u64;
        let (mut reader, hierarchy, _) = Reader::open(
            Box::new(BufReader::new(Cursor::new(source))),
            "sparse.vcd".into(),
            None,
        )
        .unwrap();
        reader
            .tokens
            .input
            .seek(SeekFrom::Start(reader.body))
            .unwrap();
        reader.tokens.offset = reader.body;
        reader.position = None;
        reader.records_read = 0;
        let selected = HashSet::from([hierarchy.signal("selected").unwrap().index()]);
        let mut times = Vec::new();
        let (flow, _, _) = reader
            .walk_with_progress(
                u64::MAX,
                Some(&selected),
                None,
                |_, time, _, _| {
                    times.push(time.ticks());
                    ControlFlow::<()>::Continue(())
                },
                || ControlFlow::Break(()),
            )
            .unwrap();
        assert_eq!(flow, ControlFlow::Break(()));
        assert_eq!(times, [0, 1, 2]);
        assert!(reader.tokens.offset < length);
        assert!(reader.records_read < 300_003);
    }

    #[cfg(unix)]
    #[test]
    fn parallel_open_keeps_chunk_boundaries_and_rejects_invalid_joins() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("parallel-vcd-{}.vcd", std::process::id()));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let header = b"$var real 1 ! label $end $enddefinitions $end ";
        let valid = b"#0 sOne ! $comment first $end\n#1 sTwo !\n$comment note $end\n#2 sThree !\n";
        let mut source = header.to_vec();
        source.extend_from_slice(valid);
        std::fs::write(&path, &source).unwrap();
        let (reader, _, metadata) = Reader::open(
            Box::new(BufReader::new(File::open(&path).unwrap())),
            "valid.vcd".into(),
            None,
        )
        .unwrap();
        let mut reader = reader;
        reader.rebuild_index_for_test();
        let cut = source
            .windows(3)
            .position(|bytes| bytes == b"\n#1")
            .unwrap()
            + 1;
        let (span, comments, encodings, index) = reader
            .parallel_open_with_cuts(
                File::open(&path).unwrap(),
                &[reader.body as usize, cut, source.len()],
                true,
            )
            .unwrap();
        assert_eq!(span, metadata.time_span);
        assert_eq!(comments, ["first", "note"]);
        assert_eq!(encodings, reader.encodings);
        // Both cuts are real tick boundaries, so the merged positions cover every
        // record and equal the positions of the serial opening walk.
        let parallel_index = index.as_ref().unwrap();
        let serial_index = reader.index.as_ref().unwrap();
        for signal in 0..reader.encodings.len() {
            let positions = |index: &index::Index| {
                index
                    .entries(signal)
                    .iter()
                    .map(|entry| (entry.time, entry.offset))
                    .collect::<Vec<_>>()
            };
            assert_eq!(positions(parallel_index), positions(serial_index));
        }

        let mut real_source = header.to_vec();
        real_source.extend_from_slice(b"#0 r1 !");
        std::fs::write(&path, real_source).unwrap();
        let (real_reader, _, _) = Reader::open(
            Box::new(BufReader::new(File::open(&path).unwrap())),
            "real.vcd".into(),
            None,
        )
        .unwrap();
        assert_eq!(real_reader.encodings[0], Encoding::Real);

        for (body, split) in [
            (b"#0 r1 !\n#1 sTwo !".as_slice(), b"\n#1".as_slice()),
            (b"#3 sOne !\n#2 sTwo !".as_slice(), b"\n#2".as_slice()),
            (b"#0 sOne !\n#1 sTwo ?".as_slice(), b"\n#1".as_slice()),
        ] {
            let mut source = header.to_vec();
            source.extend_from_slice(body);
            std::fs::write(&path, &source).unwrap();
            let cut = source
                .windows(split.len())
                .position(|bytes| bytes == split)
                .unwrap()
                + 1;
            assert!(
                real_reader
                    .parallel_open_with_cuts(
                        File::open(&path).unwrap(),
                        &[real_reader.body as usize, cut, source.len()],
                        true
                    )
                    .is_none()
            );
            assert!(
                Reader::open(
                    Box::new(BufReader::new(File::open(&path).unwrap())),
                    "invalid.vcd".into(),
                    None,
                )
                .is_err()
            );
        }

        let mut source = header.to_vec();
        source.extend_from_slice(b"#0 sOne !\n$comment text\n#777 inside $end\n#1 sTwo !");
        std::fs::write(&path, &source).unwrap();
        let cut = source
            .windows(5)
            .position(|bytes| bytes == b"\n#777")
            .unwrap()
            + 1;
        assert!(
            reader
                .parallel_open_with_cuts(
                    File::open(&path).unwrap(),
                    &[reader.body as usize, cut, source.len()],
                    true
                )
                .is_none()
        );
        assert!(
            Reader::open(
                Box::new(BufReader::new(File::open(&path).unwrap())),
                "comment.vcd".into(),
                None,
            )
            .is_ok()
        );
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn parallel_samples_match_serial_tick_normalization() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("parallel-samples-{}.vcd", std::process::id()));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = b"$var wire 1 ! a $end $var wire 1 \" b $end $enddefinitions $end \
            #0 1! 0! 0\"\n#1 0! 1\"\n#2 0! 1! 1\"\n#2 1! 0\"\n#3 0! 1!\n#4 1! 0\"\n#5 0!\n#6 1\"\n";
        std::fs::write(&path, source).unwrap();
        let (mut reader, hierarchy, _) = Reader::open(
            Box::new(BufReader::new(File::open(&path).unwrap())),
            "parallel.vcd".into(),
            None,
        )
        .unwrap();
        let signals = [
            hierarchy.signal("a").unwrap(),
            hierarchy.signal("b").unwrap(),
        ];
        let cut = |marker: &[u8]| {
            source
                .windows(marker.len())
                .position(|bytes| bytes == marker)
                .unwrap()
                + 1
        };
        reader.parallel = Some(ParallelSource {
            file: Arc::new(File::open(&path).unwrap()),
            cuts: vec![
                reader.body as usize,
                cut(b"\n#2"),
                cut(b"\n#4"),
                source.len(),
            ],
            last_time: 6,
        });
        let mut serial =
            crate::open_bytes_with("serial.vcd", source.as_slice().into(), "vcd-native").unwrap();
        let serial_signals = [
            serial.hierarchy().signal("a").unwrap(),
            serial.hierarchy().signal("b").unwrap(),
        ];
        assert!(reader.parallel_prefix(&signals, Time::ZERO).is_none());
        for tick in [1, 2, 3, 4, 5, 6, 10] {
            let time = Time::from_ticks(tick);
            let (actual, position) = reader.parallel_prefix(&signals, time).unwrap();
            assert!(position.offset >= reader.body);
            let expected = serial.samples(&serial_signals, time).unwrap();
            for (actual, expected) in actual.iter().zip(&expected) {
                match (actual, expected) {
                    (
                        Sample::Value {
                            value: a,
                            changed_at: at,
                            ..
                        },
                        Sample::Value {
                            value: b,
                            changed_at: bt,
                            ..
                        },
                    ) => {
                        assert!(a.as_ref().same_value(b.as_ref()), "tick {tick}");
                        assert_eq!(at, bt, "tick {tick}");
                    }
                    (Sample::Missing { .. }, Sample::Missing { .. }) => {}
                    _ => panic!("parallel sample disagrees at tick {tick}"),
                }
            }
        }
        // The remaining checks cover the chunk path's own cut validation.
        reader.disable_index();
        let second_same_tick = source
            .windows(3)
            .enumerate()
            .filter(|(_, bytes)| *bytes == b"\n#2")
            .nth(1)
            .unwrap()
            .0
            + 1;
        reader.parallel.as_mut().unwrap().cuts =
            vec![reader.body as usize, second_same_tick, source.len()];
        assert!(
            reader
                .parallel_prefix(&signals, Time::from_ticks(3))
                .is_none()
        );
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn indexed_prefix_matches_serial_and_bounds_events() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("indexed-prefix-{}.vcd", std::process::id()));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = b"$var wire 4 ! a $end $var wire 1 \" b $end $var real 1 $ r $end \
            $var event 1 # e $end $enddefinitions $end \
            $dumpvars b0000 ! 0\" r1.5 $ 1# $end\n\
            #1 b0001 ! 1#\n#2 b0001 ! 0\"\n#2 b0010 !\n#3 b0010 ! 0\" 1#\n#4 b0011 !\n";
        std::fs::write(&path, source).unwrap();
        let (mut reader, hierarchy, _) = Reader::open(
            Box::new(BufReader::new(File::open(&path).unwrap())),
            "indexed.vcd".into(),
            None,
        )
        .unwrap();
        let signals = ["a", "b", "r"].map(|name| hierarchy.signal(name).unwrap());
        let event = hierarchy.signal("e").unwrap();
        reader.force_parallel_for_test(File::open(&path).unwrap(), vec![source.len()], 4);
        assert!(reader.has_index());
        // A dump-block event record is a snapshot, not an occurrence.
        let times = reader
            .index
            .as_ref()
            .unwrap()
            .entries(event.index())
            .iter()
            .map(|entry| entry.time)
            .collect::<Vec<_>>();
        assert_eq!(times, [1, 3]);
        let mut serial =
            crate::open_bytes_with("serial.vcd", source.as_slice().into(), "vcd-native").unwrap();
        let reference = ["a", "b", "r"].map(|name| serial.hierarchy().signal(name).unwrap());
        let observe = |samples: Vec<Sample>| {
            samples
                .into_iter()
                .map(|sample| match sample {
                    Sample::Value {
                        value, changed_at, ..
                    } => format!("{value:?}@{changed_at:?}"),
                    Sample::Missing { .. } => "missing".into(),
                    Sample::Event { occurrences, .. } => format!("event:{occurrences}"),
                })
                .collect::<Vec<_>>()
        };
        for tick in [1, 2, 3, 4] {
            let time = Time::from_ticks(tick);
            let (samples, position) = reader.parallel_prefix(&signals, time).unwrap();
            assert!(position.offset >= reader.body);
            assert_eq!(
                observe(samples),
                observe(serial.samples(&reference, time).unwrap()),
                "prefix at {tick}"
            );
        }
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn indexed_prefix_keeps_chronology_when_a_block_bounds_the_resume() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("indexed-block-{}.vcd", std::process::id()));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = b"$var wire 1 ! a $end $enddefinitions $end \
            #0 0!\n#5 $dumpall 1! $end\n#6 0!\n";
        std::fs::write(&path, source).unwrap();
        let (mut reader, hierarchy, _) = Reader::open(
            Box::new(BufReader::new(File::open(&path).unwrap())),
            "block.vcd".into(),
            None,
        )
        .unwrap();
        let signal = hierarchy.signal("a").unwrap();
        reader.force_parallel_for_test(File::open(&path).unwrap(), vec![source.len()], 6);
        let source_ref = reader.parallel.as_ref().unwrap();
        // The next selected record is a dump-block snapshot. The indexed path
        // must not resume there, and the chunk path keeps the exact state.
        assert!(
            reader
                .indexed_prefix(source_ref, &[signal], Time::from_ticks(4))
                .is_none()
        );
        let (samples, _) = reader
            .parallel_prefix(&[signal], Time::from_ticks(4))
            .unwrap();
        assert!(matches!(
            samples.as_slice(),
            [Sample::Value { value, changed_at: None, .. }]
                if value.as_ref().same_value(
                    Value::Bits(crate::Bits::from_ascii(b"0").unwrap()).as_ref()
                )
        ));
        // The snapshot itself is persistent state.
        assert!(
            reader
                .indexed_prefix(source_ref, &[signal], Time::from_ticks(5))
                .is_some()
        );
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unproven_changed_at_keeps_the_chronological_state() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("indexed-proof-{}.vcd", std::process::id()));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut source = Vec::from(b"$var wire 1 ! a $end $enddefinitions $end \n#0 0!\n" as &[u8]);
        for tick in 1..=70 {
            source.extend_from_slice(format!("#{tick} 0!\n").as_bytes());
        }
        source.extend_from_slice(b"#71 1!\n");
        std::fs::write(&path, &source).unwrap();
        let (mut reader, hierarchy, _) = Reader::open(
            Box::new(BufReader::new(File::open(&path).unwrap())),
            "proof.vcd".into(),
            None,
        )
        .unwrap();
        let signal = hierarchy.signal("a").unwrap();
        reader.force_parallel_for_test(File::open(&path).unwrap(), vec![source.len()], 71);
        assert!(reader.has_index());
        let source_ref = reader.parallel.as_ref().unwrap();
        // More than the proof budget of identical tick-final values.
        assert!(
            reader
                .indexed_prefix(source_ref, &[signal], Time::from_ticks(70))
                .is_none()
        );
        let (samples, _) = reader
            .parallel_prefix(&[signal], Time::from_ticks(70))
            .unwrap();
        assert!(matches!(
            samples.as_slice(),
            [Sample::Value {
                changed_at: None,
                ..
            }]
        ));
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn byte_small_file_and_metadata_opens_retain_no_unused_index() {
        let source = b"$var wire 1 ! a $end $enddefinitions $end #0 0!\n#1 1!\n";
        let bytes =
            crate::open_bytes_with("bytes.vcd", source.as_slice().into(), "vcd-native").unwrap();
        let crate::backends::Reader::Vcd(reader) = &bytes.reader else {
            unreachable!()
        };
        assert!(reader.index.is_none());
        assert!(reader.parallel.is_none());

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("small-open-{}.vcd", std::process::id()));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, source).unwrap();
        let wave = crate::open_with(&path, "vcd-native").unwrap();
        let crate::backends::Reader::Vcd(reader) = &wave.reader else {
            unreachable!()
        };
        assert!(reader.index.is_none());
        assert!(reader.parallel.is_none());

        // Metadata-only opening validates the same body without a query index.
        let (reader, _, metadata) = Reader::open_metadata(
            Box::new(BufReader::new(File::open(&path).unwrap())),
            "metadata.vcd".into(),
            File::open(&path).ok(),
        )
        .unwrap();
        assert!(reader.index.is_none());
        assert_eq!(metadata.time_span(), wave.metadata().time_span());
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn indexed_seed_is_literal_and_survives_selection_stops() {
        use crate::{ScanRef, TimeRange};
        use std::sync::atomic::Ordering;

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("indexed-seed-{}.vcd", std::process::id()));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // An excursion inside tick 5 returns to the tick-3 value, so it does not
        // advance changed_at. Tick 7 changes it.
        let source = b"$var wire 2 ! a $end $enddefinitions $end \
            #2 b00 !\n#3 b01 !\n#5 b11 !\n#5 b01 !\n#7 b11 !\n#7 b00 !\n";
        std::fs::write(&path, source).unwrap();
        // A record position is the separator before its token; the decoder
        // skips leading whitespace.
        let at = |marker: &[u8]| {
            (source
                .windows(marker.len())
                .position(|bytes| bytes == marker)
                .unwrap()
                + 2) as u64
        };
        let file_len = source.len() as u64;
        let bits = |text: &str| Value::Bits(crate::Bits::from_ascii(text.as_bytes()).unwrap());
        let (mut reader, hierarchy, _) = Reader::open(
            Box::new(BufReader::new(File::open(&path).unwrap())),
            "seed.vcd".into(),
            None,
        )
        .unwrap();
        let signal = hierarchy.signal("a").unwrap();
        reader.force_parallel_for_test(File::open(&path).unwrap(), vec![source.len()], 7);
        assert!(reader.has_index());
        let source_ref = reader.parallel.as_ref().unwrap();
        // Literal values, normalized change times and the resume record itself.
        for (time, value, changed, resume) in [
            (2, "00", None, (at(b"#3 b01"), 3)),
            (3, "01", Some(3), (at(b"#5 b11"), 5)),
            (4, "01", Some(3), (at(b"#5 b11"), 5)),
            (5, "01", Some(3), (at(b"#7 b11"), 7)),
            (6, "01", Some(3), (at(b"#7 b11"), 7)),
            (7, "00", Some(7), (file_len, 7)),
            (10, "00", Some(7), (file_len, 10)),
        ] {
            let (samples, position) = reader
                .indexed_prefix(source_ref, &[signal], Time::from_ticks(time))
                .unwrap();
            match samples.as_slice() {
                [
                    Sample::Value {
                        value: observed,
                        changed_at,
                        ..
                    },
                ] => {
                    assert!(
                        observed.as_ref().same_value(bits(value).as_ref()),
                        "value at {time}"
                    );
                    assert_eq!(changed_at.map(Time::ticks), changed, "changed_at at {time}");
                }
                _ => panic!("expected one value at {time}"),
            }
            assert_eq!(
                (position.offset, position.time),
                resume,
                "resume after {time}"
            );
        }
        assert_eq!(reader.index_queries.load(Ordering::Relaxed), 7);
        drop(reader);

        // One retained Selection carries the Initial and Change stops, the
        // repeat and the earlier query, so dropping it cannot hide stale state.
        let mut wave = crate::open_with(&path, "vcd-native").unwrap();
        let signal = wave.hierarchy().signal("a").unwrap();
        let crate::backends::Reader::Vcd(reader) = &mut wave.reader else {
            unreachable!()
        };
        reader.force_parallel_for_test(File::open(&path).unwrap(), vec![source.len()], 7);
        let mut selection = wave.select(&[signal]).unwrap();
        let probe = |selection: &crate::Selection<'_>| selection.vcd_index_queries().unwrap();
        fn check(
            selection: &mut crate::Selection<'_>,
            time: u64,
            value: &str,
            changed: Option<u64>,
        ) {
            let expected = Value::Bits(crate::Bits::from_ascii(value.as_bytes()).unwrap());
            match selection
                .samples(Time::from_ticks(time))
                .unwrap()
                .as_slice()
            {
                [
                    Sample::Value {
                        value: observed,
                        changed_at,
                        ..
                    },
                ] => {
                    assert!(
                        observed.as_ref().same_value(expected.as_ref()),
                        "value at {time}"
                    );
                    assert_eq!(changed_at.map(Time::ticks), changed, "changed_at at {time}");
                }
                _ => panic!("expected one value at {time}"),
            }
        }
        // The first point proves an engine-driven seed, not a direct helper call.
        let seeded = probe(&selection);
        check(&mut selection, 6, "01", Some(3));
        assert!(
            probe(&selection) > seeded,
            "first point must seed from the index"
        );
        // An eligible replay still serves a repeated point without a new seed.
        let stable = probe(&selection);
        check(&mut selection, 6, "01", Some(3));
        assert_eq!(probe(&selection), stable, "an eligible replay must win");
        // A completed forward point, then a backward point on the same Selection.
        check(&mut selection, 7, "00", Some(7));
        let seeded = probe(&selection);
        check(&mut selection, 3, "01", Some(3));
        assert!(
            probe(&selection) > seeded,
            "a backward point must re-seed instead of replaying"
        );

        // Break on Initial through the retained Selection, then repeat at once.
        let outcome = selection
            .scan(
                TimeRange::closed(Time::from_ticks(5), Time::from_ticks(7)),
                |record| {
                    assert!(matches!(record, ScanRef::Initial { .. }));
                    ControlFlow::Break(())
                },
            )
            .unwrap();
        assert!(outcome.is_break());
        let seeded = probe(&selection);
        check(&mut selection, 6, "01", Some(3));
        assert!(
            probe(&selection) > seeded,
            "repeat after an Initial stop must re-seed"
        );

        // Break on Change through the same Selection, then read an earlier tick.
        let outcome = selection
            .scan(
                TimeRange::closed(Time::from_ticks(3), Time::from_ticks(7)),
                |record| {
                    if matches!(record, ScanRef::Change { .. }) {
                        return ControlFlow::Break(());
                    }
                    ControlFlow::Continue(())
                },
            )
            .unwrap();
        assert!(outcome.is_break());
        let seeded = probe(&selection);
        check(&mut selection, 3, "01", Some(3));
        assert!(
            probe(&selection) > seeded,
            "earlier query after a Change stop must re-seed"
        );

        // A gap before the next record and the held state after EOF.
        check(&mut selection, 4, "01", Some(3));
        check(&mut selection, 100, "00", Some(7));
        drop(selection);
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    fn traversal_records(
        reader: &mut Reader,
        signals: &[Signal],
        end: Time,
        position: Option<Position>,
        stop_after: Option<usize>,
    ) -> Vec<(usize, u64, String)> {
        let mut records = Vec::new();
        let _ = reader
            .read_from(signals, end, position, |index, time, value, _| {
                records.push((index, time.ticks(), format!("{value:?}")));
                if stop_after.is_some_and(|limit| records.len() >= limit) {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            })
            .unwrap();
        records
    }

    #[cfg(unix)]
    #[test]
    fn indexed_traversal_matches_the_walk_and_respects_the_byte_boundary() {
        use std::sync::atomic::Ordering;

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("indexed-traversal-{}.vcd", std::process::id()));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = traversal_test_source();
        std::fs::write(&path, &source).unwrap();
        let open = |path: &std::path::Path| {
            let (mut reader, hierarchy, _) = Reader::open(
                Box::new(BufReader::new(File::open(path).unwrap())),
                "traversal.vcd".into(),
                None,
            )
            .unwrap();
            reader.force_parallel_for_test(
                File::open(path).unwrap(),
                vec![source.len()],
                source.windows(3).filter(|bytes| *bytes == b"#7 ").count() as u64,
            );
            (reader, hierarchy)
        };
        let (mut indexed, hierarchy) = open(&path);
        let (mut walked, _) = open(&path);
        walked.disable_index();
        assert!(indexed.has_index());
        let body = indexed.body;
        let body_position = Some(Position {
            offset: body,
            time: 0,
            block: false,
        });
        let signals = ["a", "b", "e"].map(|name| hierarchy.signal(name).unwrap());
        let at = |marker: &[u8], nth: usize| {
            source
                .windows(marker.len())
                .enumerate()
                .filter(|(_, bytes)| *bytes == marker)
                .nth(nth)
                .unwrap()
                .0 as u64
        };
        let first_tick_five = at(b"#5 ", 0);
        let second_tick_five = at(b"#5 ", 1);
        // From the first #5 line across the unselected gap, from a mid-tick #5
        // line, from the body before the gap and from the body across it.
        for (position, end, expect_traversal) in [
            (
                Some(Position {
                    offset: first_tick_five,
                    time: 5,
                    block: false,
                }),
                Time::from_ticks(7),
                true,
            ),
            (
                Some(Position {
                    offset: second_tick_five,
                    time: 5,
                    block: false,
                }),
                Time::from_ticks(5),
                false,
            ),
            (None, Time::from_ticks(6), false),
            (body_position, Time::from_ticks(7), true),
        ] {
            let before = indexed.traversal_queries.load(Ordering::Relaxed);
            let before_records = indexed.traversal_records.load(Ordering::Relaxed);
            let observed = traversal_records(&mut indexed, &signals, end, position, None);
            assert!(!observed.is_empty());
            assert_eq!(
                indexed.traversal_queries.load(Ordering::Relaxed) > before,
                expect_traversal,
                "unexpected traversal decision at {position:?}"
            );
            if expect_traversal {
                assert_eq!(
                    indexed.traversal_records.load(Ordering::Relaxed) - before_records,
                    observed.len()
                );
            }
            assert_eq!(
                observed,
                traversal_records(&mut walked, &signals, end, position, None),
                "traversal differs at {position:?}"
            );
        }
        // Initial dump records apply at tick zero, so even an early end sees them
        // through both paths.
        assert_eq!(
            traversal_records(&mut indexed, &signals, Time::from_ticks(1), None, None),
            traversal_records(&mut walked, &signals, Time::from_ticks(1), None, None)
        );
        // A stop returns the same prefix.
        let position = Some(Position {
            offset: second_tick_five,
            time: 5,
            block: false,
        });
        let stopped = traversal_records(
            &mut indexed,
            &signals,
            Time::from_ticks(7),
            position,
            Some(2),
        );
        assert_eq!(stopped.len(), 2);
        assert_eq!(
            stopped,
            traversal_records(
                &mut walked,
                &signals,
                Time::from_ticks(7),
                position,
                Some(2)
            )
        );
        // A dense selection over the same window keeps the walk.
        let dense = ["a", "b", "e", "noise"].map(|name| hierarchy.signal(name).unwrap());
        let before = indexed.traversal_queries.load(Ordering::Relaxed);
        assert_eq!(
            traversal_records(&mut indexed, &dense, Time::from_ticks(7), None, None),
            traversal_records(&mut walked, &dense, Time::from_ticks(7), None, None)
        );
        assert_eq!(
            indexed.traversal_queries.load(Ordering::Relaxed),
            before,
            "a dense selection must keep the walk"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn resumed_position_never_publishes_a_later_tick() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("indexed-resume-{}.vcd", std::process::id()));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = b"$var wire 1 ! a $end $enddefinitions $end #0 0!\n#1 1!\n#2 0!\n";
        std::fs::write(&path, source).unwrap();
        let (mut reader, hierarchy, _) = Reader::open(
            Box::new(BufReader::new(File::open(&path).unwrap())),
            "resume.vcd".into(),
            None,
        )
        .unwrap();
        let signal = hierarchy.signal("a").unwrap();
        let offset = source.windows(3).position(|bytes| bytes == b"#2 ").unwrap() as u64 + 3;
        let position = Position {
            offset,
            time: 2,
            block: false,
        };
        let mut observed = Vec::new();
        let _ = reader
            .read_from(
                &[signal],
                Time::from_ticks(1),
                Some(position),
                |_, time, _, _| {
                    observed.push(time.ticks());
                    ControlFlow::<()>::Continue(())
                },
            )
            .unwrap();
        assert!(observed.is_empty());
        assert_eq!(reader.position().map(|position| position.time), Some(2));
        let _ = reader
            .read_from(
                &[signal],
                Time::from_ticks(2),
                Some(position),
                |_, time, _, _| {
                    observed.push(time.ticks());
                    ControlFlow::<()>::Continue(())
                },
            )
            .unwrap();
        assert_eq!(observed, [2]);
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn canceled_chunk_stops_reading_before_eof() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("canceled-chunk-{}.vcd", std::process::id()));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"0123456789").unwrap();
        let canceled = Arc::new(AtomicBool::new(false));
        let mut reader = RangeReader {
            file: Arc::new(File::open(&path).unwrap()),
            start: 0,
            len: 10,
            cursor: 0,
            canceled: Some(Arc::clone(&canceled)),
        };
        let mut buf = [0; 2];
        assert_eq!(reader.read(&mut buf).unwrap(), 2);
        canceled.store(true, Ordering::Relaxed);
        assert_eq!(
            reader.read(&mut buf).unwrap_err().kind(),
            io::ErrorKind::Other
        );
        assert_eq!(reader.cursor, 2);
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn parallel_full_keeps_strings_serial() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("parallel-string-{}.vcd", std::process::id()));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = b"$var real 1 ! text $end $enddefinitions $end #0 sHello !\n#1 sWorld !\n";
        std::fs::write(&path, source).unwrap();
        let (mut reader, hierarchy, _) = Reader::open(
            Box::new(BufReader::new(File::open(&path).unwrap())),
            "strings.vcd".into(),
            None,
        )
        .unwrap();
        let signal = hierarchy.signal("text").unwrap();
        assert_eq!(signal.encoding(), Encoding::String);
        let cut = source
            .windows(3)
            .position(|bytes| bytes == b"\n#1")
            .unwrap()
            + 1;
        reader.force_parallel_for_test(File::open(&path).unwrap(), vec![cut, source.len()], 1);
        assert!(
            reader
                .parallel_full(&[signal], Time::from_ticks(1), &mut |_, _, _, _| {
                    ControlFlow::<()>::Continue(())
                })
                .is_none()
        );
        let before = reader.traversal_queries.load(Ordering::Relaxed);
        let mut observed = Vec::new();
        let _ = reader
            .read(&[signal], Time::from_ticks(1), |_, time, value| {
                observed.push(format!("{}:{value:?}", time.ticks()));
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
        assert_eq!(observed, ["0:String(\"Hello\")", "1:String(\"World\")"]);
        assert_eq!(
            reader.traversal_queries.load(Ordering::Relaxed),
            before,
            "strings keep the walk"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn parallel_paths_fall_back_before_wide_value_allocation() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tmp")
            .join(format!("wide-prefix-{}.vcd", std::process::id()));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = b"$var wire 8000000 ! wide $end $enddefinitions $end #0 b0 !\n#1 b1 !\n";
        std::fs::write(&path, source).unwrap();
        let (mut reader, hierarchy, _) = Reader::open(
            Box::new(BufReader::new(File::open(&path).unwrap())),
            "wide.vcd".into(),
            None,
        )
        .unwrap();
        let cut = source
            .windows(3)
            .position(|bytes| bytes == b"\n#1")
            .unwrap()
            + 1;
        reader.force_parallel_for_test(File::open(&path).unwrap(), vec![cut, source.len()], 1);
        let signal = hierarchy.signal("wide").unwrap();
        assert!(
            reader
                .parallel_prefix(&[signal], Time::from_ticks(1))
                .is_none()
        );
        assert!(
            reader
                .parallel_full(&[signal], Time::from_ticks(1), &mut |_, _, _, _| {
                    ControlFlow::<()>::Continue(())
                })
                .is_none()
        );
        let before = reader.traversal_queries.load(Ordering::Relaxed);
        let _ = reader
            .read(&[signal], Time::from_ticks(1), |_, _, _| {
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
        assert_eq!(
            reader.traversal_queries.load(Ordering::Relaxed),
            before,
            "a value wider than the token window keeps the walk"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn body_token_buffers_grow_with_token_size_not_record_count() {
        let mut text =
            String::from("$var wire 4 ! bus $end $var wire 1 s scalar $end $enddefinitions $end ");
        for tick in 0..1024 {
            text.push_str(&format!("#{tick} b10Xz ! {}s ", tick % 2));
        }
        // Every multi-byte token crosses this reader's buffer boundaries.
        let input = BufReader::with_capacity(2, Cursor::new(text.into_bytes()));
        let (mut reader, hierarchy, _) =
            Reader::open(Box::new(input), "tokens.vcd".into(), None).unwrap();
        let signals = [
            hierarchy.signal("bus").unwrap(),
            hierarchy.signal("scalar").unwrap(),
        ];
        let before = reader.tokens.growths;
        let mut records = 0;
        let _ = reader
            .read(&signals, Time::from_ticks(1023), |_, _, value| {
                if let ValueRef::Bits(bits) = value {
                    assert!(bits.width() == 1 || bits.to_string() == "10xz");
                }
                records += 1;
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
        assert_eq!(records, 2048);
        assert_eq!(reader.tokens.growths - before, 2);
    }
}
