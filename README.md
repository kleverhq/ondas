# ondas

A read-only Rust library for waveform analysis. The goal is to support more
waveform formats than any other library, including proprietary formats, through
one API with consistent query semantics.

![Ondas architecture: backends open a Waveform with metadata and hierarchy; signal selections and backend observations feed the private query engine to produce public results.](docs/images/architecture.drawio.svg)

[API documentation](https://docs.rs/ondas) · [Library model](docs/model.md)

## Usage

```toml
[dependencies]
ondas = "1"
```

See the [API examples](https://docs.rs/ondas/latest/ondas/#read-a-signal) for
opening waveforms, resolving signals and querying values.

## Cargo features

No Cargo features are enabled by default. VCD, FST and GHW readers are always
included, even with `default-features = false`.

- `fsdb-lib`: adds the FSDB reader. Requires a local Verdi FSDB Reader SDK
  selected by `VERDI_HOME`; see [SDK requirements](docs/fsdb-lib.md#dependencies-and-deployment).
- `shm-lib`: adds the SHM directory reader. Requires a compatible Xcelium reader (2021+)
  selected by `XCELIUM_HOME`; see [reader requirements](docs/shm-lib.md#dependencies-and-deployment).

- `wlf-lib`: adds the file-only WLF reader. Requires a local Questa SDK selected
  by `QUESTA_HOME`; see [SDK requirements](docs/wlf.md#build-and-input).

```toml
[dependencies]
ondas = { version = "1", features = ["fsdb-lib"] }
```

## Backends

| Backend | Format | Implementation | Documentation |
|---|---|---|---|
| `vcd-native` | VCD | Built-in Rust parser; no external parser dependency | [docs/vcd-native.md](docs/vcd-native.md) |
| `fst-lib` | FST | `fst-reader` 0.17.0 | [docs/fst-lib.md](docs/fst-lib.md) |
| `ghw-native` | GHW | Built-in Rust parser; no external parser dependency | [docs/ghw.md](docs/ghw.md) |
| `fsdb-lib` | FSDB | Optional `fsdb-lib` feature; local Verdi FSDB Reader SDK, file-only | [docs/fsdb-lib.md](docs/fsdb-lib.md) |
| `shm-lib` | SHM | Optional `shm-lib` feature; local Xcelium reader, directory-only | [docs/shm-lib.md](docs/shm-lib.md) |
| `wlf-lib` | WLF | Optional `wlf-lib` feature; local Questa SDK, file-only | [docs/wlf.md](docs/wlf.md) |

## Development

Requires Linux, Git, Docker and the [Dev Container CLI](https://github.com/devcontainers/cli).
Rust 1.88 or newer is required; the container supplies the toolchain.

```sh
./dev --install-hooks
./dev just check-local
git submodule update --init fixtures
./dev just fixtures-install
./dev just ci
./dev just msrv
```

`check-local` needs no initialized submodule or waveform payloads. `ci` includes conformance;
`./dev just conformance` runs that suite on its own.

[Development setup](docs/automation.md) · [Testing](docs/testing.md)

Licensed under Apache License 2.0.
