use std::{
    collections::{HashMap, HashSet},
    io::SeekFrom,
    ops::ControlFlow,
};

use super::Input;
use crate::{
    BitRange, BitsRef, Direction, Encoding, Error, Format, Hierarchy, Metadata, Result, Signal,
    Time, TimeSpan, TimeUnit, Timescale, ValueRef,
    hierarchy::{ScopeData, VariableData},
};

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

    fn next(&mut self) -> Result<Option<Vec<u8>>> {
        self.take(true, None)?;
        let mut word = Vec::new();
        self.take(false, Some(&mut word))?;
        Ok((!word.is_empty()).then_some(word))
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

pub(crate) struct Reader {
    tokens: Tokens,
    body: u64,
    ids: HashMap<Vec<u8>, usize>,
    encodings: Vec<Encoding>,
}

impl Reader {
    pub(crate) fn open(
        input: Box<dyn Input>,
        source_name: String,
    ) -> Result<(Self, Hierarchy, Metadata)> {
        let mut tokens = Tokens { input, offset: 0 };
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
                    let name = name(&tokens.required()?)?;
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
                            parent,
                            kind,
                            definition_name: None,
                            packing: None,
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
                            parent,
                            is_constant: matches!(kind.as_str(), "parameter" | "real-parameter"),
                            kind,
                            direction: Direction::Implicit,
                            range,
                            type_name: None,
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
        let mut reader = Self {
            tokens,
            body,
            ids,
            encodings,
        };
        let (_, span) = reader.walk(u64::MAX, None, Some(&mut metadata.comments), |_, _, _| {
            ControlFlow::<()>::Continue(())
        })?;
        metadata.time_span = span;
        let hierarchy = Hierarchy::new(scopes, variables, reader.encodings.clone());
        Ok((reader, hierarchy, metadata))
    }

    pub(crate) fn read<B>(
        &mut self,
        signals: &[Signal],
        end: Time,
        visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        self.tokens.input.seek(SeekFrom::Start(self.body))?;
        self.tokens.offset = self.body;
        let selected = signals.iter().map(|s| s.index()).collect();
        self.walk(end.ticks(), Some(&selected), None, visitor)
            .map(|(flow, _)| flow)
    }

    fn walk<B>(
        &mut self,
        end: u64,
        selected: Option<&HashSet<usize>>,
        mut comments: Option<&mut Vec<String>>,
        mut visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>) -> ControlFlow<B>,
    ) -> Result<(ControlFlow<B>, Option<TimeSpan>)> {
        let mut time = 0;
        let mut span = None::<TimeSpan>;
        let mut block = false;
        let mut bits = Vec::new();
        let mut string = String::new();
        let mut seen = if selected.is_none() {
            vec![false; self.encodings.len()]
        } else {
            Vec::new()
        };
        while let Some(word) = self.tokens.next()? {
            if word[0] == b'#' {
                if block {
                    return Err(self.tokens.error("timestamp inside dump block"));
                }
                let next = integer(&word[1..])?;
                if next < time {
                    return Err(self.tokens.error("backwards timestamp"));
                }
                time = next;
                if time > end {
                    break;
                }
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
            let prefix = word[0].to_ascii_lowercase();
            let (payload, id) = if matches!(prefix, b'b' | b'r' | b's') {
                (&word[1..], self.tokens.required()?)
            } else {
                (&word[..1], word[1..].to_vec())
            };
            let index = *self
                .ids
                .get(&id)
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
                        ValueRef::Bits(BitsRef::from_ascii(&bits).expect("validated bits"))
                    } else {
                        ValueRef::Event
                    }
                }
                _ => return Err(self.tokens.error("value conflicts with storage encoding")),
            };
            // Checkpoints describe event snapshots, not observable triggers.
            if selected
                && !(block && matches!(value, ValueRef::Event))
                && let ControlFlow::Break(value) = visitor(index, Time::from_ticks(time), value)
            {
                return Ok((ControlFlow::Break(value), span));
            }
        }
        if block {
            return Err(self.tokens.error("unterminated dump block"));
        }
        Ok((ControlFlow::Continue(()), span))
    }
}

fn name(bytes: &[u8]) -> Result<String> {
    utf8(bytes.strip_prefix(b"\\").unwrap_or(bytes))
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
