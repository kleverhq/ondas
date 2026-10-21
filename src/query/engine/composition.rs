use super::*;

pub(in crate::query) enum Computation {
    Concat {
        slot: usize,
        inputs: Vec<(usize, u32)>,
        width: u32,
    },
    Slice {
        slot: usize,
        input: usize,
        signal: Signal,
    },
}

pub(super) struct Plan {
    pub(super) retained: Vec<Signal>,
    pub(super) indices: Vec<usize>,
    pub(super) bases: Vec<Signal>,
    pub(super) groups: HashMap<usize, Vec<usize>>,
    pub(super) computations: Vec<Computation>,
}

impl Plan {
    pub(super) fn new(waveform: &Waveform, signals: &[Signal]) -> Result<Self> {
        let mut plan = Self {
            retained: Vec::new(),
            indices: Vec::with_capacity(signals.len()),
            bases: Vec::new(),
            groups: HashMap::new(),
            computations: Vec::new(),
        };
        let mut indices = HashMap::new();
        let mut pending = Vec::new();
        for &selected in signals {
            pending.push((selected, false));
            while let Some((signal, expanded)) = pending.pop() {
                if indices.contains_key(&signal) {
                    continue;
                }
                waveform.validate(signal)?;
                if signal.encoding() == Encoding::Unsupported {
                    return Err(Error::UnsupportedSignal { signal });
                }
                if signal.is_computed() && !expanded {
                    pending.push((signal, true));
                    if signal.is_slice() {
                        pending.push((signal.base(), false));
                    } else {
                        pending.extend(
                            waveform
                                .composition_parts(signal)
                                .iter()
                                .rev()
                                .map(|&part| (part, false)),
                        );
                    }
                    continue;
                }
                let slot = plan.retained.len();
                indices.insert(signal, slot);
                plan.retained.push(signal);
                if signal.is_computed() {
                    if signal.is_slice() {
                        plan.computations.push(Computation::Slice {
                            slot,
                            input: indices[&signal.base()],
                            signal,
                        });
                    } else {
                        plan.computations.push(Computation::Concat {
                            slot,
                            inputs: waveform
                                .composition_parts(signal)
                                .iter()
                                .map(|part| (indices[part], part.width().expect("bit operand")))
                                .collect(),
                            width: signal.width().expect("validated composition"),
                        });
                    }
                } else {
                    let entries = plan.groups.entry(signal.index()).or_insert_with(|| {
                        plan.bases.push(signal.base());
                        Vec::new()
                    });
                    entries.push(slot);
                }
            }
            plan.indices.push(indices[&selected]);
        }
        Ok(plan)
    }
}

fn current(slot: &Slot) -> Option<ValueRef<'_>> {
    slot.pending
        .as_ref()
        .or_else(|| slot.state.as_ref().map(|state| &state.value))
        .map(Value::as_ref)
}

