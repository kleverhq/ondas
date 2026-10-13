mod body;
mod hierarchy;
#[cfg(test)]
#[path = "../../../tests/support/ghw_bytes.rs"]
mod test_bytes;
mod types;
mod wire;

use super::Input;
use crate::{
    BitsRef, Encoding, Hierarchy, Metadata, Result, Signal, Time, TimeUnit, Timescale, ValueRef,
};
use hierarchy::Recipe;
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
};
use types::Types;
use wire::Wire;

pub(crate) enum Notification<'v> {
    Value {
        index: usize,
        time: Time,
        value: ValueRef<'v>,
    },
    Tick(Time),
}

pub(crate) struct Reader {
    wire: Wire,
    types: Types,
    recipes: Vec<Recipe>,
    active: Vec<(u32, usize)>,
    body_start: u64,
    #[cfg(test)]
    pub(crate) records_read: usize,
    #[cfg(test)]
    pub(crate) assembly_bytes: usize,
}

struct Buffer {
    index: usize,
    digits: Vec<u8>,
    real: f64,
    required: usize,
    known: usize,
    wkt: u8,
    vector: bool,
}

impl Buffer {
    fn update(&mut self, position: usize, value: body::Scalar, first: bool) {
        if first {
            self.known += 1;
        }
        let bits = match value {
            body::Scalar::Real(bits) => {
                self.real = f64::from_bits(bits);
                return;
            }
            body::Scalar::Ordinal(ordinal) if self.wkt != 0 => {
                let alphabet: &[u8] = if self.wkt == 3 { b"ux01zwlh-" } else { b"01" };
                self.digits[if self.vector { position } else { 0 }] = alphabet[ordinal as usize];
                return;
            }
            body::Scalar::Ordinal(ordinal) => u64::from(ordinal),
            body::Scalar::Integer(value) => value as u64,
        };
        for (shift, digit) in self.digits.iter_mut().rev().enumerate() {
            *digit = b'0' + ((bits >> shift) & 1) as u8;
        }
    }

    fn value(&self) -> ValueRef<'_> {
        if self.digits.is_empty() {
            ValueRef::Real(self.real)
        } else {
            ValueRef::Bits(BitsRef::from_validated_ascii(&self.digits))
        }
    }
}

impl Reader {
    pub(crate) fn open(
        input: Box<dyn Input>,
        source_name: String,
    ) -> Result<(Self, Hierarchy, Metadata)> {
        let mut wire = Wire::new(input)?;
        wire.header()?;
        let strings = wire.strings()?;
        let mut types = Types::read(&mut wire, strings)?;
        let mut marker = wire.fixed::<4>()?;
        if &marker == b"WKT\0" {
            types.wkt(&mut wire)?;
            marker = wire.fixed()?;
        }
        if &marker != b"HIE\0" {
            return Err(wire.malformed("expected HIE section"));
        }
        let declarations = hierarchy::read(&mut wire, &types)?;
        wire.marker(b"EOH\0")?;
        let body_start = wire.offset;
        let (_, time_span) = body::walk(
            &mut wire,
            &types,
            &declarations.active,
            Time::from_ticks(u64::MAX),
            |_| ControlFlow::<()>::Continue(()),
        )?;
        let metadata = Metadata {
            source_name,
            timescale: Some(Timescale::new(1, TimeUnit::Femtosecond)),
            time_span,
            writer: None,
            date: None,
            comments: Vec::new(),
        };
        Ok((
            Self {
                wire,
                types,
                recipes: declarations.recipes,
                active: declarations.active,
                body_start,
                #[cfg(test)]
                records_read: 0,
                #[cfg(test)]
                assembly_bytes: 0,
            },
            declarations.hierarchy,
            metadata,
        ))
    }

