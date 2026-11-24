# Repository tool boundaries

`dev` and `justfile` are the command entry points. `development-workflow.md` explains SDK selection. `fixture-conformance.md` explains fixture evidence. Tool arguments and check composition belong to their executable sources.

The standalone consumers launch outside Cargo's loader environment. A library test can pass while an ordinary downstream executable still cannot load its native dependencies. FSDB output assertions also distinguish successful compatible reads from newer-input startup skips. The WDB consumer always reads a compatible DBG004 recording; a version skip cannot satisfy this gate.

The automation tests check release metadata, SDK sweeps, and native transport behavior with test inputs. Reader conformance checks real SDK and waveform behavior.
