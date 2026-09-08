# Native VCD Reading

## Reader Boundary

`vcd-native` independently reads files and owned shared bytes. It shares only
format-neutral input, hierarchy and query infrastructure with FST; it has no
parser dependency. IEEE 1364-2001 clause 18.2 is the baseline. Accepted producer
extensions are compatibility behavior, not claims about that standard.

Opening sequentially validates the complete body, determines exact time bounds
and fixes each identifier's storage encoding before public signal handles exist.
Only declarations, identifier mappings and the source are retained, not value
histories. Queries rewind to the body and replay a batch of selected signals
through the requested tick; the common query engine handles entering state,
projections, redundant writes, samples and callback termination. File inputs use
buffered I/O; bytes inputs retain their shared allocation. Sources must remain
unchanged while open.

This correctness-oriented strategy repeats parsing for later queries. It uses no
index, cache, checkpoint database, mandatory mmap or parallel parser. Performance
tuning and large-file benchmarking are separate work under the
[benchmark methodology](benchmarking.md).

## Declarations and Values

Opaque printable identifier codes are contextual tokens, not numbers or commands.
Hierarchy components preserve UTF-8 spelling without viewer-style flattening.
Compatible aliases share storage; identical repeated declarations are coalesced.
Conflicting names with different identifiers remain ambiguous. Separated bit
ranges retain direction and must agree with width; an attached `[msb:lsb]` is a
range only when its width agrees. Literal array indices and escaped brackets are
not stripped. Canonical kind names use hyphens; known SV and VHDL kinds are mapped
without exposing a simulator type system. Optional FST-style attribute annotations
are recognized but do not populate rich type metadata.

All nine logic states are preserved. Short vectors extend with zero, or their
leading nonbinary state; over-width vectors retain the least-significant digits
only after the entire payload is validated. Zero-width bit declarations remain
non-queryable, including known empty binary records; they are not converted to
real signals. Reals use checked float parsing, retaining signed zero and named
NaN/Inf. Strings decode common C, octal and two-digit hex escapes and map bytes
reversibly as Latin-1, including literal high-bit bytes, NULs and padding.
A real declaration carrying only string records becomes string storage, even for
numeric-looking text; mixed real/string histories are rejected.

Timestamps are exact integral decimal u64 ticks, including integral `.0` forms;
fractional, backwards and overflowing times fail. Decimal timescales normalize
to an exact supported unit/factor without floating-point rounding. Metadata keeps
internal whitespace, present-empty fields and ordered comments. A complete final
token needs no newline. A UTF-8 BOM is accepted. Migen's closed declaration list
may enter `$dumpvars` without `$enddefinitions`.

## Recording Controls and Limits

Persistent records inside `$dumpvars`, `$dumpall`, `$dumpoff` and `$dumpon` are
applied at the current tick. Off/on markers do not invent universal replacement
values or hidden circuit activity. `changed_at` describes observed recorded
transitions; a resume checkpoint cannot establish when an unrecorded physical
transition occurred. Entering state remains strictly before the query boundary.

Ordinary event records preserve repeated occurrences, including the first tick.
Event records inside dump blocks are checkpoint snapshots and are not emitted as
occurrences. This is a deterministic recording interpretation, not a claim to
recover physical event multiplicity on mixed checkpoint/event ticks. The public
oracle intentionally excludes those ambiguous ticks and non-bit blackout windows.

Unknown significant commands, unknown identifiers, incompatible storage aliases,
invalid values and incomplete scopes/dump blocks fail opening. Nonzero timezero
and EVCD port/strength records are unsupported; the known `port` declaration kind
can carry ordinary bit records. No-declaration inputs are rejected. Malformed
input is not converted to a successful empty waveform.

## Verification

Tiny self-contained tests exercise lexical boundaries, exact arithmetic,
classification, controls, replay and ownership. The common external conformance
runner discovers every VCD in the locked provider and checks all listed metadata,
declarations, samples and windows in both file/bytes modes. Expectations come from
the independent instrumented libgtkwave oracle, never from this implementation.
Sparse agreement is not exhaustive certification; see [testing](testing.md) and
[fixture contracts](fixtures.md).
