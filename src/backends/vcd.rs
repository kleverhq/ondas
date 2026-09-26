use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::SeekFrom,
    ops::ControlFlow,
    sync::Arc,
};

#[cfg(unix)]
use std::{
    io::{self, BufReader, Read, Seek},
    os::unix::fs::FileExt,
    sync::{
        atomic::{AtomicBool, Ordering},
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

#[derive(Clone, Copy)]
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

#[cfg(unix)]
struct OpeningChunk {
    span: Option<TimeSpan>,
    seen: Vec<bool>,
    encodings: Vec<Encoding>,
    comments: Vec<String>,
}

impl Reader {
    pub(crate) fn open(
        input: Box<dyn Input>,
        source_name: String,
        file: Option<File>,
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
                    let key = (parent, name.clone());
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
                    let parent = stack.last().copied();
                    let kind = kind.replace('_', "-");
                    if declarations.insert((
                        parent,
                        name.clone(),
                        kind.clone(),
                        index,
                        range.map(|r| (r.msb(), r.lsb())),
                    )) {
                        variables.push(VariableData {
                            name,
                            reader_name: None,
                            name_was_escaped: reference.starts_with(b"\\"),
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
            #[cfg(unix)]
            parallel: None,
        };
        metadata.time_span = if let Some((span, comments, encodings)) =
            file.and_then(|file| reader.parallel_open(file))
        {
            metadata.comments.extend(comments);
            reader.encodings = encodings;
            span
        } else {
            let (_, span, _) = reader.walk(
                u64::MAX,
                None,
                Some(&mut metadata.comments),
                |_, _, _, _| ControlFlow::<()>::Continue(()),
            )?;
            span
        };
        let hierarchy = Hierarchy::new(scopes, variables, reader.encodings.clone());
        Ok((reader, hierarchy, metadata))
    }

    #[cfg(unix)]
    fn parallel_open(
        &mut self,
        file: File,
    ) -> Option<(Option<TimeSpan>, Vec<String>, Vec<Encoding>)> {
        let end = usize::try_from(file.metadata().ok()?.len()).ok()?;
        let start = usize::try_from(self.body).ok()?;
        let length = end.checked_sub(start)?;
        let workers = thread::available_parallelism()
            .ok()?
            .get()
            .min(length / (32 * 1024 * 1024));
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
        let result = self.parallel_open_with_cuts(file.try_clone().ok()?, &cuts)?;
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
    ) -> Option<(Option<TimeSpan>, Vec<String>, Vec<Encoding>)> {
        let file = Arc::new(file);
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
                            parallel: None,
                        };
                        let mut comments = Vec::new();
                        let (_, span, seen) =
                            reader.walk(u64::MAX, None, Some(&mut comments), |_, _, _, _| {
                                ControlFlow::<()>::Continue(())
                            })?;
                        Ok::<_, Error>(OpeningChunk {
                            span,
                            seen,
                            encodings: reader.encodings,
                            comments,
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
        Some((span, comments, encodings))
    }

    #[cfg(unix)]
    pub(crate) fn parallel_prefix(
        &self,
        signals: &[Signal],
        end: Time,
    ) -> Option<(Vec<Sample>, Position)> {
        let source = self.parallel.as_ref()?;
        if signals.is_empty() || end == Time::ZERO || end.ticks() < source.last_time.div_ceil(8) {
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
                            parallel: None,
                        };
                        let mut values = HashMap::<usize, ChunkValue>::new();
                        let (_, span, _) = reader.walk(
                            end.ticks(),
                            Some(&selected),
                            None,
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

    #[cfg(all(test, unix))]
    pub(crate) fn force_parallel_for_test(
        &mut self,
        file: File,
        mut cuts: Vec<usize>,
        last_time: u64,
    ) {
        cuts.insert(0, self.body as usize);
        self.parallel = Some(ParallelSource {
            file: Arc::new(file),
            cuts,
            last_time,
        });
    }

    #[cfg(not(unix))]
    fn parallel_open(&self, _: File) -> Option<(Option<TimeSpan>, Vec<String>, Vec<Encoding>)> {
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
        self.tokens.input.seek(SeekFrom::Start(position.offset))?;
        self.tokens.offset = position.offset;
        self.position = Some(position);
        let selected = signals.iter().map(|s| s.index()).collect();
        let result = self.walk(end.ticks(), Some(&selected), None, visitor);
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
        let result = thread::scope(|scope| {
            let mut receivers = Vec::with_capacity(workers);
            let mut handles = Vec::with_capacity(workers);
            for range in source.cuts.windows(2) {
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
                    parallel: None,
                };
                let stop = range[1] as u64;
                let selected = Arc::clone(&selected);
                let handle = thread::Builder::new().spawn_scoped(scope, move || {
                    let mut batch = Vec::with_capacity(batch_len);
                    let mut early = 0;
                    let read = reader.walk(
                        u64::MAX,
                        Some(&selected),
                        None,
                        |index, time, value, position| {
                            batch.push(FullRecord {
                                index,
                                time,
                                value: FullValue::from_ref(value),
                                position,
                            });
                            // Do not make an early Break wait for a full batch
                            // when selected records are sparse.
                            if batch.len() == batch_len || early < 2 {
                                if early < 2 {
                                    early += 1;
                                }
                                if tx
                                    .send(Ok(std::mem::replace(
                                        &mut batch,
                                        Vec::with_capacity(batch_len),
                                    )))
                                    .is_err()
                                {
                                    return ControlFlow::Break(());
                                }
                            }
                            ControlFlow::Continue(())
                        },
                    );
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
            'chunks: for rx in &receivers {
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
        mut comments: Option<&mut Vec<String>>,
        mut visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>, Position) -> ControlFlow<B>,
    ) -> Result<(ControlFlow<B>, Option<TimeSpan>, Vec<bool>)> {
        let position = self.position.take();
        let mut time = position.map_or(0, |position| position.time);
        let mut span = None::<TimeSpan>;
        let mut block = position.is_some_and(|position| position.block);
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
                    b"$dumpvars" | b"$dumpall" | b"$dumpoff" | b"$dumpon" if !block => block = true,
                    b"$end" if block => block = false,
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
                (Encoding::Real, b'r') => ValueRef::Real(real(payload)?),
                (Encoding::String, b's') => {
                    decode_string(payload, &mut string, selected)?;
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

fn decode_string(bytes: &[u8], output: &mut String, materialize: bool) -> Result<()> {
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
        if materialize {
            output.push(char::from(byte));
        }
    }
    Ok(())
}

fn timescale(text: &str) -> Result<Timescale> {
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
        let cut = source
            .windows(3)
            .position(|bytes| bytes == b"\n#1")
            .unwrap()
            + 1;
        let (span, comments, encodings) = reader
            .parallel_open_with_cuts(
                File::open(&path).unwrap(),
                &[reader.body as usize, cut, source.len()],
            )
            .unwrap();
        assert_eq!(span, metadata.time_span);
        assert_eq!(comments, ["first", "note"]);
        assert_eq!(encodings, reader.encodings);

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
                        &[real_reader.body as usize, cut, source.len()]
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
                    &[reader.body as usize, cut, source.len()]
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
        let mut observed = Vec::new();
        let _ = reader
            .read(&[signal], Time::from_ticks(1), |_, time, value| {
                observed.push(format!("{}:{value:?}", time.ticks()));
                ControlFlow::<()>::Continue(())
            })
            .unwrap();
        assert_eq!(observed, ["0:String(\"Hello\")", "1:String(\"World\")"]);
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
