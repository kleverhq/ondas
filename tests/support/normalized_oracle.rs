// Independent derivation from version-1 evidence. No ondas code is used here.
use serde_json::{Value as Json, json};

fn tick(value: &Json) -> u64 {
    value.as_str().unwrap().parse().unwrap()
}

fn is_nan(value: &Json) -> bool {
    value.get("real_bits").is_some_and(|bits| {
        f64::from_bits(u64::from_str_radix(bits.as_str().unwrap(), 16).unwrap()).is_nan()
    })
}

/// Version-1 equality does not constrain NaN payload, sign or signaling bits.
pub fn legacy_value(value: &Json) -> Json {
    if is_nan(value) {
        json!({"real_bits": "7ff8000000000000"})
    } else {
        value.clone()
    }
}

// Complete windows may omit redundant writes, but their final value at every
// recorded tick is known under version-1 equality. Events count unit records.
fn final_ticks(window: &Json) -> Vec<(u64, Json)> {
    let mut result: Vec<(u64, Json)> = Vec::new();
    for record in window["changes"].as_array().unwrap() {
        let time = tick(&record["time"]);
        let event = record["value"].get("event").is_some();
        let value = if event {
            json!({"occurrences": "1"})
        } else {
            legacy_value(&record["value"])
        };
        if let Some((previous_time, previous_value)) = result.last_mut()
            && *previous_time == time
        {
            if event {
                let count = tick(&previous_value["occurrences"])
                    .checked_add(1)
                    .expect("oracle event count overflow");
                *previous_value = json!({"occurrences": count.to_string()});
            } else {
                *previous_value = value;
            }
        } else {
            result.push((time, value));
        }
    }
    result
}

/// Normalized observable changes modulo version-1 NaN equivalence.
pub fn changes(window: &Json) -> Vec<(u64, Json)> {
    let mut previous = window["initial"].get("value").map(legacy_value);
    let mut result = Vec::new();
    for (time, value) in final_ticks(window) {
        if value.get("occurrences").is_some() || previous.as_ref() != Some(&value) {
            result.push((time, value.clone()));
        }
        previous = Some(value);
    }
    result
}

/// Prepared independent evidence for repeated point reads in one complete window.
/// Test-side snapshots avoid re-deriving an entire history at every candidate.
pub struct WindowEvidence {
    start: u64,
    end: u64,
    event: bool,
    initial: Json,
    ticks: Vec<(u64, Json)>,
}

impl WindowEvidence {
    pub fn new(window: &Json, windows: &[Json], event: bool) -> Self {
        let start = tick(&window["start"]);
        let end = tick(&window["end"]);
        // An empty window has no entering-state assertion at all.
        let initial_time = if event || start > end {
            None
        } else {
            start.checked_sub(1).and_then(|before| {
                let initial = if window["initial"].is_null() {
                    json!({"missing": true})
                } else {
                    window["initial"].clone()
                };
                // Recursive coverage starts strictly earlier: no cycles or gaps.
                sample_assertions(&initial, windows, before)
                    .get("changed_at")
                    .map(tick)
            })
        };
        let snapshot = |state: Option<&Json>, changed_at: Option<u64>| {
            let Some(value) = state else {
                return json!({"missing": true});
            };
            let mut result = json!({"value": value});
            if let Some(time) = changed_at {
                result["changed_at"] = json!(time.to_string());
            }
            result
        };
        let mut state = window["initial"].get("value").map(legacy_value);
        // Raw initial timestamps are not normalized timestamp evidence.
        let mut changed_at = initial_time;
        let initial = snapshot(state.as_ref(), changed_at);
        let ticks = final_ticks(window)
            .into_iter()
            .map(|(time, value)| {
                if event {
                    return (time, value);
                }
                if state.as_ref().is_some_and(is_nan) && is_nan(&value) {
                    changed_at = None;
                } else if state.as_ref() != Some(&value) {
                    changed_at = state.as_ref().map(|_| time);
                }
                state = Some(value);
                (time, snapshot(state.as_ref(), changed_at))
            })
            .collect();
        Self {
            start,
            end,
            event,
            initial,
            ticks,
        }
    }

