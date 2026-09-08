# WLF Integration

## Reader Boundary

WLF is the Questa/ModelSim waveform format in the Ondas model. A reader may use a
vendor library or an independent implementation. Format identity is distinct
from the selected reader, its SDK version, and its runtime configuration. The
illustrative name `vendor-wlf` does not reserve an actual backend identifier.

The adapter exposes only the common read-only [waveform model](model.md), not a
vendor database API. Source handles, callbacks, buffer layout, and native errors
stay private. Record concrete reader support and limitations here from verified
integration evidence rather than assuming every stored object maps to an Ondas
signal.

## Adaptation Constraints

Preserve hierarchy declarations separately from queryable histories, including
aliases, source ranges, constants, type metadata, and unsupported value classes.
Map source time into absolute ticks. Reader-specific scheduling information does
not introduce a public delta-cycle coordinate.

Establish input-mode, resource-lifetime, thread, and reentrancy constraints against
the chosen reader. Do not assume bytes input works because a file reader exists.
Release resources on failed opening, early callback termination, and late query
failure as well as on ordinary completion. Missing libraries or licenses are
availability errors, not evidence that the waveform is malformed.

## Environment, Tests, and Benchmarks

Use an explicit ignored local devcontainer profile for vendor installations,
network settings, and licenses, following [automation](automation.md). Public
source and images must not embed private paths, SDK files, or license topology.
Separate SDK-dependent build output when testing different versions. Prefer a
build/documentation boundary that leaves runtime discovery until opening.

Validate the real adapter through the common [conformance strategy](testing.md)
and external [fixture providers](fixtures.md). Retain artifact provenance and
assert only supported observations. Do not approximate the vendor API with a
large mock or place inaccessible private artifacts behind public sidecars.

Explicit WLF suites fail on missing runtime or fixtures instead of skipping.
They remain separate from the reproducible public CI gate. WLF performance cases
belong to their own format target; reader comparisons use the same artifact,
parameters, and [measurement boundaries](benchmarking.md).
