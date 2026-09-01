/// An absolute waveform time expressed in source ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Time(u64);

impl Time {
    /// The zero tick.
    pub const ZERO: Self = Self(0);

    /// Creates an absolute time from a source tick count.
    pub const fn from_ticks(ticks: u64) -> Self {
        Self(ticks)
    }

    /// Returns the absolute source tick count.
    pub const fn ticks(self) -> u64 {
        self.0
    }
}

/// The inclusive span from the first to the last recorded tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSpan {
    _private: (),
}

impl TimeSpan {
    /// Returns the first recorded tick.
    pub const fn first(self) -> Time {
        panic!("backend implementation")
    }

    /// Returns the last recorded tick.
    pub const fn last(self) -> Time {
        panic!("backend implementation")
    }
}

/// An inclusive absolute tick range, optionally unbounded at the waveform end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRange {
    _private: (),
}

impl TimeRange {
    /// Creates a range covering zero through the waveform end.
    pub const fn all() -> Self {
        panic!("query implementation")
    }

    /// Creates a range from `start` through the waveform end.
    pub const fn from(_start: Time) -> Self {
        panic!("query implementation")
    }

    /// Creates the inclusive range `[start, end]`.
    ///
    /// The range is empty when `start` is later than `end`.
    pub const fn closed(_start: Time, _end: Time) -> Self {
        panic!("query implementation")
    }

    /// Creates the single-tick range `[time, time]`.
    pub const fn point(_time: Time) -> Self {
        panic!("query implementation")
    }

    /// Returns the inclusive start tick.
    pub const fn start(self) -> Time {
        panic!("query implementation")
    }

    /// Returns the inclusive end tick, or `None` for the waveform end.
    pub const fn end(self) -> Option<Time> {
        panic!("query implementation")
    }

    /// Returns whether the bounded range starts after its end.
    pub const fn is_empty(self) -> bool {
        panic!("query implementation")
    }
}

/// The exact duration of one tick as an integer factor and unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Timescale {
    _private: (),
}

impl Timescale {
    /// Returns the integer unit multiplier per tick.
    pub const fn factor(self) -> u32 {
        panic!("backend implementation")
    }

    /// Returns the unit multiplied by [`Self::factor`].
    pub const fn unit(self) -> TimeUnit {
        panic!("backend implementation")
    }
}

/// A physical time unit used by a waveform timescale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TimeUnit {
    /// Seconds.
    Second,
    /// Milliseconds.
    Millisecond,
    /// Microseconds.
    Microsecond,
    /// Nanoseconds.
    Nanosecond,
    /// Picoseconds.
    Picosecond,
    /// Femtoseconds.
    Femtosecond,
    /// Attoseconds.
    Attosecond,
    /// Zeptoseconds.
    Zeptosecond,
}
