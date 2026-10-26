use super::{Reader, body, container::ARENA_SPAN, malformed};
use crate::{Result, Sample, Signal, Time, Value};
use std::ops::ControlFlow;

impl Reader {
    // Certify final per-tick values with one small storage window and at most
    // two page cursors. Incomplete ticks use the chronological reference path.
    pub(crate) fn sample_bits(
        &mut self,
        signals: &[Signal],
        time: Time,
    ) -> Result<Option<Vec<Sample>>> {
        for signal in signals {
            let recipe = &self.recipes[signal.index()];
            if recipe.len == 0 || recipe.len > ARENA_SPAN as usize || signal.is_slice() {
                return Ok(None);
            }
        }
        let mut samples = Vec::new();
        for &signal in signals {
            let recipe = &self.recipes[signal.index()];
            let end = recipe
                .start
                .checked_add(recipe.len as u64)
                .ok_or_else(|| malformed("point address overflow"))?;
            let lo = usize::try_from(recipe.start / ARENA_SPAN)
                .map_err(|_| malformed("point arena overflow"))?;
            let hi = usize::try_from((end - 1) / ARENA_SPAN)
                .map_err(|_| malformed("point arena overflow"))?;
            if hi >= self.container.arenas.len() {
                return Err(malformed("point value exceeds handle space"));
            }
            let selected: Vec<_> = (lo..=hi).collect();
            let mut raw = vec![0; recipe.len];
            let mut known = vec![false; recipe.len];
            let mut known_count = 0;
            let mut value: Option<Value> = None;
            let mut established = None;
            let mut changed_at = None;
            let mut digits = Vec::new();
            let result = body::backwards(
                &mut self.source,
                &self.container.arenas,
                &selected,
                time.ticks(),
                |event| {
                    match event {
                        body::Event::Write { address, data } => {
                            let start = address.max(recipe.start);
                            let stop = (address + data.len() as u64).min(end);
                            if start >= stop {
                                return Ok(ControlFlow::Continue(()));
                            }
                            // Reverse order: the last source write supplies each byte.
                            for i in (start - recipe.start) as usize..(stop - recipe.start) as usize
                            {
                                if !known[i] {
                                    raw[i] = data[(recipe.start + i as u64 - address) as usize];
                                    known[i] = true;
                                    known_count += 1;
                                }
                            }
                        }
                        body::Event::Tick(tick) => {
                            if known_count == 0 {
                                return Ok(ControlFlow::Continue(()));
                            }
                            if known_count != raw.len() {
                                return Ok(ControlFlow::Break(false));
                            }
                            let decoded = recipe.codec.decode(&raw, recipe.shift, &mut digits)?;
                            if let Some(current) = &value {
                                if !current.as_ref().same_value(decoded) {
                                    changed_at = established.map(Time::from_ticks);
                                    return Ok(ControlFlow::Break(true));
                                }
                            } else {
                                value = Some(decoded.to_owned());
                            }
                            established = Some(tick);
                            known.fill(false);
                            known_count = 0;
                        }
                    }
                    Ok(ControlFlow::Continue(()))
                },
            )?;
            if result.is_none() || result == Some(ControlFlow::Break(false)) {
                return Ok(None);
            }
            samples.push(match value {
                Some(value) => Sample::Value {
                    signal,
                    value,
                    changed_at,
                },
                None => Sample::Missing { signal },
            });
        }
        Ok(Some(samples))
    }
}