    pub(crate) fn read<B>(
        &mut self,
        signals: &[Signal],
        end: Time,
        mut visitor: impl for<'v> FnMut(Notification<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        if signals.is_empty() {
            return Ok(ControlFlow::Continue(()));
        }
        self.wire.seek(self.body_start)?;
        let mut buffers = Vec::new();
        let mut dependencies = HashMap::<u32, Vec<(usize, usize)>>::new();
        for signal in signals {
            let recipe = self
                .recipes
                .get(signal.index())
                .ok_or_else(|| self.wire.malformed("invalid private recipe index"))?;
            let (_, _, wkt) = self
                .types
                .scalar(recipe.scalar_type)
                .ok_or_else(|| self.wire.malformed("non-scalar recipe encoding"))?;
            let slot = buffers.len();
            for (position, &id) in recipe.ids.iter().enumerate() {
                dependencies.entry(id).or_default().push((slot, position));
            }
            let width = match recipe.encoding {
                Encoding::Bits { width } => width as usize,
                Encoding::Real => 0,
                _ => return Err(self.wire.unsupported("recipe value class")),
            };
            buffers.push(Buffer {
                index: signal.index(),
                digits: vec![b'0'; width],
                real: 0.0,
                required: recipe.ids.len(),
                known: 0,
                wkt,
                vector: recipe.vector,
            });
        }
        #[cfg(test)]
        {
            self.assembly_bytes = buffers
                .iter()
                .map(|b| b.digits.len() + usize::from(b.digits.is_empty()) * 8)
                .sum();
        }
        let mut known = HashSet::new();
        let mut marks = vec![false; buffers.len()];
        let mut touched = Vec::new();
        let (flow, _) = body::walk(&mut self.wire, &self.types, &self.active, end, |event| {
            match event {
                body::Event::Scalar { id, value } => {
                    #[cfg(test)]
                    {
                        self.records_read += 1;
                    }
                    if let Some(positions) = dependencies.get(&id) {
                        let first = known.insert(id);
                        for &(slot, position) in positions {
                            buffers[slot].update(position, value, first);
                            if !marks[slot] {
                                marks[slot] = true;
                                touched.push(slot);
                            }
                        }
                    }
                }
                body::Event::ListEnd(time) => {
                    for slot in touched.drain(..) {
                        marks[slot] = false;
                        let buffer = &buffers[slot];
                        if buffer.known == buffer.required
                            && let ControlFlow::Break(value) = visitor(Notification::Value {
                                index: buffer.index,
                                time,
                                value: buffer.value(),
                            })
                        {
                            return ControlFlow::Break(value);
                        }
                    }
                }
                body::Event::Tick(time) => {
                    if let ControlFlow::Break(value) = visitor(Notification::Tick(time)) {
                        return ControlFlow::Break(value);
                    }
                }
            }
            ControlFlow::Continue(())
        })?;
        Ok(flow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ScanRef, TimeRange};

    #[test]
    fn early_break_has_a_scalar_read_budget_and_bounded_assembly() {
        for length in [10, 10000] {
            let mut wave =
                crate::open_bytes("budget.ghw", test_bytes::recording(length).into()).unwrap();
            let wide = wave.hierarchy().signal("top.wide").unwrap();
            let low = wide.slice(0, 0).unwrap();
            let signals = [wide, low, wide, low];
            let result = wave
                .scan(&signals, TimeRange::all(), |record| {
                    if let ScanRef::Change { time, .. } = record
                        && time.ticks() == 1
                    {
                        ControlFlow::Break(7)
                    } else {
                        ControlFlow::Continue(())
                    }
                })
                .unwrap();
            assert_eq!(result, ControlFlow::Break(7));
            let crate::backends::Reader::Ghw(reader) = &wave.reader else {
                panic!("GHW");
            };
            assert_eq!(
                reader.records_read, 5,
                "four snapshot scalars and one selected change; no unrelated suffix"
            );
            assert_eq!(
                reader.assembly_bytes, 3,
                "one whole recipe despite projections and duplicates"
            );
        }
    }

    #[test]
    fn binary64_reads_cross_tiny_wire_buffers() {
        use std::io::{BufReader, Cursor};
        for capacity in [1, 2] {
            let (mut reader, hierarchy, _) = Reader::open(
                Box::new(BufReader::with_capacity(
                    capacity,
                    Cursor::new(test_bytes::real_recording(true, 1, 0x7ff8000000001234)),
                )),
                "real.ghw".into(),
            )
            .unwrap();
            reader.wire = reader.wire.with_buffer_capacity(capacity);
            let r = hierarchy.signal("top.r").unwrap();
            let mut values = Vec::new();
            let flow = reader
                .read(&[r], Time::from_ticks(4), |event| {
                    if let Notification::Value {
                        time,
                        value: ValueRef::Real(real),
                        ..
                    } = event
                    {
                        values.push((time.ticks(), real.to_bits()));
                    }
                    ControlFlow::<()>::Continue(())
                })
                .unwrap();
            assert!(flow.is_continue());
            assert_eq!(values, [(3, 0x8000000000000000), (4, 0x7ff8000000001234)]);
        }
    }

