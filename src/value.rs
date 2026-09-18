use std::fmt;

/// A borrowed view of a waveform value.
///
/// Views supplied to query callbacks can reference backend buffers and are valid
/// only during that callback. Use [`Self::to_owned`] to retain a value. Views from
/// [`Value::as_ref`], [`Initial::value`](crate::Initial::value), or
/// [`Change::value`](crate::Change::value) instead borrow owned storage and remain
/// valid for that borrow; not every view is callback-only.
///
/// Bit-vector signedness is a declaration interpretation, not a separate value
/// variant. Persistent histories expose final recorded states at source ticks,
/// with at most one net change per selection entry and tick. Intermediate
/// same-tick excursions are not exposed. Events carry observed per-tick counts,
/// not persistent state or an ordering of individual occurrences.
///
/// Queries detect persistent net changes by comparing all logic states distinctly,
/// strings by exact contents, and reals by their binary64 bit patterns. Signed zeros and
/// different NaN patterns therefore differ. This preserves the representation
/// supplied by the reader, not source precision or NaN payloads already lost
/// during decoding. Numerical equality remains caller policy.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum ValueRef<'a> {
    /// A borrowed bit-vector value whose signedness comes from its declaration.
    Bits(
        /// The bit-vector view.
        BitsRef<'a>,
    ),
    /// A floating-point value.
    Real(
        /// The floating-point payload.
        f64,
    ),
    /// A borrowed string value.
    String(
        /// The string payload.
        &'a str,
    ),
    /// An aggregate of event observations at one tick.
    ///
    /// Scan and trace records have positive counts. Counts reflect reader
    /// observations, including [reader limitations](crate#reader-details),
    /// not events omitted by the producer.
    Event {
        /// The number of observed occurrences at this tick.
        occurrences: u64,
    },
}

/// An owned waveform value suitable for long-term storage.
///
/// [`ValueRef::to_owned`] retains a borrowed value independently of its source;
/// [`Self::as_ref`] borrows this owner's storage. See [`ValueRef`] for persistent
/// value, signedness, and event semantics.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Value {
    /// An owned bit-vector value whose signedness comes from its declaration.
    Bits(
        /// The bit-vector payload.
        Bits,
    ),
    /// A floating-point value.
    Real(
        /// The floating-point payload.
        f64,
    ),
    /// An owned string value.
    String(
        /// The string payload.
        Box<str>,
    ),
    /// An aggregate of event observations at one tick; see [`ValueRef::Event`].
    Event {
        /// The number of observed occurrences at this tick.
        occurrences: u64,
    },
}

impl ValueRef<'_> {
    pub(crate) fn same_value(self, other: ValueRef<'_>) -> bool {
        match (self, other) {
            (Self::Bits(left), ValueRef::Bits(right)) => left.iter_msb().eq(right.iter_msb()),
            (Self::Real(left), ValueRef::Real(right)) => left.to_bits() == right.to_bits(),
            (Self::String(left), ValueRef::String(right)) => left == right,
            (Self::Event { occurrences: left }, ValueRef::Event { occurrences: right }) => {
                left == right
            }
            _ => false,
        }
    }

    /// Copies this borrowed value into owned storage.
    pub fn to_owned(self) -> Value {
        match self {
            Self::Bits(bits) => Value::Bits(bits.to_owned()),
            Self::Real(real) => Value::Real(real),
            Self::String(string) => Value::String(string.into()),
            Self::Event { occurrences } => Value::Event { occurrences },
        }
    }
}

impl Value {
    #[cfg(test)]
    pub(crate) fn same_value(&self, other: &Self) -> bool {
        self.as_ref().same_value(other.as_ref())
    }

    /// Borrows this owned value without copying its payload.
    pub fn as_ref(&self) -> ValueRef<'_> {
        match self {
            Self::Bits(bits) => ValueRef::Bits(bits.as_ref()),
            Self::Real(real) => ValueRef::Real(*real),
            Self::String(string) => ValueRef::String(string),
            Self::Event { occurrences } => ValueRef::Event {
                occurrences: *occurrences,
            },
        }
    }
}

/// A resolved state in the supported nine-state logic domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Logic {
    /// Logic zero.
    Zero,
    /// Logic one.
    One,
    /// An unknown forcing value.
    X,
    /// High impedance.
    Z,
    /// A weak logic one.
    H,
    /// An uninitialized value.
    U,
    /// A weak unknown value.
    W,
    /// A weak logic zero.
    L,
    /// A don't-care value.
    DontCare,
}

impl Logic {
    fn from_ascii(byte: u8) -> Option<Self> {
        match byte.to_ascii_lowercase() {
            b'0' => Some(Self::Zero),
            b'1' => Some(Self::One),
            b'x' => Some(Self::X),
            b'z' => Some(Self::Z),
            b'h' => Some(Self::H),
            b'u' => Some(Self::U),
            b'w' => Some(Self::W),
            b'l' => Some(Self::L),
            b'-' => Some(Self::DontCare),
            _ => None,
        }
    }
}

