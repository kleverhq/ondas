# Waveform observations and identity

Rustdoc in `src/` defines public contracts. This document explains the reasons for them. `docs/images/architecture.drawio.svg` shows the common read path.

## Declarations are not histories

A path or a native history ID cannot identify a declaration. FSDB can contain a module and a struct at the same path, with separate children. Aliases can share storage but have different declaration metadata. A tree keyed by path would merge separate declarations. A variable table keyed by history would lose their metadata. The declaration defines how to interpret a value. Observed bits do not establish signedness, a logic domain or an enum mapping.

Name conversion needs evidence about the source language and spelling. A leading backslash can be Verilog syntax, a literal character or a conversion artifact. Range metadata does not permit removal of brackets from an escaped identifier. A converter can lose a component boundary or truncate a name. Ondas cannot recover that information. Similar spelling and equal histories do not prove a structural relationship.

## Recorded state is not circuit state

The common layer observes completed source ticks. Results do not depend on record order across signals. Intermediate writes are not separate circuit states. A snapshot or a recording-resume record establishes an observation. It does not establish the time of an unrecorded transition. The first observed value can therefore have an unknown change time. Event counts preserve what the reader exposes. They cannot recover events collapsed by the producer or identify initialization that the reader cannot distinguish.

Unknown metadata stays absent. A conventional time unit in an external oracle does not establish the source's physical resolution. An unavailable observation is also different from an HDL unknown value.

## Composition keeps availability dependencies

A composition with one observed part fills its Missing parts with X. A composition with no observed parts stays Missing. A result slice depends on this rule. Suppose `hi` is Missing and `lo` is observed. The upper slice of their concatenation is observed X, but a direct `hi` read is Missing. Moving a projection inside a concatenation can therefore change the result. Preserve this dependency in nested definitions.

The latest timestamp of a part does not determine the computed change time. The engine must compare completed assembled states. A narrow result can require wide or deeply nested operands. The result width alone therefore does not bound working state. Automatic assembly needs proven membership and bit mapping. Adjacent names or ranges are insufficient.

## Query boundaries are not decoder boundaries

Drivers determine when a consumer can inspect state. They do not restrict which selected histories a sequential reader must advance. Updates outside the drivers still affect current and preceding observations. A selected payload can fail before the condition matches, even if the consumer never requests it. The contract does not guarantee deferred errors.

Callback delivery, reader traversal and SDK memory use have separate boundaries. Completing a tick can require a look ahead. A native reader can load selected histories before the first callback. `Break` stops delivery but does not undo that work. Bounded common query state excludes source bytes, indices, decompression buffers, SDK caches and caller output. `benchmark-methodology.md` explains how to separate these costs.

Reader optimizations must preserve the observations and error/stop rules in Rustdoc. Extra candidates and the physical decode order can change. The current explicit-time batch traverses the prefix in time order. It does not use the point-read shortcuts. A batch and repeated points can therefore have different costs and validation timing. Their successful observations must still agree.
