# WLF namespace ownership and bounded delivery

`native/wlf.c`, `src/backends/wlf.rs` and `src/backends/wlf/worker.rs` own the SDK boundary. Public setup and supported values are in crate Rustdoc.

## Header compatibility is not ABI qualification

The shim uses exported interfaces absent from the SDK header, including setCurrentNamespace and wlfFileOpenByMode. SV enum tables also need a qualified private descriptor layout. A public API version label does not establish compatibility with those interfaces. Qualify each selected headers/runtime pair with real data; a successful symbol lookup cannot establish descriptor semantics.

The SDK must have no ELF SONAME. Its absolute library path propagates into the executable; changing QUESTA_HOME after compilation does not switch runtimes. Missing native dependencies can fail process startup even for another input format. The standalone consumer exists to catch dependencies hidden by Cargo's loader environment.

## One worker protects native global state

The SDK has one active resolution and can retain expression lookup pointers into a closed file. One worker owns all SDK objects. Before changing resolution or reading another header, it releases native query owners. The shim selects the live namespace before reading or releasing resources and clears expression lookup pointers at owner boundaries. A retained lite namespace owner prevents later opens from using a namespace left by a closed file; SDK initialization remains active for the process lifetime.

Caller visitors execute outside the worker and need not be Send. A suspended logical query keeps owned declarations and a small read position, not SDK pointers. On resumption it reopens and replays from its query start, discarding completed-tick slots already delivered. This preserves pending caller state and event counts across nested independent reads without retaining a complete history.

## A batch boundary must be a tick boundary

The shim bounds ordinary responses by records and value bytes, but one complete tick can exceed both thresholds. Splitting that tick would publish provisional state or lose event multiplicity. Valid earlier ticks can precede a later conversion error; a visitor break before that error takes priority. SDK caches and caller-owned traces remain separate storage costs.

Cold bit-only points share a short range read. If that range cannot prove a projection's change time, unresolved projections share one chronological pass. Constant projections can require the full prefix. Cold bit-only windows establish entering state before their start; other cold queries, especially events, retain chronological traversal. Seek-skipped records are not decoded or validated. Prepared selected-state reuse is distinct from a retained history.

## Producer and formatter boundaries

STARTLOG and ENDLOG are not event occurrences. Writer collapse settings can remove activity before reading, and logging resumes do not establish hidden physical transitions. Stored persistent state can remain available across a logging gap without describing unrecorded activity.

The native string formatter returns contents for the qualified string type; arbitrary object display text is not a string transport. Embedded-NUL contents are unavailable. SV enum keys use the same native word conversion as values. Qualified VHDL enums instead need checked ordinal-to-label conversion; array containers have no inferred scalar packing or table.

Verilog escape removal needs recognized terminating syntax. The vcd2wlf importer can truncate escaped names or split a source scope, with the same loss visible in vendor exports. Ondas cannot reconstruct that source text. Diagnostic spelling is not an alternate addressing grammar.

Native faults can terminate the process, and SDK diagnostics can appear on process streams. A worker thread preserves thread ownership; it does not provide process isolation.
