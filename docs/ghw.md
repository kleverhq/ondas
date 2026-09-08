# GHW Integration

## Reader Boundary

GHW is the GHDL waveform format in the Ondas model. Its open-source reader
integration uses `wellen`. The adapter maps source declarations and histories
into the [shared model](model.md); it does not expose a GHDL type system or a
format-specific database API.

Treat artifacts as binary data. Use real GHW files to establish supported source
constructs and record concrete decoding limitations here. Do not infer complete
language coverage from recognition of the file format.

## Declaration and Value Mapping

Keep declaration type names, enumeration labels and encodings, direction,
constant properties, and source ranges where the reader provides them. Unknown
information remains absent rather than guessed. A constant declaration is not
defined by a history with no transitions.

Preserve the distinction between declaration indices and normalized query-bit
positions. Represent supported multi-state logic without collapsing distinct
states to binary zero or one. Enumeration metadata describes declarations; it
does not introduce a separate public runtime-value variant.

Declarations that cannot supply queryable histories and histories with unsupported
value encodings are different cases. Preserve available hierarchy information
and use the corresponding public contracts instead of failing unrelated lookups
or synthesizing values.

## Verification and Measurement

Run the common [conformance suite](testing.md) on real GHW fixtures, emphasizing
source ranges, type and enumeration metadata, constants/generics, logic states,
and the boundary of unsupported encodings. Apply only assertions supported by
artifact evidence; sparse oracles do not claim exhaustive hierarchy coverage.

Use explicit `wellen` selection and exercise each input mode that the integration
supports. Check both observation correctness and reader failure translation.
GHW performance workloads belong in their own format target under the
[benchmarking contract](benchmarking.md); they need not duplicate a VCD/FST
fixture set.
