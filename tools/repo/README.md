# Repository tool boundaries

`dev` and `justfile` are the command entry points. `development-workflow.md` explains installed hook snapshots and SDK selection; `fixture-conformance.md` explains fixture evidence. Tool arguments and check composition belong to their executable sources.

The standalone consumers launch outside Cargo's loader environment. A library test can pass while an ordinary downstream executable still cannot load its native dependencies. FSDB output assertions also distinguish successful compatible reads from newer-input startup skips.

The automation tests use temporary repositories and fake commands. They check host index/path dispatch and SDK sweep behavior without qualifying a native reader. Real SDK and waveform behavior belongs to reader conformance.
