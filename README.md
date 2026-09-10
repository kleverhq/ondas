# ondas

> Under development. Expect changes to the code and API.

A read-only Rust library for waveform analysis. The goal is to support more
waveform formats than any other library, including proprietary formats, through
one API with consistent query semantics. FST and VCD readers are available today.

- Exact hierarchy paths and aliases, with declaration metadata kept intact.
- Samples, range queries and reusable signal selections in source ticks.
- Separate persistent state and event counts, with explicit missing values.
- Bit projections and streaming callbacks alongside owned query results.

## Development

Requires Linux, Git, Docker and the [Dev Container CLI](https://github.com/devcontainers/cli).
Rust 1.88 or newer is required; the container supplies the toolchain.

```sh
./dev --install-hooks
./dev just check-local
# Configure ONDAS_FIXTURES in .env before installation/full CI.
./dev just fixtures-install
./dev just ci
./dev just msrv
```

`check-local` needs no external fixtures. `ci` includes conformance;
`./dev just conformance` runs that suite on its own.

[API documentation](https://docs.rs/ondas) · [Development setup](docs/automation.md)
· [Library model](docs/model.md) · [Testing](docs/testing.md)

Licensed under Apache License 2.0.