    pub fn sample(&self, time: u64) -> Json {
        assert!(self.start <= time && time <= self.end);
        let end = self.ticks.partition_point(|(tick, _)| *tick <= time);
        let latest = end.checked_sub(1).map(|index| &self.ticks[index]);
        if self.event {
            return latest
                .filter(|(tick, _)| *tick == time)
                .map_or_else(|| json!({"occurrences":"0"}), |(_, value)| value.clone());
        }
        let mut result = latest.map_or(&self.initial, |(_, value)| value).clone();
        if is_nan(&result["value"]) && latest.is_none_or(|(tick, _)| *tick != time) {
            // v1 may omit payload-only writes between recorded NaN observations.
            result.as_object_mut().unwrap().remove("changed_at");
        }
        result
    }
}

/// Point state and only those normalized change times this window proves.
pub fn sample(window: &Json, time: u64, event: bool) -> Json {
    WindowEvidence::new(window, &[], event).sample(time)
}

/// Keep a sparse point/initial assertion, replacing its raw timestamp only when
/// covering complete windows prove a normalized timestamp. Never rewrite input.
pub fn sample_assertions(expected: &Json, windows: &[Json], time: u64) -> Json {
    let event = expected.get("occurrences").is_some();
    if expected.get("value").is_none() && !event && expected.get("missing").is_none() {
        return expected.clone();
    }
    let mut result = expected.clone();
    result.as_object_mut().unwrap().remove("changed_at");
    let mut proven = None;
    for window in windows {
        if tick(&window["start"]) <= time && time <= tick(&window["end"]) {
            let derived = WindowEvidence::new(window, windows, event).sample(time);
            for key in ["value", "missing", "occurrences"] {
                assert_eq!(
                    legacy_value(&expected[key]),
                    derived[key],
                    "inconsistent point/window value evidence: {key}"
                );
            }
            if let Some(time) = derived.get("changed_at") {
                if let Some(previous) = &proven {
                    assert_eq!(previous, time, "inconsistent normalized timestamp evidence");
                }
                proven = Some(time.clone());
            }
        }
    }
    if let Some(time) = proven {
        result["changed_at"] = time;
    }
    result
}

// Original v1 histories retain distinct intermediate values and unit events.
// These helpers validate source evidence, not current public normalization.
fn legacy_changes(window: &Json) -> Vec<(u64, Json)> {
    let mut previous = window["initial"].get("value").map(legacy_value);
    let mut result = Vec::new();
    for record in window["changes"].as_array().unwrap() {
        let value = legacy_value(&record["value"]);
        if value.get("event").is_some() || previous.as_ref() != Some(&value) {
            result.push((tick(&record["time"]), value.clone()));
        }
        previous = Some(value);
    }
    result
}

fn legacy_sample(window: &Json, time: u64, event: bool) -> Json {
    let changes = legacy_changes(window);
    if event {
        return json!({"occurrences": changes.iter().filter(|(tick, _)| *tick == time).count().to_string()});
    }
    let mut state = if window["initial"].is_null() {
        json!({"missing": true})
    } else {
        window["initial"].clone()
    };
    for (tick, value) in changes.into_iter().take_while(|(tick, _)| *tick <= time) {
        let known = state.get("value").is_some();
        state = json!({"value": value});
        if known {
            state["changed_at"] = json!(tick.to_string());
        }
    }
    state
}

fn assert_legacy_samples(left: &Json, right: &Json) {
    for key in ["value", "missing", "occurrences"] {
        assert_eq!(
            legacy_value(&left[key]),
            legacy_value(&right[key]),
            "inconsistent version-1 evidence: {key}"
        );
    }
    if let (Some(left), Some(right)) = (
        left.get("changed_at").filter(|v| !v.is_null()),
        right.get("changed_at").filter(|v| !v.is_null()),
    ) {
        assert_eq!(left, right, "inconsistent version-1 timestamp evidence");
    }
}

