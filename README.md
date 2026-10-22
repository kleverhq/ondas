# ondas

A read-only Rust library for waveform analysis across VCD, FST, GHW, FSDB, SHM and WLF.

Public contracts and runnable examples live in Rustdoc under `src/`. The [released API reference](https://docs.rs/ondas) describes the published crate; `waveform-model.md` explains the common model's design choices.

VCD, FST and GHW are built in even with `default-features = false`. Optional `fsdb-lib`, `shm-lib` and `wlf-lib` features add native SDK readers. Their deployment constraints are in `fsdb-reader.md`, `shm-reader.md` and `wlf-reader.md`. Enabling a native feature can make the executable depend on that SDK before Rust starts, including when reading another format.

Contributor entry points are `dev` and `justfile`. Follow `AGENTS.md` for required checks, `development-workflow.md` for worktree and environment pitfalls, and `fixture-conformance.md` for independent evidence. Benchmark interpretation belongs in `benchmark-methodology.md`.

Licensed under Apache License 2.0.
