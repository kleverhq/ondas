# VCD Integration

## Reader Boundary

VCD is the textual waveform format in the Ondas format model. Its open-source
reader integration uses the `wellen` backend. Wellen types and source identifiers
stay inside the adapter; callers see the common [library model](model.md).

Reader implementation and format identity are separate. Adding another VCD
reader does not create another public format or a reader-specific query API.
Record concrete dependency constraints and decoding limitations here when they
are established by an integration; do not infer them from a filename extension.

## Normalization

Preserve exact hierarchy components and declaration metadata before applying
Ondas path formatting. Source identifiers are not public signal handles. Map
alias declarations to shared whole-signal identity while preserving each
declaration's name, range, and type metadata.

Translate source timescale and timestamps into exact Ondas ticks and metadata.
Adapt decoded values and events to the shared observation semantics rather than
exposing reader buffers or textual encodings. Do not invent missing ranges or
state to make an input appear more complete.

## Verification

Use real VCD artifacts with backend-neutral [fixture oracles](fixtures.md).
Prioritize escaped names, aliases, declaration ranges, supported value classes,
unknown states, delayed first values, redundant writes, and distinct same-tick
observations. Use malformed inputs for reader error mapping, not missing files
or unavailable dependencies disguised as waveform errors.

A very small inline VCD is useful when the text makes a self-contained test
obvious. Larger and shared cases belong in external providers, not the source
tree. Verify each supported file/bytes input mode explicitly; the public presence
of bytes-opening functions is not proof of support by every reader.

Measure VCD workloads in a format-specific target using the
[common benchmark methodology](benchmarking.md). Text parsing and query costs
need separate setup boundaries; do not interpret an open measurement as a
cold-filesystem result.
