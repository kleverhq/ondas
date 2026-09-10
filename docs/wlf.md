# WLF integration

WLF is the Questa/ModelSim waveform format. A reader may use a vendor library or
an independent implementation. Format, reader, SDK version and runtime settings
are separate choices; `vendor-wlf` is an example name, not a reserved backend.

Expose the read-only [common model](model.md), not a vendor database API. Handles,
callbacks, buffer layouts and native errors stay private. Record support and
limits from verified integration evidence; not every stored object is an Ondas
signal.

## Mapping and resources

Keep declarations separate from histories, preserving aliases, ranges, constants,
type metadata and unsupported classes. Map time to absolute ticks without adding
a public delta-cycle coordinate.

Verify input modes, lifetimes, threading and reentrancy against the chosen reader.
File support does not imply bytes support. Release resources after failed opening,
early termination and late query errors as well as normal completion. Missing
libraries or licenses are availability errors, not malformed waveforms.

## Environment and verification

Use an ignored local profile for vendor installations, network settings and
licenses under [automation policy](automation.md). Keep private paths, SDK files
and license topology out of public source/images. Separate SDK-dependent outputs
across versions; prefer runtime discovery at opening over build-time discovery.

Test the actual adapter through [conformance](testing.md) and external
[providers](fixtures.md). Preserve provenance and assert only established
observations. A large vendor mock is not a substitute, and public sidecars must
not refer to inaccessible private artifacts.

Explicit WLF suites fail on missing runtimes or fixtures and remain separate from
public CI. Performance cases use a WLF target; compare readers on the same
artifact, parameters and [measurement boundaries](benchmarking.md).
