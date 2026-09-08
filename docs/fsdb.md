# FSDB Integration

## Reader and Runtime Boundary

FSDB is a proprietary waveform format in the Ondas model. Reading can use a
vendor library or an independent implementation. Format identity does not select
a vendor product, SDK version, or a fixed backend name. Choose a stable
lower-kebab-case backend identifier when a concrete reader is integrated;
`vendor-fsdb` is an illustrative name, not a reserved implementation contract.

Keep reader handles, callbacks, native buffers, and FFI private. Integrate only
the waveform observations described by the [common model](model.md), not a
public wrapper around every vendor database operation.

## Availability and Resources

Distinguish missing runtime dependencies or licenses from malformed waveforms
and failures during a query. A backend that requires file input must reject
unsupported bytes input explicitly; the common opening API does not require
all readers to support both modes. Do not silently fall back from an explicitly
selected reader to another implementation.

Resource ownership must cover successful opening, partial initialization,
selection preparation, callback termination, and late read errors. Borrowed
buffer views cannot escape their callback lifetime; retained observations need
owned data. Evaluate thread and reentrancy constraints against the actual reader,
not assumptions about another vendor API.

Do not record unverified SDK paths, environment variables, supported-version
matrices, or linker flags as instructions. Concrete adapter dependencies,
capabilities, and limitations belong here once established. Private installation
paths, license topology, and credentials never belong in public documentation.

## Build and Verification

Follow [automation policy](automation.md): runtime mounts and environment belong
in an ignored local profile, SDK-dependent build outputs are separated, and
public builds/documentation do not acquire an implicit vendor requirement.
Documentation-only compilation is not evidence of working native integration.

Exercise the real reader against artifacts from a suitable external provider,
using the same [fixture contract](fixtures.md) and public conformance suite as
open formats. A mock vendor API cannot establish adapter correctness. Public
providers must not contain placeholders or sidecars for inaccessible private
artifacts.

An explicitly requested FSDB test or benchmark fails clearly when required
fixtures, libraries, or licenses are unavailable. It does not silently skip, and
its presence must not alter the public CI gate. Compare multiple FSDB readers or
SDK configurations only over identical workloads with controlled setup, following
[benchmarking](benchmarking.md).
