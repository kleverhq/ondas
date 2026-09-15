// Test-only ownership prototype. No types or methods here are exported by ondas.
// The fixed generated source proves callback composition, not backend execution.
use ondas::{SampleRef, Selection, Signal, Time, TimeRange, Value, ValueRef, Waveform};
use std::ops::ControlFlow;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidEntry,
    InvalidTime,
    Consumer,
}
pub type Result<T> = std::result::Result<T, Error>;

pub struct Owner<'w> {
    selection: Selection<'w>,
    before: [Value; 2],
    current: [Value; 2],
}

/// One completed candidate tick with selective, callback-borrowed samples.
///
/// Only `time()` and its checked predecessor are readable. The predecessor may
/// precede the candidate range; there is none at tick zero. Reads may repeat in
/// any order. Driver entries affect advancement, not what can be read. Subset
/// indices identify original selection positions, including repeats.
///
/// This context is not an eager caller snapshot or arbitrary-history interface.
/// Only requested samples reach the visitor. A sequential source may still
/// decode records while advancing. Borrowed samples live only for their visitor;
/// copying to owned samples is explicit.
///
/// A context cannot escape its candidate callback:
///
/// ```compile_fail,E0521
/// # mod prototype { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/query_contract.rs")); }
/// # use std::ops::ControlFlow;
/// # use ondas::TimeRange;
/// let mut wave = prototype::fixture();
/// let mut owner = prototype::owner(&mut wave);
/// let mut escaped = None;
/// owner.query(TimeRange::all(), &[0], |context| {
///     escaped = Some(context);
///     Ok(ControlFlow::<()>::Continue(()))
/// }).unwrap();
/// println!("{:?}", escaped.unwrap().time());
/// ```
///
/// A sample cannot escape even into the surrounding candidate callback:
///
/// ```compile_fail,E0521
/// # mod prototype { include!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/query_contract.rs")); }
/// # use std::ops::ControlFlow;
/// # use ondas::TimeRange;
/// let mut wave = prototype::fixture();
/// let mut owner = prototype::owner(&mut wave);
/// owner.query(TimeRange::all(), &[0], |context| {
///     let mut escaped = None;
///     context.visit_samples(context.time(), &[1], |_, sample| {
///         escaped = Some(sample);
///         Ok(ControlFlow::<()>::Continue(()))
///     })?;
///     println!("{:?}", escaped.unwrap());
///     Ok(ControlFlow::<()>::Continue(()))
/// }).unwrap();
/// ```
pub struct QueryContext<'a> {
    time: Time,
    signals: &'a [Signal],
    before: &'a [Value],
    current: &'a [Value],
}

fn validate_indices(len: usize, indices: &[usize]) -> Result<()> {
    if indices.iter().any(|&index| index >= len) {
        return Err(Error::InvalidEntry);
    }
    Ok(())
}

impl Owner<'_> {
    /// Advances increasing unique candidates and lends one context at a time.
    ///
    /// The driver subset is separate from readable selection entries. Empty
    /// drivers invoke no callback; indices are validated before advancement.
    /// Conservative extra candidates are permitted. No complete candidate or
    /// history list is needed. Outer Break/Err stops execution; a fresh query is
    /// valid afterward. This prototype generates four ticks from fixed state.
    pub fn query<B>(
        &mut self,
        range: TimeRange,
        drivers: &[usize],
        mut visitor: impl FnMut(&QueryContext<'_>) -> Result<ControlFlow<B>>,
    ) -> Result<ControlFlow<B>> {
        validate_indices(self.selection.signals().len(), drivers)?;
        if drivers.is_empty() || range.is_empty() {
            return Ok(ControlFlow::Continue(()));
        }
        for tick in 0..4 {
            let time = Time::from_ticks(tick);
            if time < range.start() || range.end().is_some_and(|end| time > end) {
                continue;
            }
            self.before[0] = Value::Real(tick.saturating_sub(1) as f64);
            self.current[0] = Value::Real(tick as f64);
            let context = QueryContext {
                time,
                signals: self.selection.signals(),
                before: &self.before,
                current: &self.current,
            };
            if let ControlFlow::Break(value) = visitor(&context)? {
                return Ok(ControlFlow::Break(value));
            }
        }
        Ok(ControlFlow::Continue(()))
    }
}

impl QueryContext<'_> {
    pub fn time(&self) -> Time {
        self.time
    }

    /// Visits the requested subset in request order, after validating it fully.
    ///
    /// Empty subsets invoke no callback. Read-visitor Break stops this visit,
    /// not automatically the outer query. The caller may propagate it or
    /// continue with another read. Err propagates unchanged at either boundary.
    pub fn visit_samples<B>(
        &self,
        time: Time,
        indices: &[usize],
        mut visitor: impl for<'v> FnMut(usize, SampleRef<'v>) -> Result<ControlFlow<B>>,
    ) -> Result<ControlFlow<B>> {
        let values = if time == self.time {
            self.current
        } else if self.time.ticks().checked_sub(1).map(Time::from_ticks) == Some(time) {
            self.before
        } else {
            return Err(Error::InvalidTime);
        };
        validate_indices(self.signals.len(), indices)?;
        for &index in indices {
            let sample = SampleRef::Value {
                signal: self.signals[index],
                value: values[index].as_ref(),
                changed_at: None,
            };
            if let ControlFlow::Break(value) = visitor(index, sample)? {
                return Ok(ControlFlow::Break(value));
            }
        }
        Ok(ControlFlow::Continue(()))
    }
}