    #[test]
    fn batch_shares_decoding_and_fresh_reads_survive_unwinding() {
        let mut wave = crate::open_bytes("reuse.ghw", test_bytes::recording(0).into()).unwrap();
        let wide = wave.hierarchy().signal("top.wide").unwrap();
        let low = wide.slice(0, 0).unwrap();
        let _ = wave
            .samples(&[wide, low, wide], Time::from_ticks(4))
            .unwrap();
        let crate::backends::Reader::Ghw(reader) = &wave.reader else {
            panic!("GHW");
        };
        assert_eq!(reader.records_read, 9);
        assert_eq!(reader.assembly_bytes, 3);
        let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = wave.scan(&[wide], TimeRange::all(), |_| -> ControlFlow<()> {
                panic!("visitor");
            });
        }));
        assert!(stopped.is_err());
        let sample = wave.sample(wide, Time::from_ticks(1)).unwrap();
        assert!(
            matches!(sample, crate::Sample::Value { value: crate::Value::Bits(bits), .. } if bits.as_ref().to_string()=="101")
        );
    }

    #[test]
    fn late_io_failure_does_not_publish_an_unfinished_tick() {
        use std::io::{self, BufRead, Cursor, Read, Seek, SeekFrom};
        use std::sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        };
        struct Failing {
            input: Cursor<Vec<u8>>,
            cutoff: Arc<AtomicU64>,
        }
        impl Read for Failing {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                let bytes = self.fill_buf()?;
                let n = output.len().min(bytes.len());
                output[..n].copy_from_slice(&bytes[..n]);
                self.consume(n);
                Ok(n)
            }
        }
        impl BufRead for Failing {
            fn fill_buf(&mut self) -> io::Result<&[u8]> {
                let remaining = self
                    .cutoff
                    .load(Ordering::Relaxed)
                    .checked_sub(self.input.position())
                    .ok_or_else(|| io::Error::other("injected read failure"))?;
                if remaining == 0 {
                    return Err(io::Error::other("injected read failure"));
                }
                let bytes = self.input.fill_buf()?;
                Ok(&bytes[..bytes
                    .len()
                    .min(usize::try_from(remaining).unwrap_or(usize::MAX))])
            }
            fn consume(&mut self, n: usize) {
                self.input.consume(n);
            }
        }
        impl Seek for Failing {
            fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
                self.input.seek(from)
            }
        }
        let data = test_bytes::recording(0);
        let mut wave = crate::open_bytes("failing.ghw", data.clone().into()).unwrap();
        let cutoff = Arc::new(AtomicU64::new(u64::MAX));
        let fail_at = data.windows(4).position(|b| b == b"ESN\0").unwrap() + 4 + 4 + 2;
        let (reader, _, _) = Reader::open(
            Box::new(Failing {
                input: Cursor::new(data),
                cutoff: cutoff.clone(),
            }),
            "failing.ghw".into(),
        )
        .unwrap();
        // The diagnostic input has exactly the same declared recipes.
        wave.reader = crate::backends::Reader::Ghw(Box::new(reader));
        let wide = wave.hierarchy().signal("top.wide").unwrap();
        cutoff.store(fail_at as u64, Ordering::Relaxed);
        let mut delivered = 0;
        let result = wave.scan(&[wide], TimeRange::all(), |_| {
            delivered += 1;
            ControlFlow::<()>::Continue(())
        });
        assert!(matches!(result, Err(crate::Error::Io(_))));
        assert_eq!(delivered, 0);
        cutoff.store(u64::MAX, Ordering::Relaxed);
        let sample = wave.sample(wide, Time::from_ticks(3)).unwrap();
        assert!(
            matches!(sample, crate::Sample::Value { value: crate::Value::Bits(bits), .. } if bits.as_ref().to_string()=="111")
        );
    }
}
