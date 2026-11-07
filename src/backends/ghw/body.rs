use std::ops::ControlFlow;

use super::{
    types::{Storage, Types},
    wire::Wire,
};
use crate::{Result, Time, TimeSpan};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scalar {
    Ordinal(u8),
    Integer(i64),
    Real(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Event {
    Scalar {
        id: u32,
        rank: usize,
        time: Time,
        offset: u64,
        value: Scalar,
        /// True for cycle lists, where every record carries a rank delta.
        cyclic: bool,
    },
    ListEnd(Time),
    Tick(Time),
}

// Keep checked buffered decoding inside the caller's scalar loop.
#[inline(always)]
pub(super) fn scalar(wire: &mut Wire, types: &Types, id: usize) -> Result<Scalar> {
    let (storage, literals, _) = types
        .scalar(id)
        .ok_or_else(|| wire.malformed("non-scalar body type"))?;
    Ok(match storage {
        Storage::B2 | Storage::E8 => {
            let ordinal = wire.byte()?;
            if ordinal as usize >= literals.len() {
                return Err(wire.malformed("body enum ordinal exceeds base literals"));
            }
            Scalar::Ordinal(ordinal)
        }
        Storage::I32 | Storage::P32 => Scalar::Integer(wire.sleb(32)?),
        Storage::I64 | Storage::P64 => Scalar::Integer(wire.sleb(64)?),
        Storage::F64 => Scalar::Real(wire.real_bits()?),
    })
}

#[derive(Default)]
struct Clock {
    current: Option<i64>,
    first: Option<i64>,
}

impl Clock {
    fn span(&self) -> Option<TimeSpan> {
        Some(TimeSpan::new(
            Time::from_ticks(self.first? as u64),
            Time::from_ticks(self.current? as u64),
        ))
    }

    // Some(flow) ends traversal; None permits decoding the new time's values.
    fn enter<B>(
        &mut self,
        next: i64,
        end: Time,
        wire: &Wire,
        visitor: &mut impl FnMut(Event) -> ControlFlow<B>,
    ) -> Result<Option<ControlFlow<B>>> {
        if next < 0 || self.current.is_some_and(|previous| next < previous) {
            return Err(wire.malformed("negative or backward body time"));
        }
        if let Some(previous) = self.current
            && next > previous
            && let ControlFlow::Break(value) =
                visitor(Event::Tick(Time::from_ticks(previous as u64)))
        {
            return Ok(Some(ControlFlow::Break(value)));
        }
        if next as u64 > end.ticks() {
            return Ok(Some(ControlFlow::Continue(())));
        }
        self.first.get_or_insert(next);
        self.current = Some(next);
        Ok(None)
    }
}

/// Most buffered bytes one record can occupy: a five-byte rank delta and a
/// ten-byte 64-bit integer. A window with fewer bytes left is left to the
/// checked readers instead of being decoded in the bulk loop.
const WINDOW_RECORD: usize = 15;

/// Decode one cycle rank delta from a buffered window. `None` means the checked
/// reader must handle the field, including every malformed encoding.
#[inline]
fn window_uleb(bytes: &[u8]) -> Option<(u32, usize)> {
    let mut value = 0u32;
    for group in 0..5 {
        let byte = *bytes.get(group)?;
        if group == 4 && byte & 0xf0 != 0 {
            return None;
        }
        value |= u32::from(byte & 0x7f) << (7 * group);
        if byte & 0x80 == 0 {
            return Some((value, group + 1));
        }
    }
    None
}

/// Decode one value from a buffered window. `None` means the checked reader
/// must handle the field, so ordinal, signed-range and endian rules keep one
/// implementation and one error text.
#[inline]
fn window_value(
    bytes: &[u8],
    types: &Types,
    big_endian: bool,
    id: usize,
) -> Option<(Scalar, usize)> {
    let (storage, literals, _) = types.scalar(id)?;
    Some(match storage {
        Storage::B2 | Storage::E8 => {
            let ordinal = *bytes.first()?;
            if ordinal as usize >= literals.len() {
                return None;
            }
            (Scalar::Ordinal(ordinal), 1)
        }
        Storage::I32 | Storage::P32 => {
            let (value, length) = super::wire::decode_sleb(bytes, 32)?;
            (Scalar::Integer(value), length)
        }
        Storage::I64 | Storage::P64 => {
            let (value, length) = super::wire::decode_sleb(bytes, 64)?;
            (Scalar::Integer(value), length)
        }
        Storage::F64 => {
            let raw = *bytes.first_chunk::<8>()?;
            let bits = if big_endian {
                u64::from_be_bytes(raw)
            } else {
                u64::from_le_bytes(raw)
            };
            (Scalar::Real(bits), 8)
        }
    })
}

fn descriptor(wire: &mut Wire) -> Result<()> {
    let expected = [if wire.big_endian { 2 } else { 1 }, 4, 1, 0];
    if wire.fixed::<4>()? != expected {
        return Err(wire.unsupported("footer size/order layout"));
    }
    Ok(())
}

/// One sequential state machine validates opening and replays selected queries.
/// It keeps no source values or timestamp table. Scalar consumers own assembly.
pub(super) fn walk<B>(
    wire: &mut Wire,
    types: &Types,
    active: &[(u32, usize)],
    end: Time,
    mut visitor: impl FnMut(Event) -> ControlFlow<B>,
) -> Result<(ControlFlow<B>, Option<TimeSpan>)> {
    let mut clock = Clock::default();
    let mut footer = false;
    loop {
        // Reading through fill_buf preserves actual I/O failures. A clean EOF is
        // permitted here only; all unfinished fields use Wire's checked reads.
        if wire.at_eof()? {
            break;
        }
        let tag = wire.fixed::<4>()?;
        match &tag {
            b"SNP\0" | b"CYC\0" if footer => {
                return Err(wire.malformed("body section after footer"));
            }
            b"SNP\0" => {
                wire.zeros()?;
                let time = wire.i64()?;
                if let Some(flow) = clock.enter(time, end, wire, &mut visitor)? {
                    return Ok((flow, clock.span()));
                }
                for (rank, &(id, ty)) in active.iter().enumerate() {
                    let offset = wire.offset;
                    let value = scalar(wire, types, ty)?;
                    if let ControlFlow::Break(value) = visitor(Event::Scalar {
                        id,
                        rank,
                        time: Time::from_ticks(time as u64),
                        offset,
                        value,
                        cyclic: false,
                    }) {
                        return Ok((ControlFlow::Break(value), clock.span()));
                    }
                }
                wire.marker(b"ESN\0")?;
                if let ControlFlow::Break(value) =
                    visitor(Event::ListEnd(Time::from_ticks(time as u64)))
                {
                    return Ok((ControlFlow::Break(value), clock.span()));
                }
            }
            b"CYC\0" => {
                let mut time = wire.i64()?;
                if let Some(flow) = clock.enter(time, end, wire, &mut visitor)? {
                    return Ok((flow, clock.span()));
                }
                loop {
                    let mut rank = 0usize;
                    loop {
                        // Bulk-decode the records that the buffered window covers
                        // completely. Records it leaves out are taken by the checked
                        // readers below, one at a time, so values, offsets, ordering
                        // and error texts stay identical.
                        let mut consumed = 0usize;
                        let mut decoded_rank = rank;
                        let mut stopped = None;
                        {
                            let big_endian = wire.big_endian;
                            let base = wire.offset;
                            let bytes = wire.window()?;
                            while bytes.len() - consumed >= WINDOW_RECORD {
                                let at = consumed;
                                let Some((delta, width)) = window_uleb(&bytes[at..]) else {
                                    break;
                                };
                                if delta == 0 {
                                    break;
                                }
                                let Some(next_rank) = decoded_rank.checked_add(delta as usize)
                                else {
                                    break;
                                };
                                let Some(&(id, ty)) = active.get(next_rank - 1) else {
                                    break;
                                };
                                let Some((value, length)) =
                                    window_value(&bytes[at + width..], types, big_endian, ty)
                                else {
                                    break;
                                };
                                decoded_rank = next_rank;
                                consumed = at + width + length;
                                if let ControlFlow::Break(value) = visitor(Event::Scalar {
                                    id,
                                    rank: next_rank - 1,
                                    time: Time::from_ticks(time as u64),
                                    offset: base + (at + width) as u64,
                                    value,
                                    cyclic: true,
                                }) {
                                    stopped = Some(value);
                                    break;
                                }
                            }
                        }
                        wire.consume_window(consumed);
                        rank = decoded_rank;
                        if let Some(value) = stopped {
                            return Ok((ControlFlow::Break(value), clock.span()));
                        }
                        if consumed != 0 {
                            continue;
                        }
                        let delta = wire.uleb()? as usize;
                        if delta == 0 {
                            break;
                        }
                        rank = rank
                            .checked_add(delta)
                            .ok_or_else(|| wire.malformed("cycle scalar rank overflow"))?;
                        let &(id, ty) = active.get(rank - 1).ok_or_else(|| {
                            wire.malformed("cycle scalar rank exceeds active table")
                        })?;
                        let offset = wire.offset;
                        let value = scalar(wire, types, ty)?;
                        if let ControlFlow::Break(value) = visitor(Event::Scalar {
                            id,
                            rank: rank - 1,
                            time: Time::from_ticks(time as u64),
                            offset,
                            value,
                            cyclic: true,
                        }) {
                            return Ok((ControlFlow::Break(value), clock.span()));
                        }
                    }
                    if let ControlFlow::Break(value) =
                        visitor(Event::ListEnd(Time::from_ticks(time as u64)))
                    {
                        return Ok((ControlFlow::Break(value), clock.span()));
                    }
                    let delta = wire.sleb(64)?;
                    if delta == -1 {
                        wire.marker(b"ECY\0")?;
                        break;
                    }
                    if delta < 0 {
                        return Err(wire.malformed("negative cycle time increment"));
                    }
                    time = time
                        .checked_add(delta)
                        .ok_or_else(|| wire.malformed("cycle time overflow"))?;
                    if let Some(flow) = clock.enter(time, end, wire, &mut visitor)? {
                        return Ok((flow, clock.span()));
                    }
                }
            }
            b"DIR\0" => {
                if footer {
                    return Err(wire.malformed("duplicate directory"));
                }
                // A footer ends the value stream; no subsequent same-time list
                // is permitted. Deliver the completed tick before footer work.
                if let Some(time) = clock.current
                    && let ControlFlow::Break(value) =
                        visitor(Event::Tick(Time::from_ticks(time as u64)))
                {
                    return Ok((ControlFlow::Break(value), clock.span()));
                }
                footer = true;
                descriptor(wire)?;
                let count = wire.count("directory entry")?;
                if count as u64 > wire.len.saturating_sub(wire.offset) / 8 {
                    return Err(wire.malformed("directory count exceeds remaining input"));
                }
                for _ in 0..count {
                    let name = wire.fixed::<4>()?;
                    if name[3] != 0 || !name[..3].iter().all(u8::is_ascii_uppercase) {
                        return Err(wire.malformed("invalid directory section name"));
                    }
                    let _offset = wire.u32()?;
                }
                wire.marker(b"EOD\0")?;
            }
            b"TAI\0" => {
                if !footer
                    && let Some(time) = clock.current
                    && let ControlFlow::Break(value) =
                        visitor(Event::Tick(Time::from_ticks(time as u64)))
                {
                    return Ok((ControlFlow::Break(value), clock.span()));
                }
                footer = true;
                descriptor(wire)?;
                let _directory = wire.u32()?;
                if !wire.at_eof()? {
                    return Err(wire.malformed("data after tailer"));
                }
                break;
            }
            _ if tag[3] != 0 => return Err(wire.malformed("invalid body section marker")),
            _ => {
                return Err(wire.unsupported(format_args!(
                    "body section {}",
                    String::from_utf8_lossy(&tag[..3])
                )));
            }
        }
    }
    if !footer
        && let Some(time) = clock.current
        && let ControlFlow::Break(value) = visitor(Event::Tick(Time::from_ticks(time as u64)))
    {
        return Ok((ControlFlow::Break(value), clock.span()));
    }
    Ok((ControlFlow::Continue(()), clock.span()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn logic_types() -> Types {
        let bytes =
            b"TYP\0\0\0\0\0\x01\0\0\0\x17\0\x09\x01\x02\x03\x04\x05\x06\x07\x08\x09\0".to_vec();
        let mut wire = Wire::new(Box::new(Cursor::new(bytes))).unwrap();
        Types::read(
            &mut wire,
            [
                "", "'U'", "'X'", "'0'", "'1'", "'Z'", "'W'", "'L'", "'H'", "'-'",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        )
        .unwrap()
    }

    fn snapshot(time: i64, values: &[u8], bytes: &mut Vec<u8>) {
        bytes.extend(b"SNP\0\0\0\0\0");
        bytes.extend(time.to_le_bytes());
        bytes.extend(values);
        bytes.extend(b"ESN\0");
    }

    #[test]
    fn active_ranks_and_equal_times_across_sections() {
        let types = logic_types();
        let active = [(1, 0), (3, 0)];
        let mut bytes = Vec::new();
        snapshot(0, &[3, 2], &mut bytes);
        bytes.extend(b"CYC\0");
        bytes.extend(0i64.to_le_bytes());
        bytes.extend([2, 3, 0, 0, 1, 2, 0, 127]);
        bytes.extend(b"ECY\0");
        snapshot(0, &[2, 2], &mut bytes);
        bytes.extend(b"CYC\0");
        bytes.extend(1i64.to_le_bytes());
        bytes.extend([0, 127]);
        bytes.extend(b"ECY\0");
        for capacity in [1, 2, 4096] {
            let mut wire = Wire::new(Box::new(BufReader::with_capacity(
                capacity,
                Cursor::new(bytes.clone()),
            )))
            .unwrap()
            .with_buffer_capacity(capacity);
            let mut events = Vec::new();
            let (flow, span) = walk(
                &mut wire,
                &types,
                &active,
                Time::from_ticks(u64::MAX),
                |event| {
                    events.push(event);
                    ControlFlow::<()>::Continue(())
                },
            )
            .unwrap();
            assert!(flow.is_continue());
            assert_eq!(
                span,
                Some(TimeSpan::new(Time::from_ticks(0), Time::from_ticks(1)))
            );
            assert_eq!(
                events
                    .iter()
                    .filter_map(|e| if let Event::Tick(t) = e {
                        Some(t.ticks())
                    } else {
                        None
                    })
                    .collect::<Vec<_>>(),
                [0, 1]
            );
            let scalars = events
                .iter()
                .filter_map(|e| {
                    if let Event::Scalar { id, value, .. } = e {
                        Some((*id, *value))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            assert_eq!(
                scalars,
                [
                    (1, Scalar::Ordinal(3)),
                    (3, Scalar::Ordinal(2)),
                    (3, Scalar::Ordinal(3)),
                    (1, Scalar::Ordinal(2)),
                    (1, Scalar::Ordinal(2)),
                    (3, Scalar::Ordinal(2))
                ]
            );
        }
    }

    #[test]
    fn tick_break_and_end_bound_do_not_decode_the_next_values() {
        let types = logic_types();
        let active = [(1, 0)];
        let mut bytes = Vec::new();
        snapshot(0, &[3], &mut bytes);
        bytes.extend(b"CYC\0");
        bytes.extend(1i64.to_le_bytes());
        let boundary = bytes.len();
        bytes.extend([1, 255, 0, 127]);
        bytes.extend(b"ECY\0");
        let mut wire = Wire::new(Box::new(Cursor::new(bytes.clone()))).unwrap();
        let (flow, _) = walk(
            &mut wire,
            &types,
            &active,
            Time::from_ticks(u64::MAX),
            |event| {
                if matches!(event, Event::Tick(_)) {
                    ControlFlow::Break("done")
                } else {
                    ControlFlow::Continue(())
                }
            },
        )
        .unwrap();
        assert_eq!(flow, ControlFlow::Break("done"));
        assert_eq!(wire.offset, boundary as u64);
        let mut wire = Wire::new(Box::new(Cursor::new(bytes.clone()))).unwrap();
        assert!(
            walk(&mut wire, &types, &active, Time::from_ticks(0), |_| {
                ControlFlow::<()>::Continue(())
            })
            .unwrap()
            .0
            .is_continue()
        );
        assert_eq!(wire.offset, boundary as u64);
        let mut wire = Wire::new(Box::new(Cursor::new(bytes))).unwrap();
        assert!(matches!(
            walk(
                &mut wire,
                &types,
                &active,
                Time::from_ticks(u64::MAX),
                |_| ControlFlow::<()>::Continue(())
            ),
            Err(crate::Error::Malformed { .. })
        ));
    }

    #[test]
    fn truncation_bad_ranks_and_time_overflow_fail_before_tick_delivery() {
        let types = logic_types();
        let active = [(1, 0)];
        for values in [
            vec![2, 2, 0, 127],
            vec![1, 255, 0, 127],
            vec![0, 126],
            vec![1],
            vec![0, 127, b'E', b'C', b'Y'],
        ] {
            let mut bytes = b"CYC\0".to_vec();
            bytes.extend(0i64.to_le_bytes());
            bytes.extend(values);
            let mut wire = Wire::new(Box::new(Cursor::new(bytes))).unwrap();
            let mut ticks = 0;
            assert!(
                walk(
                    &mut wire,
                    &types,
                    &active,
                    Time::from_ticks(u64::MAX),
                    |event| {
                        ticks += usize::from(matches!(event, Event::Tick(_)));
                        ControlFlow::<()>::Continue(())
                    }
                )
                .is_err()
            );
            assert_eq!(ticks, 0);
        }
        let mut bytes = b"CYC\0".to_vec();
        bytes.extend(i64::MAX.to_le_bytes());
        bytes.extend([0, 1]);
        let mut wire = Wire::new(Box::new(Cursor::new(bytes))).unwrap();
        assert!(
            walk(
                &mut wire,
                &types,
                &active,
                Time::from_ticks(u64::MAX),
                |_| ControlFlow::<()>::Continue(())
            )
            .is_err()
        );
        let mut wire = Wire::new(Box::new(Cursor::new(Vec::new()))).unwrap();
        assert_eq!(
            walk(&mut wire, &types, &active, Time::from_ticks(0), |_| {
                ControlFlow::<()>::Continue(())
            })
            .unwrap()
            .1,
            None
        );
    }

    #[test]
    fn known_footer_layout_and_optional_absence() {
        let types = logic_types();
        let active = [(1, 0)];
        let mut bytes = Vec::new();
        snapshot(0, &[2], &mut bytes);
        bytes.extend(b"DIR\0\x01\x04\x01\0\0\0\0\0EOD\0TAI\0\x01\x04\x01\0\0\0\0\0");
        let mut wire = Wire::new(Box::new(BufReader::with_capacity(1, Cursor::new(bytes))))
            .unwrap()
            .with_buffer_capacity(1);
        let mut ticks = 0;
        let (flow, _) = walk(
            &mut wire,
            &types,
            &active,
            Time::from_ticks(u64::MAX),
            |event| {
                ticks += usize::from(matches!(event, Event::Tick(_)));
                ControlFlow::<()>::Continue(())
            },
        )
        .unwrap();
        assert!(flow.is_continue());
        assert_eq!(ticks, 1);
    }

    #[test]
    #[ignore = "requires the installed GHW corpus"]
    fn source_body_validation_classifies_the_complete_corpus() {
        use std::fs::{self, File};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/ghw");
        let mut paths = fs::read_dir(root)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_dir())
            .collect::<Vec<_>>();
        paths.sort();
        let mut passed = 0;
        for path in &paths {
            let mut wire = Wire::new(Box::new(BufReader::new(
                File::open(path.join("waveform.ghw")).unwrap(),
            )))
            .unwrap();
            wire.header().unwrap();
            let strings = wire.strings().unwrap();
            let parsed = Types::read(&mut wire, strings);
            if path.file_name().unwrap() == "ghw0002-ghdl-issue-538" {
                assert!(matches!(parsed, Err(crate::Error::Malformed { .. })));
                continue;
            }
            let mut types = parsed.unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let mut tag = wire.fixed::<4>().unwrap();
            if &tag == b"WKT\0" {
                types.wkt(&mut wire).unwrap();
                tag = wire.fixed().unwrap();
            }
            assert_eq!(&tag, b"HIE\0");
            let declarations = super::super::hierarchy::read(&mut wire, &types)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            wire.marker(b"EOH\0").unwrap();
            let result = walk(
                &mut wire,
                &types,
                &declarations.active,
                Time::from_ticks(u64::MAX),
                |_| ControlFlow::<()>::Continue(()),
            );
            if matches!(
                path.file_name().unwrap().to_str().unwrap(),
                "ghw0044-cpu-uppercase" | "ghw0050-cpu-lowercase" | "ghw0059-toplevel-tb"
            ) {
                assert!(
                    matches!(result, Err(crate::Error::Malformed { .. })),
                    "{}",
                    path.display()
                );
            } else {
                let (flow, span) = result.unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                assert!(flow.is_continue());
                assert!(span.is_some());
                passed += 1;
            }
        }
        assert_eq!(passed + 4, paths.len());
        eprintln!(
            "GHW source body probe: {passed} complete, 3 malformed cycles, 1 malformed missing nested range"
        );
    }
}

#[cfg(test)]
mod window_parity {
    use super::*;
    use std::io::{BufReader, Cursor};

    /// Read-ahead sizes that keep the bulk window smaller than one record, exactly
    /// at its edge, and much larger than it.
    const CAPACITIES: [usize; 10] = [1, 2, 3, 4, 7, 8, 15, 16, 17, 4096];

    /// Header, string pool and type table for these inputs: I32, I64, F64 and a
    /// three-literal enumeration. Body bytes follow at the returned offset.
    fn waveform(body: &[u8]) -> (Vec<u8>, u64) {
        let strings = ["", "i32", "i64", "f64", "e8", "a", "b", "c"];
        let mut bytes = b"GHDLwave\n\x10\0\x01\x01\x04\x01\0STR\0\0\0\0\0".to_vec();
        bytes.extend((strings.len() as u32).to_le_bytes());
        bytes.extend((strings.iter().map(|s| s.len()).sum::<usize>() as u32).to_le_bytes());
        for string in strings {
            bytes.extend(string.as_bytes());
            bytes.push(0);
        }
        bytes.extend(b"EOS\0TYP\0\0\0\0\0");
        bytes.extend(4u32.to_le_bytes());
        // Scalar I32, I64, F64 and an E8 with three literals, then the terminator.
        bytes.extend([25, 1, 26, 2, 27, 3, 23, 4, 3, 5, 6, 7]);
        bytes.push(0);
        let body_start = bytes.len() as u64;
        bytes.extend(body);
        (bytes, body_start)
    }

    /// 130 active scalars: ranks 1..=3 are the integer and real types, the rest
    /// are the enumeration, so rank deltas above 127 have room to be encoded.
    fn active(types: &Types) -> Vec<(u32, usize)> {
        let mut wire = Wire::new(Box::new(Cursor::new(vec![1u8, 2, 3, 4]))).unwrap();
        let nodes = (0..4)
            .map(|_| types.type_id(&mut wire).unwrap())
            .collect::<Vec<_>>();
        let mut active = vec![(1, nodes[0]), (2, nodes[1]), (3, nodes[2])];
        active.extend((4..=130u32).map(|id| (id, nodes[3])));
        active
    }

    fn parsed(bytes: &[u8], capacity: usize) -> (Wire, Types, Vec<(u32, usize)>) {
        let mut wire = Wire::new(Box::new(BufReader::with_capacity(
            capacity,
            Cursor::new(bytes.to_vec()),
        )))
        .unwrap()
        .with_buffer_capacity(capacity);
        wire.header().unwrap();
        let strings = wire.strings().unwrap();
        let types = Types::read(&mut wire, strings).unwrap();
        let active = active(&types);
        (wire, types, active)
    }

    /// Walk a body with one read-ahead capacity and return every event, the error
    /// text that ended the walk, and the reader position the walk stopped at.
    fn walk_body(
        bytes: &[u8],
        body_start: u64,
        capacity: usize,
    ) -> (Vec<Event>, Option<String>, u64) {
        let (mut wire, types, active) = parsed(bytes, capacity);
        wire.seek(body_start).unwrap();
        let mut events = Vec::new();
        let result = walk(
            &mut wire,
            &types,
            &active,
            Time::from_ticks(u64::MAX),
            |event| {
                events.push(event);
                ControlFlow::<()>::Continue(())
            },
        );
        (
            events,
            result.err().map(|error| error.to_string()),
            wire.offset,
        )
    }

    /// Walk a body with a visitor that stops after `limit` events.
    fn walk_until(
        bytes: &[u8],
        body_start: u64,
        capacity: usize,
        limit: usize,
    ) -> (Vec<Event>, u64) {
        let (mut wire, types, active) = parsed(bytes, capacity);
        wire.seek(body_start).unwrap();
        let mut events = Vec::new();
        let result = walk(
            &mut wire,
            &types,
            &active,
            Time::from_ticks(u64::MAX),
            |event| {
                events.push(event);
                if events.len() == limit {
                    return ControlFlow::Break(());
                }
                ControlFlow::<()>::Continue(())
            },
        );
        assert!(result.unwrap().0.is_break());
        (events, wire.offset)
    }

    fn snapshot(time: i64, values: &[u8], bytes: &mut Vec<u8>) {
        bytes.extend(b"SNP\0\0\0\0\0");
        bytes.extend(time.to_le_bytes());
        bytes.extend(values);
        bytes.extend(b"ESN\0");
    }

    /// A body that stores one value of every supported storage and uses a
    /// two-byte rank delta plus a noncanonical one-byte value encoding.
    fn mixed_body() -> Vec<u8> {
        let mut body = Vec::new();
        let mut values = vec![0x80, 0x00]; // Noncanonical zero for I32.
        values.extend([0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x7f]);
        values.extend(1.5f64.to_bits().to_le_bytes());
        values.extend([2u8; 127]);
        snapshot(0, &values, &mut body);
        body.extend(b"CYC\0");
        body.extend(0i64.to_le_bytes());
        body.extend([0x81, 0x01, 2]); // Delta 129 reaches the enumeration.
        body.extend([0x81, 0x00, 1]); // Noncanonical delta 1 reaches rank 130.
        body.push(0);
        body.push(1);
        body.extend([0x01, 0xff, 0xff, 0xff, 0xff, 0x07]); // I32 maximum.
        body.push(0x01);
        body.extend([0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x7f]);
        body.push(0x01); // I64 minimum.
        body.extend(1.5f64.to_bits().to_le_bytes());
        body.push(0x01);
        body.push(0);
        body.push(0);
        body.push(0x7f);
        body.extend(b"ECY\0");
        body
    }

    #[test]
    fn bulk_window_preserves_every_event_and_offset_across_buffer_boundaries() {
        let body = mixed_body();
        let (bytes, body_start) = waveform(&body);
        let (expected, error, offset) =
            walk_body(&bytes, body_start, CAPACITIES[CAPACITIES.len() - 1]);
        assert_eq!(error, None);
        let scalars = expected
            .iter()
            .filter(|event| matches!(event, Event::Scalar { .. }))
            .count();
        let list_ends = expected
            .iter()
            .filter(|event| matches!(event, Event::ListEnd(_)))
            .count();
        let ticks = expected
            .iter()
            .filter(|event| matches!(event, Event::Tick(_)))
            .count();
        assert_eq!((scalars, list_ends, ticks), (136, 3, 2));
        assert!(expected.iter().any(|event| matches!(
            event,
            Event::Scalar {
                value: Scalar::Integer(i64::MIN),
                ..
            }
        )));
        assert!(expected.iter().any(|event| matches!(
            event,
            Event::Scalar {
                value: Scalar::Real(bits),
                ..
            } if *bits == 1.5f64.to_bits()
        )));
        assert!(expected.iter().any(|event| matches!(
            event,
            Event::Scalar {
                value: Scalar::Integer(2_147_483_647),
                ..
            }
        )));
        for capacity in CAPACITIES {
            let (events, error, position) = walk_body(&bytes, body_start, capacity);
            assert_eq!(error, None, "capacity {capacity}");
            assert_eq!(events, expected, "capacity {capacity}");
            assert_eq!(position, offset, "capacity {capacity}");
        }
    }

    #[test]
    fn stopping_mid_list_leaves_the_same_events_and_position() {
        let body = mixed_body();
        let (bytes, body_start) = waveform(&body);
        let expected = walk_until(&bytes, body_start, CAPACITIES[CAPACITIES.len() - 1], 7);
        assert_eq!(expected.0.len(), 7);
        for capacity in CAPACITIES {
            assert_eq!(
                walk_until(&bytes, body_start, capacity, 7),
                expected,
                "capacity {capacity}"
            );
        }
    }

    /// A cycle list that starts with the malformed bytes and is padded afterwards,
    /// so the bulk window has room and the record really reaches it first.
    fn padded_cycle(malformed: &[u8]) -> Vec<u8> {
        let mut body = b"CYC\0".to_vec();
        body.extend(0i64.to_le_bytes());
        body.extend(malformed);
        body.extend(std::iter::repeat_n(2u8, 64));
        body
    }

    #[test]
    fn malformed_cycle_fields_report_the_same_error_and_position_at_every_boundary() {
        let valid = mixed_body();
        let truncated_value = [
            b"CYC\0".to_vec(),
            0i64.to_le_bytes().to_vec(),
            vec![1, 0x80],
        ]
        .concat();
        let backward_time = [
            b"CYC\0".to_vec(),
            5i64.to_le_bytes().to_vec(),
            vec![1, 1, 0, 0x7f],
            b"ECY\0".to_vec(),
            b"CYC\0".to_vec(),
            0i64.to_le_bytes().to_vec(),
            vec![1, 1, 0, 0x7f],
            b"ECY\0".to_vec(),
        ]
        .concat();
        let bodies = [
            &valid[..valid.len() - 6],
            &truncated_value[..],
            &backward_time[..],
            &padded_cycle(&[0x04, 3])[..], // Ordinal outside the base literals.
            &padded_cycle(&[0xff, 0xff, 0xff, 0xff, 0x1f])[..], // Rank delta above 32 bits.
            &padded_cycle(&[0x01, 0x80, 0x80, 0x80, 0x80, 0x80])[..], // I32 SLEB above 32 bits.
            &padded_cycle(&[0xc8, 0x01, 1])[..], // Rank beyond the active table.
            &padded_cycle(&[0x01, 1, 0x00, 0x7e])[..], // Negative cycle time step.
        ];
        for body in bodies {
            let (bytes, body_start) = waveform(body);
            let (reference, error, offset) =
                walk_body(&bytes, body_start, CAPACITIES[CAPACITIES.len() - 1]);
            let error = error.expect("malformed body must be rejected");
            for capacity in CAPACITIES {
                let (events, other, position) = walk_body(&bytes, body_start, capacity);
                assert_eq!(other, Some(error.clone()), "capacity {capacity}");
                assert_eq!(events, reference, "capacity {capacity}");
                assert_eq!(position, offset, "capacity {capacity}");
            }
        }
    }
}
