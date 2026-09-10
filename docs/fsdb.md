# FSDB integration

FSDB is proprietary. A reader may use a vendor library or an independent
implementation; the format does not choose a product, SDK version or backend
name. Assign a stable lower-kebab-case name when integrating a reader.
`vendor-fsdb` is an example, not a reserved name.

Expose the [common waveform model](model.md), not the vendor database API. Keep
handles, callbacks, native buffers and FFI private.

## Runtime and resources

Missing libraries or licenses are availability errors, not malformed input or
query failures. Reject unsupported bytes input explicitly. A reader need not
support both input modes, and explicit selection must never fall back.

Manage resources through partial initialization, preparation, callback termination,
late errors and successful completion. Borrowed buffers cannot escape callbacks;
retained observations need owned data. Check thread and reentrancy constraints
against the actual reader.

Document dependencies and limits only after verification. Do not publish guessed
SDK paths, environment variables, version matrices or linker flags. Private
installation paths, license topology and credentials stay out of public docs.

## Builds and tests

Follow [automation policy](automation.md): use ignored profiles for runtime
mounts/environment, separate SDK-dependent outputs, and keep public builds/docs
free of implicit vendor requirements. A documentation build does not prove a
working native integration.

Test the actual reader with external artifacts under the same
[fixture contract](fixtures.md) and conformance suite as open formats. A vendor
mock is not adapter evidence. Public catalogs must not contain placeholders or
sidecars for inaccessible private artifacts.

Explicit FSDB tests and benchmarks fail on missing fixtures, libraries or licenses
rather than skipping. They do not change the public CI gate. Compare readers or
SDK versions using identical workloads and controlled setup under
[benchmarking](benchmarking.md).
