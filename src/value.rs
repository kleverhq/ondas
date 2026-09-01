use std::{fmt, marker::PhantomData};

use crate::stub;

/// A borrowed view of a waveform value.
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
    /// A single event occurrence.
    Event,
}

/// An owned waveform value suitable for long-term storage.
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
    /// A single event occurrence.
    Event,
}

impl ValueRef<'_> {
    /// Copies this borrowed value into owned storage.
    pub fn to_owned(self) -> Value {
        unimplemented!("value implementation")
    }
}

impl Value {
    /// Borrows this owned value without copying its payload.
    pub fn as_ref(&self) -> ValueRef<'_> {
        unimplemented!("value implementation")
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

/// A borrowed, opaque view of a bit-vector value.
#[derive(Debug, Clone, Copy)]
pub struct BitsRef<'a> {
    _data: PhantomData<&'a [u8]>,
}

/// An owned, opaque bit-vector value.
#[derive(Debug, Clone)]
pub struct Bits {
    _private: (),
}

impl<'a> BitsRef<'a> {
    /// Returns the number of bits in the vector.
    pub fn width(self) -> u32 {
        unimplemented!("value implementation")
    }

    /// Returns the bit at `index`, where zero is the least-significant bit.
    ///
    /// Returns `None` when `index` is outside the vector.
    pub fn bit(self, _index: u32) -> Option<Logic> {
        unimplemented!("value implementation")
    }

    /// Iterates over every bit from most to least significant.
    pub fn iter_msb(self) -> impl ExactSizeIterator<Item = Logic> + 'a {
        stub::<std::iter::Empty<Logic>>()
    }

    /// Iterates over every bit from least to most significant.
    pub fn iter_lsb(self) -> impl ExactSizeIterator<Item = Logic> + 'a {
        stub::<std::iter::Empty<Logic>>()
    }

    /// Copies this borrowed vector into owned storage.
    pub fn to_owned(self) -> Bits {
        unimplemented!("value implementation")
    }
}

impl fmt::Display for BitsRef<'_> {
    /// Formats the vector as a most-significant-bit-first logic string.
    fn fmt(&self, _formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        unimplemented!("value implementation")
    }
}

impl Bits {
    /// Returns the number of bits in the vector.
    pub fn width(&self) -> u32 {
        unimplemented!("value implementation")
    }

    /// Borrows this vector without copying its storage.
    pub fn as_ref(&self) -> BitsRef<'_> {
        unimplemented!("value implementation")
    }
}
