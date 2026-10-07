# GHW integration

GHW is GHDL's binary waveform format. The planned adapter uses `wellen` to map
declarations and histories to the [common model](model.md). It must keep the GHDL
type system and format-specific database API private. Use real files to establish
supported constructs and decoding limits. Format recognition does not prove
language coverage.

## Mapping

Preserve available type names, enumeration labels/encodings, direction, constants
and source ranges. Leave unknown information absent. A lack of transitions does
not make a declaration constant.

Declaration indices and normalized query-bit positions differ. Preserve each
logic state. Do not reduce these states to binary values. Enumeration metadata belongs
to declarations, not a separate runtime-value variant.

A declaration without a queryable history differs from a history with an
unsupported encoding. Preserve its hierarchy information without inventing values
or failing unrelated lookups.

## Verification

Use real GHW fixtures with the [common conformance suite](testing.md). Check
ranges, types/enumerations, constants/generics, logic states, unsupported encodings
and reader error translation. Sparse observations do not establish complete
hierarchy coverage.

Select `wellen` explicitly and test each supported input mode. GHW performance
cases belong in their own target under the [benchmarking policy](benchmarking.md).
They do not need to reproduce a VCD/FST fixture set.
