/// An absolute waveform time expressed in source ticks.
///
/// This is not a backend-local time index. [`Timescale`] describes the duration
/// of a tick when known; the API exposes no global timestamp table. Delta cycles
/// are not modeled separately. A point sample uses the final state after all
/// changes to that signal at the tick. Traces and scans preserve distinct
/// same-signal changes within a tick; order across different signals is unspecified.
/// Recorded metadata bounds do not restrict query times.
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
    first: Time,
    last: Time,
}

impl TimeSpan {
    pub(crate) const fn new(first: Time, last: Time) -> Self {
        Self { first, last }
    }

    /// Returns the first recorded tick.
    pub const fn first(self) -> Time {
        self.first
    }

    /// Returns the last recorded tick.
    pub const fn last(self) -> Time {
        self.last
    }
}

/// An inclusive absolute tick range, optionally unbounded at the waveform end.
///
/// Both bounds are included. An absent end means EOF, not a synthetic timestamp.
/// A bounded range with `start > end` is empty, not an error: its scan makes no
/// visitor calls and its trace has neither an initial state nor changes.
/// [`TimeRange::point`] covers all changes at a single tick, with any state
/// strictly before that tick represented separately as an initial state.
/// [`Metadata::time_span`](crate::Metadata::time_span) describes recorded ticks;
/// it does not restrict caller-supplied query bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRange {
    start: Time,
    end: Option<Time>,
}

impl TimeRange {
    /// Creates a range covering zero through the waveform end.
    pub const fn all() -> Self {
        Self::from(Time::ZERO)
    }

    /// Creates a range from `start` through the waveform end.
    pub const fn from(start: Time) -> Self {
        Self { start, end: None }
    }

    /// Creates the inclusive range `[start, end]`.
    ///
    /// The range is empty when `start` is later than `end`.
    pub const fn closed(start: Time, end: Time) -> Self {
        Self {
            start,
            end: Some(end),
        }
    }

    /// Creates the single-tick range `[time, time]`.
    pub const fn point(time: Time) -> Self {
        Self::closed(time, time)
    }

    /// Returns the inclusive start tick.
    pub const fn start(self) -> Time {
        self.start
    }

    /// Returns the inclusive end tick, or `None` for the waveform end.
    pub const fn end(self) -> Option<Time> {
        self.end
    }

    /// Returns whether the bounded range starts after its end.
    pub const fn is_empty(self) -> bool {
        match self.end {
            Some(end) => self.start.ticks() > end.ticks(),
            None => false,
        }
    }
}

/// The exact duration of one tick as an integer factor and unit.
///
/// The representation preserves `factor × unit`, not floating-point seconds per tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Timescale {
    factor: u32,
    unit: TimeUnit,
}

impl Timescale {
    pub(crate) const fn new(factor: u32, unit: TimeUnit) -> Self {
        Self { factor, unit }
    }

    /// Returns the integer unit multiplier per tick.
    pub const fn factor(self) -> u32 {
        self.factor
    }

    /// Returns the unit multiplied by [`Self::factor`].
    pub const fn unit(self) -> TimeUnit {
        self.unit
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inclusive_ranges_preserve_bounds() {
        let start = Time::from_ticks(7);
        let end = Time::from_ticks(11);
        let all = TimeRange::all();
        assert_eq!(all.start(), Time::ZERO);
        assert_eq!(all.end(), None);
        assert!(!all.is_empty());
        let unbounded = TimeRange::from(start);
        assert_eq!(unbounded.start(), start);
        assert_eq!(unbounded.end(), None);
        assert!(!unbounded.is_empty());
        let closed = TimeRange::closed(start, end);
        assert_eq!(closed.start(), start);
        assert_eq!(closed.end(), Some(end));
        assert!(!closed.is_empty());
        let reversed = TimeRange::closed(end, start);
        assert_eq!(reversed.start(), end);
        assert_eq!(reversed.end(), Some(start));
        assert!(reversed.is_empty());
        for time in [Time::ZERO, start, Time::from_ticks(u64::MAX)] {
            let point = TimeRange::point(time);
            assert_eq!(point.start(), time);
            assert_eq!(point.end(), Some(time));
            assert!(!point.is_empty());
            assert!(!TimeRange::from(time).is_empty());
        }
        assert!(TimeRange::closed(Time::from_ticks(u64::MAX), Time::ZERO).is_empty());
    }

    #[test]
    fn exact_time_metadata() {
        const FIRST: Time = Time::from_ticks(3);
        const SPAN: TimeSpan = TimeSpan::new(FIRST, Time::from_ticks(u64::MAX));
        const SCALE: Timescale = Timescale::new(10, TimeUnit::Nanosecond);
        const _: () = {
            assert!(SPAN.first().ticks() == 3);
            assert!(SPAN.last().ticks() == u64::MAX);
            assert!(SCALE.factor() == 10);
            assert!(matches!(SCALE.unit(), TimeUnit::Nanosecond));
            assert!(TimeRange::all().start().ticks() == 0);
            assert!(TimeRange::from(FIRST).end().is_none());
            assert!(TimeRange::closed(FIRST, Time::ZERO).is_empty());
            assert!(!TimeRange::point(FIRST).is_empty());
        };
        let first = FIRST;
        let last = Time::from_ticks(u64::MAX);
        let span = SPAN;
        assert_eq!(SCALE, Timescale::new(10, TimeUnit::Nanosecond));
        assert_eq!(span.first(), first);
        assert_eq!(span.last(), last);
        assert_eq!(last.ticks(), u64::MAX);
        assert!(Time::ZERO < first && first < last);
        let units = [
            TimeUnit::Second,
            TimeUnit::Millisecond,
            TimeUnit::Microsecond,
            TimeUnit::Nanosecond,
            TimeUnit::Picosecond,
            TimeUnit::Femtosecond,
            TimeUnit::Attosecond,
            TimeUnit::Zeptosecond,
        ];
        for unit in units {
            let scale = Timescale::new(100, unit);
            assert_eq!(scale.factor(), 100);
            assert_eq!(scale.unit(), unit);
        }
    }
}
