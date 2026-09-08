# Public API Test Map

Rustdoc in [`src/`](../src/lib.rs) owns the contracts; [testing.md](testing.md)
owns test strategy. This map points to executable observations, not an API
reference or a claim that every branch is tested. Test names below are relative
to the named file's test module.

## Self-contained checks

Run `./dev --exec-only cargo test --locked --lib` in a running development
container. Shared queries use the private `Waveform::memory` helper and exercise
public query results, independently of FST decoding.

| Public surface / contract area | Tests and observations |
| --- | --- |
| Opening functions, `Format`, opening `Error` categories | [`src/waveform.rs`](../src/waveform.rs): `opening_errors_distinguish_detection_selection_and_io`; `detection_prefers_content_and_falls_back_to_case_insensitive_extensions` checks recognition precedence, extension hints, explicit selection, and I/O rejection. Successful FST opening is covered by the external paths below. |
| `Waveform::metadata`, `Metadata` optional fields and comments | `metadata_preserves_optional_fields_and_comment_order` in `src/waveform.rs` checks normalized metadata access, absent fields, ordered duplicate/empty comments, and iterator length. It does not test reader extraction. |
| `Hierarchy` traversal/lookups/aliases; `Scope`, `Variable`, enumeration and declaration metadata | [`src/hierarchy/tests.rs`](../src/hierarchy/tests.rs): `hierarchy_views_aliases_and_metadata`, `root_declarations_duplicate_scopes_and_extreme_ranges`; includes cloned hierarchy lifetime, root declarations, duplicate scopes, original range endpoints, and enumeration labels. |
| Exact paths, selectors, `LookupError`, `PathError`, `PathFormatError` | `exact_lookup_precedes_selectors`, `paths_round_trip_and_keep_exact_components`, `quoted_json_escapes_and_surrogates`, `path_errors_have_byte_offsets`, `exact_unicode_paths_do_not_normalize_names`; covers parsed/component-built paths, `FromStr`, canonical/Verilog spelling, byte offsets, ambiguity, signal-less declarations, and distinct Unicode component sequences. |
| `Signal` identity/projection methods, `Encoding`, `SliceError`, `BitRange` | `signals_compose_normalize_and_validate`, `exact_lookup_precedes_selectors`, `unrepresentable_range_width_panics` in `src/hierarchy/tests.rs`; equality, full-width normalization, relative slicing, base aliases, bounds, non-bit rejection, foreign handles, and range overflow. |
| `Time`, `TimeRange`, `TimeSpan`, `Timescale`, `TimeUnit` | [`src/time.rs`](../src/time.rs): `inclusive_ranges_preserve_bounds`, `exact_time_metadata`; inclusive/reversed/unbounded ranges, zero and maximum ticks, exact units/factors, and const evaluation. Query effects are checked separately below. |
| `Bits`, `BitsRef`, `Logic`, `Value`, `ValueRef` | [`src/value.rs`](../src/value.rs): `ascii_states_and_iteration`, `slices_and_owned_storage`, `values_round_trip_and_compare`, `real_dedup_uses_nan_equivalence_and_signed_zero`; nine-state ordering/indexing, empty vectors, non-byte-aligned slices, source-independent ownership, strings, real payloads, events, and internal value comparison rules. |

The following tests are in [`src/query/tests.rs`](../src/query/tests.rs), through
`Waveform` one-shot methods and reusable `Selection` queries:

