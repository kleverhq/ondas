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

The initial triage changed no tracked implementation files and made no issue updates.

## Execution plan

Implement in dependency order: #20 (literal names), #23 (visibility), #19
(datatype decoding), #24 (identity/source spelling), #21 (hierarchy performance),
then #22 (cold point sampling). Each issue receives its own implementation commit,
read-only critical review by `sol` at high reasoning, and a GitHub issue comment
with the commit and verification evidence. The main agent implements and
investigates; subagents only review. Do not merge or publish.

Use `./dev` for all project commands. SDK work uses the canonical ignored local
profile and private lock copied from `ondas-private`, with any previous local
profile backed up under `tmp/`. Never modify provider repositories. SDK-derived
scratch and logs stay ignored. Run the smallest regression first; then the
relevant full gate (`just ci-fsdb` with required private fixtures for FSDB changes,
`just ci` plus MSRV for shared/public changes). All commits must pass installed
host hooks. Before the final handoff, run public CI, strict FSDB CI, MSRV,
doctests and package dry-runs on the final state.

Observable completion is the acceptance behavior in each issue, not merely
compilation: exact-name ambiguity for #20, available hidden metadata for #23,
readable integral enums with labels for #19, consistent identities plus source
spelling for #24, independently measured reductions of opening/teardown overhead
for #21 and cold sampling overhead for #22 without semantic regressions. Record
actual before/after timings; no unverified speedup target is promised.

### Progress

- [x] Commit initial triage (`0840202`).
- [x] #20: escaped scalar range suffix; normalization and duplicate-name lookup
  regressions pass; strict SDK/private gate passed. Critical review caught a
  separately printed scalar range after an escaped identifier; corrected and
  added space/tab boundary coverage.
- [x] #23: public local `Scope::is_hidden`, native SDK flag propagation,
  conservative duplicate-scope conflict handling, SDK/demo and hierarchy
  regressions. Strict SDK/private gate and doctests passed; `sol` high critic
  returned no substantive findings. No hierarchy filtering is performed.
- [x] #19: read SDK datatype blocks before declarations, preserve enum1/2/3
  names and value/label tables, accept typed enum/packed variable callbacks, and
  decode supported per-bit enum histories. The real SDK demo regression verifies
  width, kind, table and initial value. Strict SDK/private gate passed; `sol`
  high critic reported no substantive findings. Custom non-per-bit histories
  remain explicitly unsupported, not inferred from enum labels.
- [ ] #24: implementation and `sol` high review complete; converter-backed
  VCD/FST file+bytes regression passes. Diff saved in the stash named
  `issue24: reviewed provenance implementation pending provider authorization`.
  Public FST conformance exposed a locked oracle retaining an escape marker in
  identity; strict private FSDB conformance also needs oracle path corrections.
  Awaiting explicit permission for provider changes before completing this task.
- [x] #21: defer the full variable-path index with shared `OnceLock`; preserve
  exact lookup/ambiguity semantics. On the same public large FST, ten paired
  release measurements changed Ondas median open/open+drop from 133.91/233.58 ms
  to 33.56/39.03 ms; Wellen controls stayed near 50/55 ms. The first variable
  lookup still pays the construction cost. Public conformance, MSRV, doctests,
  regression tests passed; `sol` high review was clean.
- [x] #22: cold bit-only SDK point seeks finish each aligned tick and compare
  earlier completed projected states to prove exact `changed_at`. Mixed/event
  selections retain chronological replay. The bounded after-point snapshot is
  reusable only for the same point or later windows. Independent chronological
  reference tests, quiet-cache regressions, full public and strict SDK/private
  gates, MSRV, doctests and both package dry-runs passed. A replacement `sol` high
  critic completed cleanly after the first attempt hit its provider usage limit.
  Exact issue workload, five alternating release pairs: median query 4817.18 ms
  before versus 106.90 ms after (351 unique signals); complete sample values and
  change timestamps matched the baseline.
- [ ] Final validation and temporary-document cleanup before merge.

### Surprises & discoveries

The installed converter's launcher requires Bash and its executable also requires
GUI runtime libraries absent from the reader-only container. Do not add GUI
packages to the public development image merely for a regression fixture.
The #20 regression therefore exercises the SDK descriptor normalization and
public hierarchy lookup directly; the existing SDK corpus remains the native
integration gate. For #22 benchmarking, missing freely distributable runtime
libraries were supplied under ignored `tmp/`, and the existing mounted SDK
converter produced the public picorv32 input. No public image or canonical
profile was changed. SDK-generated root logs were moved into ignored scratch.

### Decision log

- 2026-09-22: Keep authoritative SDK bounds separate from the scalar `[0:0]`
  name fallback. An attached suffix is literal for escaped names; a whitespace-
  separated suffix can still declare a scalar range. Escaped vectors also retain
  authoritative SDK bounds and their separate printed range suffix.
- 2026-09-22: SDK `ffrReadDataTypeDefByBlkIdx(0)` reads all existing blocks in
  one call; do not copy the neighboring consumer's extra block loop. Copy enum
  strings under the SDK lock, using native declaration indices rather than
  shared signal indices. The dedicated SDK enum regression exercises this ABI.
- 2026-09-22: Preserve one implementation commit per issue; update this living
  plan alongside each completed task rather than creating progress-only commits.

### Outcomes & retrospective

#20 (`ac2d471`), #23 (`9b97938`) and #19 (`8369f6c`) are pushed with validation
comments on their respective issues. #24 has a newly discovered provider-boundary
blocker, not a waiver of conformance. #21 avoids work that metadata-only consumers
do not need rather than changing the lookup algorithm or claiming faster queries.

The #22 implementation was also compiled against the installed Verdi 2021 SDK;
its SDK demo enum regression passed. This supplements, but does not replace,
strict runtime conformance with the canonical SDK profile.

Implementation is in progress. Final conclusions will be recorded with measured
results, remaining limitations, review outcomes and linked issue comments.

Plan revision: 2026-09-22, added the implementation sequence, acceptance and
verification policy following the request to execute all six issues.
