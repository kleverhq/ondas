use ondas::{Error, Result, SampleRef, Selection, Time, TimeRange, Value, ValueRef, Waveform};
use std::ops::ControlFlow;

fn fixture() -> Waveform {
    ondas::open_bytes(
        "contract.vcd",
        (&b"$scope module top $end $var real 1 c control $end $var real 1 p payload $end $upscope $end $enddefinitions $end #0 r0 c r9 p #1 r1 c #2 r2 c #3 r3 c"[..]).into(),
    ).unwrap()
}

fn selection(wave: &mut Waveform) -> Selection<'_> {
    let signals = [
        wave.hierarchy().signal("top.control").unwrap(),
        wave.hierarchy().signal("top.payload").unwrap(),
    ];
    wave.select(&signals).unwrap()
}

#[test]
fn one_owner_supports_adjacent_reads_and_conditional_payload() {
    let mut wave = fixture();
    let mut selection = selection(&mut wave);
    let mut payloads = Vec::new();
    let mut candidates = Vec::new();
    let _ = selection
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
                assert_eq!(
                    controls,
                    [
                        t.ticks() as f64 - 1.0,
                        t.ticks() as f64,
                        t.ticks() as f64 - 1.0
                    ]
                );
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
    let mut selection = selection(&mut wave);
    assert_eq!(
        selection
            .query(TimeRange::all(), &[], |_| -> Result<ControlFlow<()>> {
                panic!("empty drivers")
            })
            .unwrap(),
        ControlFlow::Continue(())
    );
    assert!(matches!(
        selection.query(TimeRange::all(), &[2], |_| -> Result<ControlFlow<()>> {
            panic!("invalid driver")
        }),
        Err(Error::InvalidSelectionIndex { index: 2, len: 2 })
    ));
    let result = selection.query(TimeRange::all(), &[0, 0], |ctx| {
        assert_eq!(ctx.time(), Time::ZERO);
        assert!(matches!(
            ctx.visit_samples(
                Time::from_ticks(u64::MAX),
                &[],
                |_, _| -> Result<ControlFlow<()>> { panic!("invalid time") }
            ),
            Err(Error::InvalidQueryTime { .. })
        ));
        assert!(matches!(
            ctx.visit_samples(ctx.time(), &[0, 2], |_, _| -> Result<ControlFlow<()>> {
                panic!("prevalidate entire subset")
            }),
            Err(Error::InvalidSelectionIndex { index: 2, len: 2 })
        ));
        assert_eq!(
            ctx.visit_samples(ctx.time(), &[], |_, _| -> Result<ControlFlow<()>> {
                panic!("empty subset")
            })?,
            ControlFlow::Continue(())
        );
        assert_eq!(
            ctx.visit_samples(ctx.time(), &[1, 0], |index, _| {
                assert_eq!(index, 1);
                Ok(ControlFlow::Break("subset"))
            })?,
            ControlFlow::Break("subset")
        );
        Ok(ControlFlow::Break("query"))
    });
    assert_eq!(result.unwrap(), ControlFlow::Break("query"));
    let error = selection
        .query(TimeRange::all(), &[0], |_| -> Result<ControlFlow<()>> {
            Err(std::io::Error::other("query consumer").into())
        })
        .unwrap_err();
    assert!(error.to_string().contains("query consumer"));
    let error = selection
        .query(TimeRange::all(), &[0], |ctx| {
            ctx.visit_samples(ctx.time(), &[1], |_, _| -> Result<ControlFlow<()>> {
                Err(std::io::Error::other("sample consumer").into())
            })
        })
        .unwrap_err();
    assert!(error.to_string().contains("sample consumer"));
    let mut candidates = Vec::new();
    let _ = selection
        .query(TimeRange::all(), &[0, 0], |ctx| {
            candidates.push(ctx.time().ticks());
            Ok(ControlFlow::<()>::Continue(()))
        })
        .unwrap();
    assert_eq!(candidates, [0, 1, 2, 3]);
    let mut payload_candidates = Vec::new();
    let _ = selection
        .query(TimeRange::all(), &[1], |ctx| {
            payload_candidates.push(ctx.time().ticks());
            Ok(ControlFlow::<()>::Continue(()))
        })
        .unwrap();
    assert_eq!(payload_candidates, [0]);
}