| Public surface / contract area | Tests and observations |
| --- | --- |
| `sample`, `samples`, `visit_samples`, `Sample` / `SampleRef` | `samples_hold_final_tick_state_and_count_every_event`, `genuine_tick_zero_events_and_borrowed_sample_breaks_survive`; missing persistent state, exact event multiplicity, final same-tick state, selection order/duplicates, result signal access, and borrowed early stop. |
| `select`, selection signal/hierarchy access, `scan`, `traces` | `scan_emits_initials_first_and_keeps_duplicate_entries`, `multiple_initials_follow_selection_order_not_history_order`; distinct histories enter in selection order, duplicate entries retain observations, events/missing histories have no initial, and all initials precede changes. Cross-signal order within a tick is not required. |
| `trace`, `Trace`, `Initial`, `Change`; projection and candidate coherence | `slices_have_independent_changes_and_entering_states`, `composed_projections_cohere_at_inclusive_boundaries`; explicit expected values/times for composed/equal projections, hidden base changes, intermediate same-tick values, initial timestamps, trace bounds, final samples, and increasing in-range candidate supersets. |
| Empty ranges/histories, EOF bounds, candidate callbacks | `empty_ranges_and_selections_do_not_visit_or_read`, `empty_histories_and_eof_bounds_are_successful_observations`; no calls/reads for empty selections or reversed ranges, missing versus zero events, unbounded ranges ending at EOF (empty when starting later), and held state in bounded ranges after EOF. |
| Owned persistence across callbacks, queries, and waveform destruction | `borrowed_mixed_values_remain_owned_after_queries_and_source_drop`; retained sample/scan copies and owned traces/samples preserve real, Unicode/NUL string, and nine-state values against explicit expectations. |
| `ControlFlow` completion/break and query errors | `breaks_stop_the_reader_and_late_errors_preserve_prior_callbacks`, `invalid_and_unsupported_signals_fail_before_queries`; scan/candidate early stop, late scan observations, failed owned queries, no sample callbacks on read failure, and foreign/unsupported handles. |

## Reader-specific evidence and limits

[`tests/conformance.rs`](../tests/conformance.rs) contains external FST and VCD
observations. `full_fst_pool` and `full_vcd_pool` discover every artifact of their
format, and `pool_queries` batches
all listed samples/windows, with per-case results and no failure allowlist.
Listed declarations are matched from public traversal; focused cases additionally
exercise exact lookup and alias iterators. Native kind normalization has regressions
`real_parameter_kind_is_canonical` and `compound_scope_kinds_are_canonical` in
[`src/backends/fst.rs`](../src/backends/fst.rs). Full-pool metadata assertions also
check present-but-empty header text.
Its `open` helper explicitly selects the corresponding native reader for file and bytes input;
`metadata`, `hierarchy`, `sample_queries`, `window_queries`, and `scan_queries`
apply oracle assertions through the public API. `automatic_opening_uses_content_and_keeps_logical_names`
checks automatic opening, `Waveform::format` / `backend`, and logical source names.
`counter_slice_projections_file` / `_bytes` and `foreign_handle_validation_file` /
`_bytes` provide targeted reader-backed checks. These paths require the fixture
provider and are not executed by the library-unit command above.

[`src/backends/fst.rs`](../src/backends/fst.rs) also has self-contained adaptation
checks: `ranges_require_an_explicit_separated_bit_suffix` and
`exact_timescales_do_not_wrap_or_use_floating_point`.

[`tests/vcd_native.rs`](../tests/vcd_native.rs) runs without external inputs and
checks successful opening, full-body rejection, exact large ticks/decimal scales,
metadata extraction, contextual codes, literal array names, duplicate declarations,
zero-width signals, real/string classification, Latin-1 escapes/literal bytes,
dumpall-only state, strict blackout-boundary entering states, event checkpoints,
alias projections, same-tick ordering, Break/replay and output ownership. Run it
with `./dev cargo test --locked --test vcd_native`.

Memory tests do not establish FST buffer reuse, resource bounds, malformed-input
hardening, or vendor-reader behavior. Oracle assertions cover only supplied
observations; opening a file is not semantic coverage of its contents. Optional
first-observation `changed_at` and extra candidate times remain allowed. FST's
first-tick event limitation is documented in [crate rustdoc](../src/lib.rs), not
imposed on genuine memory-backend events. Reserved error variants without an
active reader path are not exercised by constructing synthetic enum values.
