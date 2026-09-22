# Wavepeek integration issue triage

Reviewed on 2026-09-22 against Ondas `351f638f` on `release/1.0.0-rc`.
All six open issues were inspected. A read-only `sol` review with high reasoning
provided additional FSDB code analysis. No SDK reproductions were rerun during
this triage; distinguish source-backed conclusions from independently measured
results below.

The intended follow-up is to address all six issues on the current branch.
The release recommendations below preserve the original triage; they distinguish
release blockers from migration requirements and performance work, not the scope
of that follow-up.

## Summary

Most reports are substantiated, unlike the withdrawn zero-width parameter/event
request. They are not all blockers for a stable library release.

| Issue | Finding | Original recommendation |
|---|---|---|
| [#19: FSDB datatype/enum](https://github.com/kleverhq/ondas/issues/19) | Missing enum metadata also leaves some values unsupported; some declaration callback forms are not handled. | Accept, high priority; address before 1.0. |
| [#20: FSDB escaped identifiers](https://github.com/kleverhq/ondas/issues/20) | A literal name suffix is mistaken for a range and removed. | Correctness fix before 1.0. |
| [#21: FST open/drop](https://github.com/kleverhq/ondas/issues/21) | Opening and dropping a large hierarchy is measurably slower than Wellen. | Accept; not independently a 1.0 blocker. |
| [#22: FSDB cold sample](https://github.com/kleverhq/ondas/issues/22) | A cold late point sample replays preceding history. | Important optimization, not a rushed patch. |
| [#23: FSDB hidden scopes](https://github.com/kleverhq/ondas/issues/23) | The SDK visibility flag is discarded. | Useful, relatively small metadata extension. |
| [#24: declaration spelling](https://github.com/kleverhq/ondas/issues/24) | Cross-format identifier consistency and original-spelling preservation are separate requirements. | Separate the correctness and compatibility aspects. |

## #19: datatype-backed declarations and enumeration metadata

The issue comment materially strengthens the original report: an integral enum
cannot be read even as bits, not merely displayed with its enum labels.

The native adapter recognizes a limited set of primitive datatype codes, does
not read datatype definitions, and handles only the ordinary variable tree
callback. Rust publishes no enum metadata. These code paths explain both the
unsupported encoding and the metadata gap:

- `native/fsdb.cpp:89-113`: primitive encoding classification.
- `native/fsdb.cpp:177-195`: variable callback coverage.
- `native/fsdb.cpp:233-236`: hierarchy loading.
- `src/backends/fsdb.rs:312-315`: absent datatype/enum interpretation metadata.

Address the concrete datatype-backed declarations and values in the reproduction
through the existing enumeration API. Do not expand the task into implementing
the entire SDK type system.

This is a significant FSDB functional gap and should be addressed before 1.0.
The change is not trivial: SDK-backed tests must verify actual value decoding,
widths and enum mappings, rather than merely checking that labels exist.
Datatype callback variants, native ownership and supported SDK versions are
implementation risks.

## #20: literal range suffixes in escaped identifiers

`src/backends/fsdb.rs:279` infers a scalar range from a name ending in `[0:0]`.
The subsequent `declared_name` call removes that suffix. For an escaped identifier,
the suffix can be literal identifier text.

This corrupts signal identity; it is not a cosmetic output-format difference.
Narrowly correct the name-based fallback without discarding authoritative SDK
range metadata. Cover escaped scalar names, duplicate declarations and the
separate ordinary vector case with regressions.

Fix before 1.0. There is no sound basis for rejecting this report.

## #21: FST opening and teardown performance

Independently reproduced with a release executable using the issue's probe,
Ondas from the current checkout and Wellen 0.25.6. The input was the public
Chipyard ClusteredRocketConfig Dhrystone FST. After a warm-up pair, ten pairs ran
in fresh processes with alternating reader order. Measurements use the probe's
internal timer, not launcher startup time.

| Median | Ondas | Wellen |
|---|---:|---:|
| Open | 142.69 ms | 51.11 ms |
| Open plus drop | 252.82 ms | 54.76 ms |

Best observed open times were 134.89 ms and 48.47 ms respectively; best
open-plus-drop times were 201.47 ms and 51.55 ms.

The difference is real. Ondas builds a full-path lookup index while constructing
the hierarchy, copying path components in the process (`src/hierarchy.rs:276-302`).
This is a profiling candidate, not a demonstrated explanation for the entire
opening and teardown difference.

The overhead matters for a short-lived CLI. Accept and profile it, but neither
incorrect results nor a requirement to match Wellen's speed has been established.
Do not delay the stable API solely until performance parity is reached.

The disposable probe and measurement driver are under `tmp/issue-triage/`;
they are not durable repository artifacts.

## #22: cold FSDB point sampling

Cold replay from the beginning is present in the implementation and documented
in `docs/fsdb-lib.md`. The reported 24x difference was not independently measured
during this review.

A fresh `Waveform::samples` selection has no saved replay position. Replacing its
replay with an SDK seek is not automatically correct. An optimized path must
preserve:

- Final persistent state after all records at the requested tick.
- `Missing` before the first known state and held state after the last record.
- Normalized `changed_at`, rather than timestamps of redundant writes or
  net-equal excursions.
- Projection-specific observations and the existing event semantics.

Accept and prioritize this for Wavepeek migration, without promising a particular
speedup before profiling. It becomes an independent 1.0 release blocker only if
a cold-point latency requirement is explicitly adopted. Avoid a rushed native
seek patch that trades correctness for speed.

## #23: hidden-scope metadata

The SDK supplies a hidden-scope flag. The native descriptor, private ABI and
public hierarchy do not preserve it. This is missing functionality, rather than
a clear violation of the current documented hierarchy contract.

Prefer exposing the authoritative flag so consumers can implement their
visibility policy. Do not filter by name, and do not remove subtrees from the
library's default hierarchy merely to reproduce one CLI's output: doing so would
make available data inaccessible. Define how ancestor visibility affects a
consumer's traversal.

Implement this for full Wavepeek migration. In isolation it could also be added
compatibly after 1.0.

## #24: canonical identifiers versus original spelling

Separate two requirements:

1. **Consistent logical identifiers across VCD and FST.** The VCD reader removes
   a Verilog escape introducer, whereas the FST path currently retains it in
   names. Inspect and correct the format boundary before freezing the contract:
   an escape marker must not accidentally become part of signal identity.
2. **Original declaration spelling**, including an unnecessary escape such as
   `\plain`. This is additional information for preserving Wavepeek's CLI
   surface, not a prerequisite for a semantically correct waveform reader.

Relevant paths are `src/backends/vcd.rs:505`, FST scope/variable construction and
`declared_name` in `src/backends/fst.rs`, and the identity/escaping contract in
`src/hierarchy.rs:19-29`.

The original recommendation was not to treat original-spelling preservation as a
blocking Ondas defect. It is nevertheless a legitimate compatibility feature
when explicitly included in the migration scope. Preserving source spelling
must remain separate from canonical identity, and scope names need the same
policy as variable names. Do not require Wellen's presentation conventions to
become Ondas's default formatting.

## Closed #18: zero-width parameter events

[#18](https://github.com/kleverhq/ondas/issues/18) was correctly withdrawn.
The synthetic reproduction demonstrated Wellen's special treatment of zero-width
parameters, not evidence of an actual producer event encoding. Wavepeek is
correcting the test to an explicit event declaration while retaining its event
assertions. No Ondas change is required for that issue.

## Original release prioritization

- Before 1.0: #20, bounded datatype/value support from #19, and the canonical-name
  aspect of #24.
- For migration: #23, followed by focused work on #22 and profiling #21.
- Reject the claim that preserving legacy spelling is inherently required for
  correctness; handle it explicitly as compatibility metadata instead.

## Follow-up constraints and blockers

No hard external blocker is currently established. Before implementation:

- Reproduce #19, #20 and #23 with the real SDK and appropriate fixtures. Earlier
  release preparation passed the strict FSDB gate, but that does not verify
  these new cases. Keep proprietary payloads and derived evidence outside public
  commits and reports.
- Define the #24 boundary between canonical names and optional source spelling
  before adding metadata. Source spelling and identity must not be conflated.
- Profile #21 and #22 independently. There is no verified minimum acceptable
  latency or required speedup yet; correctness tests must remain authoritative.
- Keep all library fixes on `release/1.0.0-rc`. Changes to Wavepeek or fixture
  provider repositories still require explicit permission.
- Before merging, promote durable contracts and conclusions into their owning
  rustdoc/developer documents and remove this temporary task directory.

This triage changed no tracked implementation files and made no issue updates.
