# VCD reader

`vcd-native` reads files and shared owned bytes without a parser dependency. It
shares input, hierarchy and query code with FST, not format decoding. IEEE
1364-2001 clause 18.2 is the baseline; producer extensions are compatibility
choices, not standard requirements.

## Opening and queries

Opening validates the entire body, determines exact time bounds and fixes storage
encodings before returning signal handles. The reader keeps the source,
declarations and identifier mappings, not histories. File input is buffered;
bytes input retains its shared allocation. Sources must remain unchanged while
open.

Queries rewind and replay selected signals through the requested tick. Shared
query code handles entering state, projections, redundant writes, samples and
termination. This repeats parsing rather than using an index, cache, checkpoint
database, mandatory mmap or parallel parser. Performance tuning and large-file
measurements follow the [benchmarking policy](benchmarking.md).

## Declarations and values

Printable identifier codes are contextual tokens, not numbers or commands.
Hierarchy components preserve UTF-8 spelling. Compatible aliases share storage,
identical repeated declarations merge, and conflicting names with different
identifiers remain ambiguous.

Separated bit ranges preserve direction and must agree with width. An attached
`[msb:lsb]` is a range only if its width agrees. Literal array indices and escaped
brackets stay in names. Canonical kind names use hyphens; known SV/VHDL kinds map
to the common model without exposing simulator types. FST-style attributes are
recognized but do not populate rich type metadata.

Bits preserve all nine states. Short vectors extend with zero or their leading
nonbinary state. Over-width vectors keep the least-significant digits after
validation of the full payload. Zero-width bits, including known empty binary
records, remain non-queryable rather than becoming real signals.

Checked real parsing preserves signed zero and named NaN/Inf. Strings decode
common C, octal and two-digit hex escapes into Latin-1, preserving literal high-bit
bytes, NULs and padding. A real declaration carrying only strings gets string
storage, even for numeric-looking text. Mixed real/string histories fail.

Times are exact integral decimal u64 ticks, including `.0` forms. Fractional,
backwards and overflowing timestamps fail. Decimal timescales normalize to an
exact supported factor/unit without floating-point rounding. Metadata preserves
internal whitespace, present-empty fields and comment order. A final token needs
no newline; a UTF-8 BOM is accepted. Migen's closed declaration list may enter
`$dumpvars` without `$enddefinitions`.

## Controls and limits

Persistent records inside `$dumpvars`, `$dumpall`, `$dumpoff` and `$dumpon` apply
at the current tick. Markers do not invent off-values or hidden circuit activity.
`changed_at` describes recorded transitions; a resume checkpoint cannot establish
when an unrecorded physical transition occurred. Entering state is strictly before
the query boundary.

Ordinary event records retain repetitions, including at the first tick. Records
inside dump blocks are snapshots and do not count as occurrences. This policy
does not recover physical multiplicity on mixed checkpoint/event ticks. The
public oracle excludes those ticks and non-bit blackout windows.

Unknown significant commands or identifiers, incompatible storage aliases,
invalid values, incomplete scopes/dump blocks and no-declaration inputs fail
opening. Nonzero timezero and EVCD port/strength records are unsupported; a `port`
declaration can carry ordinary bits. Malformed input never becomes a successful
empty waveform.

## Verification

Self-contained tests cover lexical boundaries, exact arithmetic, classification,
controls, replay and ownership. External conformance discovers every VCD in the
locked provider and checks listed metadata, declarations, samples and windows in
file/bytes modes. The independent instrumented libgtkwave oracle supplies the
expectations, not this reader. Sparse agreement is not exhaustive certification;
see [testing](testing.md) and [fixtures](fixtures.md).
