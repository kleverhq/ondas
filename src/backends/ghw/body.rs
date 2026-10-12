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
    Scalar { id: u32, value: Scalar },
    ListEnd(Time),
    Tick(Time),
}

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
                for &(id, ty) in active {
                    let value = scalar(wire, types, ty)?;
                    if let ControlFlow::Break(value) = visitor(Event::Scalar { id, value }) {
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
                        let value = scalar(wire, types, ty)?;
                        if let ControlFlow::Break(value) = visitor(Event::Scalar { id, value }) {
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
            .unwrap();
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
                    if let Event::Scalar { id, value } = e {
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
        let mut wire =
            Wire::new(Box::new(BufReader::with_capacity(1, Cursor::new(bytes)))).unwrap();
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
