use std::{collections::HashMap, sync::Arc};

use super::*;
use crate::{CompositionError, Encoding};

pub(super) struct Definition {
    pub(super) parts: Arc<[Signal]>,
    width: u32,
}

#[derive(Default)]
pub(super) struct Compositions {
    definitions: Vec<Definition>,
    indices: HashMap<Arc<[Signal]>, usize>,
}

impl Waveform {
    /// Creates a static bit composition in most-significant-part-first order.
    ///
    /// Each part must be a nonempty bit signal from this waveform. Parts may
    /// be source signals, slices, earlier compositions or their slices. Parts
    /// retain their internal bit order. The total width must fit in u32. Order
    /// alone sets contiguous result positions, with no holes or overlaps.
    /// Repeated parts retain each placement. No source declaration is created.
    ///
    /// If any part has an observed value, its bits are retained and Missing
    /// parts are filled with X. An observed all-X value counts as observed.
    /// If every part is Missing, the composition is Missing. Slices select
    /// from that assembled value. These rules also apply at nested composition
    /// boundaries. Direct source reads still distinguish Missing from X.
    ///
    /// The result works with ordinary sample, scan, query and Trace operations.
    /// They observe completed ticks and report net changes of the computed
    /// value or its selected slice, not intermediate part writes. A result slice
    /// retains every part's availability dependency, including discarded bits.
    ///
    /// Identical ordered parts in this waveform return equal handles. A single
    /// part returns that handle. Different layouts are not equated by observed
    /// values or by algebraic rewrites. The waveform owns each distinct immutable
    /// definition until it drops; the input slice can be discarded. A copied
    /// handle does not keep the waveform alive. Create definitions before
    /// borrowing the waveform through a Selection.
    ///
    /// Foreign handles return [`Error::InvalidSignal`], Unsupported encodings
    /// return [`Error::UnsupportedSignal`], and other invalid requests return
    /// [`Error::InvalidComposition`]. Validation does not read values and a
    /// failed request registers no definition. Reader failures remain errors.
    ///
    /// ```
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use ondas::{Sample, Time, Value};
    /// let input = b"$var wire 4 h hi $end $var wire 4 l lo $end \
    ///     $enddefinitions $end #2 b0110 l #3 bxxxx h";
    /// let mut wave = ondas::open_bytes("parts.vcd", input.as_slice().into())?;
    /// let hi = wave.hierarchy().signal("hi")?;
    /// let lo = wave.hierarchy().signal("lo")?;
    /// let bus = wave.concat(&[hi, lo])?;
    /// assert_eq!(bus, wave.concat(&[hi, lo])?);
    /// let high = bus.slice(7, 4)?;
    /// let nested = wave.concat(&[high, hi])?;
    /// let samples = wave.samples(&[bus, high, nested, hi], Time::from_ticks(2))?;
    /// let texts: Vec<_> = samples.iter().map(|sample| match sample {
    ///     Sample::Value { value: Value::Bits(bits), .. } => Some(bits.as_ref().to_string()),
    ///     Sample::Missing { .. } => None,
    ///     _ => unreachable!(),
    /// }).collect();
    /// assert_eq!(texts, [Some("xxxx0110".into()), Some("xxxx".into()),
    ///     Some("xxxxxxxx".into()), None]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn concat(&mut self, parts: &[Signal]) -> Result<Signal> {
        if parts.is_empty() {
            return Err(CompositionError::Empty.into());
        }
        let mut width = 0u32;
        for (part, &signal) in parts.iter().enumerate() {
            self.validate(signal)?;
            if signal.encoding() == Encoding::Unsupported {
                return Err(Error::UnsupportedSignal { signal });
            }
            let part_width = signal
                .width()
                .ok_or(CompositionError::NotBits { part, signal })?;
            if part_width == 0 {
                return Err(CompositionError::EmptyPart { part, signal }.into());
            }
            width = width
                .checked_add(part_width)
                .ok_or(CompositionError::WidthOverflow)?;
        }
        if parts.len() == 1 {
            return Ok(parts[0]);
        }
        let index = if let Some(&index) = self.compositions.indices.get(parts) {
            index
        } else {
            let index = self.compositions.definitions.len();
            let parts = Arc::<[Signal]>::from(parts);
            self.compositions.indices.insert(parts.clone(), index);
            self.compositions
                .definitions
                .push(Definition { parts, width });
            index
        };
        Ok(self.hierarchy.computed_signal(index, width))
    }

    pub(crate) fn validate(&self, signal: Signal) -> Result<usize> {
        if !signal.is_computed() {
            return self.hierarchy.validate(signal);
        }
        let encoding = self
            .compositions
            .definitions
            .get(signal.index())
            .map(|definition| Encoding::Bits {
                width: definition.width,
            });
        self.hierarchy.validate_encoding(signal, encoding)
    }

    pub(crate) fn composition_parts(&self, signal: Signal) -> &[Signal] {
        &self.compositions.definitions[signal.index()].parts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_is_checked_before_registration() {
        let mut wave = Waveform::memory(
            vec![
                Encoding::Bits { width: 0 },
                Encoding::Bits { width: u32::MAX },
                Encoding::Bits { width: 1 },
                Encoding::Real,
                Encoding::Unsupported,
            ],
            vec![],
            None,
        );
        let handles: Vec<_> = wave.hierarchy().signals().collect();
        assert!(matches!(
            wave.concat(&[]),
            Err(Error::InvalidComposition(CompositionError::Empty))
        ));
        assert!(matches!(
            wave.concat(&[handles[0]]),
            Err(Error::InvalidComposition(CompositionError::EmptyPart {
                part: 0,
                ..
            }))
        ));
        assert!(matches!(
            wave.concat(&[handles[1], handles[2]]),
            Err(Error::InvalidComposition(CompositionError::WidthOverflow))
        ));
        assert!(matches!(
            wave.concat(&[handles[2], handles[3]]),
            Err(Error::InvalidComposition(CompositionError::NotBits {
                part: 1,
                ..
            }))
        ));
        assert!(matches!(
            wave.concat(&[handles[4]]),
            Err(Error::UnsupportedSignal { .. })
        ));
        for encoding in [Encoding::Real, Encoding::String, Encoding::Event] {
            let mut other = Waveform::memory(vec![encoding], vec![], None);
            let handle = other.hierarchy().signals().next().unwrap();
            assert!(matches!(
                other.concat(&[handle]),
                Err(Error::InvalidComposition(CompositionError::NotBits { .. }))
            ));
            assert!(matches!(
                crate::BitRange::new(0, 0).slice(handle, 0, 0),
                Err(crate::SliceError::NotBits)
            ));
        }
        assert!(wave.compositions.definitions.is_empty());
        let other = Waveform::memory(vec![Encoding::Bits { width: 1 }], vec![], None);
        let foreign = other.hierarchy().signals().next().unwrap();
        assert!(matches!(
            wave.concat(&[foreign]),
            Err(Error::InvalidSignal { .. })
        ));
        let bus = wave.concat(&[handles[2], handles[2]]).unwrap();
        assert_eq!(bus, wave.concat(&[handles[2], handles[2]]).unwrap());
        assert_eq!(wave.concat(&[bus]).unwrap(), bus);
        assert_eq!(wave.compositions.definitions.len(), 1);
        assert_eq!(bus.width(), Some(2));
        assert_eq!(wave.hierarchy().aliases(bus).unwrap().count(), 0);
        assert!(matches!(
            other.hierarchy().aliases(bus),
            Err(Error::InvalidSignal { .. })
        ));
    }
}
