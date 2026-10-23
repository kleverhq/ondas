# WLF namespace ownership and bounded delivery

`native/wlf.c`, `src/backends/wlf.rs` and `src/backends/wlf/worker.rs` define the SDK boundary. Crate Rustdoc describes public setup and supported values.

## Header compatibility is not ABI qualification

The shim uses exported interfaces absent from the SDK header. These include setCurrentNamespace and wlfFileOpenByMode. SV enum tables also need a qualified private descriptor layout. A public API version label does not prove compatibility with these interfaces. Check each selected headers/runtime pair with real data. Symbol lookup alone cannot establish descriptor semantics.

The SDK must have no ELF SONAME. Its absolute library path becomes a dependency of the executable. Changing QUESTA_HOME after compilation does not switch runtimes. Missing native dependencies can prevent startup, also for another input format. The standalone consumer checks dependencies that Cargo's loader environment can hide.

## One worker protects native global state

The SDK has one active resolution. It can retain expression lookup pointers into a closed file. One worker owns all SDK objects. It releases native query owners before it changes resolution or reads another header. The shim selects the live namespace before reading or releasing resources. It clears expression lookup pointers at owner boundaries. A retained lite namespace owner protects later opens from a namespace left by a closed file. SDK initialization remains active for the process lifetime.

Caller visitors execute outside the worker and need not be Send. A suspended logical query keeps owned declarations and a small read position. It keeps no SDK pointers. To resume, it reopens and replays from the query start. It discards slots from completed ticks already delivered. This preserves pending caller state and event counts across nested independent reads. It does not retain a complete history.

## A batch boundary must be a tick boundary

The shim limits ordinary responses by record count and value bytes. One complete tick can exceed both limits. Splitting that tick would publish provisional state or lose event multiplicity. Valid earlier ticks can precede a later conversion error. A visitor break before that error takes priority. SDK caches and caller-owned traces remain separate storage costs.

Cold bit-only points share a short range read. This range might not prove a projection's change time. Unresolved projections then share one chronological pass. Constant projections can require the full prefix. Cold bit-only windows establish entering state before their start. Other cold queries, especially events, keep chronological traversal. Seeking can skip records without decoding or validating them. Reusing prepared selected state does not retain a history.

## Producer and formatter boundaries

STARTLOG and ENDLOG are not event occurrences. Writer collapse settings can remove activity before reading. A logging resume does not establish hidden physical transitions. Stored persistent state can remain available across a logging gap. It does not describe unrecorded activity.

The native string formatter returns contents for the qualified string type. Arbitrary object display text is not a string transport. Embedded-NUL contents are unavailable. SV enum keys use the same native word conversion as values. Qualified VHDL enums need checked ordinal-to-label conversion. Array containers have no inferred scalar packing or enum table.

Verilog escape removal needs recognized terminating syntax. The vcd2wlf importer can truncate escaped names or split a source scope. Vendor exports show the same loss. Ondas cannot reconstruct the source text. Diagnostic spelling does not define another addressing grammar.

Native faults can terminate the process. SDK diagnostics can appear on process streams. A worker thread preserves thread ownership. It does not provide process isolation.