pub fn fixture() -> Waveform {
    ondas::open_bytes(
        "contract.vcd",
        (&b"$scope module top $end $var real 1 c control $end $var real 1 p payload $end $upscope $end $enddefinitions $end"[..]).into(),
    ).unwrap()
}

pub fn owner(wave: &mut Waveform) -> Owner<'_> {
    let signals = [
        wave.hierarchy().signal("top.control").unwrap(),
        wave.hierarchy().signal("top.payload").unwrap(),
    ];
    Owner {
        // Hold the actual exclusive Selection borrow; only source values are synthetic.
        selection: wave.select(&signals).unwrap(),
        before: [Value::Real(0.0), Value::Real(9.0)],
        current: [Value::Real(0.0), Value::Real(9.0)],
    }
}

#[test]
fn one_owner_supports_adjacent_reads_and_conditional_payload() {
    let mut wave = fixture();
    let mut owner = owner(&mut wave);
    let mut payloads = Vec::new();
    let mut candidates = Vec::new();
    let _ = owner
        .query(
            TimeRange::closed(Time::from_ticks(1), Time::from_ticks(3)),
            &[0],
            |ctx| {
                let t = ctx.time();
                candidates.push(t.ticks());
                let before = Time::from_ticks(t.ticks().checked_sub(1).unwrap());
                let mut controls = Vec::new();
                for time in [before, t, before] {
                    let _ = ctx.visit_samples(time, &[0], |index, sample| {
                        assert_eq!(index, 0);
                        let SampleRef::Value {
                            value: ValueRef::Real(value),
                            ..
                        } = sample
                        else {
                            panic!("expected real")
                        };
                        controls.push(value);
                        Ok(ControlFlow::<()>::Continue(()))
                    })?;
                }
                assert_eq!(controls[0], controls[2]);
                if controls[1] == 2.0 {
                    let _ = ctx.visit_samples(t, &[1, 0, 1], |index, sample| {
                        let SampleRef::Value { value, .. } = sample else {
                            panic!("expected value")
                        };
                        payloads.push((index, value.to_owned()));
                        Ok(ControlFlow::<()>::Continue(()))
                    })?;
                }
                Ok(ControlFlow::<()>::Continue(()))
            },
        )
        .unwrap();
    drop(owner);
    drop(wave);
    assert_eq!(candidates, [1, 2, 3]);
    assert_eq!(
        payloads.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
        [1, 0, 1]
    );
    assert!(matches!(&payloads[0].1, Value::Real(value) if *value == 9.0));
}

#[test]
fn empty_invalid_break_and_error_boundaries_are_explicit() {
    let mut wave = fixture();
    let mut owner = owner(&mut wave);
    assert_eq!(
        owner.query(TimeRange::all(), &[], |_| -> Result<ControlFlow<()>> {
            panic!("empty drivers")
        }),
        Ok(ControlFlow::Continue(()))
    );
    assert_eq!(
        owner.query(TimeRange::all(), &[2], |_| -> Result<ControlFlow<()>> {
            panic!("invalid driver")
        }),
        Err(Error::InvalidEntry)
    );
    let result = owner.query(TimeRange::all(), &[0, 0], |ctx| {
        assert_eq!(ctx.time(), Time::ZERO);
        assert_eq!(
            ctx.visit_samples(
                Time::from_ticks(u64::MAX),
                &[],
                |_, _| -> Result<ControlFlow<()>> { panic!("invalid time") }
            ),
            Err(Error::InvalidTime)
        );
        assert_eq!(
            ctx.visit_samples(ctx.time(), &[0, 2], |_, _| -> Result<ControlFlow<()>> {
                panic!("prevalidate entire subset")
            }),
            Err(Error::InvalidEntry)
        );
        assert_eq!(
            ctx.visit_samples(ctx.time(), &[], |_, _| -> Result<ControlFlow<()>> {
                panic!("empty subset")
            }),
            Ok(ControlFlow::Continue(()))
        );
        let result = ctx.visit_samples(ctx.time(), &[1, 0], |index, _| {
            assert_eq!(index, 1);
            Ok(ControlFlow::Break("subset"))
        })?;
        assert_eq!(result, ControlFlow::Break("subset"));
        Ok(ControlFlow::Break("query"))
    });
    assert_eq!(result, Ok(ControlFlow::Break("query")));
    assert_eq!(
        owner.query(TimeRange::all(), &[0], |_| -> Result<ControlFlow<()>> {
            Err(Error::Consumer)
        }),
        Err(Error::Consumer)
    );
    assert_eq!(
        owner.query(TimeRange::all(), &[0], |ctx| {
            ctx.visit_samples(ctx.time(), &[1], |_, _| -> Result<ControlFlow<()>> {
                Err(Error::Consumer)
            })
        }),
        Err(Error::Consumer)
    );
    let mut calls = 0;
    let _ = owner
        .query(TimeRange::all(), &[0], |_| {
            calls += 1;
            Ok(ControlFlow::<()>::Continue(()))
        })
        .unwrap();
    assert_eq!(calls, 4);
}