/// A borrowed, opaque view of a bit-vector value.
///
/// Index zero is the least-significant, rightmost bit. [`Display`](fmt::Display)
/// emits a most-significant-to-least-significant logic string using lowercase
/// letters. Storage is opaque and can borrow native backend data without an
/// intermediate string. Views can also borrow [`Bits`] storage;
/// callback-supplied views must be copied with [`Self::to_owned`] to outlive the call.
#[derive(Debug, Clone, Copy)]
pub struct BitsRef<'a> {
    data: &'a [u8],
}

/// An owned, opaque bit-vector value.
#[derive(Debug, Clone)]
pub struct Bits {
    data: Box<[u8]>,
}

impl<'a> BitsRef<'a> {
    pub(crate) fn from_ascii(bytes: &'a [u8]) -> Option<Self> {
        u32::try_from(bytes.len()).ok()?;
        bytes
            .iter()
            .all(|&byte| Logic::from_ascii(byte).is_some())
            .then_some(Self { data: bytes })
    }

    /// Borrows an inclusive slice whose normalized bounds are already validated.
    pub(crate) fn slice(self, msb: u32, lsb: u32) -> Self {
        let end = self.data.len() - lsb as usize;
        let start = self.data.len() - msb as usize - 1;
        Self {
            data: &self.data[start..end],
        }
    }

    /// Returns the number of bits in the vector.
    pub fn width(self) -> u32 {
        self.data.len() as u32
    }

    /// Returns the bit at `index`, where zero is the least-significant bit.
    ///
    /// Returns `None` when `index` is outside the vector.
    pub fn bit(self, index: u32) -> Option<Logic> {
        let offset = self
            .data
            .len()
            .checked_sub(index as usize)?
            .checked_sub(1)?;
        Logic::from_ascii(self.data[offset])
    }

    /// Iterates over every bit from most to least significant.
    pub fn iter_msb(self) -> impl ExactSizeIterator<Item = Logic> + 'a {
        self.data
            .iter()
            .map(|&byte| Logic::from_ascii(byte).expect("validated logic byte"))
    }

    /// Iterates over every bit from least to most significant.
    pub fn iter_lsb(self) -> impl ExactSizeIterator<Item = Logic> + 'a {
        self.data
            .iter()
            .rev()
            .map(|&byte| Logic::from_ascii(byte).expect("validated logic byte"))
    }

    /// Copies this borrowed vector into owned storage.
    pub fn to_owned(self) -> Bits {
        Bits {
            data: self.data.iter().map(u8::to_ascii_lowercase).collect(),
        }
    }
}

impl fmt::Display for BitsRef<'_> {
    /// Formats the vector as a most-significant-bit-first logic string.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.data {
            write!(formatter, "{}", byte.to_ascii_lowercase() as char)?;
        }
        Ok(())
    }
}

impl Bits {
    #[cfg(test)]
    pub(crate) fn from_ascii(bytes: &[u8]) -> Option<Self> {
        BitsRef::from_ascii(bytes).map(BitsRef::to_owned)
    }

    /// Returns the number of bits in the vector.
    pub fn width(&self) -> u32 {
        self.as_ref().width()
    }

    /// Borrows this vector without copying its storage.
    pub fn as_ref(&self) -> BitsRef<'_> {
        BitsRef { data: &self.data }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_states_and_iteration() {
        let bits = BitsRef::from_ascii(b"01XZHUWL-").unwrap();
        let expected = [
            Logic::Zero,
            Logic::One,
            Logic::X,
            Logic::Z,
            Logic::H,
            Logic::U,
            Logic::W,
            Logic::L,
            Logic::DontCare,
        ];
        assert_eq!(bits.width(), 9);
        assert_eq!(bits.to_string(), "01xzhuwl-");
        assert_eq!(bits.iter_msb().collect::<Vec<_>>(), expected);
        assert_eq!(
            bits.iter_lsb().collect::<Vec<_>>(),
            expected.into_iter().rev().collect::<Vec<_>>()
        );
        let mut iter = bits.iter_msb();
        assert_eq!(iter.len(), 9);
        assert_eq!(iter.next(), Some(Logic::Zero));
        assert_eq!(iter.len(), 8);
        let mut iter = bits.iter_lsb();
        assert_eq!(iter.len(), 9);
        assert_eq!(iter.next(), Some(Logic::DontCare));
        assert_eq!(iter.len(), 8);
        for index in 0..9 {
            assert_eq!(bits.bit(index), Some(expected[8 - index as usize]));
        }
        assert_eq!(bits.bit(9), None);
        assert_eq!(bits.bit(u32::MAX), None);
        assert!(BitsRef::from_ascii(b"10?").is_none());
        assert!(Bits::from_ascii(b"\xff").is_none());
        let empty = BitsRef::from_ascii(b"").unwrap();
        assert_eq!(empty.width(), 0);
        assert_eq!(empty.bit(0), None);
        assert_eq!(empty.iter_msb().len(), 0);
        assert_eq!(empty.iter_lsb().len(), 0);
        assert_eq!(empty.to_string(), "");
    }

