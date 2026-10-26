mod body;
mod container;
mod debug;
mod hierarchy;
mod model;
mod point;
mod types;
mod wire;

use crate::{Error, Format};

use super::Input;
use crate::{Hierarchy, Metadata, Result, Signal, Time, TimeSpan, TimeUnit, Timescale, ValueRef};
use container::{ARENA_SPAN, Container};
use hierarchy::Recipe;
use std::{collections::BTreeMap, ops::ControlFlow};
use wire::Source;

pub(crate) struct Reader {
    source: Source,
    container: Container,
    recipes: Vec<Recipe>,
}

struct Buffer<'a> {
    index: usize,
    recipe: &'a Recipe,
    raw: Vec<u8>,
    known: Vec<bool>,
    known_count: usize,
    digits: Vec<u8>,
    dirty: bool,
}

impl Reader {
    pub(crate) fn open(
        input: Box<dyn Input>,
        source_name: String,
    ) -> Result<(Self, Hierarchy, Metadata)> {
        let mut source = Source::new(input)?;
        let container = Container::read(&mut source)?;
        let types = types::read(&mut source, container.rtti)?;
        let debug = debug::read(&mut source, &container, types.len())?;
        let (hierarchy, recipes) = hierarchy::read(&debug, &types)?;
        let precision = (debug.precision + 15) as usize;
        let unit = [
            TimeUnit::Femtosecond,
            TimeUnit::Picosecond,
            TimeUnit::Nanosecond,
            TimeUnit::Microsecond,
            TimeUnit::Millisecond,
            TimeUnit::Second,
        ][precision / 3];
        let metadata = Metadata {
            source_name,
            timescale: Some(Timescale::new(10_u32.pow((precision % 3) as u32), unit)),
            time_span: Some(TimeSpan::new(
                Time::from_ticks(0),
                Time::from_ticks(container.end),
            )),
            writer: None,
            date: None,
            comments: Vec::new(),
        };
        Ok((
            Self {
                source,
                container,
                recipes,
            },
            hierarchy,
            metadata,
        ))
    }

    pub(crate) fn read<B>(
        &mut self,
        signals: &[Signal],
        end: Time,
        visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        self.read_from(signals, Time::ZERO, end, visitor)
    }

    pub(crate) fn read_from<B>(
        &mut self,
        signals: &[Signal],
        from: Time,
        end: Time,
        mut visitor: impl for<'v> FnMut(usize, Time, ValueRef<'v>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        if from > end {
            return Ok(ControlFlow::Continue(()));
        }
        let mut buffers = Vec::new();
        let mut routes: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for signal in signals {
            let recipe = self
                .recipes
                .get(signal.index())
                .ok_or_else(|| malformed("private recipe index"))?;
            if recipe.len == 0 {
                return Err(malformed("empty recorded value"));
            }
            let last = recipe
                .start
                .checked_add(recipe.len as u64 - 1)
                .ok_or_else(|| malformed("value address overflow"))?;
            let lo = usize::try_from(recipe.start / ARENA_SPAN)
                .map_err(|_| malformed("arena index overflow"))?;
            let hi = usize::try_from(last / ARENA_SPAN)
                .map_err(|_| malformed("arena index overflow"))?;
            if hi >= self.container.arenas.len() {
                return Err(malformed("value exceeds handle space"));
            }
            let slot = buffers.len();
            for arena in lo..=hi {
                routes.entry(arena).or_default().push(slot);
            }
            let mut raw = Vec::new();
            raw.try_reserve_exact(recipe.len)
                .map_err(|_| malformed("cannot allocate selected value"))?;
            raw.resize(recipe.len, 0);
            let mut known = Vec::new();
            known
                .try_reserve_exact(recipe.len)
                .map_err(|_| malformed("cannot allocate selected availability"))?;
            known.resize(recipe.len, false);
            buffers.push(Buffer {
                index: signal.index(),
                recipe,
                raw,
                known,
                known_count: 0,
                digits: Vec::new(),
                dirty: false,
            });
        }
        let arenas: Vec<_> = routes.keys().copied().collect();
        let mut touched = Vec::new();
        body::walk(
            &mut self.source,
            &self.container.arenas,
            &arenas,
            end.ticks(),
            |event| {
                match event {
                    body::Event::Write { address, data } => {
                        let arena = (address / ARENA_SPAN) as usize;
                        for &slot in &routes[&arena] {
                            let b = &mut buffers[slot];
                            let start = b.recipe.start;
                            if b.recipe.time_parameter && address != start {
                                continue;
                            }
                            let lo = address.max(start);
                            let hi = (address + data.len() as u64).min(start + b.raw.len() as u64);
                            if lo >= hi {
                                continue;
                            }
                            let range = (lo - start) as usize..(hi - start) as usize;
                            b.raw[range.clone()].copy_from_slice(
                                &data[(lo - address) as usize..(hi - address) as usize],
                            );
                            if b.known_count != b.raw.len() {
                                for known in &mut b.known[range] {
                                    if !*known {
                                        b.known_count += 1;
                                        *known = true;
                                    }
                                }
                            }
                            if !b.dirty {
                                b.dirty = true;
                                touched.push(slot);
                            }
                        }
                    }
                    body::Event::Tick(time) => {
                        for slot in touched.drain(..) {
                            let b = &mut buffers[slot];
                            b.dirty = false;
                            // Reconstruct partial storage writes before a certified
                            // boundary, but leave its normalized state to the caller.
                            if time < from.ticks() || b.known_count != b.raw.len() {
                                continue;
                            }
                            let value =
                                b.recipe
                                    .codec
                                    .decode(&b.raw, b.recipe.shift, &mut b.digits)?;
                            if let ControlFlow::Break(value) =
                                visitor(b.index, Time::from_ticks(time), value)
                            {
                                return Ok(ControlFlow::Break(value));
                            }
                        }
                    }
                }
                Ok(ControlFlow::Continue(()))
            },
        )
    }
}

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
