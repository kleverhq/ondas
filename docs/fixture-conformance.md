# Fixture evidence and normalized conformance

The pinned `fixtures/` gitlink owns `fixtures/schemas/fixture.schema.json` and its `$defs.oracle`, along with producer validation and payload delivery. `tests/conformance.rs` executes the consumer profile; `tests/support/normalized_oracle.rs` derives independent expected observations. These are different responsibilities. A valid sidecar can contain assertions the consumer cannot execute; preflight rejects them rather than counting them as passes.

`justfile` owns installation and reader gates. They never download, repair or generate inputs during a query. Fixture discovery uses the selected format and explicit reader so a change in automatic selection cannot remove an adapter from coverage. A deliberately malformed artifact can be a valid negative test.

## Evidence is sparse, but a window is complete

An omitted field makes no assertion. A permitted null asserts absence except for an unknown `changed_at`. Empty hierarchy lists do not rule out unlisted objects. Metadata, tags, relations and provenance are not value observations. A metadata/hierarchy-only oracle does not establish history coverage; an absent or empty oracle supplies none.

A successful version-1 window contains the complete ordered observations of one history inside inclusive bounds, with state strictly before the start stored separately. Redundant persistent writes may be absent, but different intermediate values and event multiplicity must remain. Overlapping windows and samples must agree; they are not alternative acceptable answers. A sample alone does not fill an uncovered interval.

Version-1 event windows contain unit occurrences. Derived per-tick counts are temporary expectations, not new serialized fields. Do not write normalized output back into the sidecars or silently upgrade the schema. Likewise, the schema's signal IDs describe source histories, not reader handles. Without native alias identity, distinct handles must each satisfy the same asserted history; equal observations do not prove aliases.

## Raw writes do not prove normalized change times

Group complete-window records by tick and derive the final state independently of production normalization. A persistent excursion back to its entering value creates no net change. Project values before deriving slice changes; discarded-bit activity must not advance a slice's change time. Events aggregate at their exact tick and do not persist across gaps.

A raw `changed_at` can name a write or an intermediate excursion removed by normalization. Sparse samples can establish values, presence and event counts without establishing a normalized timestamp. A complete window can prove a new net-change time after a known entering state. Earlier complete windows can carry that proof into later windows, but never across an uncovered interval. First establishment with no known predecessor does not prove a transition time. Check public timestamp bounds even when no exact time is justified; when a reader supplies a known time, compare it with any proven time.

Version 1 equates all NaNs. It cannot establish payload, sign or signaling-bit identity. Ignore only the additional NaN-payload transitions that this evidence cannot resolve; actual normalized output must still contain at most one record per slot/tick and no repeated identical persistent representation. Signed zeros and non-NaN binary64 patterns retain exact comparisons.

NaN equivalence also limits timestamp proofs. NaN-to-NaN observations do not prove representation equality, and omitted payload-only writes can invalidate a later establishment time. A definite non-NaN-to-NaN transition proves that tick, not every later NaN sample's change time. Use separately authored exact-bit cases for the stronger library identity contract.

## Legacy observations need explicit interpretation

Some pinned version-1 string observations contain non-ASCII values. They now prove `NonAsciiString` rejection rather than successful string equality. The independent derivation keeps samples and complete prefixes before the first proven failure bound; other signals keep their evidence. Sparse evidence may bound an error without identifying the first offending tick. VCD's full opening validation can reject the file before any query.

Native GHDL observations use a conventional 1 fs scale even though GHW stores no resolution. Conformance checks that known convention separately and expects absent Ondas timescale without changing raw ticks. The convention is not evidence of physical time.

Some pinned FSDB/WLF paths retain SDK Verilog spelling. The explicit audited mapping in `logical_name_oracle` adapts only those inputs and does not call the production name adapter. Other paths stay exact. Keep raw sidecars, hashes and observations unchanged; conversion or SDK spelling is not a reason for broad name heuristics.

## Counters and skips have limited meaning

The shared generated reader counts retained slot values, advances, read starts and leases. These counters can detect a replay or buffering regression in that path; they do not account for a newly added queue or SDK storage. Add accounting when the state being measured changes. SHM collection counters similarly exclude SST2's internal expression work. A callback API alone is not evidence of streaming inside a reader.

The reference histories in `src/waveform/semantics_tests.rs` are authored separately from raw observations and production projection code. Keep that independence when changing semantics. Common pools compare point, batch, scan, Trace and query observations against complete oracle windows rather than treating agreement between operations as the oracle. Reader-specific failures and ABI/loader behavior need their own evidence outside the sparse schema.

FSDB can report a confirmed newer source version after a failed open. Pool and benchmark wrappers report those skips separately. Focused tests can return after printing SKIP, so a passing Rust test count alone does not imply its reader path ran. The standalone consumer can also demonstrate process startup while skipping a newer input; that proves loading, not query coverage for the skipped file. Other opening failures and observation mismatches remain failures. WLF and SHM do not have that source-version skip rule.

## Corpus and artifact identity

Select the gitlink deliberately and materialize its payloads explicitly. A fixture release tag locates assets; it does not replace the selected commit. Local sidecar edits or unpublished recordings can support development, but they are not evidence reproducible from the released pin. Do not run submodule update over a fixture development branch. Shared results need an available fixture commit and the matching parent gitlink.

For SHM, artifact identity covers the exact component set. The digest hashes the sorted canonical JSON component manifest with ASCII escapes, not concatenated DSN/TRN bytes. A partial directory or an extra component is a different artifact. Hash and containment checks belong to preflight, not every sample query. The loader is `tests/support/fixtures.rs`.
