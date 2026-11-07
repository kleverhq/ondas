mod body;
mod hierarchy;
mod identity;
mod index;
#[cfg(test)]
#[path = "../../../tests/support/ghw_bytes.rs"]
mod test_bytes;
mod types;
mod validated;
mod wire;

use super::Input;
use crate::{BitsRef, Encoding, Hierarchy, Metadata, Result, Signal, Time, ValueRef};
use identity::Identity;
use std::ops::{ControlFlow, Range};
use std::sync::Arc;
use types::Types;
use validated::{CACHE_BYTES, Validated};
use wire::Wire;

/// Why one logical open runs. A metadata-only request must not leave a parked
/// validated hierarchy behind, so it never parks one of its own and releases the
/// one it reused.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Purpose {
    Waveform,
    Metadata,
}

/// One parked validated source, shared by every logical open of this backend.
struct Cache {
    slot: std::sync::Mutex<Option<Arc<Validated>>>,
    #[cfg(test)]
    hits: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    parses: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    #[allow(dead_code)]
    skipped: std::sync::atomic::AtomicUsize,
}

static CACHE: Cache = Cache::new();

impl Cache {
    const fn new() -> Self {
        Self {
            slot: std::sync::Mutex::new(None),
            #[cfg(test)]
            hits: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            parses: std::sync::atomic::AtomicUsize::new(0),
            #[cfg(test)]
            skipped: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Arc<Validated>>> {
        self.slot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Identity of the parked source, if any.
    fn parked_identity(&self) -> Option<identity::ContentIdentity> {
        self.slot()
            .as_ref()
            .and_then(|validated| validated.identity)
    }

    /// The parked source when its content identity matches.
    fn take_matching(&self, identity: &identity::ContentIdentity) -> Option<Arc<Validated>> {
        self.slot()
            .as_ref()
            .filter(|validated| validated.identity == Some(*identity))
            .map(Arc::clone)
    }

    /// Park one validated source. A source above the byte bound is not parked, and
    /// a replaced source is dropped outside the lock.
    fn park(&self, candidate: Arc<Validated>) {
        if candidate.retained_bytes > CACHE_BYTES {
            return;
        }
        let replaced = self.slot().replace(candidate);
        drop(replaced);
    }

    /// Release the parked source.
    fn clear(&self) {
        let parked = self.slot().take();
        drop(parked);
    }
}

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
    validated: Arc<Validated>,
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
    #[inline(always)]
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
        Self::open_with(input, source_name, Purpose::Waveform, &CACHE)
    }

    /// Read owned source metadata with the same validation and reuse path, but
    /// without leaving a parked validated hierarchy behind.
    pub(crate) fn metadata(input: Box<dyn Input>, source_name: String) -> Result<Metadata> {
        let (reader, _, metadata) = Self::open_with(input, source_name, Purpose::Metadata, &CACHE)?;
        drop(reader);
        Ok(metadata)
    }

    fn open_with(
        input: Box<dyn Input>,
        source_name: String,
        purpose: Purpose,
        cache: &Cache,
    ) -> Result<(Self, Hierarchy, Metadata)> {
        let mut input = input;
        if let Some(parked) = cache.parked_identity() {
            // A parked source is only usable for byte-identical content, and the
            // length is part of that identity: a different length is a certain miss
            // and needs no scan. Otherwise the scan reads the whole current input; a
            // failure here is an optional miss and the parser below reports whatever
            // error it reaches.
            let length = input.seek(std::io::SeekFrom::End(0))?;
            input.seek(std::io::SeekFrom::Start(0))?;
            if length == parked.length
                && identity::scan(&mut *input).is_ok_and(|current| current == parked)
                && let Some(validated) = cache.take_matching(&parked)
            {
                let mut wire = Wire::new(input)?;
                if wire.len == parked.length {
                    #[cfg(test)]
                    cache
                        .hits
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    wire.version = validated.version;
                    wire.big_endian = validated.big_endian;
                    let metadata = Metadata {
                        source_name,
                        timescale: None,
                        time_span: validated.time_span,
                        writer: None,
                        date: None,
                        comments: Vec::new(),
                    };
                    let hierarchy = validated.hierarchy.reopened();
                    let reader = Self {
                        wire,
                        validated,
                        #[cfg(test)]
                        records_read: 0,
                        #[cfg(test)]
                        assembly_bytes: 0,
                    };
                    if purpose == Purpose::Metadata {
                        // A metadata-only request must not keep a parked hierarchy.
                        cache.clear();
                    }
                    return Ok((reader, hierarchy, metadata));
                }
                // The length changed under the identity, so this is a miss.
                input = wire.into_input();
            }
        }
        Self::parse(input, source_name, purpose, cache)
    }

    /// Cold or cache-miss parse: validation runs over a watched input, so the
    /// identity describes exactly the bytes this parse consumed.
    fn parse(
        input: Box<dyn Input>,
        source_name: String,
        purpose: Purpose,
        cache: &Cache,
    ) -> Result<(Self, Hierarchy, Metadata)> {
        #[cfg(test)]
        cache
            .parses
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let identity = Identity::new();
        let mut wire = Wire::new(identity.watch(input))?;
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
                let Some(positions) = &mut index else {
                    return ControlFlow::<()>::Continue(());
                };
                match event {
                    body::Event::Scalar {
                        rank,
                        time,
                        offset,
                        cyclic,
                        ..
                    } => {
                        if !positions.push(rank, time, offset, cyclic) {
                            index = None;
                        }
                    }
                    body::Event::ListEnd(_) => positions.list_end(),
                    body::Event::Tick(_) => {}
                }
                ControlFlow::<()>::Continue(())
            },
        )?;
        let complete = match &mut index {
            Some(positions) => positions.finish(),
            None => true,
        };
        if !complete {
            // An incomplete representation is discarded: the sequential walker
            // answers instead, and opening validation already finished.
            index = None;
        }
        let version = wire.version;
        let big_endian = wire.big_endian;
        let hierarchy = declarations.hierarchy;
        let recipes = declarations.recipes;
        let recipe_slots = recipes.capacity();
        let active = declarations.active;
        let active_slots = active.capacity();
        // The parser consumes the whole input, so the proof covers what it validated.
        let proven = identity.finish(wire.len);
        let retained_bytes = validated::retained_bytes(
            &types,
            &recipes,
            recipe_slots,
            active_slots,
            &hierarchy,
            index.as_ref(),
        );
        let validated = Arc::new(Validated {
            identity: proven,
            hierarchy: hierarchy.clone(),
            types,
            recipes,
            active,
            body_start,
            version,
            big_endian,
            time_span,
            index,
            retained_bytes,
        });
        if proven.is_some() && purpose == Purpose::Waveform {
            cache.park(Arc::clone(&validated));
        } else if purpose == Purpose::Metadata {
            // A metadata-only request must not keep a parked hierarchy.
            cache.clear();
        }
        let metadata = Metadata {
            source_name,
            timescale: None,
            time_span: validated.time_span,
            writer: None,
            date: None,
            comments: Vec::new(),
        };
        let reader = Self {
            wire,
            validated,
            #[cfg(test)]
            records_read: 0,
            #[cfg(test)]
            assembly_bytes: 0,
        };
        Ok((reader, hierarchy, metadata))
    }

    /// Query diagnostics of the completed index, for focused tests.
    #[cfg(test)]
    pub(super) fn work(&self) -> Option<index::Work> {
        self.validated.index.as_ref().map(index::Index::work)
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
        self.wire.seek(self.validated.body_start)?;
        let mut buffers = Vec::new();
        let mut dependencies = Vec::new();
        for signal in signals {
            let recipe = self
                .validated
                .recipes
                .get(signal.index())
                .ok_or_else(|| self.wire.malformed("invalid private recipe index"))?;
            let (_, _, wkt) = self
                .validated
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
        let broad = ids.len() >= self.validated.active.len().div_ceil(2);
        let routes = broad.then(|| dependency_ranges(&self.validated.active, &dependencies));
        // The block permits the closure attribute on MSRV. Force scalar-event
        // specialization in the walker instead of a per-record callback.
        let mut consume = {
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
                            let first = !dependencies[range.start].seen;
                            if first {
                                dependencies[range.start].seen = true;
                            }
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
        if let Some(index) = &self.validated.index
            && !broad
            && let Some(flow) = index.read(
                &mut self.wire,
                &self.validated.types,
                &self.validated.active,
                &ids,
                end,
                &mut consume,
            )?
        {
            return Ok(flow);
        }
        let (flow, _) = body::walk(
            &mut self.wire,
            &self.validated.types,
            &self.validated.active,
            end,
            consume,
        )?;
        Ok(flow)
    }
}

#[cfg(test)]
mod reuse_tests;
#[cfg(test)]
mod tests;