    #[test]
    fn slices_and_owned_storage() {
        let mut source = b"01XZHUWL-".to_vec();
        let owned = {
            let bits = BitsRef::from_ascii(&source).unwrap();
            assert_eq!(bits.slice(8, 0).to_string(), "01xzhuwl-");
            assert_eq!(bits.slice(0, 0).to_string(), "-");
            assert_eq!(bits.slice(8, 8).to_string(), "0");
            assert_eq!(bits.slice(6, 2).to_string(), "xzhuw");
            let nested = bits.slice(6, 2).slice(3, 1);
            assert_eq!(nested.width(), 3);
            assert_eq!(nested.to_string(), "zhu");
            assert_eq!(nested.bit(0), Some(Logic::U));
            nested.to_owned()
        };
        source.fill(b'0');
        assert_eq!(owned.width(), 3);
        assert_eq!(owned.as_ref().to_string(), "zhu");
        assert_eq!(
            Bits::from_ascii(b"XzU").unwrap().as_ref().to_string(),
            "xzu"
        );
    }

    #[test]
    fn values_round_trip_and_compare() {
        let bits = ValueRef::Bits(BitsRef::from_ascii(b"01X").unwrap()).to_owned();
        assert!(bits.same_value(&Value::Bits(Bits::from_ascii(b"01x").unwrap())));
        assert!(ValueRef::Bits(BitsRef::from_ascii(b"01X").unwrap()).same_value(bits.as_ref()));
        assert!(!bits.same_value(&Value::Bits(Bits::from_ascii(b"01z").unwrap())));
        assert!(!bits.same_value(&Value::Bits(Bits::from_ascii(b"1x").unwrap())));
        match bits.as_ref() {
            ValueRef::Bits(view) => assert_eq!(view.to_string(), "01x"),
            _ => panic!("expected bits"),
        }
        let text = {
            let source = String::from("retained string");
            ValueRef::String(&source).to_owned()
        };
        assert!(matches!(text.as_ref(), ValueRef::String("retained string")));
        assert!(text.same_value(&text.as_ref().to_owned()));
        assert!(!text.same_value(&Value::String("different".into())));
        let real = ValueRef::Real(3.5).to_owned();
        assert!(matches!(real.as_ref(), ValueRef::Real(3.5)));
        assert!(real.same_value(&real.as_ref().to_owned()));
        let event = ValueRef::Event { occurrences: 7 }.to_owned();
        assert!(matches!(event.as_ref(), ValueRef::Event { occurrences: 7 }));
        assert!(event.same_value(&event.as_ref().to_owned()));
        assert!(!event.same_value(&Value::Event { occurrences: 1 }));
        assert!(!real.same_value(&text));
        assert!(!event.same_value(&real));
    }

    #[test]
    fn real_identity_and_conversion_preserve_bits() {
        let patterns = [
            0x7ff8_0000_0000_0001, // Quiet NaN, first payload.
            0x7ff8_0000_0000_0002, // Different payload.
            0xfff8_0000_0000_0001, // Different sign.
            0x7ff0_0000_0000_0001, // Signaling NaN.
            0.0f64.to_bits(),
            (-0.0f64).to_bits(),
            f64::INFINITY.to_bits(),
            f64::NEG_INFINITY.to_bits(),
            3.5f64.to_bits(),
        ];
        for left in patterns {
            let value = ValueRef::Real(f64::from_bits(left));
            let owned = value.to_owned();
            let ValueRef::Real(real) = owned.as_ref() else {
                panic!("expected real")
            };
            assert_eq!(real.to_bits(), left);
            for right in patterns {
                let other = Value::Real(f64::from_bits(right));
                assert_eq!(value.same_value(other.as_ref()), left == right);
                assert_eq!(owned.same_value(&other), left == right);
            }
        }
    }

    #[test]
    fn identity_distinguishes_every_logic_state() {
        for left in b"01xzhuwl-" {
            for right in b"01xzhuwl-" {
                let left_value = BitsRef::from_ascii(std::slice::from_ref(left)).unwrap();
                let right_value = BitsRef::from_ascii(std::slice::from_ref(right)).unwrap();
                assert_eq!(
                    ValueRef::Bits(left_value).same_value(ValueRef::Bits(right_value)),
                    left == right
                );
            }
        }
    }
}
