mod body;
mod hierarchy;
mod index;
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
use std::ops::{ControlFlow, Range};
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
    index: Option<index::Index>,
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

struct Dependency {
    id: u32,
    slot: usize,
    position: usize,
    seen: bool,
}

// The active table and dependencies both use ascending source IDs.
fn dependency_ranges(active: &[(u32, usize)], dependencies: &[Dependency]) -> Vec<Range<usize>> {
    let mut cursor = 0;
    let ranges = active
        .iter()
        .map(|&(id, _)| {
            let start = cursor;
            while dependencies.get(cursor).is_some_and(|d| d.id == id) {
                cursor += 1;
            }
            start..cursor
        })
        .collect();
    debug_assert_eq!(cursor, dependencies.len());
    ranges
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
        let mut index = index::Index::new(declarations.active.len());
        let (_, time_span) = body::walk(
            &mut wire,
            &types,
            &declarations.active,
            Time::from_ticks(u64::MAX),
            |event| {
                if let body::Event::Scalar {
                    rank, time, offset, ..
                } = event
                    && let Some(positions) = &mut index
                    && !positions.push(rank, time, offset)
                {
                    index = None;
                }
                ControlFlow::<()>::Continue(())
            },
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
                index,
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
        let mut dependencies = Vec::new();
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
                dependencies.push(Dependency {
                    id,
                    slot,
                    position,
                    seen: false,
                });
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
        dependencies.sort_unstable_by_key(|dependency| dependency.id);
        #[cfg(test)]
        {
            self.assembly_bytes = buffers
                .iter()
                .map(|b| b.digits.len() + usize::from(b.digits.is_empty()) * 8)
                .sum();
        }
        let mut marks = vec![false; buffers.len()];
        let mut touched = Vec::new();
        let mut ids = dependencies
            .iter()
            .map(|dependency| dependency.id)
            .collect::<Vec<_>>();
        ids.dedup(); // Dependencies are already sorted by source ID.
        let broad = ids.len() >= self.active.len().div_ceil(2);
        let routes = broad.then(|| dependency_ranges(&self.active, &dependencies));
        // The block permits the closure attribute on MSRV. Force scalar-event
        // specialization in the walker instead of a per-record callback.
        let consume = {
            #[inline(always)]
            |event| {
                match event {
                    body::Event::Scalar {
                        id, rank, value, ..
                    } => {
                        #[cfg(test)]
                        {
                            self.records_read += 1;
                        }
                        let range = if let Some(routes) = &routes {
                            routes[rank].clone()
                        } else {
                            let start =
                                dependencies.partition_point(|dependency| dependency.id < id);
                            let mut end = start;
                            while dependencies.get(end).is_some_and(|d| d.id == id) {
                                end += 1;
                            }
                            start..end
                        };
                        if !range.is_empty() {
                            // The first entry owns this source scalar's seen flag.
                            let first =
                                !std::mem::replace(&mut dependencies[range.start].seen, true);
                            for dependency in &dependencies[range] {
                                let slot = dependency.slot;
                                buffers[slot].update(dependency.position, value, first);
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
            }
        };
        // Broad selections keep the sequential path without a many-chain merge.
        if let Some(index) = &self.index
            && !broad
        {
            index.read(
                &mut self.wire,
                &self.types,
                &self.active,
                &ids,
                end,
                consume,
            )
        } else {
            let (flow, _) = body::walk(&mut self.wire, &self.types, &self.active, end, consume)?;
            Ok(flow)
        }
    }
}

#[cfg(test)]
mod tests;