impl Computation {
    pub(super) fn complete(&self, slots: &mut [Slot], active: &mut Vec<usize>) {
        let (slot, value) = match self {
            Self::Concat {
                slot,
                inputs,
                width,
            } => {
                if !inputs
                    .iter()
                    .any(|&(input, _)| slots[input].pending.is_some())
                {
                    return;
                }
                let mut observed = false;
                let mut bytes = Vec::with_capacity(*width as usize);
                for &(input, width) in inputs {
                    if let Some(ValueRef::Bits(bits)) = current(&slots[input]) {
                        observed = true;
                        bytes.extend_from_slice(bits.ascii());
                    } else {
                        // Each input is a bit-vector slot; absence is Missing.
                        bytes.resize(bytes.len() + width as usize, b'x');
                    }
                }
                if !observed {
                    return;
                }
                (
                    *slot,
                    Value::Bits(
                        crate::BitsRef::from_ascii(&bytes)
                            .expect("validated part bits")
                            .to_owned(),
                    ),
                )
            }
            Self::Slice {
                slot,
                input,
                signal,
            } => {
                let Some(value) = slots[*input].pending.as_ref() else {
                    return;
                };
                (*slot, project(value.as_ref(), *signal).to_owned())
            }
        };
        if slots[slot].pending.is_none() {
            active.push(slot);
        }
        slots[slot].pending = Some(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bit_value(value: &str) -> Value {
        Value::Bits(crate::Bits::from_ascii(value.as_bytes()).unwrap())
    }

    #[test]
    fn nine_states_partial_failure_and_bounded_computed_slots() {
        let mut wave = Waveform::memory(
            vec![Encoding::Bits { width: 9 }, Encoding::Bits { width: 1 }],
            (0..300)
                .flat_map(|t| {
                    [
                        (
                            0,
                            t,
                            bit_value(if t % 2 == 0 { "uzwhl-01x" } else { "01xzhuWl-" }),
                        ),
                        (1, t, bit_value("1")),
                    ]
                })
                .collect(),
            None,
        );
        let sources: Vec<_> = wave.hierarchy().signals().collect();
        let bus = wave
            .concat(&[sources[0], sources[1], sources[0].slice(8, 5).unwrap()])
            .unwrap();
        let cut = bus.slice(12, 3).unwrap();
        let mut selection = wave.select(&[bus, cut, bus]).unwrap();
        assert_eq!(selection.bases.len(), 2);
        let slot_count = selection.retained_signals.len();
        let mut slots = vec![Slot::default(); slot_count];
        let mut ticks = 0;
        let _ = selection
            .read_ticks(Time::from_ticks(299), &mut slots, |t, _, slots| {
                ticks += 1;
                let retained_bytes: usize = slots
                    .iter()
                    .map(|slot| {
                        slot.state.as_ref().map_or(0, |s| match s.value.as_ref() {
                            ValueRef::Bits(b) => b.width() as usize,
                            _ => 0,
                        }) + slot.pending.as_ref().map_or(0, |v| match v.as_ref() {
                            ValueRef::Bits(b) => b.width() as usize,
                            _ => 0,
                        })
                    })
                    .sum();
                assert_eq!(slots.len(), slot_count);
                assert!(retained_bytes <= 2 * (9 + 1 + 4 + 14 + 10));
                assert!(t.ticks() < 300);
                Ok(ControlFlow::<()>::Continue(()))
            })
            .unwrap();
        assert_eq!(ticks, 300);
        drop(selection);
        let sample = wave.sample(bus, Time::ZERO).unwrap();
        let Sample::Value {
            value: Value::Bits(bits),
            ..
        } = sample
        else {
            panic!()
        };
        assert_eq!(bits.as_ref().to_string(), "uzwhl-01x1uzwh");
        let mut failed = Waveform::memory(
            vec![Encoding::Bits { width: 1 }; 2],
            vec![
                (0, 0, bit_value("0")),
                (1, 0, bit_value("1")),
                (0, 1, bit_value("1")),
                (1, 1, bit_value("0")),
                (0, 2, bit_value("0")),
            ],
            Some(4),
        );
        let parts: Vec<_> = failed.hierarchy().signals().collect();
        let computed = failed.concat(&parts).unwrap();
        let mut times = Vec::new();
        let result = failed.scan(&[computed], TimeRange::all(), |record| {
            if let ScanRef::Change { time, .. } = record {
                times.push(time.ticks());
            }
            ControlFlow::<()>::Continue(())
        });
        assert!(result.is_err());
        assert_eq!(times, vec![0]);
        assert!(failed.trace(computed, TimeRange::all()).is_err());
        assert!(failed.sample(computed, Time::ZERO).is_ok());
    }

    #[test]
    fn planning_nested_projections_does_not_recurse() {
        let mut wave = Waveform::memory(
            vec![Encoding::Bits { width: 1 }; 2],
            vec![(0, 0, bit_value("1"))],
            None,
        );
        let parts: Vec<_> = wave.hierarchy().signals().collect();
        let mut bus = wave.concat(&parts).unwrap();
        for _ in 0..2048 {
            bus = wave.concat(&[bus.slice(1, 1).unwrap(), parts[1]]).unwrap();
        }
        let sample = wave.sample(bus, Time::ZERO).unwrap();
        let Sample::Value {
            value: Value::Bits(bits),
            ..
        } = sample
        else {
            panic!()
        };
        assert_eq!(bits.as_ref().to_string(), "1x");
    }
}