/// Check typed source-evidence consistency before deciding whether a payload
/// is installed. Preserve the v1 intermediate-record semantics during validation.
pub fn validate_overlaps(signal: &Json) {
    let windows = signal
        .get("windows")
        .and_then(Json::as_array)
        .map_or(&[][..], Vec::as_slice);
    let samples = signal
        .get("samples")
        .and_then(Json::as_array)
        .map_or(&[][..], Vec::as_slice);
    let event = signal["encoding"]["kind"] == "event";
    for (index, point) in samples.iter().enumerate() {
        let time = tick(&point["time"]);
        for other in &samples[index + 1..] {
            if tick(&other["time"]) == time {
                assert_legacy_samples(point, other);
            }
        }
        for window in windows {
            if tick(&window["start"]) <= time && time <= tick(&window["end"]) {
                assert_legacy_samples(point, &legacy_sample(window, time, event));
            }
        }
    }
    for (index, left) in windows.iter().enumerate() {
        for right in &windows[index + 1..] {
            let start = tick(&left["start"]).max(tick(&right["start"]));
            let end = tick(&left["end"]).min(tick(&right["end"]));
            if start > end {
                continue;
            }
            if !event {
                let entering = |window: &Json| {
                    if start == tick(&window["start"]) {
                        if window["initial"].is_null() {
                            json!({"missing": true})
                        } else {
                            window["initial"].clone()
                        }
                    } else {
                        legacy_sample(window, start.checked_sub(1).unwrap(), false)
                    }
                };
                assert_legacy_samples(&entering(left), &entering(right));
            }
            let records = |window| {
                legacy_changes(window)
                    .into_iter()
                    .filter(|(tick, _)| start <= *tick && *tick <= end)
                    .collect::<Vec<_>>()
            };
            assert_eq!(
                records(left),
                records(right),
                "inconsistent version-1 overlapping histories"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preparing_an_empty_window_does_not_assert_its_null_initial_is_missing() {
        let earlier = json!({"start":"0","end":"1","initial":null,"changes":[{"time":"0","value":{"bits":"1"}}]});
        let empty = json!({"start":"2","end":"1","initial":null,"changes":[]});
        let windows = [earlier, empty];
        let evidence = WindowEvidence::new(&windows[1], &windows, false);
        assert!(std::panic::catch_unwind(|| evidence.sample(2)).is_err());
    }

    #[test]
    fn prepared_evidence_keeps_repeated_out_of_order_reads_independent() {
        let window = json!({"start":"1","end":"8","initial":{"value":{"real_bits":"0000000000000000"}},"changes":[
            {"time":"1","value":{"real_bits":"7ff8000000000001"}},
            {"time":"3","value":{"real_bits":"7ff8000000000001"}},
            {"time":"5","value":{"real_bits":"8000000000000000"}},
            {"time":"6","value":{"real_bits":"7ff8000000000002"}}
        ]});
        let evidence = WindowEvidence::new(&window, &[], false);
        for (time, changed_at) in [
            (7, None),
            (6, Some("6")),
            (4, None),
            (1, Some("1")),
            (6, Some("6")),
        ] {
            let actual = evidence.sample(time);
            assert_eq!(actual["value"], json!({"real_bits":"7ff8000000000000"}));
            assert_eq!(actual.get("changed_at").and_then(Json::as_str), changed_at);
        }
        let events = json!({"start":"1","end":"4","initial":null,"changes":[
            {"time":"1","value":{"event":true}}, {"time":"1","value":{"event":true}},
            {"time":"3","value":{"event":true}}, {"time":"3","value":{"event":true}}
        ]});
        let evidence = WindowEvidence::new(&events, &[], true);
        for (time, count) in [(3, "2"), (2, "0"), (1, "2"), (4, "0"), (3, "2")] {
            assert_eq!(evidence.sample(time), json!({"occurrences":count}));
        }
    }

    #[test]
    fn excursions_and_raw_timestamps_are_not_net_changes() {
        let window = json!({"start":"10","end":"20","initial":{"value":{"bits":"0"},"changed_at":"8"},"changes":[
            {"time":"10","value":{"bits":"1"}}, {"time":"10","value":{"bits":"0"}},
            {"time":"12","value":{"bits":"1"}}, {"time":"13","value":{"bits":"0"}},
            {"time":"13","value":{"bits":"1"}}, {"time":"14","value":{"bits":"1"}}
        ]});
        assert_eq!(changes(&window), [(12, json!({"bits":"1"}))]);
        assert_eq!(sample(&window, 10, false), json!({"value":{"bits":"0"}}));
        assert_eq!(
            sample(&window, 13, false),
            json!({"value":{"bits":"1"},"changed_at":"12"})
        );
        let raw = json!({"value":{"bits":"1"},"changed_at":"13"});
        assert_eq!(
            sample_assertions(&raw, std::slice::from_ref(&window), 13)["changed_at"],
            "12"
        );
        assert_eq!(raw["changed_at"], "13");
        assert!(sample_assertions(&raw, &[], 13).get("changed_at").is_none());
    }

    #[test]
    fn first_unknown_establishment_and_event_counts_are_distinct() {
        let persistent = json!({"start":"0","end":"9","initial":null,"changes":[
            {"time":"2","value":{"bits":"0"}}, {"time":"2","value":{"bits":"x"}}
        ]});
        assert_eq!(sample(&persistent, 1, false), json!({"missing":true}));
        assert_eq!(sample(&persistent, 2, false), json!({"value":{"bits":"x"}}));
        assert_eq!(changes(&persistent), [(2, json!({"bits":"x"}))]);
        let events = json!({"start":"0","end":"9","initial":null,"changes":[
            {"time":"0","value":{"event":true}}, {"time":"0","value":{"event":true}}, {"time":"3","value":{"event":true}}
        ]});
        assert_eq!(
            changes(&events),
            [
                (0, json!({"occurrences":"2"})),
                (3, json!({"occurrences":"1"}))
            ]
        );
        assert_eq!(sample(&events, 0, true), json!({"occurrences":"2"}));
        assert_eq!(sample(&events, 2, true), json!({"occurrences":"0"}));
        assert_eq!(sample(&events, 9, true), json!({"occurrences":"0"}));
    }

    #[test]
    fn nan_payload_gaps_do_not_invent_change_times() {
        let window = json!({"start":"0","end":"9","initial":{"value":{"real_bits":"7ff8000000000001"}},"changes":[
            {"time":"1","value":{"real_bits":"fff8000000000002"}},
            {"time":"3","value":{"real_bits":"0000000000000000"}},
            {"time":"4","value":{"real_bits":"7ff0000000000001"}}
        ]});
        assert_eq!(
            changes(&window).iter().map(|(t, _)| *t).collect::<Vec<_>>(),
            [3, 4]
        );
        assert!(sample(&window, 1, false).get("changed_at").is_none());
        assert_eq!(sample(&window, 3, false)["changed_at"], "3");
        assert_eq!(sample(&window, 4, false)["changed_at"], "4");
        assert!(sample(&window, 5, false).get("changed_at").is_none());
        assert_ne!(
            legacy_value(&json!({"real_bits":"0000000000000000"})),
            legacy_value(&json!({"real_bits":"8000000000000000"}))
        );
        assert_ne!(
            legacy_value(&json!({"string":"é\0"})),
            legacy_value(&json!({"string":"é"}))
        );
    }

    #[test]
    #[should_panic(expected = "inconsistent point/window value evidence")]
    fn contradictory_point_and_window_evidence_is_rejected() {
        let window = json!({"start":"0","end":"2","initial":null,"changes":[{"time":"0","value":{"bits":"1"}}]});
        sample_assertions(&json!({"value":{"bits":"0"}}), &[window], 1);
    }

    #[test]
    #[should_panic(expected = "inconsistent normalized timestamp evidence")]
    fn contradictory_proven_timestamps_are_rejected() {
        let first = json!({"start":"1","end":"3","initial":{"value":{"bits":"1"}},"changes":[{"time":"1","value":{"bits":"0"}}]});
        let second = json!({"start":"1","end":"3","initial":{"value":{"bits":"1"}},"changes":[{"time":"2","value":{"bits":"0"}}]});
        sample_assertions(&json!({"value":{"bits":"0"}}), &[first, second], 3);
    }

    #[test]
    fn typed_overlap_conflicts_preserve_version_one_meaning() {
        for signal in [
            json!({"encoding":{"kind":"bits","width":1},"windows":[
                {"start":"0","end":"3","initial":null,"changes":[{"time":"0","value":{"bits":"0"}}]},
                {"start":"1","end":"3","initial":{"value":{"bits":"1"}},"changes":[]}
            ]}),
            json!({"encoding":{"kind":"bits","width":1},"windows":[
                {"start":"0","end":"3","initial":null,"changes":[{"time":"0","value":{"bits":"0"}}]},
                {"start":"1","end":"3","initial":null,"changes":[]}
            ]}),
            json!({"encoding":{"kind":"event"},"windows":[
                {"start":"0","end":"3","initial":null,"changes":[{"time":"1","value":{"event":true}}]},
                {"start":"1","end":"3","initial":null,"changes":[{"time":"1","value":{"event":true}},{"time":"1","value":{"event":true}}]}
            ]}),
            json!({"encoding":{"kind":"bits","width":1},"windows":[
                {"start":"0","end":"3","initial":null,"changes":[{"time":"1","value":{"bits":"0"}},{"time":"1","value":{"bits":"1"}}]},
                {"start":"0","end":"3","initial":null,"changes":[{"time":"1","value":{"bits":"1"}}]}
            ]}),
        ] {
            assert!(std::panic::catch_unwind(|| validate_overlaps(&signal)).is_err());
        }
        let window = json!({"start":"0","end":"3","initial":null,"changes":[{"time":"0","value":{"bits":"1"}}]});
        assert!(
            std::panic::catch_unwind(|| sample_assertions(&json!({"missing":true}), &[window], 1))
                .is_err()
        );
        let window = json!({"start":"0","end":"3","initial":null,"changes":[{"time":"1","value":{"event":true}}]});
        assert!(
            std::panic::catch_unwind(|| sample_assertions(
                &json!({"occurrences":"2"}),
                &[window],
                1
            ))
            .is_err()
        );
    }

    #[test]
    fn proven_initial_times_propagate_only_through_complete_coverage() {
        let first = json!({"start":"0","end":"9","initial":null,"changes":[
            {"time":"0","value":{"bits":"0"}},{"time":"4","value":{"bits":"1"}},
            {"time":"8","value":{"bits":"0"}},{"time":"8","value":{"bits":"1"}}
        ]});
        let second = json!({"start":"10","end":"20","initial":{"value":{"bits":"1"},"changed_at":"8"},"changes":[]});
        let third = json!({"start":"21","end":"25","initial":{"value":{"bits":"1"},"changed_at":"8"},"changes":[]});
        let point = json!({"value":{"bits":"1"},"changed_at":"8"});
        assert_eq!(
            sample_assertions(&point, &[first.clone(), second.clone(), third], 24)["changed_at"],
            "4"
        );
        let mut gap = second;
        gap["start"] = json!("11");
        assert!(
            sample_assertions(&point, &[first, gap], 19)
                .get("changed_at")
                .is_none()
        );
    }

    #[test]
    fn earlier_complete_windows_can_prove_an_initial_time() {
        let window = json!({"start":"0","end":"9","initial":null,"changes":[
            {"time":"0","value":{"bits":"0"}}, {"time":"3","value":{"bits":"1"}},
            {"time":"4","value":{"bits":"0"}}, {"time":"8","value":{"bits":"1"}},
            {"time":"8","value":{"bits":"0"}}
        ]});
        let raw_initial = json!({"value":{"bits":"0"},"changed_at":"8"});
        assert_eq!(
            sample_assertions(&raw_initial, &[window], 9)["changed_at"],
            "4"
        );
    }
}
