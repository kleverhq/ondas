# Waveform observations and identity

Rustdoc in `src/` owns public contracts. This document explains the choices behind them; `docs/images/architecture.drawio.svg` shows the common read path.

## Declarations are not histories

Neither a path nor a native history ID can identify a declaration. FSDB can contain a module and a struct at the same path, with separate children. Aliases can share storage while retaining different declaration metadata. A path-keyed tree or a history-keyed variable table would lose these distinctions. Interpretation belongs to the declaration; observed bits do not establish signedness, a logic domain or an enum mapping.

Name adaptation needs evidence about the source language and spelling. A leading backslash can be Verilog syntax, a literal character or a conversion artifact. Range metadata does not authorize removing brackets from an escaped identifier. Once a converter loses a component boundary or truncates a name, Ondas cannot recover it. Similar spelling and equal histories do not establish a structural relationship.

## Recorded state is not circuit state

The common layer observes completed source ticks. This removes dependence on cross-signal record order and avoids presenting intermediate writes as separate circuit states. A snapshot or recording-resume record establishes an observation, not the time of an unrecorded transition. The first observed value therefore need not have a known change time. Event counts preserve what the reader exposes; they cannot recover producer-collapsed events or distinguish initialization when the reader cannot.

Unknown metadata stays absent rather than becoming a format convention. In particular, a conventional time unit in an external oracle does not establish the source's physical resolution. Likewise, an unavailable observation is not an HDL unknown value.

## Composition keeps availability dependencies

A composition with one observed part fills its Missing parts with X; an entirely Missing composition stays Missing. A result slice depends on that assembled availability. If `hi` is Missing and `lo` is observed, the upper slice of their concatenation is observed X even though a direct `hi` read is Missing. Pushing a projection through concatenation can therefore change the result. Preserve this dependency through nested definitions.

Computed change times also belong to the computed observation, not to the latest timestamp of a part. The engine must compare completed assembled states. A narrow result can require wide or deeply nested operands, so its working state is not bounded by the result width alone. Automatic assembly needs proven membership and bit mapping; adjacency in a name or range is insufficient.

## Query boundaries are not decoder boundaries

Drivers determine when a consumer can inspect state. They do not restrict which selected histories a sequential reader must advance. Non-driver updates still affect current and preceding observations. A selected payload can fail before the condition matches, even if the consumer never requests it. Deferred errors are not part of the contract.

Callback delivery, reader traversal and SDK residency have separate boundaries. Completing a tick can require looking ahead; a native reader can load selected histories before the first callback. `Break` stops delivery but does not undo that work. Bounded common query state excludes source bytes, indices, decompression buffers, SDK caches and caller output. `benchmark-methodology.md` keeps these costs separate.

Reader optimizations must preserve the observations and error/stop rules in Rustdoc, not a particular extra-candidate stream or physical decode schedule. The current explicit-time batch uses chronological prefix traversal rather than the point-read shortcuts. A batch and repeated points can therefore have different costs and validation timing without different successful observations.
