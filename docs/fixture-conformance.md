# Fixture evidence and normalized conformance

The pinned `fixtures/` gitlink owns `fixtures/schemas/fixture.schema.json` and its `$defs.oracle`. It also owns producer validation and payload delivery. `tests/conformance.rs` executes the consumer profile. `tests/support/normalized_oracle.rs` derives independent expected observations. These responsibilities differ. A valid sidecar can contain assertions that the consumer cannot execute. Preflight rejects such assertions instead of counting them as passes.

`justfile` defines installation and reader gates. They never download, repair or generate inputs during a query. Fixture discovery uses the selected format and explicit reader. A change in automatic reader selection therefore cannot remove an adapter from coverage. A deliberately malformed artifact can be a valid negative test.

## Evidence is sparse, but a window is complete

An omitted field makes no assertion. A permitted null asserts absence, except when `changed_at` is unknown. Empty hierarchy lists do not rule out unlisted objects. Metadata, tags, relations and provenance are not value observations. An oracle with only metadata or hierarchy does not prove history behavior. An absent or empty oracle supplies no coverage.

A successful version-1 window contains all ordered observations of one history inside inclusive bounds. It stores state strictly before the start separately. Redundant persistent writes can be absent. Different intermediate values and event multiplicity must remain. Overlapping windows and samples must agree. They are not alternative acceptable answers. A sample alone cannot fill an uncovered interval.

Version-1 event windows contain unit occurrences. Derived per-tick counts are temporary expectations, not new serialized fields. Do not write normalized output back into sidecars or silently upgrade the schema. Schema signal IDs identify source histories, not reader handles. Without native alias identity, distinct handles must each satisfy the same asserted history. Equal observations do not prove aliases.

## Raw writes do not prove normalized change times

Group complete-window records by tick. Derive final state independently of production normalization. A persistent excursion back to its entering value creates no net change. Project values before deriving slice changes. Activity in discarded bits must not advance a slice's change time. Events aggregate at their exact tick. They do not persist across gaps.

A raw `changed_at` can identify a write or an intermediate excursion removed by normalization. Sparse samples can establish values, presence and event counts. They do not necessarily establish a normalized timestamp. A complete window can prove a net-change time after a known entering state. Earlier complete windows can carry that proof into later windows. They cannot carry it across an uncovered interval. First establishment without a known predecessor does not prove a transition time.

Check public timestamp bounds even when no exact time is justified. If a reader supplies a known time, compare it with any proven time.

Version 1 equates all NaNs. It cannot establish payload, sign or signaling-bit identity. Ignore only the additional NaN-payload transitions that this evidence cannot resolve. Actual normalized output must still contain at most one record per slot/tick. It must not repeat an identical persistent representation. Signed zeros and non-NaN binary64 patterns keep exact comparisons.

NaN equivalence also limits timestamp proofs. NaN-to-NaN observations do not prove representation equality. Omitted payload-only writes can invalidate a later establishment time. A definite non-NaN-to-NaN transition proves that tick. It does not prove the change time for every later NaN sample. Use separately authored exact-bit cases for the stronger library identity contract.

## Legacy observations need explicit interpretation

Some pinned version-1 string observations contain non-ASCII values. They now prove `NonAsciiString` rejection, not successful string equality. The independent derivation keeps samples and complete prefixes before the first proven failure bound. Other signals keep their evidence. Sparse evidence can bound an error without identifying the first offending tick. VCD's full opening validation can reject the file before any query.

Native GHDL observations use a conventional 1 fs scale. GHW stores no resolution. Conformance checks this known convention separately and expects absent Ondas timescale. Raw ticks remain unchanged. The convention does not prove physical time.

Some pinned FSDB/WLF/WDB paths keep native Verilog spelling. The explicit audited mapping in `logical_name_oracle` adapts only those inputs. It does not call the production name adapter. Other paths stay exact. Keep raw sidecars, hashes and observations unchanged. Conversion or native spelling does not justify general name heuristics. In particular, WDB can contain literal dots within one native component; do not split them.

## Counters and skips have limited meaning

The shared generated reader counts retained slot values, advances, read starts and leases. These counters can detect replay or buffering regressions in that path. They do not measure a new queue or SDK storage. Add accounting when the measured state changes. SHM collection counters also exclude SST2's internal expression work. A callback API alone does not prove that a reader streams internally.

`src/waveform/semantics_tests.rs` defines reference histories separately from raw observations and production projection code. Keep this independence when changing semantics. Common pools compare point, batch, scan, Trace and query observations against complete oracle windows. Agreement between operations is not the oracle. Reader-specific failures and ABI/loader behavior need evidence outside the sparse schema.

FSDB can report a confirmed newer source version after a failed open. Pool and benchmark wrappers report these skips separately. Focused tests can return after printing SKIP. A passing Rust test count therefore does not prove that its reader path ran. A standalone consumer can also start successfully and then skip a newer input. This proves loading, not query coverage for that file. Other opening failures and observation mismatches remain failures. WLF and SHM do not have this source-version skip rule.

Simulation WDB with qualified DBG 004, 005 and 006 runs in file and byte modes. Recognized non-simulation producers, including ILA recordings, report explicit unsupported skips. Unknown layouts and malformed data are failures. A metadata-only matrix does not establish history support. Wide enum constant controls establish their tables and constant values; they do not prove dynamic wide enum transport.

## Corpus and artifact identity

Select the gitlink deliberately and materialize payloads explicitly. A fixture release tag locates assets. It does not replace the selected commit. Local sidecar edits or unpublished recordings can support development. They are not reproducible evidence from the released pin. Do not run submodule update over a fixture development branch. Shared results need an available fixture commit and the matching parent gitlink.

For SHM, artifact identity covers the exact component set. The digest hashes the sorted canonical JSON component manifest with ASCII escapes. It does not hash concatenated DSN/TRN bytes. A partial directory or an extra component is a different artifact. Preflight checks hashes and containment. These checks do not belong in every sample query. The loader is `tests/support/fixtures.rs`.
