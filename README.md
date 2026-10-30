# ondas

A read-only Rust library for waveform analysis across VCD, FST, GHW, FSDB, SHM, WLF and WDB.

![Ondas architecture: readers supply declarations and observations to the common query engine.](docs/images/architecture.drawio.svg)

## Usage

```toml
[dependencies]
ondas = "=2.0.0"
```

Public contracts and runnable examples live in Rustdoc under `src/`. See the [released API reference](https://docs.rs/ondas) for the published crate. `waveform-model.md` explains the reasons for the common model.

For upgrades from 1.x, see the [2.0.0 migration notes](CHANGELOG.md#200---2026-10-30). The versioned Rustdoc has examples for [opening and reader limits](https://docs.rs/ondas/2.0.0/ondas/), [hierarchy](https://docs.rs/ondas/2.0.0/ondas/struct.Hierarchy.html), [values](https://docs.rs/ondas/2.0.0/ondas/enum.ValueRef.html), [queries](https://docs.rs/ondas/2.0.0/ondas/struct.Selection.html#method.query), [explicit times](https://docs.rs/ondas/2.0.0/ondas/struct.Selection.html#method.samples_at), [saved traces](https://docs.rs/ondas/2.0.0/ondas/struct.Trace.html#method.sample) and [errors](https://docs.rs/ondas/2.0.0/ondas/enum.Error.html).

## Cargo features

No Cargo features are enabled by default. VCD, FST and GHW are built in, also with `default-features = false`. Native readers use optional Cargo features with the same names as their reader IDs. They need a local SDK at build time and runtime.

```toml
[dependencies]
ondas = { version = "=2.0.0", features = ["fsdb-lib"] }
```

## Readers

| Reader | Format | Implementation | Design and limits |
| --- | --- | --- | --- |
| `vcd-native` | VCD | Built-in Rust parser | `vcd-reader.md` |
| `fst-lib` | FST | Published `fst-reader` decoder | `fst-reader.md` |
| `ghw-native` | GHW | Built-in Rust parser | `ghw-reader.md` |
| `fsdb-lib` | FSDB | Optional Verdi Reader SDK; file input | `fsdb-reader.md` |
| `shm-lib` | SHM | Optional Xcelium reader; directory input | `shm-reader.md` |
| `wlf-lib` | WLF | Optional Questa SDK; file input | `wlf-reader.md` |
| `wdb-lib` | WDB | Optional Vivado reader; file input | `wdb-reader.md` |

A native feature can make the executable depend on its SDK before Rust starts. This also affects applications that read another format. Each reader document explains its deployment constraints.

## Development

Contributor entry points are `dev` and `justfile`. `AGENTS.md` gives repository rules. `development-workflow.md` explains hooks, required checks and environment limits. `fixture-conformance.md` explains independent evidence. Benchmark interpretation belongs in `benchmark-methodology.md`.

Licensed under Apache License 2.0.
